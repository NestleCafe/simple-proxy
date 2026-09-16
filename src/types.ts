// 前端与 Rust 后端共享的数据契约（字段名与 serde camelCase 保持一致）

export type Protocol = 'openai' | 'anthropic';
export type CloseBehavior = 'minimize' | 'exit';
export type InstanceState = 'running' | 'stopped' | 'error';

export interface ProxyInstance {
  id: string;
  name: string;
  enabled: boolean;
  target: string;
  httpPort: number;
  protocol: Protocol;
  reasoningEffort?: string | null;
  headers: Record<string, string>;
}

export interface AppConfig {
  version: number;
  closeBehavior: CloseBehavior;
  proxies: ProxyInstance[];
}

// get_config 命令返回
export interface ConfigPayload extends AppConfig {
  loadWarning?: string | null; // config.json 损坏时的提示
}

// list_instances 命令返回
export interface InstanceInfo extends ProxyInstance {
  state: InstanceState;
  error?: string | null;
}

// save_config 命令返回
export interface SaveConfigResult {
  restartRequired: string[]; // 需要重启才能生效的实例 id 列表
}

// proxy-log 事件 payload
export interface LogEntry {
  time: string; // "YYYY-MM-DD HH:mm:ss"
  instanceId: string;
  instanceName: string;
  method: string;
  path: string;
  target: string;
  status?: number | null;
  durationMs?: number | null;
  error?: string | null;
}

// instance-state 事件 payload
export interface InstanceStateEvent {
  id: string;
  state: InstanceState;
  error?: string | null;
}
