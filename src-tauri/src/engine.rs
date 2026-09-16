//! 实例引擎：管理各代理实例的生命周期（端口监听、优雅启停）、配置读写与运行状态同步。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

use crate::config::{self, AppConfig, ProxyInstance};
use crate::logbus;
use crate::proxy::{self, InstanceRuntimeConfig, LogSink};

/// 停止实例时等待服务优雅退出的上限；超时则强制中止监听任务，
/// 避免长连接（如 SSE 流式响应）把停止流程拖住。
const STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// 连接上游的建立超时。
///
/// 注意：不设置整体请求超时（`ClientBuilder::timeout`），否则会打断 SSE 等长流式响应。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// 每个上游主机允许的空闲连接数（连接池复用，减少重复握手）
const POOL_MAX_IDLE_PER_HOST: usize = 16;

/// get_config 命令的返回：配置内容 + 配置损坏时的加载警告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPayload {
    /// 配置版本号
    pub version: u32,
    /// 关窗行为：minimize | exit
    pub close_behavior: String,
    /// 代理实例列表
    pub proxies: Vec<ProxyInstance>,
    /// 配置损坏提示；正常为 None
    pub load_warning: Option<String>,
}

/// list_instances 命令的返回：实例完整配置 + 运行状态
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    /// 实例唯一标识
    pub id: String,
    /// 实例名称
    pub name: String,
    /// 是否启用
    pub enabled: bool,
    /// 转发目标地址
    pub target: String,
    /// 本地监听端口
    pub http_port: u16,
    /// 目标协议：openai | anthropic
    pub protocol: String,
    /// 推理强度（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// 附加的静态请求头
    pub headers: BTreeMap<String, String>,
    /// 运行状态：running | stopped | error
    pub state: String,
    /// 错误原因（state 为 error 时提供）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// save_config 命令的返回
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConfigResult {
    /// 需要重启才能生效的实例 id 列表
    pub restart_required: Vec<String>,
}

/// 实例运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceState {
    /// 正在监听
    Running,
    /// 未运行
    Stopped,
    /// 启动失败或服务异常退出
    Error,
}

impl InstanceState {
    /// 转换为前端约定的状态字符串
    fn as_str(self) -> &'static str {
        match self {
            InstanceState::Running => "running",
            InstanceState::Stopped => "stopped",
            InstanceState::Error => "error",
        }
    }
}

impl Default for InstanceState {
    /// 默认状态：未运行
    fn default() -> Self {
        InstanceState::Stopped
    }
}

/// 单个实例的运行时记录
#[derive(Debug, Default)]
struct InstanceRuntime {
    /// 运行状态
    state: InstanceState,
    /// 最近一次错误原因
    error: Option<String>,
    /// 优雅停止信号；仅运行中为 Some
    shutdown: Option<oneshot::Sender<()>>,
    /// 监听任务句柄；仅运行中为 Some
    handle: Option<tauri::async_runtime::JoinHandle<()>>,
    /// 启动时使用的配置副本（用于计算 restartRequired）
    started_config: Option<ProxyInstance>,
    /// 是否正在进行启动/停止（防止同一实例被并发操作）
    transitioning: bool,
}

/// 引擎的内部可变状态
struct EngineInner {
    /// 当前生效的配置（内存副本）
    config: AppConfig,
    /// 配置文件路径
    config_path: PathBuf,
    /// 配置加载警告
    load_warning: Option<String>,
    /// 实例运行时记录（按需惰性创建）
    runtimes: HashMap<String, InstanceRuntime>,
}

impl EngineInner {
    /// 实例展示名：优先取配置中的名称，缺失（已删除或名称为空）时退化为 id
    fn instance_label(&self, id: &str) -> String {
        self.config
            .proxies
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| id.to_string())
    }
}

