// 日志 store：保存代理请求日志（环形缓冲，最多 1000 条）并提供过滤与自动滚动开关
import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import type { LogEntry } from '../types';

/** 日志条数上限，超出后丢弃最旧记录 */
const MAX_ENTRIES = 1000;

export const useLogsStore = defineStore('logs', () => {
  // 日志条目列表（最多 MAX_ENTRIES 条）
  const entries = ref<LogEntry[]>([]);
  // 当前过滤的实例 id，null 表示全部实例
  const filterInstanceId = ref<string | null>(null);
  // 是否自动滚动到最新日志
  const autoScroll = ref(true);

  // 按实例过滤后的日志条目
  const filteredEntries = computed(() =>
    filterInstanceId.value === null
      ? entries.value
      : entries.value.filter((entry) => entry.instanceId === filterInstanceId.value),
  );

  /**
   * 追加一条日志；超过上限时丢弃最旧的记录（环形缓冲）。
   * @param entry 日志条目
   */
  function append(entry: LogEntry): void {
    entries.value.push(entry);
    const overflow = entries.value.length - MAX_ENTRIES;
    if (overflow > 0) {
      entries.value.splice(0, overflow);
    }
  }

  /** 清空全部日志 */
  function clear(): void {
    entries.value = [];
  }

  /**
   * 设置实例过滤条件。
   * @param id 实例 id，传 null 表示显示全部实例的日志
   */
  function setFilter(id: string | null): void {
    filterInstanceId.value = id;
  }

  /**
   * 设置是否自动滚动到最新日志。
   * @param v 是否自动滚动
   */
  function setAutoScroll(v: boolean): void {
    autoScroll.value = v;
  }

  return {
    entries,
    filterInstanceId,
    autoScroll,
    filteredEntries,
    append,
    clear,
    setFilter,
    setAutoScroll,
  };
});
