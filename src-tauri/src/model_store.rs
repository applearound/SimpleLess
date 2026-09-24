//! 本地识别模型管理：就绪状态探测、流式下载与 sha256 校验、删除清理。
//! 下载进度与终态经 Tauri 事件推送给设置页。

use crate::{asr_local, config};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

/// 进度事件名：负载 { downloaded, total }，单位字节
pub const EVENT_PROGRESS: &str = "local-model-progress";
/// 终态事件名：负载 { state, error? }
pub const EVENT_STATE: &str = "local-model-state";

struct ModelFileSpec {
    name: &'static str,
    urls: &'static [&'static str],
    sha256: &'static str,
}

/// 下载源按顺序回落：hf-mirror 境内直连最快，HuggingFace 与 GitHub 备选
const FILES: &[ModelFileSpec] = &[
    ModelFileSpec {
        name: asr_local::TOKENS_FILE,
        urls: &[
            "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
            "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
        ],
        sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
    },
    ModelFileSpec {
        name: asr_local::VAD_FILE,
        urls: &[
            "https://hf-mirror.com/csukuangfj/vad/resolve/main/silero_vad.onnx",
            "https://huggingface.co/csukuangfj/vad/resolve/main/silero_vad.onnx",
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
        ],
        sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
    },
    ModelFileSpec {
        name: asr_local::MODEL_FILE,
        urls: &[
            "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
            "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
        ],
        sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum LocalModelState {
    NotDownloaded,
    Downloading { downloaded: u64, total: u64 },
    Failed { error: String },
    Ready,
}

/// 进行中下载任务的控制块，供状态查询与取消共用
#[derive(Clone)]
struct DownloadControl {
    cancel: Arc<AtomicBool>,
    /// (downloaded, total)，total 在拿到全部 Content-Length 后固定
    progress: Arc<Mutex<(u64, u64)>>,
    error: Arc<Mutex<Option<String>>>,
}

fn active() -> &'static Mutex<Option<DownloadControl>> {
    static ACTIVE: OnceLock<Mutex<Option<DownloadControl>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(None))
}

/// 当前模型状态：下载中优先报进度，其次按文件是否齐备判定
pub fn model_state(app: &AppHandle) -> LocalModelState {
    let guard = active().lock().unwrap();
    if let Some(control) = guard.as_ref() {
        if let Some(err) = control.error.lock().unwrap().clone() {
            return LocalModelState::Failed { error: err };
        }
        let (downloaded, total) = *control.progress.lock().unwrap();
        return LocalModelState::Downloading { downloaded, total };
    }
    drop(guard);
    if asr_local::model_ready(&config::local_model_dir(app)) {
        LocalModelState::Ready
    } else {
        LocalModelState::NotDownloaded
    }
}

#[tauri::command]
pub fn get_local_model_status(app: AppHandle) -> Result<LocalModelState, String> {
    Ok(model_state(&app))
}

