//! 文本插入：把文本写入剪贴板后模拟 Ctrl+V，粘贴到当前光标位置。

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// 把文本写入剪贴板后模拟 Ctrl+V 粘贴到当前光标位置
pub fn insert_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

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
