//! 系统托盘：创建托盘图标与菜单（显示窗口 / 退出），并处理菜单与图标事件。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

/// 托盘图标 id（便于按 id 查找与管理）
const TRAY_ID: &str = "simple-proxy-tray";

/// 菜单项 id：显示窗口
const MENU_ID_SHOW: &str = "show";

/// 菜单项 id：退出应用
const MENU_ID_QUIT: &str = "quit";

/// 主窗口 label（与 tauri.conf.json 中的窗口配置一致）
const MAIN_WINDOW_LABEL: &str = "main";

/// 初始化系统托盘（图标 + 菜单：显示窗口 / 退出），并处理菜单事件。
///
/// - 菜单「显示窗口」或左键单击托盘图标：显示并聚焦主窗口
/// - 菜单「退出」：直接退出应用（进程退出即停止全部代理实例）
///
/// 托盘图标构建后会注册进 Tauri 的资源表，无需额外保存到 State 即可常驻。
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, MENU_ID_SHOW, "显示窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_ID_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        // 左键单击留给「显示窗口」，菜单改为右键触发
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_ID_SHOW => show_main_window(app),
            MENU_ID_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // 图标取自打包图标（default_window_icon）；缺失时退化为无图标托盘，保证菜单仍可用
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    // 保持返回值存活到函数结束，避免托盘被提前释放
    let _tray = builder.build(app)?;
    Ok(())
}

/// 显示并聚焦主窗口；窗口不存在时静默忽略。
///
/// 供托盘事件与「第二个实例启动」回调复用。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
