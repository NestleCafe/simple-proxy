//! 日志总线：把代理访问日志与实例状态变更以 Tauri 事件推送给前端。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::proxy::{AccessLogEntry, LogSink};

/// 访问日志事件名（前端使用 onProxyLog 订阅）
pub const EVENT_PROXY_LOG: &str = "proxy-log";

/// 实例状态事件名（前端使用 onInstanceState 订阅）
pub const EVENT_INSTANCE_STATE: &str = "instance-state";

/// 系统提示日志使用的方法名
const INFO_METHOD: &str = "INFO";

/// instance-state 事件负载（camelCase，与前端 InstanceStateEvent 对应）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceStatePayload {
    /// 实例 id
    id: String,
    /// 运行状态：running | stopped | error
    state: String,
    /// 错误原因（state 为 error 时提供）
    error: Option<String>,
}

/// 生成访问日志回调：把每条日志作为 "proxy-log" 事件推送给前端。
///
/// 前端可能尚未注册监听（应用启动早期），发送失败时直接忽略。
pub fn make_log_sink(app: AppHandle) -> LogSink {
    Arc::new(move |entry: AccessLogEntry| {
        let _ = app.emit(EVENT_PROXY_LOG, entry);
    })
}

/// 推送实例状态变更事件（"running" | "stopped" | "error"）。
pub fn emit_instance_state(app: &AppHandle, id: &str, state: &str, error: Option<&str>) {
    let payload = InstanceStatePayload {
        id: id.to_string(),
        state: state.to_string(),
        error: error.map(str::to_string),
    };
    let _ = app.emit(EVENT_INSTANCE_STATE, payload);
}

/// 推送一条系统提示日志（method 固定为 "INFO"，path 存放提示文本）。
///
/// 用于「监听已启动」「监听已停止」等不针对具体请求的提示，便于用户在日志面板中观察。
pub fn emit_info_log(
    app: &AppHandle,
    instance_id: &str,
    instance_name: &str,
    target: &str,
    message: &str,
) {
    let entry = AccessLogEntry {
        time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        instance_id: instance_id.to_string(),
        instance_name: instance_name.to_string(),
        method: INFO_METHOD.to_string(),
        path: message.to_string(),
        target: target.to_string(),
        status: None,
        duration_ms: None,
        error: None,
    };
    let _ = app.emit(EVENT_PROXY_LOG, entry);
}
