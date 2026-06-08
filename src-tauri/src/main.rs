// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        // 解决部分 Linux 环境下 (如 NVIDIA 驱动、Wayland) WebKit2GTK 创建 EGL display 失败或白屏导致崩溃的 BUG
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").is_err() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
        // 对于 Wayland 会话，强制使用 X11 后端 (XWayland)，彻底规避 AppImage 内部打包的 Wayland 相关库与系统冲突导致的 EGL 崩溃
        if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("GDK_BACKEND").is_err() {
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    pitch_analyzer_tauri_lib::run()
}
