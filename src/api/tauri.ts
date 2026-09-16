// 对 @tauri-apps/api 的统一封装：Tauri 命令调用与事件订阅
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppConfig,
  CloseBehavior,
  ConfigPayload,
  InstanceInfo,
  InstanceStateEvent,
  LogEntry,
  SaveConfigResult,
  StartAllOutcome,
} from '../types';

/**
 * 读取应用配置（config.json），若文件损坏会通过 loadWarning 返回提示。
 * @returns 配置内容与加载警告
 */
export function getConfig(): Promise<ConfigPayload> {
  return invoke<ConfigPayload>('get_config');
}

/**
 * 获取全部代理实例的运行时信息（含运行状态）。
 * @returns 实例信息列表
 */
export function listInstances(): Promise<InstanceInfo[]> {
  return invoke<InstanceInfo[]>('list_instances');
}

/**
 * 保存应用配置；返回需要重启才能生效的实例 id 列表。
 * @param config 待保存的完整配置
 * @returns 保存结果（restartRequired）
 */
export function saveConfig(config: AppConfig): Promise<SaveConfigResult> {
  return invoke<SaveConfigResult>('save_config', { config });
}

/**
 * 启动指定代理实例。
 * @param id 实例 id
 */
export function startInstance(id: string): Promise<void> {
  return invoke<void>('start_instance', { id });
}

/**
 * 停止指定代理实例。
 * @param id 实例 id
 */
export function stopInstance(id: string): Promise<void> {
  return invoke<void>('stop_instance', { id });
}

/**
 * 重启指定代理实例。
 * @param id 实例 id
 */
export function restartInstance(id: string): Promise<void> {
  return invoke<void>('restart_instance', { id });
}

/**
 * 停止全部正在运行的代理实例（不修改各实例的启用开关）。
 * 单个实例失败不中断其余实例，此时会以中文原因拒绝。
 */
export function stopAllInstances(): Promise<void> {
  return invoke<void>('stop_all_instances');
}

/**
 * 启动全部已启用且当前未运行的代理实例（跳过原本禁用的实例）。
 * 单个实例失败（如端口被占用）不中断其余实例，返回成功数量与失败明细。
 */
export function startAllInstances(): Promise<StartAllOutcome> {
  return invoke<StartAllOutcome>('start_all_instances');
}

/**
 * 订阅代理请求日志事件（proxy-log）。
 * @param handler 收到日志条目时的回调
 * @returns 取消订阅函数
 */
export function onProxyLog(handler: (entry: LogEntry) => void): Promise<UnlistenFn> {
  return listen<LogEntry>('proxy-log', (event) => handler(event.payload));
}

/**
 * 订阅实例状态变更事件（instance-state）。
 * @param handler 收到状态变更时的回调
 * @returns 取消订阅函数
 */
export function onInstanceState(handler: (state: InstanceStateEvent) => void): Promise<UnlistenFn> {
  return listen<InstanceStateEvent>('instance-state', (event) => handler(event.payload));
}

/**
 * 订阅「关闭窗口」询问事件（close-requested）：尚未确认关窗行为时，
 * 后端会阻止关闭并推送该事件，由前端弹窗询问用户。
 * @param handler 收到关闭请求时的回调
 * @returns 取消订阅函数
 */
export function onCloseRequested(handler: () => void): Promise<UnlistenFn> {
  return listen('close-requested', () => handler());
}

/**
 * 响应「关闭窗口」询问：按所选行为执行（隐藏到托盘 / 退出应用）。
 * @param behavior 关窗行为：minimize（最小化到托盘）/ exit（直接退出应用）
 * @param remember 是否记住本次选择（写入配置，之后关窗不再询问）
 */
export function resolveCloseRequest(behavior: CloseBehavior, remember: boolean): Promise<void> {
  return invoke<void>('resolve_close_request', { behavior, remember });
}