#[tauri::command]
pub fn download_local_model(app: AppHandle) -> Result<(), String> {
    let mut guard = active().lock().unwrap();
    if guard.is_some() {
        return Err("模型正在下载中".into());
    }
    let control = DownloadControl {
        cancel: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(Mutex::new((0, 0))),
        error: Arc::new(Mutex::new(None)),
    };
    *guard = Some(control.clone());
    drop(guard);

    tauri::async_runtime::spawn(async move {
        let outcome = run_download(&app, &control).await;
        // 无论成败，任务结束后释放槽位并广播终态
        *active().lock().unwrap() = None;
        let (state, error) = match &outcome {
            Ok(()) => ("ready", None),
            Err(e) => (if e == "已取消" { "cancelled" } else { "failed" }, Some(e.clone())),
        };
        let _ = app.emit(
            EVENT_STATE,
            &serde_json::json!({ "state": state, "error": error }),
        );
        if let Err(e) = outcome {
            eprintln!("[model_store] 下载失败: {e}");
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_local_model_download() -> Result<(), String> {
    let guard = active().lock().unwrap();
    match guard.as_ref() {
        Some(control) => {
            control.cancel.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err("当前没有进行中的下载".into()),
    }
}

#[tauri::command]
pub fn delete_local_model(app: AppHandle) -> Result<(), String> {
    let guard = active().lock().unwrap();
    if guard.is_some() {
        return Err("模型正在下载中，请先取消".into());
    }
    drop(guard);
    let dir = config::local_model_dir(&app);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除模型目录失败: {e}")),
    }
}

#[tauri::command]
pub fn set_asr_engine(app: AppHandle, engine: String) -> Result<(), String> {
    let engine = match engine.as_str() {
        "cloud" => config::AsrEngine::Cloud,
        "local" => {
            if !asr_local::model_ready(&config::local_model_dir(&app)) {
                return Err("本地模型尚未就绪，请先下载".into());
            }
            config::AsrEngine::Local
        }
        other => return Err(format!("未知的识别引擎: {other}")),
    };
    let mut cfg = config::load(&app);
    cfg.asr_engine = engine;
    config::save(&app, &cfg);
    Ok(())
}

async fn run_download(app: &AppHandle, control: &DownloadControl) -> Result<(), String> {
    let dir = config::local_model_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模型目录失败: {e}"))?;

    // 清理上次可能残留的半成品
    for spec in FILES {
        let _ = std::fs::remove_file(part_path(&dir, spec.name));
    }

    let client = reqwest::Client::new();
    let total = fetch_total_sizes(&client).await?;
    *control.progress.lock().unwrap() = (0, total);

    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    for spec in FILES {
        download_one(&client, spec, &dir, control, total, &mut downloaded, &mut last_emit, app)
            .await?;
    }
    Ok(())
}

/// 预取全部文件大小用于进度条；个别源拿不到 Content-Length 时按零计入，
/// 进度条仍可用已下载字节数驱动
async fn fetch_total_sizes(client: &reqwest::Client) -> Result<u64, String> {
    let mut total = 0u64;
    for spec in FILES {
        for url in spec.urls {
            match client.head(*url).send().await {
                Ok(resp) => {
                    if let Some(len) = resp.content_length() {
                        total += len;
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
async fn download_one(
    client: &reqwest::Client,
    spec: &ModelFileSpec,
    dir: &Path,
    control: &DownloadControl,
    total: u64,
    downloaded: &mut u64,
    last_emit: &mut u64,
    app: &AppHandle,
) -> Result<(), String> {
    let part = part_path(dir, spec.name);
    let final_path = dir.join(spec.name);
    if final_path.is_file() {
        // 已就绪的文件跳过，重试时无需重复下载
        let len = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
        *downloaded += len;
        return Ok(());
    }

    let mut last_err = String::new();
    for url in spec.urls {
        match download_from(client, url, &part, spec, control, total, downloaded, last_emit, app)
            .await
        {
            Ok(()) => {
                std::fs::rename(&part, &final_path)
                    .map_err(|e| format!("落盘 {name} 失败: {e}", name = spec.name))?;
                return Ok(());
            }
            Err(e) => {
                if control.cancel.load(Ordering::SeqCst) {
                    let _ = std::fs::remove_file(&part);
                    return Err("已取消".into());
                }
                let _ = std::fs::remove_file(&part);
                last_err = e;
            }
        }
    }
    Err(format!(
        "下载 {name} 失败，已尝试全部下载源: {last_err}",
        name = spec.name
    ))
}

#[allow(clippy::too_many_arguments)]
async fn download_from(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    spec: &ModelFileSpec,
    control: &DownloadControl,
    total: u64,
    downloaded: &mut u64,
    last_emit: &mut u64,
    app: &AppHandle,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("连接失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("服务端返回 {}", resp.status()));
    }

    use futures_util::StreamExt;
    let mut file = std::fs::File::create(part).map_err(|e| format!("创建临时文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut stream = resp.bytes_stream();
    use std::io::Write;
    while let Some(chunk) = stream.next().await {
        if control.cancel.load(Ordering::SeqCst) {
            return Err("已取消".into());
        }
        let chunk = chunk.map_err(|e| format!("传输中断: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {e}"))?;
        *downloaded += chunk.len() as u64;
        *control.progress.lock().unwrap() = (*downloaded, total);
        // 节流推送：每 512KB 或收尾时发一次，避免事件风暴
        if *downloaded - *last_emit >= 512 * 1024 {
            *last_emit = *downloaded;
            let _ = app.emit(EVENT_PROGRESS, &serde_json::json!({ "downloaded": downloaded, "total": total }));
        }
    }
    let actual = format!("{:x}", hasher.finalize());
    if !spec.sha256.is_empty() && actual != spec.sha256 {
        return Err("文件校验失败，可能传输损坏".into());
    }
    Ok(())
}

fn part_path(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(format!("{name}.part"))
}
