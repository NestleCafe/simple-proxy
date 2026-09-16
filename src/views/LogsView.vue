<template>
  <div class="logs-view">
    <div class="logs-toolbar">
      <el-select
        :model-value="logsStore.filterInstanceId"
        class="instance-select"
        placeholder="全部实例"
        clearable
        @change="handleFilterChange"
      >
        <el-option
          v-for="instance in configStore.instances"
          :key="instance.id"
          :label="`端口 ${instance.httpPort}`"
          :value="instance.id"
        />
      </el-select>
      <div class="auto-scroll-toggle">
        <el-switch :model-value="logsStore.autoScroll" @change="handleAutoScrollChange" />
        <span class="auto-scroll-label">自动滚动</span>
      </div>
      <el-button type="danger" plain @click="handleClear">
        <el-icon><Delete /></el-icon>
        <span>清空</span>
      </el-button>
      <div class="logs-stats">
        显示 {{ logsStore.filteredEntries.length }} / 共 {{ logsStore.entries.length }} 条
      </div>
    </div>

    <div ref="listRef" class="logs-list">
      <template v-if="logsStore.filteredEntries.length === 0">
        <div class="logs-empty">
          <el-empty description="暂无日志，代理收到请求后会实时显示" :image-size="80" />
        </div>
      </template>
      <template v-else>
        <div
          v-for="(entry, index) in logsStore.filteredEntries"
          :key="`${entry.time}-${entry.instanceId}-${index}`"
          class="log-row"
          :class="{ 'is-info': isInfoEntry(entry) }"
        >
          <span class="log-cell log-time">{{ entry.time }}</span>
          <span class="log-cell log-instance" :title="`端口 ${entry.instancePort}`">{{ entry.instancePort }}</span>
          <span class="log-cell log-method" :class="methodClass(entry.method)">{{ entry.method }}</span>
          <span class="log-cell log-path" :title="entry.path">{{ entry.path }}</span>
          <span class="log-cell log-target" :title="entry.target">{{ entry.target }}</span>
          <span class="log-cell log-status" :class="statusClass(entry)">{{ statusText(entry) }}</span>
          <span class="log-cell log-duration">{{ formatDuration(entry.durationMs) }}</span>
          <span class="log-cell log-error" :title="entry.error ?? ''">{{ entry.error }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { Delete } from '@element-plus/icons-vue';
import { ElMessageBox } from 'element-plus';
import { useConfigStore } from '../stores/config';
import { useLogsStore } from '../stores/logs';
import type { LogEntry } from '../types';

const configStore = useConfigStore();
const logsStore = useLogsStore();

// 日志列表滚动容器引用
const listRef = ref<HTMLDivElement | null>(null);

// HTTP 方法到着色类名的映射，未列出的方法统一使用默认灰色
const methodClassMap: Record<string, string> = {
  GET: 'method-get',
  POST: 'method-post',
  PUT: 'method-put',
  PATCH: 'method-patch',
  DELETE: 'method-delete',
  HEAD: 'method-other',
  OPTIONS: 'method-other',
  INFO: 'method-info',
};

/**
 * 判断是否为系统提示类日志（method 为 INFO）。
 * @param entry 日志条目
 * @returns 是否为系统提示日志
 */
function isInfoEntry(entry: LogEntry): boolean {
  return entry.method === 'INFO';
}

/**
 * 获取方法列的着色类名。
 * @param method HTTP 方法或 INFO
 * @returns 对应的 CSS 类名
 */
function methodClass(method: string): string {
  return methodClassMap[method] ?? 'method-other';
}

/**
 * 获取状态码的着色类名：2xx 绿、3xx 蓝、4xx 橙、5xx 红，无状态码视为失败。
 * @param entry 日志条目
 * @returns 对应的 CSS 类名
 */
function statusClass(entry: LogEntry): string {
  if (isInfoEntry(entry)) {
    return 'status-info';
  }
  const status = entry.status;
  if (status === null || status === undefined) {
    return 'status-error';
  }
  if (status < 300) {
    return 'status-2xx';
  }
  if (status < 400) {
    return 'status-3xx';
  }
  if (status < 500) {
    return 'status-4xx';
  }
  return 'status-5xx';
}

/**
 * 获取状态码展示文本，请求失败（无状态码）时显示 ERR。
 * @param entry 日志条目
 * @returns 状态码文本
 */
function statusText(entry: LogEntry): string {
  if (isInfoEntry(entry)) {
    return '—';
  }
  return entry.status === null || entry.status === undefined ? 'ERR' : String(entry.status);
}

/**
 * 格式化耗时展示，无耗时（如失败的请求）时显示占位符。
 * @param durationMs 耗时（毫秒），可能为空
 * @returns 形如 45ms 的文本
 */
function formatDuration(durationMs: number | null | undefined): string {
  return durationMs === null || durationMs === undefined ? '—' : `${durationMs}ms`;
}

/**
 * 处理实例过滤变化：清空选择时回退为显示全部实例。
 * @param value 下拉框选中的实例 id，清空时为 null/undefined
 */
function handleFilterChange(value: string | null | undefined): void {
  logsStore.setFilter(value ? value : null);
}

/**
 * 处理自动滚动开关变化。
 * @param value 开关的新值
 */
function handleAutoScrollChange(value: boolean | string | number): void {
  logsStore.setAutoScroll(value === true);
}

/** 清空全部日志（先经二次确认） */
async function handleClear(): Promise<void> {
  try {
    await ElMessageBox.confirm('确定要清空全部日志吗？', '清空日志', {
      confirmButtonText: '清空',
      cancelButtonText: '取消',
      type: 'warning',
    });
  } catch {
    // 用户取消，不执行清空
    return;
  }
  logsStore.clear();
}

/** 滚动日志列表到底部（等待 DOM 更新后执行） */
async function scrollToBottom(): Promise<void> {
  if (!logsStore.autoScroll) {
    return;
  }
  await nextTick();
  const container = listRef.value;
  if (container) {
    container.scrollTop = container.scrollHeight;
  }
}

// 日志条数变化时，若开启自动滚动则始终停留在最新一条
watch(
  () => logsStore.filteredEntries.length,
  () => {
    void scrollToBottom();
  },
);
</script>

<style scoped>
.logs-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}

