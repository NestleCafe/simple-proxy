//! 反向代理核心模块：为每个代理实例构建一个 axum Router，负责转发头组装、转发路径拼接、
//! 推理强度注入、请求体流式/缓冲转发、上游响应回传与访问日志。
//!
//! 转发语义对齐 legacy 的 `proxy-server.js`，差异点：
//! - 注入判定把 JSON `null` 也视为「未指定」（比 legacy 的 `undefined` 略宽）
//! - 响应头会剔除长度/编码类头与 hop-by-hop 头，由本地 HTTP 服务器重新计算响应帧
//!
//! 监听端口与启停由 engine 侧负责；本模块只提供 Router。

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{OriginalUri, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::stream::StreamExt;
use reqwest::Url;
use serde::Serialize;

/// hop-by-hop 头：属于逐跳传输信息，不应透传给上游（与 legacy 一致）
const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 注入推理强度时缓冲请求体的上限（64 MB），超过则报错并返回 502
const MAX_INJECT_BODY_BYTES: usize = 64 * 1024 * 1024;

/// 上游请求/响应流的统一错误类型
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// 上游响应体流（装箱以便在自定义 Stream 中做 pin 投影）
type BoxedByteStream = Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, BoxError>> + Send>>;

/// 单个实例的运行时配置（由 config.rs 的 ProxyInstance 转换而来）
#[derive(Debug, Clone)]
pub struct InstanceRuntimeConfig {
    /// 实例唯一标识
    pub instance_id: String,
    /// 实例名称
    pub instance_name: String,
    /// 原始 target URL 字符串
    pub target: String,
    /// 目标协议：openai | anthropic
    pub protocol: String,
    /// 推理强度；未配置则不做注入
    pub reasoning_effort: Option<String>,
    /// 附加的静态请求头
    pub static_headers: BTreeMap<String, String>,
}

/// 访问日志事件（时间/实例/方法/路径/目标/状态码/耗时）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessLogEntry {
    /// 本地时间 "YYYY-MM-DD HH:mm:ss"
    pub time: String,
    /// 实例 id
    pub instance_id: String,
    /// 实例名称
    pub instance_name: String,
    /// 请求方法
    pub method: String,
    /// 客户端原始 path（含 query）
    pub path: String,
    /// 实际转发到的完整 URL（origin + forwardPath）
    pub target: String,
    /// 上游状态码；失败为 None
    pub status: Option<u16>,
    /// 耗时（毫秒）
    pub duration_ms: Option<u64>,
    /// 失败原因（含请求体解析失败等提示）
    pub error: Option<String>,
}

/// 访问日志回调：由 engine 侧注入（写入日志总线/文件等）
pub type LogSink = Arc<dyn Fn(AccessLogEntry) + Send + Sync>;

/// 实例代理状态（在 Router 内共享）
struct ProxyState {
    /// 共享的上游 HTTP 客户端（连接复用由调用方统一管理）
    client: reqwest::Client,
    /// 实例运行时配置
    cfg: Arc<InstanceRuntimeConfig>,
    /// 访问日志回调
    log_sink: LogSink,
    /// 目标 URL 解析结果；解析失败时为 None，请求统一返回 502
    target_url: Option<Url>,
}

/// 构建实例的 axum Router：通过 fallback 捕获所有方法与路径。
///
/// - `client`：调用方共享的 reqwest 客户端（一个 Client 复用全部实例，勿在内部新建）
/// - `cfg`：实例运行时配置
/// - `log_sink`：访问日志回调，每次请求完成（含失败）回调一次
pub fn build_router(
    client: reqwest::Client,
    cfg: Arc<InstanceRuntimeConfig>,
    log_sink: LogSink,
) -> Router {
    let target_url = Url::parse(&cfg.target).ok();
    let state = Arc::new(ProxyState {
        client,
        cfg,
        log_sink,
        target_url,
    });

    Router::new().fallback(handle_request).with_state(state)
}

