//! AMAX Dashboard — Tauri 桌面应用入口

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    amax_lib::run()
}