.logs-toolbar {
  display: flex;
  align-items: center;
  flex: 0 0 auto;
  gap: 12px;
}

.instance-select {
  width: 220px;
}

.auto-scroll-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
}

.auto-scroll-label {
  font-size: 13px;
  color: #606266;
}

.logs-stats {
  margin-left: auto;
  font-size: 13px;
  color: #909399;
}

.logs-list {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: auto;
  background-color: #ffffff;
  border: 1px solid #e5e7eb;
  border-radius: 4px;
}

.logs-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
}

.log-row {
  display: flex;
  align-items: center;
  flex: 0 0 auto;
  gap: 12px;
  padding: 3px 10px;
  font-family: Consolas, 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
  color: #303133;
  border-bottom: 1px solid #f2f3f5;
}

.log-row:nth-child(odd) {
  background-color: #fafbfc;
}

.log-row.is-info {
  color: #9ca3af;
  background-color: #f6f8fb;
}

.log-row:hover {
  background-color: #ecf3ff;
}

.log-cell {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.log-time {
  flex: 0 0 138px;
  color: #909399;
}

.log-instance {
  flex: 0 0 110px;
  color: #606266;
}

.log-method {
  flex: 0 0 54px;
  font-weight: 600;
}

.method-get {
  color: #67c23a;
}

.method-post {
  color: #409eff;
}

.method-put {
  color: #e6a23c;
}

.method-patch {
  color: #b88230;
}

.method-delete {
  color: #f56c6c;
}

.method-other {
  color: #909399;
}

.method-info {
  color: #9ca3af;
}

.log-path {
  flex: 1 1 24%;
  min-width: 0;
}

.log-target {
  flex: 1 1 28%;
  min-width: 0;
  color: #909399;
}

.log-status {
  flex: 0 0 46px;
  font-weight: 600;
  text-align: right;
}

.status-2xx {
  color: #67c23a;
}

.status-3xx {
  color: #409eff;
}

.status-4xx {
  color: #e6a23c;
}

.status-5xx {
  color: #f56c6c;
}

.status-error {
  color: #f56c6c;
}

.status-info {
  color: #c0c4cc;
}

.log-duration {
  flex: 0 0 62px;
  color: #909399;
  text-align: right;
}

.log-error {
  flex: 1 1 18%;
  min-width: 0;
  color: #f56c6c;
}
</style>