/// 处理一个代理请求：组装转发头与转发路径，转发到上游并流式回传响应。
///
/// 说明：客户端中断连接时，axum/hyper 会丢弃本 future，进而丢弃上游请求/响应流，
/// reqwest 会随之中止上游连接，因此无需额外处理（与 legacy 的 destroy 语义一致）。
async fn handle_request(
    State(state): State<Arc<ProxyState>>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let started = Instant::now();
    let cfg = Arc::clone(&state.cfg);

    // 客户端原始 path + query：OriginalUri 保留了未被改写的原始请求目标
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| uri.path().to_string());

    // 目标地址在构建 Router 时解析，解析失败时所有请求统一返回 502
    let Some(target_url) = state.target_url.as_ref() else {
        let message = format!("目标地址无效: {}", cfg.target);
        let entry = base_entry(&cfg, &method, &path_and_query, "");
        return log_and_respond_error(&state, entry, started, message);
    };

    // 转发路径 = 目标 URL 的 path 前缀 + 客户端原始 path+query
    let forward_path = resolve_forward_path(target_url, &path_and_query);
    let upstream_url = build_upstream_url(target_url, &forward_path);
    let mut entry = base_entry(&cfg, &method, &path_and_query, upstream_url.as_str());

    // 1. 组装转发请求头（透传客户端头 + 静态头覆盖 + Host 覆盖为目标主机）
    let forward_headers =
        build_forward_headers(&headers, &cfg.static_headers, &target_host(target_url));

    // 2. 创建上游请求（复用共享 Client 的连接池实现 keep-alive）
    let mut request = state
        .client
        .request(method, upstream_url)
        .headers(forward_headers);

    // 3. 请求体：需要注入推理强度时缓冲改写，否则流式直转
    let should_inject = cfg
        .reasoning_effort
        .as_deref()
        .map(|effort| !effort.is_empty())
        .unwrap_or(false)
        && matches!(cfg.protocol.as_str(), "openai" | "anthropic");

    if should_inject {
        match axum::body::to_bytes(body, MAX_INJECT_BODY_BYTES).await {
            Ok(raw) => {
                let mut payload = raw.clone();
                // 空请求体无需解析，直接原样转发（不做注入）
                if !raw.is_empty() {
                    match serde_json::from_slice::<serde_json::Value>(&raw) {
                        Ok(mut value) => {
                            if let Some(effort) = cfg.reasoning_effort.as_deref() {
                                if inject_reasoning_effort(&mut value, &cfg.protocol, effort) {
                                    match serde_json::to_vec(&value) {
                                        Ok(encoded) => payload = Bytes::from(encoded),
                                        Err(err) => {
                                            entry.error = Some(format!(
                                                "注入推理强度后序列化请求体失败，按原始请求体转发: {err}"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            // 解析失败：跳过注入、原样转发，并把原因记入访问日志
                            entry.error =
                                Some(format!("请求体 JSON 解析失败，跳过推理强度注入: {err}"));
                        }
                    }
                }
                // 缓冲模式下显式设置 content-length，避免使用分块编码（与 legacy 一致）
                request = request
                    .header(header::CONTENT_LENGTH, payload.len())
                    .body(payload);
            }
            Err(err) => {
                let message = format!(
                    "请求体读取失败（缓冲上限 {} MB）: {err}",
                    MAX_INJECT_BODY_BYTES / (1024 * 1024)
                );
                return log_and_respond_error(&state, entry, started, message);
            }
        }
    } else if request_has_body(&headers, body.size_hint().exact()) {
        // 无需改写请求体：零缓冲流式直转（请求体长度未知时 reqwest 使用分块编码，
        // 与 legacy 的 clientReq.pipe(upstreamReq) 行为等价）
        request = request.body(reqwest::Body::wrap_stream(
            body.into_data_stream()
                .map(|item| item.map_err(|err| Box::new(err) as BoxError)),
        ));
    }
    // 无请求体的请求（GET/HEAD 等）不设置 body，直接发送

    // 4. 转发请求：上游错误（连接失败/超时/DNS 失败等）返回 502，实例不退出
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            let message = format!("上游请求失败: {err}");
            return log_and_respond_error(&state, entry, started, message);
        }
    };

    // 5. 上游响应：透传状态码与响应头，响应体流式回传（不缓冲）
    let status = upstream.status();
    entry.status = Some(status.as_u16());
    let response_headers = filter_response_headers(upstream.headers());

    let stream = AccessLogStream {
        inner: Box::pin(
            upstream
                .bytes_stream()
                .map(|item| item.map_err(|err| Box::new(err) as BoxError)),
        ),
        sink: Arc::clone(&state.log_sink),
        pending: Some(entry),
        started,
    };

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}

/// 组装转发给上游的请求头：
/// 1. 透传客户端原始请求头（剔除 host 与 hop-by-hop 头）
/// 2. 附加配置的静态请求头（与透传头同名时以静态配置为准）
/// 3. Host 覆盖为目标主机（reqwest/hyper 仅在缺失时才自行设置，不会覆盖这里的值）
/// 4. 移除 content-length（流式路径交给 reqwest 分块，注入路径单独显式设置）
fn build_forward_headers(
    client_headers: &HeaderMap,
    static_headers: &BTreeMap<String, String>,
    target_host: &str,
) -> HeaderMap {
    let mut forward = HeaderMap::new();

    for (name, value) in client_headers.iter() {
        if *name == header::HOST || is_hop_by_hop(name) {
            continue;
        }
        // 同名多值（如多个 cookie）保持原样透传
        forward.append(name.clone(), value.clone());
    }

    for (name, value) in static_headers {
        // 非法的头名/头值直接忽略，避免个别配置项导致整个请求失败
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        // insert 会覆盖同名透传头（静态配置优先）
        forward.insert(name, value);
    }

    if let Ok(host) = HeaderValue::from_str(target_host) {
        forward.insert(header::HOST, host);
    }

    forward.remove(header::CONTENT_LENGTH);
    forward
}

/// 过滤上游响应头：剔除 hop-by-hop 头与长度/编码类头
/// （响应帧由本地 HTTP 服务器重新计算，避免出现冲突或非法响应头）。
fn filter_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();

    for (name, value) in headers.iter() {
        if is_hop_by_hop(name)
            || *name == header::CONTENT_LENGTH
            || *name == header::TRANSFER_ENCODING
        {
            continue;
        }
        filtered.append(name.clone(), value.clone());
    }

    filtered
}

/// 判断请求头是否属于 hop-by-hop 头
fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP_HEADERS.contains(&name.as_str())
}

/// 拼接转发路径：target 的 path 前缀（`/` 视为空，去掉尾部所有 `/`）+ 客户端原始 path+query。
///
/// 与 legacy 一致：例如 target 为 `https://a.com/v1/` 时，请求 `/chat` 转发到 `/v1/chat`。
fn resolve_forward_path(target: &Url, path_and_query: &str) -> String {
    let target_path = target.path();
    let base = if target_path == "/" {
        ""
    } else {
        target_path.trim_end_matches('/')
    };
    format!("{base}{path_and_query}")
}

/// 基于目标 URL 与转发路径生成最终上游 URL（保留 target 的 origin 与百分号编码）
fn build_upstream_url(target: &Url, forward_path: &str) -> Url {
    let mut url = target.clone();
    match forward_path.split_once('?') {
        Some((path, query)) => {
            url.set_path(path);
            url.set_query(Some(query));
        }
        None => {
            url.set_path(forward_path);
            url.set_query(None);
        }
    }
    url
}

/// 目标 Host 头取值：目标主机名（显式且非默认端口时形如 `host:port`，与 legacy 的 URL.host 一致）
fn target_host(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        (None, _) => String::new(),
    }
}