/// 应用级实例引擎：持有共享 HTTP 客户端与全部实例的运行时状态。
///
/// 并发设计：`inner` 使用 `std::sync::Mutex`，所有临界区都是同步的短逻辑，
/// 且严格保证**不跨 `await` 持有锁**（公共异步方法一律遵循
/// 「锁内取数据 → 释放锁 → await → 锁内写回」的流程），
/// 因此 `save_config` 与 `start_instance` / `stop_instance` 的互调不会死锁。
/// 同一实例的并发启停由 `InstanceRuntime::transitioning` 拦截。
pub struct Engine {
    /// Tauri 应用句柄（推送事件用）
    app: AppHandle,
    /// 全部实例共享的 HTTP 客户端（连接池复用）
    client: reqwest::Client,
    /// 访问日志回调
    log_sink: LogSink,
    /// 内部可变状态
    inner: Mutex<EngineInner>,
}

impl Engine {
    /// 创建引擎：加载磁盘配置、初始化共享 HTTP 客户端，全部实例初始为 stopped。
    ///
    /// 配置读取发生 IO 错误时回退为空配置，并把原因记入 loadWarning，保证应用仍可启动。
    pub fn new(app: AppHandle, config_path: PathBuf) -> Self {
        let (config, load_warning) = match config::load(&config_path) {
            Ok(loaded) => loaded,
            Err(err) => (
                AppConfig::default(),
                Some(format!("读取配置失败（{err}），已使用默认空配置")),
            ),
        };

        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
            .build()
            .unwrap_or_else(|err| {
                eprintln!("创建 HTTP 客户端失败（{err}），改用默认客户端");
                reqwest::Client::new()
            });

        let log_sink = logbus::make_log_sink(app.clone());

        Self {
            app,
            client,
            log_sink,
            inner: Mutex::new(EngineInner {
                config,
                config_path,
                load_warning,
                runtimes: HashMap::new(),
            }),
        }
    }

    /// 加锁访问内部状态（锁中毒时恢复数据继续运行，避免单个 panic 导致应用不可用）
    fn lock_inner(&self) -> MutexGuard<'_, EngineInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 启动指定实例：绑定监听端口并后台运行代理服务。
    ///
    /// - 实例不存在或已禁用：返回中文原因，不改变运行状态
    /// - 已在运行：直接返回 `Ok`（幂等）
    /// - 端口占用等失败：状态置 error、推送状态与日志事件，返回中文原因，不影响其它实例
    pub async fn start_instance(&self, id: &str) -> Result<(), String> {
        // 1. 锁内完成校验并占位（不跨 await：避免并发启停同一实例）
        let instance = {
            let mut inner = self.lock_inner();
            let Some(instance) = inner
                .config
                .proxies
                .iter()
                .find(|item| item.id == id)
                .cloned()
            else {
                return Err(format!("实例不存在：{id}"));
            };
            // 已禁用实例不允许启动：监听状态必须与开关语义一致
            if !instance.enabled {
                return Err("实例已禁用，请先启用后再启动".to_string());
            }

            let runtime = inner.runtimes.entry(id.to_string()).or_default();
            if runtime.transitioning {
                return Err(format!(
                    "实例「{}」正在启动或停止中，请稍后重试",
                    instance.name
                ));
            }
            if runtime.state == InstanceState::Running {
                return Ok(());
            }
            runtime.transitioning = true;
            instance
        };

        // 2. 绑定监听端口（0.0.0.0，与 legacy 版本的默认监听行为一致）
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", instance.http_port)).await {
            Ok(listener) => listener,
            Err(err) => {
                let message = format!("监听端口 {} 失败（{err}）", instance.http_port);
                mark_instance_error(&self.app, id, &message);
                return Err(message);
            }
        };

