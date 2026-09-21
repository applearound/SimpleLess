//! 文本插入：把文本写入剪贴板后模拟 Cmd+V，粘贴到当前光标位置。

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::AppHandle;

/// enigo 的 Unicode 按键依赖 HIToolbox 输入源接口，该接口仅允许主线程调用，
/// 从异步任务线程触发会挂起或被系统断言终止；整个插入流程派发到主线程执行，
/// 调用方阻塞等待结果。已在主线程时直接执行，避免派发回自身造成死锁
pub fn insert_text(app: &AppHandle, text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    if std::thread::current().name() == Some("main") {
        return do_insert(text);
    }
    let owned = text.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(do_insert(&owned));
    });
    rx.recv().map_err(|_| "插入任务派发主线程失败".to_string())?
}

fn do_insert(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("访问剪贴板失败: {e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("写入剪贴板失败: {e}"))?;

    // 给剪贴板一点落盘时间，避免个别应用读到旧内容
    std::thread::sleep(std::time::Duration::from_millis(60));

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("初始化键盘模拟失败: {e}"))?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(key_err)?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(key_err)?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(key_err)?;

    Ok(())
}

fn key_err(e: enigo::InputError) -> String {
    format!("模拟粘贴按键失败: {e}")
}