/// 判断客户端请求是否携带请求体：`body_exact_len` 为 body 的精确长度（来自 content-length）。
///
/// - 精确长度为 0：视为无请求体
/// - 精确长度大于 0：有请求体
/// - 长度未知（分块传输）：按是否存在 transfer-encoding 判断
fn request_has_body(headers: &HeaderMap, body_exact_len: Option<u64>) -> bool {
    match body_exact_len {
        Some(0) => false,
        Some(_) => true,
        None => {
            headers.contains_key(header::TRANSFER_ENCODING)
                || !headers.contains_key(header::CONTENT_LENGTH)
        }
    }
}

/// 按协议向请求体注入推理强度默认值（仅当客户端未显式指定时，缺失或为 `null` 视为未指定）。
///
/// - openai：`reasoning_effort` 缺失或为 null 时注入
/// - anthropic：`output_config` 缺失或为 null 时注入，并在 `thinking` 缺失时补 `{type: "enabled"}`
///
/// 返回是否发生了注入；非 JSON 对象（数组、字符串等）不做处理。
fn inject_reasoning_effort(
    body: &mut serde_json::Value,
    protocol: &str,
    reasoning_effort: &str,
) -> bool {
    /// 字段缺失或为 null 均视为「未指定」
    fn is_unspecified(value: Option<&serde_json::Value>) -> bool {
        matches!(value, None | Some(serde_json::Value::Null))
    }

    let Some(object) = body.as_object_mut() else {
        return false;
    };

    match protocol {
        "openai" => {
            if !is_unspecified(object.get("reasoning_effort")) {
                return false;
            }
            object.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(reasoning_effort.to_string()),
            );
            true
        }
        "anthropic" => {
            if !is_unspecified(object.get("output_config")) {
                return false;
            }
            if is_unspecified(object.get("thinking")) {
                object.insert(
                    "thinking".to_string(),
                    serde_json::json!({ "type": "enabled" }),
                );
            }
            object.insert(
                "output_config".to_string(),
                serde_json::json!({ "effort": reasoning_effort }),
            );
            true
        }
        _ => false,
    }
}