        // 3. 构建代理服务并后台运行；停止时通过 oneshot 触发优雅关闭
        let router = proxy::build_router(
            self.client.clone(),
            Arc::new(instance_to_runtime_config(&instance)),
            self.log_sink.clone(),
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let app = self.app.clone();
        let task_id = id.to_string();
        let handle = tauri::async_runtime::spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                // 发送端被丢弃（引擎已释放该实例）时同样进入关闭流程
                let _ = shutdown_rx.await;
            });
            if let Err(err) = server.await {
                // 服务异常退出（如 accept 失败）：记录状态，避免界面一直显示 running
                mark_instance_error(&app, &task_id, &format!("代理服务异常退出：{err}"));
            }
        });

        // 4. 记录运行态并通知前端
        {
            let mut inner = self.lock_inner();
            let runtime = inner.runtimes.entry(id.to_string()).or_default();
            runtime.state = InstanceState::Running;
            runtime.error = None;
            runtime.shutdown = Some(shutdown_tx);
            runtime.handle = Some(handle);
            runtime.started_config = Some(instance.clone());
            runtime.transitioning = false;
        }

        logbus::emit_instance_state(&self.app, id, InstanceState::Running.as_str(), None);
        logbus::emit_info_log(
            &self.app,
            id,
            &instance.name,
            &instance.target,
            &format!(
                "监听已启动: http://127.0.0.1:{} => {}",
                instance.http_port, instance.target
            ),
        );
        Ok(())
    }

    /// 停止指定实例：发送停止信号并等待监听任务退出（优雅退出超时则强制中止）。
    ///
    /// 幂等：未运行时归一化为 stopped 并直接返回 `Ok`。仅影响该实例。
    pub async fn stop_instance(&self, id: &str) -> Result<(), String> {
        // 1. 锁内取出运行资源（不跨 await 持有锁）
        let (taken, name, target) = {
            let mut inner = self.lock_inner();
            let label = inner.instance_label(id);
            let runtime = inner.runtimes.entry(id.to_string()).or_default();

            if runtime.transitioning {
                return Err(format!("实例「{label}」正在启动或停止中，请稍后重试"));
            }

            let name = runtime
                .started_config
                .as_ref()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| label.clone());
            let target = runtime
                .started_config
                .as_ref()
                .map(|item| item.target.clone())
                .unwrap_or_default();

            if runtime.shutdown.is_none() && runtime.handle.is_none() {
                // 未运行：仅归一化状态（顺带清除历史错误）
                runtime.state = InstanceState::Stopped;
                runtime.error = None;
                runtime.started_config = None;
                (None, name, target)
            } else {
                runtime.transitioning = true;
                (
                    Some((runtime.shutdown.take(), runtime.handle.take())),
                    name,
                    target,
                )
            }
        };

        let Some((shutdown, handle)) = taken else {
            // 未运行：同步状态即可（幂等）
            logbus::emit_instance_state(&self.app, id, InstanceState::Stopped.as_str(), None);
            return Ok(());
        };

        // 2. 释放锁后再等待任务退出
        if let Some(tx) = shutdown {
            // 接收端已随任务结束被丢弃时忽略发送失败
            let _ = tx.send(());
        }
        if let Some(mut handle) = handle {
            if tokio::time::timeout(STOP_TIMEOUT, &mut handle)
                .await
                .is_err()
            {
                // 优雅退出超时（例如仍有长连接）：强制中止并等待任务结束
                handle.abort();
                let _ = handle.await;
            }
        }

        // 3. 记录停止状态并通知前端
        {
            let mut inner = self.lock_inner();
            let runtime = inner.runtimes.entry(id.to_string()).or_default();
            runtime.state = InstanceState::Stopped;
            runtime.error = None;
            runtime.shutdown = None;
            runtime.handle = None;
            runtime.started_config = None;
            runtime.transitioning = false;
        }

        logbus::emit_instance_state(&self.app, id, InstanceState::Stopped.as_str(), None);
        logbus::emit_info_log(&self.app, id, &name, &target, "监听已停止");
        Ok(())
    }

    /// 重启指定实例：先停止（若在运行）再按最新配置启动，仅影响该实例。
    pub async fn restart_instance(&self, id: &str) -> Result<(), String> {
        self.stop_instance(id).await?;
        self.start_instance(id).await
    }

    /// 保存配置并同步实例运行状态，返回需要重启才能生效的实例 id 列表。
    ///
    /// 处理规则（配置写盘成功后）：
    /// - 被禁用且正在运行 → 立即停止（开关关闭立即停）
    /// - 被启用且未运行（含新增、重新启用、错误状态）→ 立即启动
    /// - 保持运行但转发相关字段变化 → 记入 restartRequired（不自动重启，仅提示）
    /// - 新配置中已删除 → 若在运行先停止，再清理运行时记录
    pub async fn save_config(&self, new_config: AppConfig) -> Result<Vec<String>, String> {
        // 1. 校验并写盘（不持锁，避免磁盘 IO 阻塞其它命令）
        let config_path = self.lock_inner().config_path.clone();
        config::validate(&new_config)?;
        config::save(&config_path, &new_config)?;

        // 2. 锁内计算动作计划并更新内存配置（不跨 await 持有锁）
        let plan = {
            let mut inner = self.lock_inner();
            let running: BTreeMap<String, ProxyInstance> = inner
                .runtimes
                .iter()
                .filter(|(_, runtime)| runtime.state == InstanceState::Running)
                .filter_map(|(id, runtime)| {
                    runtime
                        .started_config
                        .clone()
                        .map(|item| (id.clone(), item))
                })
                .collect();
            let known_ids: Vec<String> = inner.runtimes.keys().cloned().collect();

            let plan = plan_save_changes(&new_config, &running, &known_ids);
            inner.config = new_config;
            plan
        };

        // 3. 释放锁后逐个执行启停（start/stop 内部自行加锁，严禁在持锁时互调）
        for id in &plan.to_stop {
            if let Err(err) = self.stop_instance(id).await {
                eprintln!("保存配置后停止实例 {id} 失败: {err}");
            }
        }
        {
            // 已从配置中删除的实例：停止完成后再清理运行时记录
            let mut inner = self.lock_inner();
            for id in &plan.removed {
                inner.runtimes.remove(id);
            }
        }
        for id in &plan.to_start {
            if let Err(err) = self.start_instance(id).await {
                eprintln!("保存配置后启动实例 {id} 失败: {err}");
            }
        }

        Ok(plan.restart_required)
    }

    /// 启动全部启用中的实例（应用启动时调用）；单个实例失败不影响其它实例。
    pub async fn start_all_enabled(&self) {
        // 启动流程不向用户报错：失败信息已由 start_instance 记录日志与状态事件
        let _ = self.start_enabled_instances().await;
    }

    /// 停止全部正在运行的实例（仅改变运行状态，不修改各实例的启用开关，也不写配置）。
    ///
    /// 锁内先收集运行中实例，释放锁后再逐个停止（`stop_instance` 内部会再次加锁，
    /// 严禁在持锁时互调）；单个实例失败不中断其余实例，最终以聚合的中文原因返回。
    pub async fn stop_all(&self) -> Result<(), String> {
        let targets: Vec<(String, String)> = {
            let inner = self.lock_inner();
            plan_stop_all(&inner.runtimes)
                .into_iter()
                .map(|id| {
                    let label = inner.instance_label(&id);
                    (id, label)
                })
                .collect()
        };

        let mut failures = Vec::new();
        for (id, label) in targets {
            if let Err(err) = self.stop_instance(&id).await {
                eprintln!("停止实例 {id} 失败: {err}");
                failures.push(format!("「{label}」{err}"));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("；"))
        }
    }

    /// 启动全部已启用但当前未运行的实例（跳过被禁用的实例，不修改配置）。
    ///
    /// 与 `start_all_enabled` 共用内部实现；单个实例失败不中断其余实例，
    /// 最终以聚合的中文原因返回。
    pub async fn start_all(&self) -> Result<(), String> {
        self.start_enabled_instances().await
    }

    /// 启动全部「已启用且当前未运行」实例的共用实现。
    ///
    /// 锁内先收集目标实例，释放锁后再逐个启动；单个实例失败不中断其余实例，
    /// 失败信息以 `「实例名」原因` 的形式聚合返回。
    async fn start_enabled_instances(&self) -> Result<(), String> {
        let targets: Vec<(String, String)> = {
            let inner = self.lock_inner();
            plan_start_all(&inner.config, &inner.runtimes)
                .into_iter()
                .map(|id| {
                    let label = inner.instance_label(&id);
                    (id, label)
                })
                .collect()
        };

        let mut failures = Vec::new();
        for (id, label) in targets {
            if let Err(err) = self.start_instance(&id).await {
                eprintln!("启动实例 {id} 失败: {err}");
                failures.push(format!("「{label}」{err}"));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("；"))
        }
    }

    /// 读取配置快照（含加载警告），供 get_config 命令使用
    pub fn get_config_payload(&self) -> ConfigPayload {
        let inner = self.lock_inner();
        ConfigPayload {
            version: inner.config.version,
            close_behavior: inner.config.close_behavior.clone(),
            proxies: inner.config.proxies.clone(),
            load_warning: inner.load_warning.clone(),
        }
    }

    /// 按配置顺序列出全部实例及其运行状态，供 list_instances 命令使用
    pub fn list_instances(&self) -> Vec<InstanceInfo> {
        let inner = self.lock_inner();
        inner
            .config
            .proxies
            .iter()
            .map(|instance| {
                let runtime = inner.runtimes.get(&instance.id);
                InstanceInfo {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    enabled: instance.enabled,
                    target: instance.target.clone(),
                    http_port: instance.http_port,
                    protocol: instance.protocol.clone(),
                    reasoning_effort: instance.reasoning_effort.clone(),
                    headers: instance.headers.clone(),
                    state: runtime
                        .map(|item| item.state.as_str().to_string())
                        .unwrap_or_else(|| InstanceState::Stopped.as_str().to_string()),
                    error: runtime.and_then(|item| item.error.clone()),
                }
            })
            .collect()
    }

    /// 读取关窗行为（minimize / exit），供托盘与窗口关闭逻辑使用
    pub fn close_behavior(&self) -> String {
        self.lock_inner().config.close_behavior.clone()
    }

    /// 把实例状态置为 error 并释放监听资源，返回 (名称, 目标地址) 供日志展示使用
    fn set_error_state(&self, id: &str, message: &str) -> (String, String) {
        let mut inner = self.lock_inner();
        let (name, target) = inner
            .config
            .proxies
            .iter()
            .find(|item| item.id == id)
            .map(|item| (item.name.clone(), item.target.clone()))
            .unwrap_or_else(|| (id.to_string(), String::new()));

        let runtime = inner.runtimes.entry(id.to_string()).or_default();
        runtime.state = InstanceState::Error;
        runtime.error = Some(message.to_string());
        runtime.shutdown = None;
        runtime.handle = None;
        runtime.started_config = None;
        runtime.transitioning = false;
        (name, target)
    }
}

