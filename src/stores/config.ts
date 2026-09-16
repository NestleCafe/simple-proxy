// 配置 store：负责配置读写、实例运行状态维护与后端事件订阅
import { computed, h, ref } from 'vue';
import { defineStore } from 'pinia';
import { ElMessage, ElNotification } from 'element-plus';
import {
  getConfig,
  listInstances,
  onProxyLog,
  onInstanceState,
  restartInstance,
  saveConfig,
  startAllInstances,
  startInstance,
  stopAllInstances,
  stopInstance,
} from '../api/tauri';
import type { AppConfig, InstanceInfo, InstanceStateEvent, LogEntry, ProxyInstance } from '../types';
import { useLogsStore } from './logs';

// 模块级标记：确保后端事件只订阅一次
let listenersInitialized = false;

/**
 * 把捕获到的异常转换为可读的错误信息。
 * @param err 捕获到的异常
 * @returns 错误信息文本
 */
function toErrorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export const useConfigStore = defineStore('config', () => {
  // 已加载的配置，未加载时为 null
  const config = ref<AppConfig | null>(null);
  // 实例运行时列表（含运行状态）
  const instances = ref<InstanceInfo[]>([]);
  // 是否正在加载配置
  const loading = ref(false);
  // 配置加载警告（config.json 损坏时的提示）
  const loadWarning = ref<string | null>(null);
  // 是否正在保存配置
  const saving = ref(false);

  // 正在运行的实例数量（反映实际监听状态）
  const runningCount = computed(
    () => instances.value.filter((item) => item.state === 'running').length,
  );

  /**
   * 根据实例 id 获取监听端口号（用于多条提示文案拼接），找不到实例时回退为 id 文本。
   * @param id 实例 id
   * @returns 端口号文本或 id
   */
  function resolveInstancePort(id: string): string {
    const fromInstances = instances.value.find((item) => item.id === id);
    if (fromInstances) {
      return String(fromInstances.httpPort);
    }
    const fromConfig = config.value?.proxies.find((item) => item.id === id);
    return fromConfig ? String(fromConfig.httpPort) : id;
  }

  /** 重新拉取实例列表并覆盖本地状态 */
  async function refreshInstances(): Promise<void> {
    instances.value = await listInstances();
  }

  /** 加载配置与实例列表；配置损坏时以通知形式提示 */
  async function load(): Promise<void> {
    loading.value = true;
    try {
      const payload = await getConfig();
      config.value = payload;
      loadWarning.value = payload.loadWarning ?? null;
      if (loadWarning.value) {
        ElNotification({
          title: '配置加载警告',
          message: loadWarning.value,
          type: 'warning',
          duration: 8000,
        });
      }
      await refreshInstances();
    } catch (err) {
      ElMessage.error(`加载配置失败：${toErrorMessage(err)}`);
    } finally {
      loading.value = false;
    }
  }

  /**
   * 保存配置：提示需要重启的实例，并刷新配置与实例列表。
   * @param next 待保存的配置
   */
  async function save(next: AppConfig): Promise<void> {
    saving.value = true;
    try {
      const result = await saveConfig(next);
      if (result.restartRequired.length > 0) {
        const ports = result.restartRequired.map(resolveInstancePort).join('、');
        ElMessage.warning(`端口 ${ports} 需重启后生效`);
      } else {
        ElMessage.success('配置已保存');
      }
      await load();
    } catch (err) {
      ElMessage.error(`保存配置失败：${toErrorMessage(err)}`);
    } finally {
      saving.value = false;
    }
  }

  /**
   * 执行实例操作（启动/停止/重启）并刷新实例列表，失败时提示错误。
   * @param id 实例 id
   * @param action 具体的后端操作
   */
  async function runInstanceAction(
    id: string,
    action: (id: string) => Promise<void>,
  ): Promise<void> {
    try {
      await action(id);
      await refreshInstances();
    } catch (err) {
      const message = toErrorMessage(err);
      // 记录该错误：随后到达的 instance-state 事件不再重复弹出同一文案
      notifiedErrors.set(id, message);
      ElMessage.error(`操作失败：${message}`);
    }
  }

  /**
   * 启动指定实例。
   * @param id 实例 id
   */
  function start(id: string): Promise<void> {
    return runInstanceAction(id, startInstance);
  }

  /**
   * 停止指定实例。
   * @param id 实例 id
   */
  function stop(id: string): Promise<void> {
    return runInstanceAction(id, stopInstance);
  }

  /**
   * 重启指定实例。
   * @param id 实例 id
   */
  function restart(id: string): Promise<void> {
    return runInstanceAction(id, restartInstance);
  }

  /**
   * 执行批量启停操作（全部启动 / 全部停止）并刷新实例列表。
   * @param action 后端批量操作
   * @param label 操作名称，用于失败提示
   */
  async function runBatchAction(action: () => Promise<void>, label: string): Promise<void> {
    let failure: string | null = null;
    try {
      await action();
    } catch (err) {
      // 后端会聚合各实例的失败原因（失败原因中已带端口标识）
      failure = `${label}失败：${toErrorMessage(err)}`;
    }
    try {
      // 部分实例可能已成功启停，无论成败都刷新，保证界面与实际运行状态一致
      await refreshInstances();
    } catch (err) {
      failure ??= `刷新实例状态失败：${toErrorMessage(err)}`;
    }
    if (failure !== null) {
      ElMessage.error(failure);
    }
  }

  /**
   * 停止全部正在运行的实例（不修改各实例的启用开关）。
   */
  function stopAll(): Promise<void> {
    return runBatchAction(stopAllInstances, '停止全部实例');
  }

  /**
   * 启动全部已启用且当前未运行的实例（跳过原本禁用的实例）。
   * 部分实例启动失败（如端口被占用）时，其余实例照常启动，并提示失败明细。
   */
  async function startAll(): Promise<void> {
    try {
      const outcome = await startAllInstances();
      // 失败原因已在下方通知中提示，记录以避免 instance-state 事件重复弹出
      for (const failure of outcome.failures) {
        notifiedErrors.set(failure.id, failure.reason);
      }

      if (outcome.failures.length === 0) {
        if (outcome.startedCount > 0) {
          ElMessage.success(`已启动 ${outcome.startedCount} 个实例`);
        }
      } else {
        ElNotification({
          title: '部分实例启动失败',
          type: outcome.startedCount > 0 ? 'warning' : 'error',
          duration: 0,
          message: h('div', { style: 'line-height: 1.7' }, [
            h(
              'div',
              outcome.startedCount > 0
                ? `其余 ${outcome.startedCount} 个实例已正常启动；以下 ${outcome.failures.length} 个实例未能启动：`
                : `以下 ${outcome.failures.length} 个实例未能启动：`,
            ),
            ...outcome.failures.map((failure) => h('div', failure.reason)),
          ]),
        });
      }
    } catch (err) {
      ElMessage.error(`启动全部实例失败：${toErrorMessage(err)}`);
    } finally {
      // 无论成败都刷新，保证界面与实际运行状态一致
      try {
        await refreshInstances();
      } catch (err) {
        ElMessage.error(`刷新实例状态失败：${toErrorMessage(err)}`);
      }
    }
  }

  /**
   * 切换实例启用状态：写入配置后保存，由 Rust 侧负责立即停止/启动，前端仅保存并刷新。
   * @param instance 目标实例
   * @param enabled 目标启用状态
   */
  async function toggleEnabled(instance: ProxyInstance, enabled: boolean): Promise<void> {
    const current = config.value;
    if (!current) {
      return;
    }
    const target = current.proxies.find((item) => item.id === instance.id);
    if (!target) {
      return;
    }
    target.enabled = enabled;
    await save({
      version: current.version,
      closeBehavior: current.closeBehavior,
      closeBehaviorConfirmed: current.closeBehaviorConfirmed,
      proxies: current.proxies,
    });
  }

  // 每个实例最近一次已提示过的错误信息，避免同一条错误重复弹出提示
  const notifiedErrors = new Map<string, string>();

  /**
   * 就地更新指定实例的状态（供 instance-state 事件回调使用）。
   * 实例进入 error 状态且原因非空时弹出通知（同一错误只提示一次）。
   * @param evt 实例状态事件负载
   */
  function applyInstanceState(evt: InstanceStateEvent): void {
    const target = instances.value.find((item) => item.id === evt.id);
    if (target) {
      target.state = evt.state;
      target.error = evt.error ?? null;
    }

    const error = evt.error ?? null;
    if (evt.state === 'error' && error) {
      // 手动操作失败时 runInstanceAction 已用相同文案提示过，这里通过记录去重
      if (notifiedErrors.get(evt.id) !== error) {
        notifiedErrors.set(evt.id, error);
        ElNotification({
          title: '实例运行异常',
          // 找不到实例（如已被删除）时只展示错误原因
          message: target ? `端口 ${target.httpPort}：${error}` : error,
          type: 'error',
          duration: 8000,
        });
      }
    } else if (evt.state !== 'error') {
      // 恢复正常后清除记录，便于下次同样的错误再次提示
      notifiedErrors.delete(evt.id);
    }
  }

  /** 订阅后端事件：日志转发到 logs store，实例状态就地更新；重复调用不会重复订阅 */
  async function initEventListeners(): Promise<void> {
    if (listenersInitialized) {
      return;
    }
    listenersInitialized = true;
    const logsStore = useLogsStore();
    try {
      await onProxyLog((entry: LogEntry) => {
        logsStore.append(entry);
      });
      await onInstanceState(applyInstanceState);
    } catch (err) {
      listenersInitialized = false;
      ElMessage.error(`事件订阅失败：${toErrorMessage(err)}`);
    }
  }

  return {
    config,
    instances,
    loading,
    loadWarning,
    saving,
    runningCount,
    load,
    save,
    start,
    stop,
    restart,
    stopAll,
    startAll,
    toggleEnabled,
    applyInstanceState,
    initEventListeners,
  };
});
