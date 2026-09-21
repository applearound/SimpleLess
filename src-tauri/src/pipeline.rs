//! 录音会话状态机：响应热键开始与结束录音，串联采集、识别、
//! 润色或命令路由，并向字幕窗口推送状态事件。

use crate::{asr, audio, config, insert, llm, secrets};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
    Finalizing { text: String, mode: &'static str },
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

/// 收尾阶段：等待 ASR 定稿与润色返回，持有取消令牌
struct Polishing {
    id: u64,
    mode: Mode,
    cancel: Arc<tokio::sync::Notify>,
}

/// 会话状态机：Idle → Starting → Active → Finalizing → Polishing → Outputting → Idle，
/// 每次转换都在 slot 锁内完成，保证原子性；润色与取消靠 select! 竞争，先到先得。
/// Finalizing 为收尾态：已停止收音，等待 ASR 把已送入的音频全部识别完，
/// 期间迟到的转写文本继续上屏，但不接受新语音
enum Slot {
    Idle,
    Starting(Mode),
    Active(Active),
    Finalizing { id: u64, mode: Mode },
    Polishing(Polishing),
    Outputting,
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

/// 取消当前润色：原子置位取消令牌，迟到的润色结果将被丢弃并按原文输出
pub fn cancel_active_polish(&self) -> bool {
    let guard = self.slot.lock().unwrap();
    match *guard {
        Slot::Polishing(ref p) => {
            p.cancel.notify_one();
            true
        }
        _ => false,
    }
}

    /// 热键入口：同一模式再按一次即停止收音并进入收尾态，另一模式按下时提示忙碌
    pub fn toggle(&self, app: &AppHandle, mode: Mode) {
        let taken: Option<Active> = {
            let mut guard = self.slot.lock().unwrap();
            match *guard {
                Slot::Active(ref active) if active.mode == mode => {
                    let (fid, fmode) = (active.id, active.mode);
                    match std::mem::replace(&mut *guard, Slot::Finalizing { id: fid, mode: fmode })
                    {
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
            Slot::Active(_) => {
                drop(guard);
                emit(&app, OverlayEvent::Error {
                    text: "正在录音中，请先按当前热键结束".into(),
                    mode: mode.event_name(),
                });
                show_overlay(app);
            }
            Slot::Finalizing { .. } => {
                drop(guard);
                emit(&app, OverlayEvent::Error {
                    text: "正在等待识别收尾，请稍候".into(),
                    mode: mode.event_name(),
                });
                show_overlay(app);
            }
            Slot::Polishing(ref p) => {
                let text = if p.mode == Mode::Dictate {
                    "正在润色处理中，可点字幕条上的取消提前结束"
                } else {
                    "正在处理中，请稍候"
                };
                drop(guard);
                emit(&app, OverlayEvent::Error { text: text.into(), mode: mode.event_name() });
                show_overlay(app);
            }
            Slot::Starting(_) | Slot::Outputting => {
                drop(guard);
                emit(&app, OverlayEvent::Error {
                    text: "正在处理中，请稍候".into(),
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

/// 该 id 的会话是否仍在录音中，倒计时任务据此判断自己是否已作废
fn session_active(app: &AppHandle, id: u64) -> bool {
    let pipeline = app.state::<Pipeline>();
    let guard = pipeline.slot.lock().unwrap();
    matches!(&*guard, Slot::Active(a) if a.id == id)
}

/// 录音态 → 收尾态：槽位锁内原子转换（仅限该 id 的会话）
fn begin_finalizing(app: &AppHandle, id: u64) -> Option<Active> {
    let pipeline = app.state::<Pipeline>();
    let mut guard = pipeline.slot.lock().unwrap();
    let mode = match *guard {
        Slot::Active(ref a) if a.id == id => a.mode,
        _ => return None,
    };
    match std::mem::replace(&mut *guard, Slot::Finalizing { id, mode }) {
        Slot::Active(active) => Some(active),
        _ => None,
    }
}

/// 收尾态 → 润色态：识别已排空，创建取消令牌
fn begin_polishing(app: &AppHandle, id: u64) -> Option<Arc<tokio::sync::Notify>> {
    let pipeline = app.state::<Pipeline>();
    let mut guard = pipeline.slot.lock().unwrap();
    let (pid, pmode) = match *guard {
        Slot::Finalizing { id: fid, mode: fmode } if fid == id => (fid, fmode),
        _ => return None,
    };
    let cancel = Arc::new(tokio::sync::Notify::new());
    *guard = Slot::Polishing(Polishing {
        id: pid,
        mode: pmode,
        cancel: Arc::clone(&cancel),
    });
    Some(cancel)
}

/// 润色态 → 输出态
fn enter_output(app: &AppHandle, id: u64) -> bool {
    let pipeline = app.state::<Pipeline>();
    let mut guard = pipeline.slot.lock().unwrap();
    if matches!(&*guard, Slot::Polishing(p) if p.id == id) {
        *guard = Slot::Outputting;
        true
    } else {
        false
    }
}

/// 任意收尾态 → Idle，只允许当前会话自己释放
fn release(app: &AppHandle, id: u64) {
    let pipeline = app.state::<Pipeline>();
    let mut guard = pipeline.slot.lock().unwrap();
    let ours = match *guard {
        Slot::Finalizing { id: fid, .. } => fid == id,
        Slot::Polishing(ref p) => p.id == id,
        Slot::Outputting => true,
        _ => false,
    };
    if ours {
        *guard = Slot::Idle;
    }
}

async fn start_flow(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let cfg = config::load(&app);
    let key = secrets::get_api_key().map_err(|e| {
        format!("读取 API Key 失败: {e}。请从托盘菜单打开设置重新保存")
    })?;

    emit(app, OverlayEvent::Listening {
        text: String::new(),
        mode: mode.event_name(),
    });
    show_overlay(app);

    let session = asr::start(&cfg.asr_model, &key, &cfg.language_hints).await?;

    let audio_stopper = audio::spawn(session.audio_tx.clone())?;

    // 新会话开始，清掉上一轮可能残留的倒计时提示
    let _ = app.emit("overlay://countdown", None::<u64>);

    let id = app
        .state::<Pipeline>()
        .counter
        .fetch_add(1, Ordering::SeqCst);
    let max_seconds = cfg.max_recording_seconds;
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // 全程倒计时：字幕条实时显示剩余秒数；会话提前结束则本任务作废，
            // 不再向新会话发送旧倒计时
            for remaining in (1..=max_seconds).rev() {
                if !session_active(&app, id) {
                    return;
                }
                let _ = app.emit("overlay://countdown", Some(remaining));
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            if !session_active(&app, id) {
                return;
            }
            if let Some(active) = begin_finalizing(&app, id) {
                tauri::async_runtime::spawn(finish_flow(app, active));
            }
        });
    }

    // 先克隆一份转写接收端给转发任务，原件随后移入 Active
    let partial_rx_fwd = session.partial_rx.clone();
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

    // 槽位已入 Active，实时转写转发随后启动；事件类型随状态机阶段变化：
    // 录音中发监听态，收尾中发收尾态（文字继续刷新但显示为处理样式）
    {
        let app = app.clone();
        let mut partial_rx = partial_rx_fwd;
        let mode_name = mode.event_name();
        tauri::async_runtime::spawn(async move {
            loop {
                let text = partial_rx.borrow_and_update().clone();
                let event = match *app.state::<Pipeline>().slot.lock().unwrap() {
                    Slot::Active(_) => OverlayEvent::Listening { text, mode: mode_name },
                    Slot::Finalizing { .. } => {
                        OverlayEvent::Finalizing { text, mode: mode_name }
                    }
                    _ => break,
                };
                emit(&app, event);
                if partial_rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    Ok(())
}

/// 收尾状态机：从收尾态推进到润色、输出直至 Idle，所有退出路径都先释放槽位
async fn finish_flow(app: AppHandle, mut active: Active) {
    active.audio.stop();
    // drop 音频输入端，ASR 任务据此发送 finish-task 并收尾
    let done_rx = std::mem::replace(&mut active.done_rx, oneshot::channel().1);
    drop(active.audio_tx);
    let mode = active.mode;
    let id = active.id;
    let last_partial = active.partial_rx.borrow().clone();

    // 停止收音后进入识别收尾：文本继续刷新，界面显示为识别阶段
    emit(&app, OverlayEvent::Finalizing {
        text: last_partial,
        mode: mode.event_name(),
    });

    let transcript = match done_rx.await {
        Ok(Ok(text)) => text.trim().to_string(),
        Ok(Err(e)) => {
            release(&app, id);
            emit_error(&app, &e, mode);
            return;
        }
        Err(_) => {
            release(&app, id);
            emit_error(&app, "识别会话异常终止", mode);
            return;
        }
    };

    if transcript.is_empty() {
        release(&app, id);
        emit(&app, OverlayEvent::Result {
            text: "（没有听到内容）".into(),
            mode: mode.event_name(),
        });
        hide_overlay_later(&app);
        return;
    }

    // 识别已排空：收尾态 → 润色态，界面切换到润色阶段，此后取消按钮生效
    let Some(cancel) = begin_polishing(&app, id) else {
        return;
    };
    emit(&app, OverlayEvent::Processing {
        text: transcript.clone(),
        mode: mode.event_name(),
    });

    match mode {
        Mode::Dictate => {
            let cfg = config::load(&app);
            let final_text = match cfg.polish_mode {
                config::PolishMode::Raw => transcript.clone(),
                config::PolishMode::Polished => {
                    let key = match secrets::get_api_key() {
                        Ok(k) => k,
                        Err(e) => {
                            release(&app, id);
                            emit_error(&app, &e, mode);
                            return;
                        }
                    };
                    // 原子竞争：润色返回与用户取消先到先得，落选分支连同其
                    // Future 一起被丢弃，迟到的一方不产生任何效果
                    tokio::select! {
                        res = llm::polish(&key, &cfg.llm_model, &transcript) => match res {
                            Ok(polished) if !polished.is_empty() => polished,
                            Ok(_) => transcript.clone(),
                            Err(e) => {
                                // 润色失败不阻断输入，降级为插入原文
                                eprintln!("[pipeline] 润色失败，插入原文: {e}");
                                transcript.clone()
                            }
                        },
                        _ = cancel.notified() => transcript.clone(),
                    }
                }
            };
            output_and_finish(&app, id, final_text, mode).await;
        }
        Mode::Command => {
            let mut cfg = config::load(&app);
            let key = match secrets::get_api_key() {
                Ok(k) => k,
                Err(e) => {
                    release(&app, id);
                    emit_error(&app, &e, mode);
                    return;
                }
            };
            match llm::route_command(&key, &cfg.llm_model, &transcript, cfg.polish_mode).await {
                Ok(llm::CommandOutcome::ModeSwitched { mode: new_mode, reply }) => {
                    cfg.polish_mode = new_mode;
                    config::save(&app, &cfg);
                    output_and_finish(&app, id, reply, mode).await;
                }
                Ok(llm::CommandOutcome::Replied(msg)) => {
                    let text = if msg.is_empty() { transcript.clone() } else { msg };
                    output_and_finish(&app, id, text, mode).await;
                }
                Err(e) => {
                    release(&app, id);
                    emit_error(&app, &e, mode);
                    return;
                }
            }
        }
    }
}

/// 输出态：剪贴板粘贴插入文本，完成后回到 Idle 并展示结果
async fn output_and_finish(app: &AppHandle, id: u64, text: String, mode: Mode) {
    if !enter_output(app, id) {
        return;
    }
    if let Err(e) = insert::insert_text(app, &text) {
        release(app, id);
        emit_error(app, &e, mode);
        return;
    }
    release(app, id);
    emit(app, OverlayEvent::Result { text, mode: mode.event_name() });
    hide_overlay_later(app);
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

/// 结果展示两秒后隐藏字幕窗口；等待期间用户若已开启新一轮会话，
/// 状态机不再处于 Idle，本任务作废，窗口交由新会话管理
fn hide_overlay_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if !matches!(*app.state::<Pipeline>().slot.lock().unwrap(), Slot::Idle) {
            return;
        }
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
        let _ = win.set_always_on_top(true);
        // macOS：面板用 orderFrontRegardless 显示，不抢焦点，可跨全屏空间。
        // 裸 msg_send 必须在主线程执行，而本函数会被异步会话任务调用，
        // 统一经 run_on_main_thread 派发，否则行为未定义（不显示甚至崩溃）
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app.get_webview_panel("overlay") {
                let _ = app.run_on_main_thread(move || {
                    panel.order_front_regardless();
                });
                return;
            }
        }
        let _ = win.show();
    }
}

/// 网页侧报告内容高度后扩展字幕窗口，底边位置保持不动，气泡视觉上向上生长
#[tauri::command]
pub fn resize_overlay(app: AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("overlay") else {
        return;
    };
    let Ok(scale) = win.scale_factor() else {
        return;
    };
    let mut new_h = (height.max(60.0) * scale) as i32;
    // 安全上限：不超过主屏高度的六成，防止超长转写把窗口顶出屏幕
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let max_h = (monitor.size().height as f64 * 0.6) as i32;
        new_h = new_h.min(max_h);
    }
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let new_y = pos.y + size.height as i32 - new_h;
    let _ = win.set_size(tauri::PhysicalSize::new(size.width, new_h as u32));
    let _ = win.set_position(tauri::PhysicalPosition::new(pos.x, new_y));
}
