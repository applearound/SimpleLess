//! 应用配置：AppConfig 结构与默认值，以及 config.json 的加载与保存。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub onboarded: bool,
    pub asr_model: String,
    pub llm_model: String,
    /// 润色档位：raw 直接插入识别原文，polished 先经大模型润色
    pub polish_mode: PolishMode,
    pub language_hints: Vec<String>,
    pub hotkey_dictate: String,
    pub hotkey_command: String,
    pub max_recording_seconds: u64,
    /// 指定录音设备名；None 表示跟随系统默认输入设备。
    /// 带默认值以兼容升级前的旧配置文件
    #[serde(default)]
    pub input_device: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolishMode {
    Raw,
    Polished,
}

impl PolishMode {
    pub fn label(&self) -> &'static str {
        match self {
            PolishMode::Raw => "原文",
            PolishMode::Polished => "润色",
        }
    }
}

/// Caps Lock 在 macOS 上走 flagsChanged 事件通道，无法注册为全局热键，故按平台区分默认热键
#[cfg(target_os = "macos")]
pub const DEFAULT_HOTKEY_DICTATE: &str = "alt+space";
#[cfg(target_os = "macos")]
pub const DEFAULT_HOTKEY_COMMAND: &str = "alt+shift+space";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_HOTKEY_DICTATE: &str = "capslock";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_HOTKEY_COMMAND: &str = "ctrl+capslock";

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            onboarded: false,
            asr_model: "fun-asr-realtime".into(),
            llm_model: "qwen3.7-flash".into(),
            polish_mode: PolishMode::Polished,
            language_hints: vec!["zh".into()],
            hotkey_dictate: DEFAULT_HOTKEY_DICTATE.into(),
            hotkey_command: DEFAULT_HOTKEY_COMMAND.into(),
            max_recording_seconds: 60,
            input_device: None,
        }
    }
}

pub fn config_path(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    std::fs::create_dir_all(&dir).ok();
    dir.join("config.json")
}

pub fn load(app: &tauri::AppHandle) -> AppConfig {
    let path = config_path(app);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &tauri::AppHandle, cfg: &AppConfig) {
    let path = config_path(app);
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_defaults_parse_as_shortcuts() {
        for accel in [DEFAULT_HOTKEY_DICTATE, DEFAULT_HOTKEY_COMMAND] {
            accel
                .parse::<tauri_plugin_global_shortcut::Shortcut>()
                .unwrap_or_else(|e| panic!("热键 {accel} 无法解析: {e}"));
        }
    }
}
