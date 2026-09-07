# simple-proxy

基于 Node.js 原生模块（零依赖）的轻量反向代理：将 AI Agent 的请求转发到目标 API，并在转发时自动附加配置的请求头（如认证令牌），目标 API 的响应会原样返回给 Agent。

## 功能特性

- **单一目标转发**：客户端请求的路径与查询参数保留，并拼接在 `target` 的路径之后。例如 `target` 为 `https://api.example.com/v1` 时，请求 `/chat` 会转发到 `https://api.example.com/v1/chat`
- **请求头处理**：
  - 透传客户端原始请求头（如 `Cookie`、`User-Agent` 等）
  - 附加 `proxy.config.json` 中配置的静态请求头，同名时以静态配置为准
  - 自动替换 `Host` 为目标地址，过滤 hop-by-hop 请求头
- **响应原样返回**：目标 API 的响应（状态码、响应头、响应体）流式透传给客户端，Agent 可正常拿到响应
- **流式转发**：请求/响应体双向流式传输，内存友好；连接复用（keep-alive）
- **协议化推理强度注入**：通过 `protocol` + `reasoningEffort` 配置，按目标协议在请求体中自动注入推理强度默认值（OpenAI 协议注入 `reasoning_effort`，Anthropic 协议注入 `thinking` + `output_config.effort`）；客户端已显式指定时保持尊重，未配置时完全透明转发
- **结构化访问日志**：输出请求方法、路径、目标地址、状态码与耗时
- **一键启动**：Windows 下提供 `start.bat` 脚本

## 目录结构

```
simple-proxy/
├── proxy-server.js        # 反向代理核心实现（零依赖）
├── proxy.config.json      # 代理配置（目标地址 / 端口 / 协议 / 推理强度 / 附加请求头）
├── start.bat              # Windows 一键启动脚本
└── package.json           # 项目配置
```

## 典型用途

以 AI Agent 场景为例：Agent 的 `base_url` 配置的是代理地址（如 `http://127.0.0.1:8787`），代理在转发请求到真实 API 时自动塞入 `Authorization` 等请求头，从而无需在 Agent 端暴露密钥。

```
AI Agent ──http──▶ 代理(:8787) ──https──▶ 目标 API
```

## 快速开始

### 环境要求

- Node.js >= 14

### 启动

方式一（推荐，Windows）：

```bat
start.bat
```

方式二：

```bash
npm start
# 或
node proxy-server.js
```

启动后将 AI Agent 的接口地址（base_url）指向 `http://localhost:8787` 即可。

## 配置说明

所有配置集中在根目录的 [proxy.config.json](proxy.config.json) 中：

| 配置项 | 类型 | 说明 |
| --- | --- | --- |
| `target` | string | 转发目标地址（必填），支持 http/https，可为路径前缀（如 `https://api.example.com/v1`），客户端路径会拼接在其后 |
| `httpPort` | number | HTTP 监听端口 |
| `protocol` | string | 目标协议，`openai`（默认）或 `anthropic`，用于决定推理强度的注入方式 |
| `reasoningEffort` | string | 推理强度默认值（如 `max`、`high`）。配置后，若客户端请求体未显式指定推理强度相关字段，则按 `protocol` 注入对应字段；不配置则完全透明转发 |
| `headers` | object | 需要附加的静态请求头（键值对），如 `Authorization` |

推理强度注入行为：

- `protocol` 为 `openai` 时，注入请求体顶层字段 `reasoning_effort`（如 `"max"`）
- `protocol` 为 `anthropic` 时，注入 `thinking: { "type": "adaptive" }` 与顶层字段 `output_config: { "effort": "max" }`
- 客户端已显式传入对应字段（`reasoning_effort` / `thinking` / `output_config`）时不覆盖
- 未配置 `reasoningEffort` 或 `protocol` 为不支持的取值时，请求体不做任何改写，保持原样流式转发

### 配置示例

```json
{
  "target": "https://api.example.com",
  "httpPort": 8787,
  "protocol": "openai",
  "reasoningEffort": "max",
  "headers": {
    "Authorization": "Bearer YOUR_TOKEN",
    "X-Proxy-Source": "simple-proxy"
  }
}
```

> 修改配置文件后需重启服务生效。

## 使用示例

```bash
# 通过代理请求目标地址
curl http://localhost:8787/api/users

# 透传自定义请求头
curl -H "X-My-Trace: abc123" http://localhost:8787/hello
```

代理日志输出示例：

```
[2026-09-07T10:42:04.979Z] [GET] /hello?name=world => https://api.example.com/hello?name=world 状态码 200 耗时 45ms
```
