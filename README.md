# simple-proxy

面向 AI Agent 场景的轻量反向代理（Tauri 2 桌面应用）：将 Agent 的请求转发到目标 API，自动附加配置的请求头（如认证令牌），并按协议注入推理强度（reasoning effort）；提供图形界面管理多实例、启停控制与实时日志。

```
AI Agent ──http──▶ 代理实例(:8787) ──https──▶ 目标 API
```

## 功能特性

- **多实例并行**：每个实例监听独立端口，可同时将不同端口转发到不同目标（不同协议、不同上游），满足同一 Agent 按端口对接多家 API
- **图形界面管理**：实例的增删改查、启用/禁用、启动/停止/重启，以及一键全停/全启的「总开关」；保存运行中实例的修改后提示「需重启生效」
- **实时日志**：实例页下方实时展示访问日志（时间/实例/方法/路径/状态码/耗时），支持状态码着色、自动滚动与一键清空
- **托盘常驻**：默认点击关闭按钮时最小化到系统托盘（代理继续转发），托盘菜单提供「显示窗口 / 退出」；可在设置中改为直接退出
- **协议化推理强度注入**：按协议注入推理强度默认值（OpenAI 注入 `reasoning_effort`；Anthropic 注入 `thinking` + `output_config.effort`），客户端已显式指定时不覆盖，未配置时完全透明转发
- **请求头处理**：透传客户端原始请求头（过滤 hop-by-hop 头），附加静态请求头（同名时以静态配置为准），自动将 Host 替换为目标地址
- **响应原样返回**：状态码、响应头与响应体流式透传
- **流式转发**：请求/响应体双向流式传输，内存友好；keep-alive 连接复用
- **配置容错**：config.json 损坏时自动备份为 config.json.bak 并以空配置启动，界面提示
- **绿色版**：免安装单文件，配置文件位于 exe 同目录，随 exe 一起拷贝即可迁移
- **单实例运行**：重复启动只会激活已有窗口并自动退出，避免多开造成端口冲突（仅发布版生效）

## 技术栈

- 后端（Rust）：Tauri 2 + axum（HTTP 服务）+ reqwest（上游客户端，连接池复用）
- 前端：Vue 3 + TypeScript + Element Plus + Pinia + Vite

## 环境要求

- Node.js >= 20.19
- Rust 稳定版工具链（`stable-x86_64-pc-windows-msvc`）
- Visual Studio Build Tools（C++ 生成工具）
- Windows 10/11（WebView2 运行时，系统自带）

## 快速开始

```bash
# 安装前端依赖
npm install

# 开发模式：启动前端与 Rust，打开应用窗口（改代码即时热重载）
npm run dev

# 打包绿色版（仅产出单个 exe，配置随 exe 同目录）
npm run build
```

构建产物：`src-tauri/target/release/simple-proxy.exe`（免安装单文件；WebView2 为系统自带运行时）

> 仅调试前端界面（不启动 Tauri 窗口）可用 `npm run dev:web`。

## 使用说明

### 实例

- 「实例」页展示全部代理实例（端口 / 目标 / 协议 / 推理强度 / 状态）
- 右上角「新建实例」打开配置对话框；每行可编辑、删除、启动、停止、重启
- 点击监听端口下方的完整地址（`http://127.0.0.1:端口`）即可复制，便于直接粘贴为 Agent 的 base_url
- 应用启动后不会自动启动实例（避免开机即占用端口）：需点击「开始」或使用总开关手动启动
- 「启用」开关控制实例是否监听：关闭立即停止监听，重新打开立即启动
- 工具栏「总开关」：关闭即停止全部运行中实例，打开即启动全部已启用实例（不修改各实例的启用开关，不写配置）
- 实例启动失败（如端口被占用）会进入「错误」状态并弹出提示；一键启动时端口被占用的实例会被跳过，其余实例正常启动，并列出未能启动的实例及原因

### 实时日志

- 存在运行中实例时，实例页下方直接显示实时日志（时间 / 端口 / 方法 / 路径 / 状态码 / 耗时），最多保留最近 1000 条（超出丢弃最旧）
- 支持状态码着色（2xx 绿 / 3xx 蓝 / 4xx 橙 / 5xx 红 / 失败 ERR）、自动滚动开关、一键清空、折叠/展开
- （独立日志页入口暂时隐藏，页面代码保留在 `src/views/LogsView.vue`，需要时可在 `src/App.vue` 取消注释恢复）

### 设置

- 「关闭窗口行为」：
  - 最小化到系统托盘（默认）：关闭窗口后隐藏，代理继续转发，可通过托盘菜单「显示窗口」恢复
  - 直接退出应用：关闭窗口即退出，全部实例停止
