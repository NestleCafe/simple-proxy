// 防止 Windows 发布版弹出额外的控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 桌面端入口：委托给库中的 run 函数，便于后续扩展与测试
fn main() {
    simple_proxy_lib::run()
}