/// 构造一条日志基础信息（target 可先留空，转发后补全）
fn base_entry(
    cfg: &InstanceRuntimeConfig,
    method: &Method,
    path: &str,
    target: &str,
) -> AccessLogEntry {
    AccessLogEntry {
        time: now_local_string(),
        instance_id: cfg.instance_id.clone(),
        instance_name: cfg.instance_name.clone(),
        method: method.as_str().to_string(),
        path: path.to_string(),
        target: target.to_string(),
        status: None,
        duration_ms: None,
        error: None,
    }
}

/// 记录一条失败日志（补全耗时与错误原因）并返回 502 响应
fn log_and_respond_error(
    state: &ProxyState,
    mut entry: AccessLogEntry,
    started: Instant,
    message: String,
) -> Response {
    entry.duration_ms = Some(started.elapsed().as_millis() as u64);
    entry.error = Some(message.clone());
    (state.log_sink)(entry);
    bad_gateway(&message)
}

/// 构造 502 响应：body 为 `Bad Gateway: <原因>`，与 legacy 一致
fn bad_gateway(message: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("Bad Gateway: {message}"),
    )
        .into_response()
}

/// 生成当前本地时间字符串（"YYYY-MM-DD HH:mm:ss"，与 legacy 的日志时间格式一致）
fn now_local_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 访问日志流：包装上游响应体，在响应传输结束（正常结束、出错或客户端断开）时发出访问日志。
struct AccessLogStream {
    /// 上游响应体流
    inner: BoxedByteStream,
    /// 访问日志回调
    sink: LogSink,
    /// 待发出的日志条目；取出即表示已发出，保证只记一条
    pending: Option<AccessLogEntry>,
    /// 请求开始时间
    started: Instant,
}

impl AccessLogStream {
    /// 补全耗时后发出日志；`error` 仅在条目本身没有错误信息时写入
    fn emit(&mut self, error: Option<String>) {
        let Some(mut entry) = self.pending.take() else {
            return;
        };
        entry.duration_ms = Some(self.started.elapsed().as_millis() as u64);
        if entry.error.is_none() {
            entry.error = error;
        }
        (self.sink)(entry);
    }
}

