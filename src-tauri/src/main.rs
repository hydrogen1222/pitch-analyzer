// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        // 解决部分 Linux 环境下 (如 NVIDIA 驱动、Wayland) WebKit2GTK 创建 EGL display 失败导致崩溃的 BUG
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    pitch_analyzer_tauri_lib::run()
}