- 首次点击窗口关闭按钮时会弹窗询问（最小化到系统托盘 / 直接退出应用）：勾选「记住我的选择」后写入配置，之后关窗不再询问；不勾选则仅本次生效，下次关窗继续询问（点「取消」则窗口保持打开）
- 在设置页保存过「关闭窗口行为」即视为已确认，之后关窗不再弹窗；修改后立即生效（无需重启应用）

## 配置说明

配置持久化在**可执行文件同目录**的 `config.json`（绿色版：配置随 exe 走；一般无需手工编辑，界面修改会自动写盘）：

```
<exe 所在目录>\config.json
```

> 开发模式下配置位于 `src-tauri/target/debug/config.json`；换机器时把该文件与 exe 一起拷贝即可迁移配置。

结构示例：

```json
{
  "version": 1,
  "closeBehavior": "minimize",
  "proxies": [
    {
      "id": "3f2a9c10",
      "enabled": true,
      "target": "https://api.example.com/v1",
      "httpPort": 8787,
      "protocol": "openai",
      "reasoningEffort": "max",
      "headers": {
        "Authorization": "Bearer YOUR_TOKEN"
      }
    }
  ]
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | string | 实例唯一标识（新建时自动生成） |
| `enabled` | boolean | 是否启用；为 false 时不监听 |
| `target` | string | 转发目标地址（必填），支持 http/https，可为路径前缀，客户端路径与查询参数拼接在其后 |
| `httpPort` | number | 实例监听端口（1–65535，实例间唯一） |
| `protocol` | string | 目标协议：`openai`（默认）或 `anthropic`，决定推理强度注入方式 |
| `reasoningEffort` | string | 推理强度默认值（如 `max`、`high`）；不配置则完全透明转发 |
| `headers` | object | 附加的静态请求头，与透传头同名时以这里为准 |
| `closeBehavior` | string | 关窗行为：`minimize`（默认）/ `exit` |
| `closeBehaviorConfirmed` | boolean | 是否已确认过关窗行为；为 false（含旧配置缺省）时首次关闭窗口会弹窗询问，确认为 true 时按 `closeBehavior` 直接执行 |

推理强度注入行为（仅当相关字段「缺失或为 null」时注入，客户端显式指定时不覆盖）：

- `protocol` 为 `openai`：注入请求体顶层字段 `reasoning_effort`
- `protocol` 为 `anthropic`：注入 `output_config: { "effort": "max" }`，并在 `thinking` 缺失时设 `{ "type": "enabled" }`

> 配置文件损坏时，应用会自动将其备份为 `config.json.bak` 并以空配置启动，界面弹出提示。

## 转发行为

- 实例仅监听本机回环地址（`127.0.0.1`），不对局域网暴露；启动前会探测端口占用（IPv4/IPv6 回环均检测），已被其他程序占用时拒绝启动并提示
- 转发路径 = target 去掉尾斜杠的路径 + 客户端原始 path 与 query
  （例：target 为 `https://api.example.com/v1` 时，请求 `/chat` 转发到 `https://api.example.com/v1/chat`）
- 请求头：过滤 hop-by-hop 头（connection / keep-alive / proxy-authenticate / proxy-authorization / te / trailer / transfer-encoding / upgrade）与客户端 Host，Host 重写为目标主机；静态请求头与透传头同名时覆盖
- 未配置 `reasoningEffort`（或协议不支持）时请求体流式直转（零缓冲）；需要注入时缓冲请求体，注入后显式设置 Content-Length
- 请求体不是合法 JSON 时跳过注入、原样转发并记录日志
- 上游连接失败/超时返回 `502 Bad Gateway`，代理进程不退出；客户端断开时中止上游请求；keep-alive 连接复用

## 目录结构

```
simple-proxy/
├── src/                        # Vue 3 前端
│   ├── api/tauri.ts            # Tauri command 与事件订阅封装
│   ├── stores/                 # Pinia（config / logs）
│   └── views/                  # 实例 / 日志 / 设置视图与配置对话框
├── src-tauri/                  # Tauri 2 + Rust 后端
│   └── src/
│       ├── config.rs           # config.json 读写与校验
│       ├── proxy.rs            # 转发核心（头处理 / 路径拼接 / 注入 / 流式转发）
│       ├── engine.rs           # 实例生命周期与配置应用
│       ├── logbus.rs           # 日志与状态事件推送
│       └── tray.rs             # 系统托盘与关窗行为
├── index.html
└── package.json
```

## 许可

MIT
