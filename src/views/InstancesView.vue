<template>
  <div class="instances-view">
    <div class="instances-toolbar">
      <el-input
        v-model="keyword"
        clearable
        placeholder="搜索端口 / 目标 / 协议"
        :prefix-icon="Search"
        class="search-input"
      />
      <div class="toolbar-actions">
        <el-tooltip content="关闭将停止全部实例；打开将启动全部已启用的实例" placement="top">
          <div class="master-switch">
            <el-switch
              :model-value="hasRunning"
              :loading="switchingAll"
              :disabled="configStore.instances.length === 0"
              @change="handleToggleAll"
            />
            <span class="master-switch-label">总开关</span>
          </div>
        </el-tooltip>
        <el-button type="primary" @click="handleCreate">
          <el-icon><Plus /></el-icon>
          <span>新建实例</span>
        </el-button>
      </div>
    </div>

    <div class="instances-table-wrap">
      <el-table
        v-loading="configStore.loading"
        :data="filteredInstances"
        row-key="id"
        height="100%"
        class="instances-table"
      >
        <el-table-column label="监听端口" width="180">
          <template #default="{ row }">
            <div class="port-cell">
              <span class="port-text">{{ row.httpPort }}</span>
              <el-button
                link
                type="primary"
                size="small"
                class="port-address"
                title="点击复制"
                @click="handleCopyAddress(row.httpPort)"
              >
                {{ resolveAddress(row.httpPort) }}
              </el-button>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="target" label="目标地址" min-width="220" show-overflow-tooltip />
        <el-table-column label="协议" width="110">
          <template #default="{ row }">
            <el-tag :type="row.protocol === 'anthropic' ? 'warning' : 'primary'" effect="plain">
              {{ row.protocol }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="推理强度" width="110">
          <template #default="{ row }">
            <span class="effort-text">{{ row.reasoningEffort || '—' }}</span>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <el-tooltip
              v-if="row.state === 'error'"
              :content="row.error || '实例运行异常'"
              placement="top"
            >
              <el-tag type="danger">错误</el-tag>
            </el-tooltip>
            <el-tag v-else :type="resolveStateMeta(row.state).type">
              {{ resolveStateMeta(row.state).label }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="启用" width="90">
          <template #default="{ row }">
            <el-switch
              v-model="row.enabled"
              :loading="togglingIds.includes(row.id)"
              @change="handleToggleEnabled(row, $event)"
            />
          </template>
        </el-table-column>
        <el-table-column label="操作" header-align="center" width="250" fixed="right">
          <template #default="{ row }">
            <el-tooltip
              :content="row.enabled === false ? '实例已禁用，请先启用' : ''"
              :disabled="row.enabled !== false"
              placement="top"
            >
              <span class="action-item">
                <el-button
                  link
                  type="primary"
                  size="small"
                  :disabled="row.enabled === false || row.state === 'running'"
                  @click="configStore.start(row.id)"
                >
                  启动
                </el-button>
              </span>
            </el-tooltip>
            <el-button
              link
              type="primary"
              size="small"
              :disabled="row.state !== 'running'"
              @click="configStore.stop(row.id)"
            >
              停止
            </el-button>
            <el-button
              link
              type="primary"
              size="small"
              :disabled="row.state !== 'running'"
              @click="configStore.restart(row.id)"
            >
              重启
            </el-button>
            <el-button link type="primary" size="small" @click="handleEdit(row)">编辑</el-button>
            <el-button link type="danger" size="small" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="emptyDescription" :image-size="80" />
        </template>
      </el-table>
    </div>

    <!-- 实时日志面板：仅在有实例运行时显示 -->
    <div v-if="hasRunning" class="live-logs">
      <div class="live-logs-header">
        <span class="live-logs-title">实时日志</span>
        <span class="live-logs-count">显示 {{ liveLogs.length }} / 共 {{ logsStore.entries.length }} 条</span>
        <div class="live-logs-actions">
          <el-switch :model-value="logsStore.autoScroll" @change="handleAutoScrollChange" />
          <span class="live-logs-label">自动滚动</span>
          <el-button
            link
            type="danger"
            size="small"
            :disabled="logsStore.entries.length === 0"
            @click="handleClearLogs"
          >
            清空日志
          </el-button>
          <el-button link type="primary" size="small" @click="logsCollapsed = !logsCollapsed">
            {{ logsCollapsed ? '展开' : '折叠' }}
          </el-button>
        </div>
      </div>
      <div v-show="!logsCollapsed" ref="logListRef" class="live-logs-list">
        <template v-if="liveLogs.length === 0">
          <div class="live-logs-empty">暂无日志</div>
        </template>
        <template v-else>
          <div
            v-for="(entry, index) in liveLogs"
            :key="`${entry.time}-${entry.instanceId}-${index}`"
            class="live-log-row"
            :class="{ 'is-info': isInfoEntry(entry) }"
          >
            <span class="log-cell log-time">{{ entry.time }}</span>
            <span class="log-cell log-instance" :title="`端口 ${entry.instancePort}`">
              {{ entry.instancePort }}
            </span>
            <span class="log-cell log-method" :class="methodClass(entry.method)">
              {{ entry.method }}
            </span>
            <span class="log-cell log-path" :title="entry.path">{{ entry.path }}</span>
            <span class="log-cell log-status" :class="statusClass(entry)">{{ statusText(entry) }}</span>
            <span class="log-cell log-duration">{{ formatDuration(entry.durationMs) }}</span>
          </div>
        </template>
      </div>
    </div>

    <InstanceDialog v-model="dialogVisible" :instance="editingInstance" />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { Plus, Search } from '@element-plus/icons-vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import InstanceDialog from './InstanceDialog.vue';
import { useConfigStore } from '../stores/config';
import { useLogsStore } from '../stores/logs';
import type { InstanceInfo, InstanceState, LogEntry, ProxyInstance } from '../types';

// 实例状态标签的展示信息
interface StateMeta {
  label: string;
  type: 'success' | 'info' | 'danger';
}

// 实例状态对应的标签文案与颜色
const stateMeta: Record<InstanceState, StateMeta> = {
  running: { label: '运行中', type: 'success' },
  stopped: { label: '已停止', type: 'info' },
  error: { label: '错误', type: 'danger' },
};

/**
 * 读取实例状态对应的标签展示信息，未知状态回退为「已停止」样式。
 * @param state 实例状态
 * @returns 标签文案与颜色
 */
function resolveStateMeta(state: InstanceState): StateMeta {
  return stateMeta[state] ?? stateMeta.stopped;
}

// 实时日志面板最多展示的条数（取日志缓冲的尾部）
const LIVE_LOG_LIMIT = 100;

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

const configStore = useConfigStore();
const logsStore = useLogsStore();
// 搜索关键字（按端口/目标/协议过滤）
const keyword = ref('');
// 编辑对话框是否可见
const dialogVisible = ref(false);
// 当前编辑的实例；为 null 表示新建
const editingInstance = ref<ProxyInstance | null>(null);
// 正在切换启用状态的实例 id 列表（用于开关的 loading 保护）
const togglingIds = ref<string[]>([]);
// 总开关是否正在执行批量启停（用于开关的 loading 保护）
const switchingAll = ref(false);
// 实时日志面板是否折叠（折叠后只保留头部）
const logsCollapsed = ref(false);
// 实时日志面板的滚动容器引用
const logListRef = ref<HTMLDivElement | null>(null);

// 按关键字过滤后的实例列表
const filteredInstances = computed(() => {
  const text = keyword.value.trim().toLowerCase();
  if (!text) {
    return configStore.instances;
  }
  return configStore.instances.filter(
    (item) =>
      String(item.httpPort).includes(text) ||
      item.target.toLowerCase().includes(text) ||
      item.protocol.toLowerCase().includes(text),
  );
});

// 表格空状态提示文案
const emptyDescription = computed(() =>
  configStore.instances.length === 0 ? '暂无实例，点击右上角新建' : '没有匹配的实例',
);

// 是否存在运行中的实例：决定总开关状态与实时日志面板的显示
const hasRunning = computed(() => configStore.instances.some((item) => item.state === 'running'));

// 实时日志面板展示的日志（取日志缓冲的尾部 LIVE_LOG_LIMIT 条）
const liveLogs = computed(() => logsStore.entries.slice(-LIVE_LOG_LIMIT));

/** 打开「新建实例」对话框 */
function handleCreate(): void {
  editingInstance.value = null;
  dialogVisible.value = true;
}

/**
 * 打开「编辑实例」对话框，拷贝一份数据避免直接修改父级（store）数据。
 * @param row 待编辑的实例行数据
 */
function handleEdit(row: InstanceInfo): void {
  editingInstance.value = {
    id: row.id,
    enabled: row.enabled,
    target: row.target,
    httpPort: row.httpPort,
    protocol: row.protocol,
    reasoningEffort: row.reasoningEffort,
    headers: { ...row.headers },
  };
  dialogVisible.value = true;
}

/**
 * 切换实例启用状态，切换期间锁定对应开关。
 * @param row 目标实例行数据
 * @param value 开关的新值
 */
async function handleToggleEnabled(
  row: InstanceInfo,
  value: boolean | string | number,
): Promise<void> {
  togglingIds.value.push(row.id);
  try {
    await configStore.toggleEnabled(row, Boolean(value));
  } finally {
    togglingIds.value = togglingIds.value.filter((id) => id !== row.id);
  }
}

/**
 * 切换总开关：打开时启动全部已启用实例，关闭时停止全部运行中实例；
 * 仅改变运行状态，不修改各实例的启用开关。
 * @param value 开关的新值
 */
async function handleToggleAll(value: boolean | string | number): Promise<void> {
  switchingAll.value = true;
  try {
    if (value === true) {
      await configStore.startAll();
    } else {
      await configStore.stopAll();
    }
  } finally {
    switchingAll.value = false;
  }
}

/**
 * 处理实时日志面板的自动滚动开关（与日志页共用同一设置）。
 * @param value 开关的新值
 */
function handleAutoScrollChange(value: boolean | string | number): void {
  logsStore.setAutoScroll(value === true);
}

/** 清空全部日志（实时日志面板与全局日志缓冲） */
function handleClearLogs(): void {
  logsStore.clear();
}

/** 滚动实时日志面板到底部（等待 DOM 更新后执行） */
async function scrollToBottom(): Promise<void> {
  if (!logsStore.autoScroll) {
    return;
  }
  await nextTick();
  const container = logListRef.value;
  if (container) {
    container.scrollTop = container.scrollHeight;
  }
}

/**
 * 生成实例的本地访问地址。
 * @param httpPort 实例监听端口
 * @returns 形如 http://127.0.0.1:8787 的地址
 */
function resolveAddress(httpPort: number): string {
  return `http://127.0.0.1:${httpPort}`;
}

/**
 * 复制文本到剪贴板：优先使用异步剪贴板 API，失败时回退到临时 textarea 方案。
 * @param text 待复制的文本
 * @returns 是否复制成功
 */
async function copyToClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // 权限不足或非安全上下文时复制失败，继续尝试回退方案
    }
  }
  return copyByExecCommand(text);
}

