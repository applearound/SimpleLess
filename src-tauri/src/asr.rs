use futures_util::{SinkExt, StreamExt};
use tokio::sync::{oneshot, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// 百炼实时语音识别的 WebSocket 入口，旧域名仍可正常使用
const WS_URL: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";

pub struct AsrSession {
    /// 音频输入端，16kHz 单声道 i16 样本；所有克隆体 drop 后触发结束流程
    pub audio_tx: tokio::sync::mpsc::UnboundedSender<Vec<i16>>,
    /// 实时识别全文（已定稿句 + 当前句）
    pub partial_rx: watch::Receiver<String>,
    /// 任务结束信号，携带最终全文或错误信息
    pub done_rx: oneshot::Receiver<Result<String, String>>,
}

/// 建立一条实时识别会话，连接失败立即返回错误
pub async fn start(model: &str, key: &str, language_hints: &[String]) -> Result<AsrSession, String> {
    let mut request = WS_URL
        .into_client_request()
        .map_err(|e| format!("构造 ASR 请求失败: {e}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| "无效的 API Key".to_string())?,
    );

    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("连接百炼语音识别服务失败: {e}"))?;

    let task_id = uuid::Uuid::new_v4().to_string();
    let run_task = serde_json::json!({
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
    });

    let (audio_tx, mut audio_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
    let (partial_tx, partial_rx) = watch::channel(String::new());
    let (done_tx, done_rx) = oneshot::channel();

    tauri::async_runtime::spawn(async move {
        let mut partial_tx = partial_tx;
        let (mut write, mut read) = ws.split();

        if write.send(Message::Text(run_task.to_string())).await.is_err() {
            let _ = done_tx.send(Err("发送识别任务指令失败".into()));
            return;
        }

        // 等待 task-started 后才允许发送音频
        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Some(event) = event_name(&text) {
                        match event.as_str() {
                            "task-started" => break,
                            "task-failed" => {
                                let _ = done_tx.send(Err(task_error(&text)));
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    let _ = done_tx.send(Err(format!("识别连接中断: {e}")));
                    return;
                }
                None => {
                    let _ = done_tx.send(Err("识别连接被服务端关闭".into()));
                    return;
                }
            }
        }

        let mut finalized = String::new();
        #[allow(unused_assignments)]
        let mut outcome: Option<Result<String, String>> = None;
        let mut finish_sent = false;

        // 主循环：音频端存活时边发边收；全部 drop 后发 finish-task 并只收不发
        loop {
            if !finish_sent {
                tokio::select! {
                    chunk = audio_rx.recv() => {
                        match chunk {
                            Some(samples) => {
                                let mut bytes = Vec::with_capacity(samples.len() * 2);
                                for s in samples {
                                    bytes.extend_from_slice(&s.to_le_bytes());
                                }
                                if write.send(Message::Binary(bytes.into())).await.is_err() {
                                    outcome = Some(Err("发送音频失败，识别连接中断".into()));
                                    break;
                                }
                            }
                            None => {
                                let finish = serde_json::json!({
                                    "header": {
                                        "action": "finish-task",
                                        "task_id": task_id,
                                        "streaming": "duplex"
                                    },
                                    "payload": { "input": {} }
                                });
                                if write.send(Message::Text(finish.to_string())).await.is_err() {
                                    outcome = Some(Err("发送结束指令失败".into()));
                                    break;
                                }
                                finish_sent = true;
                            }
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Some(o) = consume_event(&text, &mut finalized, &mut partial_tx) {
                                    outcome = Some(o);
                                    break;
                                }
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                outcome = Some(Err(format!("识别连接中断: {e}")));
                                break;
                            }
                            None => {
                                outcome = Some(Err("识别连接被服务端关闭".into()));
                                break;
                            }
                        }
                    }
                }
            } else {
                // 收尾阶段：只读服务端事件直到 task-finished / task-failed
                match read.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(o) = consume_event(&text, &mut finalized, &mut partial_tx) {
                            outcome = Some(o);
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        outcome = Some(Err(format!("识别连接中断: {e}")));
                        break;
                    }
                    None => {
                        outcome = Some(Err("识别连接在任务结束前关闭".into()));
                        break;
                    }
                }
            }
        }

        match outcome {
            Some(Ok(_)) => {
                let _ = done_tx.send(Ok(finalized));
            }
            Some(Err(e)) => {
                let _ = done_tx.send(Err(e));
            }
            None => {
                let _ = done_tx.send(Err("识别会话异常结束".into()));
            }
        }
    });

    Ok(AsrSession {
        audio_tx,
        partial_rx,
        done_rx,
    })
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
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
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
            }
            None
        }
        "task-finished" => Some(Ok(finalized.clone())),
        "task-failed" => Some(Err(task_error(text))),
        _ => None,
    }
}

fn event_name(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()?
        ["header"]["event"]
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
