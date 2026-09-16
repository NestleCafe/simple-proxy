// 前端与 Rust 后端共享的数据契约（字段名与 serde camelCase 保持一致）

export type Protocol = 'openai' | 'anthropic';
export type CloseBehavior = 'minimize' | 'exit';
export type InstanceState = 'running' | 'stopped' | 'error';

export interface ProxyInstance {
  id: string;
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
  /** 是否已确认过关窗行为：false 时首次关闭窗口需弹窗询问 */
  closeBehaviorConfirmed: boolean;
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
  restartRequired: string[]; // 需要重启的实例 id 列表
}

// start_all_instances 命令返回：批量启动结果（含失败明细）
export interface StartAllOutcome {
  startedCount: number; // 已成功启动的实例数量
  failures: StartFailure[]; // 启动失败的实例明细
}

// 批量启动中单个实例的失败信息
export interface StartFailure {
  id: string;
  reason: string; // 失败原因（如「监听端口 8787 失败：端口已被其他程序占用」）
}

// proxy-log 事件 payload
export interface LogEntry {
  time: string; // "YYYY-MM-DD HH:mm:ss"
  instanceId: string;
  instancePort: number;
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