/**
 * 回退的复制方案：借助临时 textarea 与 execCommand('copy') 完成复制。
 * @param text 待复制的文本
 * @returns 是否复制成功
 */
function copyByExecCommand(text: string): boolean {
  const textarea = document.createElement('textarea');
  textarea.value = text;
  // 移出可视区域，避免复制过程引起页面滚动或闪烁
  textarea.style.position = 'fixed';
  textarea.style.top = '-1000px';
  textarea.style.opacity = '0';
  document.body.appendChild(textarea);
  textarea.select();
  try {
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    // 无论成败都要移除临时元素，避免残留 DOM
    document.body.removeChild(textarea);
  }
}

/**
 * 复制实例的本地访问地址到剪贴板，并提示复制结果。
 * @param httpPort 实例监听端口
 */
async function handleCopyAddress(httpPort: number): Promise<void> {
  const copied = await copyToClipboard(resolveAddress(httpPort));
  if (copied) {
    ElMessage.success('复制成功');
  } else {
    ElMessage.error('复制失败，请手动复制');
  }
}

/**
 * 删除实例：二次确认后保存不含该实例的配置。
 * @param row 待删除的实例行数据
 */
async function handleDelete(row: InstanceInfo): Promise<void> {
  try {
    await ElMessageBox.confirm(`确定删除监听端口 ${row.httpPort} 的实例吗？`, '删除确认', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消',
    });
  } catch {
    // 用户取消删除
    return;
  }

  const current = configStore.config;
  if (!current) {
    ElMessage.error('配置尚未加载，无法删除');
    return;
  }
  await configStore.save({
    version: current.version,
    closeBehavior: current.closeBehavior,
    closeBehaviorConfirmed: current.closeBehaviorConfirmed,
    proxies: current.proxies.filter((item) => item.id !== row.id),
  });
}