/// 把配置中的实例转换为代理模块需要的运行时配置
fn instance_to_runtime_config(instance: &ProxyInstance) -> InstanceRuntimeConfig {
    InstanceRuntimeConfig {
        instance_id: instance.id.clone(),
        instance_name: instance.name.clone(),
        target: instance.target.clone(),
        protocol: instance.protocol.clone(),
        reasoning_effort: instance.reasoning_effort.clone(),
        static_headers: instance.headers.clone(),
    }
}

/// 记录实例错误：状态置 error 并推送状态与日志事件。
///
/// 引擎尚未注册到 Tauri（应用退出过程中）时仅推送事件。
fn mark_instance_error(app: &AppHandle, id: &str, message: &str) {
    let mut name = id.to_string();
    let mut target = String::new();
    if let Some(engine) = app.try_state::<Engine>() {
        let (instance_name, instance_target) = engine.set_error_state(id, message);
        name = instance_name;
        target = instance_target;
    }

    logbus::emit_instance_state(app, id, InstanceState::Error.as_str(), Some(message));
    logbus::emit_info_log(app, id, &name, &target, message);
}

/// 保存配置后需要执行的启停动作计划
#[derive(Debug, Default, PartialEq, Eq)]
struct SavePlan {
    /// 运行中但转发相关字段已变化（不自动重启，仅提示用户）
    restart_required: Vec<String>,
    /// 需要立即停止（被禁用或已从配置中删除）
    to_stop: Vec<String>,
    /// 需要立即启动（已启用但当前未运行）
    to_start: Vec<String>,
    /// 新配置中已删除的实例 id（停止后需清理运行时记录）
    removed: Vec<String>,
}

