//! 应用主体：注册全局热键与系统托盘，提供设置命令，
//! 负责 API Key 的保存校验与首启自愈。

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
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

// macOS：字幕条面板类。可成为浮动面板但不接管键盘，
// 配合 nonactivating 标志在不激活应用的前提下显示
#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(OverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

/// 打开设置窗口；窗口已被销毁时按原配置重建，托盘入口不因关闭而失灵
fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    match tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("SimpleLess 设置")
        .inner_size(520.0, 560.0)
        .resizable(false)
        .build()
    {
        Ok(win) => {
            let _ = win.show();
            let _ = win.set_focus();
        }
        Err(e) => eprintln!("[lib] 重建设置窗口失败: {e}"),
    }
}

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
    // set 内部会直读钥匙串做回读校验，并更新进程内缓存
    secrets::set_api_key(&key)?;
    let mut cfg = config::load(&app);
    cfg.onboarded = true;
    config::save(&app, &cfg);
    Ok(())
}

#[tauri::command]
async fn list_llm_models() -> Result<Vec<String>, String> {
    let key = secrets::get_api_key().map_err(|_| "尚未保存 API Key，请先完成首次配置".to_string())?;
    llm::list_models(&key).await
}

#[tauri::command]
fn set_llm_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("模型名不能为空".into());
    }
    let mut cfg = config::load(&app);
    cfg.llm_model = model;
    config::save(&app, &cfg);
    Ok(())
}

#[tauri::command]
fn set_polish_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    let mode = match mode.as_str() {
        "raw" => config::PolishMode::Raw,
        "polished" => config::PolishMode::Polished,
        other => return Err(format!("未知的润色档位: {other}")),
    };
    let mut cfg = config::load(&app);
    cfg.polish_mode = mode;
    config::save(&app, &cfg);
    Ok(())
}

#[tauri::command]
fn set_max_recording_seconds(app: tauri::AppHandle, seconds: u64) -> Result<(), String> {
    // 下限 5 秒防误触；上限 20 分钟，超长连讲不现实
    if !(5..=1200).contains(&seconds) {
        return Err("录音时长需在 5 到 1200 秒之间".into());
    }
    let mut cfg = config::load(&app);
    cfg.max_recording_seconds = seconds;
    config::save(&app, &cfg);
    Ok(())
}

#[tauri::command]
fn list_input_devices() -> Result<Vec<String>, String> {
    Ok(audio::list_input_devices())
}

#[tauri::command]
fn set_input_device(app: tauri::AppHandle, device: Option<String>) -> Result<(), String> {
    // 空串视同未指定，跟随系统默认
    let device = device.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    let mut cfg = config::load(&app);
    cfg.input_device = device;
    config::save(&app, &cfg);
    Ok(())
}

#[tauri::command]
fn cancel_polish(app: tauri::AppHandle) -> Result<(), String> {
    if app.state::<Pipeline>().cancel_active_polish() {
        Ok(())
    } else {
        Err("当前没有可取消的处理".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 设置窗口点关闭只是隐藏，托盘是常驻入口，真正退出走托盘菜单
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        });
    // macOS：字幕窗口转 NSPanel 的支撑插件
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
        .setup(|app| {
            // macOS：转为菜单栏应用，Dock 不再显示图标，常驻入口收敛到托盘；
            // 设置窗口的 set_focus 内部带 activateIgnoringOtherApps，弹出不受影响
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let mut cfg = config::load(&app.handle());

            // 自愈：就绪标志为真但密钥实际读不到（历史假成功遗留），
            // 回退到首启向导让用户重新保存
            if cfg.onboarded && secrets::get_api_key().is_err() {
                cfg.onboarded = false;
                config::save(&app.handle(), &cfg);
            }

            // 首启向导：已配置过则隐藏主窗口，交由托盘
            if cfg.onboarded {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            // 迁移：macOS 上 Caps Lock 无法注册为全局热键，
            // 仍持有旧默认热键的配置换成当前平台默认值
            #[cfg(target_os = "macos")]
            {
                let legacy_dictate = "capslock";
                let legacy_command = "ctrl+capslock";
                let defaults = config::AppConfig::default();
                if cfg.hotkey_dictate == legacy_dictate || cfg.hotkey_command == legacy_command {
                    if cfg.hotkey_dictate == legacy_dictate {
                        cfg.hotkey_dictate = defaults.hotkey_dictate;
                    }
                    if cfg.hotkey_command == legacy_command {
                        cfg.hotkey_command = defaults.hotkey_command;
                    }
                    config::save(&app.handle(), &cfg);
                }
            }

            // macOS：普通 NSWindow 无论加什么标志都无法显示在其他应用的全屏空间之上，
            // 必须把窗口对象转为 NSPanel；nonactivating 让面板显示时不激活本应用，
            // 行为标志使其加入所有空间并可作为全屏辅助窗口悬浮
            #[cfg(target_os = "macos")]
            {
                use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
                if let Some(win) = app.get_webview_window("overlay") {
                    if let Ok(panel) = win.to_panel::<OverlayPanel>() {
                        panel.set_level(PanelLevel::Floating.value());
                        let _ = panel.add_style_mask(StyleMask::empty().nonactivating_panel().into());
                        panel.set_collection_behavior(
                            CollectionBehavior::new()
                                .full_screen_auxiliary()
                                .can_join_all_spaces()
                                .into(),
                        );
                    }
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
                let Ok(shortcut) = accel.parse::<tauri_plugin_global_shortcut::Shortcut>() else {
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
                    "settings" => show_main(&handle_menu),
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
                        show_main(&handle_click);
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_setup_status,
            save_api_key,
            list_llm_models,
            set_llm_model,
            set_polish_mode,
            set_max_recording_seconds,
            list_input_devices,
            set_input_device,
            cancel_polish,
            pipeline::resize_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ============================================================================
// 单元测试：以下代码标注 #[cfg(test)]，仅 cargo test 时编译
// ============================================================================
#[cfg(test)]
mod tests {}
