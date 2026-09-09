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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            onboarded: false,
            asr_model: "fun-asr-realtime".into(),
            llm_model: "qwen3.8-27b".into(),
            polish_mode: PolishMode::Polished,
            language_hints: vec!["zh".into()],
            hotkey_dictate: "ctrl+alt+d".into(),
            hotkey_command: "ctrl+alt+c".into(),
            max_recording_seconds: 60,
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