// 尾部日志变化时，若开启自动滚动则始终停留在最新一条
// （以最后一条日志的引用为触发条件：日志总量达到缓冲上限后长度不再变化）
watch(
  () => liveLogs.value[liveLogs.value.length - 1],
  () => {
    void scrollToBottom();
  },
);
</script>

<style scoped>
.instances-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}

.instances-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 auto;
  gap: 12px;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.master-switch {
  display: flex;
  align-items: center;
  gap: 6px;
}

.master-switch-label {
  font-size: 13px;
  color: #606266;
}

.search-input {
  width: 260px;
}

/* 表格区域：占据剩余高度并可自适应窗口大小，由 el-table 内部滚动 */
.instances-table-wrap {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

.instances-table {
  width: 100%;
  border-radius: 6px;
}

/* 监听端口单元格：上行端口号、下行可点击复制的完整地址 */
.port-cell {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  line-height: 1.4;
}

.port-text {
  font-weight: 500;
  color: #303133;
}

.port-address {
  height: auto;
  padding: 0;
  font-size: 12px;
  font-weight: 400;
}

.effort-text {
  color: #606266;
}

/* 包裹「启动」按钮的 span：保证与后续按钮的间距不变（disabled 按钮需外层元素承接 tooltip） */
.action-item {
  display: inline-flex;
  margin-right: 12px;
}

/* 实时日志面板：固定高度（折叠时只保留头部），内部滚动 */
.live-logs {
  display: flex;
  flex-direction: column;
  flex: 0 0 auto;
  background-color: #ffffff;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
}

.live-logs-header {
  display: flex;
  align-items: center;
  flex: 0 0 36px;
  height: 36px;
  gap: 10px;
  padding: 0 12px;
}

.live-logs-title {
  font-size: 13px;
  font-weight: 600;
  color: #303133;
}

.live-logs-count {
  font-size: 12px;
  color: #909399;
}

.live-logs-actions {
  display: flex;
  align-items: center;
  margin-left: auto;
  gap: 8px;
}

.live-logs-label {
  font-size: 13px;
  color: #606266;
}

.live-logs-list {
  height: 240px;
  padding: 4px 0;
  overflow: auto;
  border-top: 1px solid #f2f3f5;
}

.live-logs-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  font-size: 13px;
  color: #909399;
}

.live-log-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px 12px;
  font-family: Consolas, 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.5;
  color: #303133;
}

.live-log-row.is-info {
  color: #9ca3af;
  background-color: #f6f8fb;
}

.live-log-row:hover {
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
  flex: 0 0 50px;
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
  flex: 1 1 auto;
  min-width: 0;
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
</style>
