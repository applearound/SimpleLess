//! 百炼实时语音识别客户端：通过 WebSocket 建立双工会话，
//! 发送 16kHz PCM 音频并实时产出识别文本，结束时返回全文。

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{oneshot, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// 百炼实时语音识别的 WebSocket 入口，旧域名仍可正常使用
const WS_URL: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWriter = SplitSink<Ws, Message>;
type WsReader = SplitStream<Ws>;

pub struct AsrSession {
    /// 音频输入端，16kHz 单声道 i16 样本；所有克隆体 drop 后触发结束流程
    pub audio_tx: mpsc::UnboundedSender<Vec<i16>>,
    /// 实时识别全文（已定稿句 + 当前句）
    pub partial_rx: watch::Receiver<String>,
    /// 任务结束信号，携带最终全文或错误信息
    pub done_rx: oneshot::Receiver<Result<String, String>>,
}

/// 建立一条实时识别会话，连接失败立即返回错误。
/// 音频收发与事件处理交给后台任务，经 AsrSession 的通道交互。
pub async fn start(
    model: &str,
    key: &str,
    language_hints: &[String],
) -> Result<AsrSession, String> {
    let ws = connect(key).await?;
    let (audio_tx, audio_rx) = mpsc::unbounded_channel::<Vec<i16>>();
    let (partial_tx, partial_rx) = watch::channel(String::new());
    let (done_tx, done_rx) = oneshot::channel();

    tauri::async_runtime::spawn(run_session(SessionRun {
        ws,
        task_id: uuid::Uuid::new_v4().to_string(),
        model: model.to_string(),
        language_hints: language_hints.to_vec(),
        audio_rx,
        partial_tx,
        done_tx,
    }));

    Ok(AsrSession {
        audio_tx,
        partial_rx,
        done_rx,
    })
}

/// 后台任务的完整输入：连接参数与三条回传通道
struct SessionRun {
    ws: Ws,
    task_id: String,
    model: String,
    language_hints: Vec<String>,
    audio_rx: UnboundedReceiver<Vec<i16>>,
    partial_tx: watch::Sender<String>,
    done_tx: oneshot::Sender<Result<String, String>>,
}

/// 建立 WebSocket 连接，Authorization 携带 Bearer Key
async fn connect(key: &str) -> Result<Ws, String> {
    let mut request = WS_URL
        .into_client_request()
        .map_err(|e| format!("构造 ASR 请求失败: {e}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| "无效的 API Key".to_string())?,
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(ws, _)| ws)
        .map_err(|e| format!("连接百炼语音识别服务失败: {e}"))
}

/// 会话后台任务：握手后进入交换循环，结束时回传全文或错误
async fn run_session(run: SessionRun) {
    let SessionRun {
        ws,
        task_id,
        model,
        language_hints,
        mut audio_rx,
        mut partial_tx,
        done_tx,
    } = run;
    let (mut write, mut read) = ws.split();

    if let Err(e) = start_task(&mut write, &mut read, &task_id, &model, &language_hints).await {
        let _ = done_tx.send(Err(e));
        return;
    }

    let mut finalized = String::new();
    let outcome = exchange_loop(
        &mut write,
        &mut read,
        &mut audio_rx,
        &task_id,
        &mut finalized,
        &mut partial_tx,
    )
    .await;
    let _ = done_tx.send(outcome);
}

/// 发送 run-task 并等待 task-started，之后才允许发送音频
async fn start_task(
    write: &mut WsWriter,
    read: &mut WsReader,
    task_id: &str,
    model: &str,
    language_hints: &[String],
) -> Result<(), String> {
    let run_task = run_task_message(task_id, model, language_hints);
    write
        .send(Message::Text(run_task.to_string()))
        .await
        .map_err(|_| "发送识别任务指令失败".to_string())?;

    loop {
        match read.next().await {
            Some(Ok(Message::Text(text))) => match event_name(&text).as_deref() {
                Some("task-started") => return Ok(()),
                Some("task-failed") => return Err(task_error(&text)),
                _ => {}
            },
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(format!("识别连接中断: {e}")),
            None => return Err("识别连接被服务端关闭".into()),
        }
    }
}

/// 主交换循环：音频端存活时边发边收；全部 drop 后发 finish-task
/// 并只收不发，收到终结事件时返回全文或错误
async fn exchange_loop(
    write: &mut WsWriter,
    read: &mut WsReader,
    audio_rx: &mut UnboundedReceiver<Vec<i16>>,
    task_id: &str,
    finalized: &mut String,
    partial_tx: &mut watch::Sender<String>,
) -> Result<String, String> {
    loop {
        tokio::select! {
            chunk = audio_rx.recv() => match chunk {
                Some(samples) => send_audio(write, &samples).await?,
                None => break,
            },
            msg = read.next() => {
                if let Some(o) = read_event(msg, finalized, partial_tx) {
                    return o;
                }
            }
        }
    }

    let finish = finish_task_message(task_id);
    write
        .send(Message::Text(finish.to_string()))
        .await
        .map_err(|_| "发送结束指令失败".to_string())?;

    // 收尾阶段：只读服务端事件直到 task-finished / task-failed
    loop {
        if let Some(o) = read_event(read.next().await, finalized, partial_tx) {
            return o;
        }
    }
}

/// 发送一包音频样本
async fn send_audio(write: &mut WsWriter, samples: &[i16]) -> Result<(), String> {
    write
        .send(Message::Binary(pcm_bytes(samples)))
        .await
        .map_err(|_| "发送音频失败，识别连接中断".to_string())
}

/// 处理一条服务端消息：返回 Some 表示会话已终结
fn read_event(
    msg: Option<Result<Message, Error>>,
    finalized: &mut String,
    partial_tx: &mut watch::Sender<String>,
) -> Option<Result<String, String>> {
    match msg {
        Some(Ok(Message::Text(text))) => consume_event(&text, finalized, partial_tx),
        Some(Ok(_)) => None,
        Some(Err(e)) => Some(Err(format!("识别连接中断: {e}"))),
        None => Some(Err("识别连接被服务端关闭".into())),
    }
}

fn run_task_message(task_id: &str, model: &str, language_hints: &[String]) -> serde_json::Value {
    serde_json::json!({
        "header": {
            "action": "run-task",
            "task_id": task_id,
            "streaming": "duplex"
        },
        "payload": {
            "task_group": "audio",
            "task": "asr",
            "function": "recognition",
            "model": model,
            "parameters": {
                "format": "pcm",
                "sample_rate": 16000,
                "language_hints": language_hints
            },
            "input": {}
        }
    })
}

fn finish_task_message(task_id: &str) -> serde_json::Value {
    serde_json::json!({
        "header": {
            "action": "finish-task",
            "task_id": task_id,
            "streaming": "duplex"
        },
        "payload": { "input": {} }
    })
}

fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}

/// 解析服务端事件：返回 Some 表示任务终结
fn consume_event(
    text: &str,
    finalized: &mut String,
    partial_tx: &mut watch::Sender<String>,
) -> Option<Result<String, String>> {
    let event = event_name(text)?;
    match event.as_str() {
        "result-generated" => {
            let json_result = serde_json::from_str::<serde_json::Value>(text);

            if json_result.is_err() {
                return None;
            }

            let v = json_result.unwrap();

            let sentence = &v["payload"]["output"]["sentence"];
            let sentence_text = sentence["text"].as_str().unwrap_or("");
            let is_end = sentence["sentence_end"].as_bool().unwrap_or(false);
            let heartbeat = sentence["heartbeat"].as_bool().unwrap_or(false);
            if !heartbeat && !sentence_text.is_empty() {
                if is_end {
                    finalized.push_str(sentence_text);
                }
                let display = if is_end {
                    finalized.clone()
                } else {
                    format!("{finalized}{sentence_text}")
                };
                let _ = partial_tx.send(display);
            }

            None
        }
        "task-finished" => Some(Ok(finalized.clone())),
        "task-failed" => Some(Err(task_error(text))),
        _ => None,
    }
}

fn event_name(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text).ok()?["header"]["event"]
        .as_str()
        .map(|s| s.to_string())
}

fn task_error(text: &str) -> String {
    let v = serde_json::from_str::<serde_json::Value>(text).ok();
    let message = v
        .as_ref()
        .and_then(|v| v["header"]["error_message"].as_str())
        .unwrap_or("语音识别任务失败");
    format!("语音识别失败: {message}")
}
