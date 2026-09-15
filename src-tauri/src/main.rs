//! 程序入口，仅负责启动 Tauri 应用运行时。

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    simpleless_lib::run()
}