impl futures_util::Stream for AccessLogStream {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // 所有字段均为 Unpin，可以安全取得可变引用
        let this = self.get_mut();

        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(None) => {
                this.emit(None);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(err))) => {
                this.emit(Some(format!("上游响应传输中断: {err}")));
                Poll::Ready(Some(Err(err)))
            }
            other => other,
        }
    }
}

impl Drop for AccessLogStream {
    fn drop(&mut self) {
        // 流被提前丢弃（客户端断开或响应被取消）时仍需记录一条日志
        self.emit(Some("响应传输未完成（客户端可能已断开连接）".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use axum::http::Uri;

    /// 测试用上游捕获到的请求
    #[derive(Debug, Clone)]
    struct CapturedRequest {
        method: Method,
        uri: String,
        headers: HeaderMap,
        body: Bytes,
    }

    /// 测试用上游处理函数：记录请求并原样回显请求体
    async fn echo_handler(
        State(captured): State<Arc<Mutex<Vec<CapturedRequest>>>>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Body,
    ) -> Response {
        let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
            .await
            .expect("读取测试上游请求体失败");
        captured.lock().expect("获取锁失败").push(CapturedRequest {
            method,
            uri: uri.to_string(),
            headers,
            body: bytes.clone(),
        });
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Body::from(bytes),
        )
            .into_response()
    }

    /// 构造一个运行时配置
    fn runtime_config(target: &str, protocol: &str, effort: Option<&str>) -> InstanceRuntimeConfig {
        InstanceRuntimeConfig {
            instance_id: "test".to_string(),
            instance_name: "测试实例".to_string(),
            target: target.to_string(),
            protocol: protocol.to_string(),
            reasoning_effort: effort.map(|value| value.to_string()),
            static_headers: BTreeMap::new(),
        }
    }

    /// 启动一个测试用上游服务，返回其监听地址
    async fn spawn_upstream(captured: Arc<Mutex<Vec<CapturedRequest>>>) -> std::net::SocketAddr {
        let router = Router::new().fallback(echo_handler).with_state(captured);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试上游端口失败");
        let addr = listener.local_addr().expect("获取测试上游地址失败");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    /// 启动一个代理服务，返回其监听地址与日志收集容器
    async fn spawn_proxy(
        cfg: InstanceRuntimeConfig,
    ) -> (std::net::SocketAddr, Arc<Mutex<Vec<AccessLogEntry>>>) {
        let logs: Arc<Mutex<Vec<AccessLogEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_logs = Arc::clone(&logs);
        let sink: LogSink = Arc::new(move |entry| {
            sink_logs.lock().expect("获取日志锁失败").push(entry);
        });
        let router = build_router(reqwest::Client::new(), Arc::new(cfg), sink);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定代理端口失败");
        let addr = listener.local_addr().expect("获取代理地址失败");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (addr, logs)
    }

    /// 等待日志出现（响应结束与客户端读到响应体之间存在极短的时序差）
    async fn wait_for_logs(logs: &Arc<Mutex<Vec<AccessLogEntry>>>) -> Vec<AccessLogEntry> {
        for _ in 0..100 {
            {
                let guard = logs.lock().expect("获取日志锁失败");
                if !guard.is_empty() {
                    return guard.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        logs.lock().expect("获取日志锁失败").clone()
    }

    #[test]
    fn 转发头剔除host与hop_by_hop头且静态头优先() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8787"));
        client_headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        client_headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        client_headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("12"));
        client_headers.insert(header::UPGRADE, HeaderValue::from_static("h2c"));
        client_headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("cli"),
        );
        client_headers.insert(header::USER_AGENT, HeaderValue::from_static("test-agent"));

        let mut static_headers = BTreeMap::new();
        static_headers.insert("x-api-key".to_string(), "static".to_string());
        static_headers.insert("Authorization".to_string(), "Bearer token".to_string());

        let forward = build_forward_headers(&client_headers, &static_headers, "api.example.com");

        // host 与 hop-by-hop 头被剔除，最终 Host 为目标主机
        assert_eq!(
            forward
                .get(header::HOST)
                .and_then(|value| value.to_str().ok()),
            Some("api.example.com")
        );
        for name in HOP_BY_HOP_HEADERS {
            assert!(
                !forward.contains_key(name),
                "hop-by-hop 头 {name} 不应被透传"
            );
        }
        assert!(!forward.contains_key(header::CONTENT_LENGTH));
        // 静态头覆盖同名透传头（大小写不敏感）
        assert_eq!(
            forward
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("static")
        );
        assert_eq!(
            forward
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer token")
        );
        // 其它透传头保留
        assert_eq!(
            forward
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok()),
            Some("test-agent")
        );
    }

    #[test]
    fn 转发头host包含非默认端口() {
        let forward =
            build_forward_headers(&HeaderMap::new(), &BTreeMap::new(), "api.example.com:8443");
        assert_eq!(
            forward
                .get(header::HOST)
                .and_then(|value| value.to_str().ok()),
            Some("api.example.com:8443")
        );
    }

    #[test]
    fn 转发路径拼接与legacy一致() {
        let cases = [
            ("https://a.com", "/x?y=1", "/x?y=1"),
            ("https://a.com/", "/chat", "/chat"),
            ("https://a.com/v1/", "/chat", "/v1/chat"),
            ("https://a.com/v1///", "/chat", "/v1/chat"),
            (
                "https://a.com/v1",
                "/chat/completions?x=1",
                "/v1/chat/completions?x=1",
            ),
            ("https://a.com/v1/", "/chat?a=1&b=%2F", "/v1/chat?a=1&b=%2F"),
            (
                "https://a.com/zen/go/v1/",
                "/chat/completions",
                "/zen/go/v1/chat/completions",
            ),
        ];

        for (target, path_and_query, expected) in cases {
            let url = Url::parse(target).expect("解析目标 URL 失败");
            assert_eq!(
                resolve_forward_path(&url, path_and_query),
                expected,
                "target={target} path={path_and_query}"
            );
        }
    }

    #[test]
    fn 上游url保留路径前缀与百分号编码() {
        let target = Url::parse("https://a.com/v1/").expect("解析目标 URL 失败");
        let forward_path = resolve_forward_path(&target, "/chat?a=1&b=%2F");
        let upstream = build_upstream_url(&target, &forward_path);
        assert_eq!(upstream.as_str(), "https://a.com/v1/chat?a=1&b=%2F");

        // 无 path 前缀时 query 也应保留
        let target = Url::parse("https://a.com").expect("解析目标 URL 失败");
        let forward_path = resolve_forward_path(&target, "/x?y=1");
        let upstream = build_upstream_url(&target, &forward_path);
        assert_eq!(upstream.as_str(), "https://a.com/x?y=1");

        // 无 query 时应清空 target 自带的 query，避免残留
        let target = Url::parse("https://a.com/v1/?stale=1").expect("解析目标 URL 失败");
        let upstream = build_upstream_url(&target, "/v1/chat");
        assert_eq!(upstream.as_str(), "https://a.com/v1/chat");
    }

    #[test]
    fn 目标host仅保留非默认端口() {
        let cases = [
            ("https://a.com/v1", "a.com"),
            ("http://a.com:8787/v1", "a.com:8787"),
            ("https://a.com:443/v1", "a.com"),
            ("http://127.0.0.1:8080", "127.0.0.1:8080"),
        ];
        for (target, expected) in cases {
            let url = Url::parse(target).expect("解析目标 URL 失败");
            assert_eq!(target_host(&url), expected, "target={target}");
        }
    }

    #[test]
    fn 响应头过滤长度与hop_by_hop头() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("12"));
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let filtered = filter_response_headers(&headers);

        assert!(!filtered.contains_key(header::CONTENT_LENGTH));
        assert!(!filtered.contains_key(header::TRANSFER_ENCODING));
        assert!(!filtered.contains_key(header::CONNECTION));
        // 内容编码等头需要保留（响应体是原样透传的）
        assert_eq!(
            filtered
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok()),
            Some("gzip")
        );
        assert_eq!(
            filtered
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn openai协议注入推理强度() {
        // 字段缺失 -> 注入
        let mut body = serde_json::json!({ "model": "gpt" });
        assert!(inject_reasoning_effort(&mut body, "openai", "high"));
        assert_eq!(body["reasoning_effort"], serde_json::json!("high"));

        // 字段为 null -> 注入（比 legacy 更宽松）
        let mut body = serde_json::json!({ "reasoning_effort": null });
        assert!(inject_reasoning_effort(&mut body, "openai", "max"));
        assert_eq!(body["reasoning_effort"], serde_json::json!("max"));

        // 显式指定 -> 不覆盖
        let mut body = serde_json::json!({ "reasoning_effort": "low" });
        assert!(!inject_reasoning_effort(&mut body, "openai", "max"));
        assert_eq!(body["reasoning_effort"], serde_json::json!("low"));
    }

    #[test]
    fn anthropic协议注入输出配置() {
        // output_config 缺失 -> 注入 thinking 与 output_config
        let mut body = serde_json::json!({ "model": "claude" });
        assert!(inject_reasoning_effort(&mut body, "anthropic", "max"));
        assert_eq!(body["thinking"], serde_json::json!({ "type": "enabled" }));
        assert_eq!(
            body["output_config"],
            serde_json::json!({ "effort": "max" })
        );

        // output_config 为 null -> 注入
        let mut body = serde_json::json!({ "output_config": null });
        assert!(inject_reasoning_effort(&mut body, "anthropic", "high"));
        assert_eq!(body["thinking"], serde_json::json!({ "type": "enabled" }));
        assert_eq!(
            body["output_config"],
            serde_json::json!({ "effort": "high" })
        );

        // 已有 output_config -> 不覆盖
        let mut body = serde_json::json!({ "output_config": { "effort": "low" } });
        assert!(!inject_reasoning_effort(&mut body, "anthropic", "max"));
        assert_eq!(
            body["output_config"],
            serde_json::json!({ "effort": "low" })
        );

        // 已有 thinking 但缺少 output_config -> 保留原 thinking，仅注入 output_config
        let mut body =
            serde_json::json!({ "thinking": { "type": "disabled", "budget_tokens": 1024 } });
        assert!(inject_reasoning_effort(&mut body, "anthropic", "max"));
        assert_eq!(
            body["thinking"],
            serde_json::json!({ "type": "disabled", "budget_tokens": 1024 })
        );
        assert_eq!(
            body["output_config"],
            serde_json::json!({ "effort": "max" })
        );
    }

    #[test]
    fn 非对象json不做注入() {
        let mut body = serde_json::json!([1, 2, 3]);
        assert!(!inject_reasoning_effort(&mut body, "openai", "max"));
        let mut body = serde_json::json!("text");
        assert!(!inject_reasoning_effort(&mut body, "anthropic", "max"));
        let mut body = serde_json::json!(null);
        assert!(!inject_reasoning_effort(&mut body, "openai", "max"));
    }

    #[test]
    fn 未支持的协议不做注入() {
        let mut body = serde_json::json!({ "model": "x" });
        assert!(!inject_reasoning_effort(&mut body, "gemini", "max"));
        assert_eq!(body, serde_json::json!({ "model": "x" }));
    }

    #[test]
    fn 请求体判定覆盖常见场景() {
        let mut headers = HeaderMap::new();
        // GET 无 content-length，body 长度为 0
        assert!(!request_has_body(&headers, Some(0)));
        // POST 带 content-length
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("10"));
        assert!(request_has_body(&headers, Some(10)));
        // POST 空 body
        assert!(!request_has_body(&headers, Some(0)));
        // 分块请求体（长度未知但存在 transfer-encoding）
        let mut headers = HeaderMap::new();
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        assert!(request_has_body(&headers, None));
        // HTTP/2 场景：无 content-length 且长度未知，按有请求体处理
        assert!(request_has_body(&HeaderMap::new(), None));
    }

