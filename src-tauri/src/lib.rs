mod asr;
mod audio;
mod config;
mod insert;
mod llm;
mod pipeline;
mod secrets;

use pipeline::{Mode, Pipeline};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[tauri::command]
fn get_setup_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let cfg = config::load(&app);
    Ok(serde_json::json!({
        "onboarded": cfg.onboarded,
        "config": cfg,
    }))
}

#[tauri::command]
async fn save_api_key(app: tauri::AppHandle, key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("API Key 不能为空".into());
    }
    // 先测通再落盘，避免把坏 key 存进系统凭据管理器
    llm::verify_key(&key, &config::load(&app).llm_model).await?;
    secrets::set_api_key(&key)?;
    // 回读校验：防止凭据存储静默失败导致假就绪状态
    if secrets::get_api_key().ok().as_deref() != Some(key.as_str()) {
        return Err("API Key 写入后无法读回，保存可能未生效，请重试或检查系统设置".into());
    }
    let mut cfg = config::load(&app);
    cfg.onboarded = true;
    config::save(&app, &cfg);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let cfg = config::load(&app.handle());

            // 首启向导：已配置过则隐藏主窗口，交由托盘
            if cfg.onboarded {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            app.manage(Pipeline::new());

            // 全局热键：Ctrl+Alt+D 听写，Ctrl+Alt+C 命令，同一键再按结束
            let shortcuts = [
                (cfg.hotkey_dictate.clone(), Mode::Dictate),
                (cfg.hotkey_command.clone(), Mode::Command),
            ];
            let gs = app.global_shortcut();
            for (accel, mode) in shortcuts {
                let Ok(shortcut) =
                    accel.parse::<tauri_plugin_global_shortcut::Shortcut>()
                else {
                    eprintln!("[lib] 无法解析热键 {accel}");
                    continue;
                };
                let handle = app.handle().clone();
                gs.on_shortcut(shortcut, move |_app, _sc, event| {
                    if event.state == ShortcutState::Pressed {
                        handle.state::<Pipeline>().toggle(&handle, mode);
                    }
                })?;
            }

            // 托盘：常驻入口，设置与退出；回调参数是托盘对象，需另持应用句柄
            let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings, &quit])?;
            let handle_menu = app.handle().clone();
            let handle_click = app.handle().clone();
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("SimpleLess 语音输入")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |_tray, event| match event.id.as_ref() {
                    "settings" => {
                        if let Some(win) = handle_menu.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "quit" => handle_menu.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(move |_tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(win) = handle_click.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_setup_status, save_api_key])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
