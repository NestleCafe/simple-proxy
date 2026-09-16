//! simple-proxy 桌面应用库入口：装配配置、实例引擎与 Tauri 命令。

pub mod config;
pub mod engine;
pub mod logbus;
pub mod proxy;
pub mod tray;

use std::path::PathBuf;

use tauri::{Manager, State};

use engine::{ConfigPayload, Engine, InstanceInfo, SaveConfigResult};

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

/// 启动全部已启用且当前未运行的实例；单个实例失败时返回聚合原因。
#[tauri::command]
async fn start_all_instances(state: State<'_, Engine>) -> Result<(), String> {
    state.start_all().await
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
    tauri::Builder::default()
        .setup(|app| {
            let config_path = resolve_config_path();
            app.manage(Engine::new(app.handle().clone(), config_path));

            // 后台启动所有启用中的实例，不阻塞 setup（单个失败会在该实例上标记 error）
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(engine) = handle.try_state::<Engine>() {
                    engine.start_all_enabled().await;
                }
            });

            // 系统托盘常驻：创建失败仅记录日志，不影响主窗口与代理服务
            if let Err(err) = tray::init(app.handle()) {
                eprintln!("创建系统托盘失败（{err}），本次运行将没有托盘图标");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 每次关闭时实时读取配置，用户在设置页切换后立即生效
                let close_behavior = window
                    .app_handle()
                    .try_state::<engine::Engine>()
                    .map(|engine| engine.close_behavior())
                    // 拿不到引擎（应用退出过程中）时按默认值处理：隐藏窗口
                    .unwrap_or_else(|| "minimize".to_string());

                if close_behavior != "exit" {
                    // 默认行为：阻止关闭并隐藏到系统托盘，代理继续转发
                    api.prevent_close();
                    let _ = window.hide();
                }
                // "exit"：不阻止关闭，进程退出即停止全部实例
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
            start_all_instances
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}
