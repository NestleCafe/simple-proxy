// simple-proxy 代理实例配置
// 配置为数组：每一项是一个独立的代理实例（监听各自端口），
// 可同时将不同端口转发到不同目标（不同协议、不同上游）。
export default [
  {
    // 转发目标地址（必填）：支持 http/https，可为路径前缀，
    // 客户端请求的路径与查询参数会拼接在其后。
    target: 'https://opencode.ai/zen/go/v1',
    // 本实例的 HTTP 监听端口（需与其它实例不同）。
    httpPort: 8787,
    // 目标协议（可选）：openai（默认）或 anthropic，决定推理强度的注入方式。
    protocol: 'openai',
    // 推理强度（可选）
    // 若客户端请求体未显式指定推理强度相关字段，则按 `protocol` 注入对应字段；
    // 不配置则完全透明转发。
    reasoningEffort: 'max',
    // 附加的静态请求头（键值对），如认证令牌；与客户端透传头同名时以这里为准。
    headers: {
      'x-opencode-session': 'AGENT-OPENCODE-SESSION',
    }
  },
  {
    target: 'https://opencode.ai/zen/go',
    httpPort: 8788,
    protocol: 'anthropic',
    reasoningEffort: 'max',
    headers: {
      'x-opencode-session': 'AGENT-OPENCODE-SESSION',
    }
  }
];
