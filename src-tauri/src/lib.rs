//! simple-proxy 桌面应用库入口：装配配置、实例引擎与 Tauri 命令。

pub mod config;
pub mod engine;
pub mod logbus;
pub mod proxy;
pub mod tray;

use std::path::PathBuf;

use tauri::{Emitter, Manager, State};

use engine::{ConfigPayload, Engine, InstanceInfo, SaveConfigResult, StartAllOutcome};

/// 读取应用配置（含配置损坏时的加载警告）。
#[tauri::command]
async fn get_config(state: State<'_, Engine>) -> Result<ConfigPayload, String> {
    Ok(state.get_config_payload())
}

/// 列出全部代理实例（按配置顺序）及其运行状态。
#[tauri::command]
async fn list_instances(state: State<'_, Engine>) -> Result<Vec<InstanceInfo>, String> {
    Ok(state.list_instances())
}

/// 保存配置：写盘成功后立即同步实例运行状态，返回需要重启才生效的实例 id 列表。
#[tauri::command]
async fn save_config(
    state: State<'_, Engine>,
    config: config::AppConfig,
) -> Result<SaveConfigResult, String> {
    let restart_required = state.save_config(config).await?;
    Ok(SaveConfigResult { restart_required })
}

/// 启动指定实例（失败时返回中文原因）。
#[tauri::command]
async fn start_instance(state: State<'_, Engine>, id: String) -> Result<(), String> {
    state.start_instance(&id).await
}

/// 停止指定实例。
#[tauri::command]
async fn stop_instance(state: State<'_, Engine>, id: String) -> Result<(), String> {
    state.stop_instance(&id).await
}

/// 重启指定实例。
#[tauri::command]
async fn restart_instance(state: State<'_, Engine>, id: String) -> Result<(), String> {
    state.restart_instance(&id).await
}

/// 停止全部正在运行的实例（不修改各实例的启用开关）；单个实例失败时返回聚合原因。
#[tauri::command]
async fn stop_all_instances(state: State<'_, Engine>) -> Result<(), String> {
    state.stop_all().await
}

/// 启动全部已启用且当前未运行的实例；单个实例失败不中断其余实例，
/// 返回成功数量与失败明细（含端口被占用等原因），供界面提示哪些实例未能启动。
#[tauri::command]
async fn start_all_instances(state: State<'_, Engine>) -> Result<StartAllOutcome, String> {
    state.start_all().await
}

/// 响应前端「关闭窗口」询问：执行所选行为（隐藏到托盘 / 退出应用），remember 为真时记住选择。
#[tauri::command]
async fn resolve_close_request(
    window: tauri::WebviewWindow,
    state: State<'_, Engine>,
    behavior: String,
    remember: bool,
) -> Result<(), String> {
    if behavior != "minimize" && behavior != "exit" {
        return Err(format!(
            "closeBehavior 取值非法（{behavior}），仅支持 minimize 或 exit"
        ));
    }

    // 勾选「记住我的选择」时持久化行为（同时标记为已确认，之后关窗不再询问）
    if remember {
        state.set_close_behavior(&behavior).await?;
    }

    if behavior == "exit" {
        // 直接退出应用：进程退出即停止全部实例
        window.app_handle().exit(0);
    } else {
        // 最小化到系统托盘：隐藏窗口，代理继续转发
        let _ = window.hide();
    }
    Ok(())
}

/// 计算配置文件路径：与可执行文件同目录的 config.json（绿色版：配置随 exe 走）。
///
/// 目录不存在无需在此创建：`config::save` 写盘时会自动创建父目录。
fn resolve_config_path() -> PathBuf {
    match std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
    {
        Some(dir) => dir.join("config.json"),
        None => {
            eprintln!("获取可执行文件目录失败，改用当前工作目录存放 config.json");
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("config.json")
        }
    }
}

/// 构建并运行 Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // 单实例（仅发布版启用）：防止多开导致端口冲突，第二个实例启动时激活已有窗口并自行退出。
    // 开发模式（debug）跳过：tauri dev 的热重启在旧进程尚未完全退出时启动新进程，
    // 会与单实例锁竞争导致新实例被误判为第二实例而退出。
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        tray::show_main_window(app);
    }));

    builder
        .setup(|app| {
            let config_path = resolve_config_path();
            app.manage(Engine::new(app.handle().clone(), config_path));

            // 不自动启动实例：由用户通过「开始」按钮或总开关手动启动，避免开机即占用端口

            // 系统托盘常驻：创建失败仅记录日志，不影响主窗口与代理服务
            if let Err(err) = tray::init(app.handle()) {
                eprintln!("创建系统托盘失败（{err}），本次运行将没有托盘图标");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 每次关闭时实时读取配置，用户在设置页切换后立即生效；
                // 拿不到引擎（应用退出过程中）时按默认值处理：已确认 + 最小化到托盘
                let (behavior, confirmed) = window
                    .app_handle()
                    .try_state::<engine::Engine>()
                    .map(|engine| (engine.close_behavior(), engine.close_behavior_confirmed()))
                    .unwrap_or_else(|| ("minimize".to_string(), true));

                if !confirmed {
                    // 首次（尚未确认过关窗行为）：阻止关闭，交给前端弹窗询问
                    api.prevent_close();
                    let _ = window.app_handle().emit("close-requested", ());
                } else if behavior != "exit" {
                    // 已确认且为「最小化到托盘」：阻止关闭并隐藏窗口，代理继续转发
                    api.prevent_close();
                    let _ = window.hide();
                }
                // 已确认且为 "exit"：不阻止关闭，进程退出即停止全部实例
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_instances,
            save_config,
            start_instance,
            stop_instance,
            restart_instance,
            stop_all_instances,
            start_all_instances,
            resolve_close_request
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}
