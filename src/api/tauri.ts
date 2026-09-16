// 对 @tauri-apps/api 的统一封装：Tauri 命令调用与事件订阅
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppConfig,
  ConfigPayload,
  InstanceInfo,
  InstanceStateEvent,
  LogEntry,
  SaveConfigResult,
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
 * 单个实例失败不中断其余实例，此时会以中文原因拒绝。
 */
export function startAllInstances(): Promise<void> {
  return invoke<void>('start_all_instances');
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