/// 判断两个实例的转发相关字段是否一致（不一致则需重启才能生效）
///
/// 只比较影响转发行为的字段：名称、启用开关等变化无需重启。
fn forwarding_config_equal(started: &ProxyInstance, current: &ProxyInstance) -> bool {
    started.target == current.target
        && started.http_port == current.http_port
        && started.protocol == current.protocol
        && started.reasoning_effort == current.reasoning_effort
        && started.headers == current.headers
}

/// 计算「停止全部」的目标实例 id：仅包含运行中（正在监听）且未处于启停过渡中的实例
/// （纯函数，不依赖 Tauri，便于单元测试）。
///
/// 正在启停过渡中的实例由对应操作负责收尾，此处跳过可避免并发操作产生伪失败提示。
fn plan_stop_all(runtimes: &HashMap<String, InstanceRuntime>) -> Vec<String> {
    let mut ids: Vec<String> = runtimes
        .iter()
        .filter(|(_, runtime)| runtime.state == InstanceState::Running && !runtime.transitioning)
        .map(|(id, _)| id.clone())
        .collect();
    // 运行时记录为哈希表，排序保证停止顺序稳定（便于日志排查）
    ids.sort();
    ids
}

/// 计算「启动全部」的目标实例 id：配置中已启用且当前未运行的实例
/// （纯函数，不依赖 Tauri，便于单元测试）。
///
/// 保持配置顺序，便于日志按界面顺序展示启动过程。
fn plan_start_all(config: &AppConfig, runtimes: &HashMap<String, InstanceRuntime>) -> Vec<String> {
    config
        .proxies
        .iter()
        .filter(|item| item.enabled)
        .filter(|item| {
            runtimes
                .get(&item.id)
                .map(|runtime| runtime.state != InstanceState::Running)
                .unwrap_or(true)
        })
        .map(|item| item.id.clone())
        .collect()
}