    #[tokio::test]
    async fn 集成_转发路径请求头与流式响应() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let upstream_addr = spawn_upstream(Arc::clone(&captured)).await;
        let (proxy_addr, logs) = spawn_proxy(runtime_config(
            &format!("http://{upstream_addr}/zen/go/v1/"),
            "openai",
            None,
        ))
        .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{proxy_addr}/chat/completions?flag=1"))
            .header("x-custom", "hello")
            .header(header::CONTENT_TYPE, "application/json")
            .body(r#"{"model":"gpt"}"#)
            .send()
            .await
            .expect("请求代理失败");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.expect("读取代理响应失败");
        assert_eq!(body, r#"{"model":"gpt"}"#);

        let requests: Vec<CapturedRequest> = captured.lock().expect("获取锁失败").clone();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.uri, "/zen/go/v1/chat/completions?flag=1");
        assert_eq!(request.body, Bytes::from_static(br#"{"model":"gpt"}"#));
        // Host 被改写为目标来源（127.0.0.1:端口），且只出现一次
        let hosts: Vec<String> = request
            .headers
            .get_all(header::HOST)
            .iter()
            .filter_map(|value| value.to_str().ok().map(|value| value.to_string()))
            .collect();
        assert_eq!(hosts, vec![upstream_addr.to_string()]);
        // 自定义头透传
        assert_eq!(
            request
                .headers
                .get("x-custom")
                .and_then(|value| value.to_str().ok()),
            Some("hello")
        );
        // 未注入时请求体零缓冲直转：content-length 被剔除，由 reqwest 分块发送
        assert!(!request.headers.contains_key(header::CONTENT_LENGTH));

        let logs = wait_for_logs(&logs).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, Some(200));
        assert_eq!(logs[0].method, "POST");
        assert_eq!(logs[0].path, "/chat/completions?flag=1");
        assert_eq!(
            logs[0].target,
            format!("http://{upstream_addr}/zen/go/v1/chat/completions?flag=1")
        );
        assert!(logs[0].error.is_none(), "不应有错误: {:?}", logs[0].error);
    }

    #[tokio::test]
    async fn 集成_注入推理强度并显式设置content_length() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let upstream_addr = spawn_upstream(Arc::clone(&captured)).await;
        let (proxy_addr, logs) = spawn_proxy(runtime_config(
            &format!("http://{upstream_addr}/v1/"),
            "openai",
            Some("high"),
        ))
        .await;

        let client = reqwest::Client::new();
        // 显式指定 reasoning_effort: null，用于验证「null 也视为未指定」的注入行为
        let payload = serde_json::json!({ "model": "gpt", "reasoning_effort": null }).to_string();
        let response = client
            .post(format!("http://{proxy_addr}/chat"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .expect("请求代理失败");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.expect("读取代理响应失败");

        let requests: Vec<CapturedRequest> = captured.lock().expect("获取锁失败").clone();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.uri, "/v1/chat");

        // 注入后请求体应包含 reasoning_effort=high
        let echoed: serde_json::Value = serde_json::from_str(&body).expect("响应不是 JSON");
        assert_eq!(echoed["reasoning_effort"], serde_json::json!("high"));
        let sent: serde_json::Value =
            serde_json::from_slice(&request.body).expect("请求体不是 JSON");
        assert_eq!(sent["reasoning_effort"], serde_json::json!("high"));
        // 缓冲模式下显式设置 content-length，且等于注入后的字节长度
        let expected_length = request.body.len().to_string();
        assert_eq!(
            request
                .headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok()),
            Some(expected_length.as_str())
        );
        assert!(!request.headers.contains_key(header::TRANSFER_ENCODING));

        let logs = wait_for_logs(&logs).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, Some(200));
        assert_eq!(logs[0].target, format!("http://{upstream_addr}/v1/chat"));
    }
}
