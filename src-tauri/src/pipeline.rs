use crate::{asr, audio, config, insert, llm, secrets};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dictate,
    Command,
}

impl Mode {
    fn event_name(&self) -> &'static str {
        match self {
            Mode::Dictate => "dictate",
            Mode::Command => "command",
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase", tag = "phase", rename_all_fields = "camelCase")]
enum OverlayEvent {
    Listening { text: String, mode: &'static str },
    Processing { text: String, mode: &'static str },
    Result { text: String, mode: &'static str },
    Error { text: String, mode: &'static str },
    Hidden,
}

struct Active {
    id: u64,
    mode: Mode,
    audio: audio::AudioStopper,
    audio_tx: tokio::sync::mpsc::UnboundedSender<Vec<i16>>,
    partial_rx: watch::Receiver<String>,
    done_rx: oneshot::Receiver<Result<String, String>>,
}

/// 会话槽位：Starting 用于占住热键，避免异步建连期间重复触发
enum Slot {
    Idle,
    Starting(Mode),
    Active(Active),
}

pub struct Pipeline {
    slot: Mutex<Slot>,
    counter: AtomicU64,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(Slot::Idle),
            counter: AtomicU64::new(1),
        }
    }

    /// 热键入口：同一模式再按一次即结束，另一模式按下时提示忙碌
    pub fn toggle(&self, app: &AppHandle, mode: Mode) {
        let taken: Option<Active> = {
            let mut guard = self.slot.lock().unwrap();
            match *guard {
                Slot::Active(ref active) if active.mode == mode => {
                    match std::mem::replace(&mut *guard, Slot::Idle) {
                        Slot::Active(active) => Some(active),
                        _ => None,
                    }
                }
                _ => None,
            }
        };

        if let Some(active) = taken {
            tauri::async_runtime::spawn(finish_flow(app.clone(), active));
            return;
        }

        let mut guard = self.slot.lock().unwrap();
        match *guard {
            Slot::Idle => {
                *guard = Slot::Starting(mode);
                drop(guard);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_flow(&app, mode).await {
                        reset_to_idle(&app, mode);
                        emit_error(&app, &e, mode);
                    }
                });
            }
            Slot::Starting(_) | Slot::Active(_) => {
                drop(guard);
                emit(&app, OverlayEvent::Error {
                    text: "正在录音中，请先按当前热键结束".into(),
                    mode: mode.event_name(),
                });
                show_overlay(app);
            }
        }
    }
}

fn reset_to_idle(app: &AppHandle, _mode: Mode) {
    let pipeline = app.state::<Pipeline>();
    let mut guard = pipeline.slot.lock().unwrap();
    if let Slot::Starting(_) = *guard {
        *guard = Slot::Idle;
    }
}

async fn start_flow(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let cfg = config::load(&app);
    let key = secrets::get_api_key().map_err(|_| {
        "尚未配置百炼 API Key，请从托盘菜单打开设置完成配置".to_string()
    })?;

    emit(app, OverlayEvent::Listening {
        text: String::new(),
        mode: mode.event_name(),
    });
    show_overlay(app);

    let session = asr::start(&cfg.asr_model, &key, &cfg.language_hints).await?;

    let audio_stopper = audio::spawn(session.audio_tx.clone())?;

    // 实时识别文本转发到字幕窗口，会话结束后通道关闭自然退出
    {
        let app = app.clone();
        let mut partial_rx = session.partial_rx.clone();
        let mode_name = mode.event_name();
        tauri::async_runtime::spawn(async move {
            loop {
                let text = partial_rx.borrow_and_update().clone();
                emit(&app, OverlayEvent::Listening { text, mode: mode_name });
                if partial_rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    let id = app
        .state::<Pipeline>()
        .counter
        .fetch_add(1, Ordering::SeqCst);
    let max_seconds = cfg.max_recording_seconds;
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(max_seconds)).await;
            let pipeline = app.state::<Pipeline>();
            let mut guard = pipeline.slot.lock().unwrap();
            let is_current = match *guard {
                Slot::Active(ref a) => a.id == id,
                _ => false,
            };
            if is_current {
                match std::mem::replace(&mut *guard, Slot::Idle) {
                    Slot::Active(active) => {
                        drop(guard);
                        tauri::async_runtime::spawn(finish_flow(app, active));
                    }
                    _ => {}
                }
            }
        });
    }

    {
        let pipeline = app.state::<Pipeline>();
        let mut guard = pipeline.slot.lock().unwrap();
        if let Slot::Starting(m) = *guard {
            if m == mode {
                *guard = Slot::Active(Active {
                    id,
                    mode,
                    audio: audio_stopper,
                    audio_tx: session.audio_tx,
                    partial_rx: session.partial_rx,
                    done_rx: session.done_rx,
                });
            }
        }
    }

    Ok(())
}

async fn finish_flow(app: AppHandle, mut active: Active) {
    active.audio.stop();
    // drop 音频输入端，ASR 任务据此发送 finish-task 并收尾
    let done_rx = std::mem::replace(
        &mut active.done_rx,
        oneshot::channel().1,
    );
    drop(active.audio_tx);
    let mode = active.mode;
    let last_partial = active.partial_rx.borrow().clone();

    emit(&app, OverlayEvent::Processing {
        text: last_partial,
        mode: mode.event_name(),
    });

    let transcript = match done_rx.await {
        Ok(Ok(text)) => text.trim().to_string(),
        Ok(Err(e)) => {
            emit_error(&app, &e, mode);
            return;
        }
        Err(_) => {
            emit_error(&app, "识别会话异常终止", mode);
            return;
        }
    };

    if transcript.is_empty() {
        emit(&app, OverlayEvent::Result {
            text: "（没有听到内容）".into(),
            mode: mode.event_name(),
        });
        hide_overlay_later(&app);
        return;
    }

    match mode {
        Mode::Dictate => {
            let cfg = config::load(&app);
            let final_text = match cfg.polish_mode {
                config::PolishMode::Raw => transcript.clone(),
                config::PolishMode::Polished => {
                    let key = match secrets::get_api_key() {
                        Ok(k) => k,
                        Err(e) => {
                            emit_error(&app, &e, mode);
                            return;
                        }
                    };
                    match llm::polish(&key, &cfg.llm_model, &transcript).await {
                        Ok(polished) if !polished.is_empty() => polished,
                        Ok(_) => transcript.clone(),
                        Err(e) => {
                            // 润色失败不阻断输入，降级为插入原文
                            eprintln!("[pipeline] 润色失败，插入原文: {e}");
                            transcript.clone()
                        }
                    }
                }
            };
            if let Err(e) = insert::insert_text(&final_text) {
                emit_error(&app, &e, mode);
                return;
            }
            emit(&app, OverlayEvent::Result {
                text: final_text,
                mode: mode.event_name(),
            });
        }
        Mode::Command => {
            let mut cfg = config::load(&app);
            let key = match secrets::get_api_key() {
                Ok(k) => k,
                Err(e) => {
                    emit_error(&app, &e, mode);
                    return;
                }
            };
            match llm::route_command(&key, &cfg.llm_model, &transcript, cfg.polish_mode).await {
                Ok(llm::CommandOutcome::ModeSwitched { mode: new_mode, reply }) => {
                    cfg.polish_mode = new_mode;
                    config::save(&app, &cfg);
                    emit(&app, OverlayEvent::Result { text: reply, mode: mode.event_name() });
                }
                Ok(llm::CommandOutcome::Replied(msg)) => {
                    let text = if msg.is_empty() { transcript.clone() } else { msg };
                    emit(&app, OverlayEvent::Result { text, mode: mode.event_name() });
                }
                Err(e) => {
                    emit_error(&app, &e, mode);
                    return;
                }
            }
        }
    }

    hide_overlay_later(&app);
}

fn emit(app: &AppHandle, event: OverlayEvent) {
    if let Err(e) = app.emit("overlay://state", &event) {
        eprintln!("[pipeline] 发送字幕事件失败: {e}");
    }
}

fn emit_error(app: &AppHandle, message: &str, mode: Mode) {
    show_overlay(app);
    emit(
        app,
        OverlayEvent::Error {
            text: message.to_string(),
            mode: mode.event_name(),
        },
    );
    hide_overlay_later(app);
}

/// 结果展示两秒后隐藏字幕窗口
fn hide_overlay_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Some(win) = app.get_webview_window("overlay") {
            let _ = win.hide();
        }
        emit(&app, OverlayEvent::Hidden);
    });
}

fn show_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        if let Ok(Some(monitor)) = win.primary_monitor() {
            let size = monitor.size();
            let scale = monitor.scale_factor();
            let win_w = (780.0 * scale) as i32;
            let win_h = (150.0 * scale) as i32;
            let x = (size.width as i32 - win_w) / 2;
            let y = size.height as i32 - win_h - (96.0 * scale) as i32;
            let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let _ = win.show();
        let _ = win.set_always_on_top(true);
    }
}