/// 计算保存配置后需要执行的启停动作（纯函数，不依赖 Tauri，便于单元测试）。
///
/// - `running`：当前运行中的实例 id → 启动时使用的配置副本
/// - `known_ids`：当前存在运行时记录的实例 id（含已停止与错误状态的实例）
fn plan_save_changes(
    new_config: &AppConfig,
    running: &BTreeMap<String, ProxyInstance>,
    known_ids: &[String],
) -> SavePlan {
    let mut plan = SavePlan::default();
    let new_ids: BTreeSet<&str> = new_config
        .proxies
        .iter()
        .map(|item| item.id.as_str())
        .collect();

    // 1. 已从配置中删除的实例：运行中的需要停止，全部需要清理运行时记录
    for id in known_ids {
        if !new_ids.contains(id.as_str()) {
            plan.removed.push(id.clone());
            if running.contains_key(id) {
                plan.to_stop.push(id.clone());
            }
        }
    }

    // 2. 新配置中的实例：按「被禁用 / 已启用」与当前状态决定停止或启动
    for instance in &new_config.proxies {
        match running.get(&instance.id) {
            Some(started) => {
                if !instance.enabled {
                    // 开关关闭：立即停止（即使转发配置也变了，也无需提示重启）
                    plan.to_stop.push(instance.id.clone());
                } else if !forwarding_config_equal(started, instance) {
                    plan.restart_required.push(instance.id.clone());
                }
            }
            None => {
                if instance.enabled {
                    plan.to_start.push(instance.id.clone());
                }
            }
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个实例（id 为 a 时端口 8787，便于测试端口变化）
    fn instance(id: &str) -> ProxyInstance {
        ProxyInstance {
            id: id.to_string(),
            name: format!("实例 {id}"),
            enabled: true,
            target: "https://example.com/v1".to_string(),
            http_port: 8787,
            protocol: "openai".to_string(),
            reasoning_effort: None,
            headers: BTreeMap::new(),
        }
    }

    /// 构造配置
    fn config_of(proxies: Vec<ProxyInstance>) -> AppConfig {
        AppConfig {
            version: 1,
            close_behavior: "minimize".to_string(),
            proxies,
        }
    }

    /// 构造「运行中实例」映射（id → 启动时配置）
    fn running_of(proxies: Vec<ProxyInstance>) -> BTreeMap<String, ProxyInstance> {
        proxies
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect()
    }

    /// 构造运行时记录 id 列表
    fn ids_of(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// 对 id 列表排序，便于与期望值比较（不受计算顺序影响）
    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    #[test]
    fn 未变化的运行中实例不产生任何动作() {
        let started = instance("a");
        let plan = plan_save_changes(
            &config_of(vec![started.clone()]),
            &running_of(vec![started]),
            &ids_of(&["a"]),
        );

        assert!(plan.restart_required.is_empty(), "{plan:?}");
        assert!(plan.to_stop.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
        assert!(plan.removed.is_empty(), "{plan:?}");
    }

    #[test]
    fn 运行中实例转发字段变化需要重启() {
        let mutators: [fn(&mut ProxyInstance); 5] = [
            // target
            |item| item.target = "https://other.example.com/v1".to_string(),
            // httpPort
            |item| item.http_port = 9999,
            // protocol
            |item| item.protocol = "anthropic".to_string(),
            // reasoningEffort
            |item| item.reasoning_effort = Some("high".to_string()),
            // headers
            |item| {
                item.headers
                    .insert("x-api-key".to_string(), "secret".to_string());
            },
        ];

        for (index, mutate) in mutators.into_iter().enumerate() {
            let started = instance("a");
            let mut updated = started.clone();
            mutate(&mut updated);

            let plan = plan_save_changes(
                &config_of(vec![updated]),
                &running_of(vec![started]),
                &ids_of(&["a"]),
            );

            assert_eq!(plan.restart_required, ids_of(&["a"]), "第 {index} 个字段");
            assert!(plan.to_stop.is_empty(), "第 {index} 个字段: {plan:?}");
            assert!(plan.to_start.is_empty(), "第 {index} 个字段: {plan:?}");
        }
    }

    #[test]
    fn 仅名称变化不需要重启() {
        let started = instance("a");
        let mut renamed = started.clone();
        renamed.name = "新名称".to_string();

        let plan = plan_save_changes(
            &config_of(vec![renamed]),
            &running_of(vec![started]),
            &ids_of(&["a"]),
        );

        assert!(plan.restart_required.is_empty(), "{plan:?}");
        assert!(plan.to_stop.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
    }

    #[test]
    fn 禁用运行中实例立即停止且不提示重启() {
        let started = instance("a");
        let mut disabled = started.clone();
        disabled.enabled = false;
        // 同时修改转发字段：被禁用时只停止，不应提示重启
        disabled.target = "https://other.example.com/v1".to_string();

        let plan = plan_save_changes(
            &config_of(vec![disabled]),
            &running_of(vec![started]),
            &ids_of(&["a"]),
        );

        assert_eq!(plan.to_stop, ids_of(&["a"]));
        assert!(plan.restart_required.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
    }

    #[test]
    fn 启用未运行实例立即启动() {
        // 新增实例（无运行时记录）
        let plan = plan_save_changes(&config_of(vec![instance("new")]), &BTreeMap::new(), &[]);
        assert_eq!(plan.to_start, ids_of(&["new"]));
        assert!(plan.to_stop.is_empty(), "{plan:?}");
        assert!(plan.restart_required.is_empty(), "{plan:?}");

        // 重新启用已停止实例（存在运行时记录但未运行）
        let plan = plan_save_changes(
            &config_of(vec![instance("a")]),
            &BTreeMap::new(),
            &ids_of(&["a"]),
        );
        assert_eq!(plan.to_start, ids_of(&["a"]));
        assert!(plan.to_stop.is_empty(), "{plan:?}");
    }

    #[test]
    fn 禁用未运行实例不产生任何动作() {
        let mut disabled = instance("a");
        disabled.enabled = false;

        let plan = plan_save_changes(
            &config_of(vec![disabled]),
            &BTreeMap::new(),
            &ids_of(&["a"]),
        );

        assert!(plan.to_stop.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
        assert!(plan.restart_required.is_empty(), "{plan:?}");
        assert!(plan.removed.is_empty(), "{plan:?}");
    }

    #[test]
    fn 已删除的运行实例立即停止并清理记录() {
        let started = instance("a");
        let plan = plan_save_changes(
            &config_of(vec![]),
            &running_of(vec![started]),
            &ids_of(&["a"]),
        );

        assert_eq!(plan.to_stop, ids_of(&["a"]));
        assert_eq!(plan.removed, ids_of(&["a"]));
        assert!(plan.restart_required.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
    }

    #[test]
    fn 已删除的停止实例仅清理记录() {
        let plan = plan_save_changes(&config_of(vec![]), &BTreeMap::new(), &ids_of(&["a"]));

        assert_eq!(plan.removed, ids_of(&["a"]));
        assert!(plan.to_stop.is_empty(), "{plan:?}");
        assert!(plan.to_start.is_empty(), "{plan:?}");
    }

    #[test]
    fn 混合场景按期望分类() {
        let mut a = instance("a");
        a.http_port = 7001;
        let mut b = instance("b");
        b.http_port = 7002;
        let mut c = instance("c");
        c.http_port = 7003;
        let mut e = instance("e");
        e.http_port = 7005;

        // 运行中：a（端口将被修改）、b（无变化）、c（将被禁用）、e（将从配置中删除）
        let running = running_of(vec![a.clone(), b.clone(), c.clone(), e.clone()]);

        let mut a_updated = a.clone();
        a_updated.http_port = 8001;
        let mut c_updated = c.clone();
        c_updated.enabled = false;
        // 新配置：a（端口变化）、b（不变）、c（禁用）、d（新增）；e 被删除
        let new_config = config_of(vec![a_updated, b, c_updated, instance("d")]);

        let plan = plan_save_changes(&new_config, &running, &ids_of(&["a", "b", "c", "e"]));

        assert_eq!(sorted(plan.restart_required), ids_of(&["a"]));
        assert_eq!(sorted(plan.to_stop), ids_of(&["c", "e"]));
        assert_eq!(sorted(plan.to_start), ids_of(&["d"]));
        assert_eq!(sorted(plan.removed), ids_of(&["e"]));
    }

    /// 构造运行时记录（仅指定状态与启停过渡标记）
    fn runtime_of(state: InstanceState, transitioning: bool) -> InstanceRuntime {
        InstanceRuntime {
            state,
            transitioning,
            ..Default::default()
        }
    }

    #[test]
    fn 停止全部仅包含运行中的实例() {
        let runtimes = HashMap::from([
            ("a".to_string(), runtime_of(InstanceState::Running, false)),
            ("b".to_string(), runtime_of(InstanceState::Stopped, false)),
            ("c".to_string(), runtime_of(InstanceState::Error, false)),
            // 正在停止过渡中的实例：跳过，避免并发操作产生伪失败提示
            ("d".to_string(), runtime_of(InstanceState::Running, true)),
        ]);

        assert_eq!(plan_stop_all(&runtimes), ids_of(&["a"]));
    }

    #[test]
    fn 无运行中实例时停止全部为空() {
        let runtimes = HashMap::from([
            ("a".to_string(), runtime_of(InstanceState::Stopped, false)),
            ("b".to_string(), runtime_of(InstanceState::Error, false)),
        ]);

        assert!(plan_stop_all(&runtimes).is_empty());
    }

    #[test]
    fn 启动全部仅包含已启用且未运行的实例() {
        let mut disabled = instance("a");
        disabled.enabled = false;
        let running = instance("b");
        let stopped = instance("c");
        let failed = instance("d");

        let config = config_of(vec![disabled, running, stopped, failed]);
        let runtimes = HashMap::from([
            ("b".to_string(), runtime_of(InstanceState::Running, false)),
            ("c".to_string(), runtime_of(InstanceState::Stopped, false)),
            ("d".to_string(), runtime_of(InstanceState::Error, false)),
        ]);

        // 已禁用的 a 与运行中的 b 跳过；c（已停止）与 d（错误状态）需要启动，且保持配置顺序
        assert_eq!(plan_start_all(&config, &runtimes), ids_of(&["c", "d"]));
    }

    #[test]
    fn 启动全部包含无运行时记录的启用实例() {
        let config = config_of(vec![instance("a"), instance("b")]);

        assert_eq!(
            plan_start_all(&config, &HashMap::new()),
            ids_of(&["a", "b"])
        );
    }
}
