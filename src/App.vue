<template>
  <div class="app-layout">
    <aside class="app-sidebar">
      <div class="brand">
        <div class="brand-title">simple-proxy</div>
        <div class="brand-subtitle">AI Agent 轻量反向代理</div>
      </div>
      <el-menu
        :default-active="activeView"
        class="nav-menu"
        background-color="#1f2937"
        text-color="#cbd5e1"
        active-text-color="#ffffff"
        @select="handleMenuSelect"
      >
        <el-menu-item index="instances">
          <el-icon><Connection /></el-icon>
          <span>实例</span>
        </el-menu-item>
        <!-- 日志页入口暂时隐藏：日志已在实例页下方实时展示，恢复时取消注释
        <el-menu-item index="logs">
          <el-icon><Document /></el-icon>
          <span>日志</span>
        </el-menu-item>
        -->
        <el-menu-item index="settings">
          <el-icon><Setting /></el-icon>
          <span>设置</span>
        </el-menu-item>
      </el-menu>
    </aside>
    <section class="app-main">
      <header class="app-toolbar">
        <h2 class="toolbar-title">{{ currentTitle }}</h2>
        <div class="toolbar-actions">
          <el-tag type="info" effect="plain">
            运行中 {{ configStore.runningCount }} / {{ configStore.instances.length }}
          </el-tag>
          <el-button :loading="configStore.loading" @click="handleRefresh">
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
        </div>
      </header>
      <main class="app-content">
        <component :is="currentViewComponent" />
      </main>
    </section>

    <el-dialog
      v-model="closeDialogVisible"
      title="关闭窗口"
      width="440px"
      :close-on-click-modal="false"
    >
      <div class="close-request-body">
        <div class="close-request-tip">请选择关闭窗口时的行为：</div>
        <el-radio-group v-model="closeChoice" class="close-request-options">
          <el-radio value="minimize">最小化到系统托盘（代理继续运行）</el-radio>
          <el-radio value="exit">直接退出应用（停止全部实例）</el-radio>
        </el-radio-group>
        <el-checkbox v-model="closeRemember">记住我的选择（可在设置中修改）</el-checkbox>
      </div>
      <template #footer>
        <el-button @click="closeDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleCloseConfirm">确定</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, type Component } from 'vue';
import { Connection, Refresh, Setting } from '@element-plus/icons-vue';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { ElMessage } from 'element-plus';
import InstancesView from './views/InstancesView.vue';
// 日志页入口暂时隐藏（日志已在实例页下方实时展示），恢复时取消注释下一行
// import LogsView from './views/LogsView.vue';
import SettingsView from './views/SettingsView.vue';
import { onCloseRequested, resolveCloseRequest } from './api/tauri';
import { useConfigStore } from './stores/config';
import type { CloseBehavior } from './types';

// 当前视图标识（日志页入口暂时隐藏）
type ViewKey = 'instances' | 'settings';

// 视图标识到组件的映射
const views: Record<ViewKey, Component> = {
  instances: InstancesView,
  // logs: LogsView,
  settings: SettingsView,
};

// 视图标识到工具栏标题的映射
const viewTitles: Record<ViewKey, string> = {
  instances: '代理实例',
  // logs: '代理日志',
  settings: '设置',
};

const configStore = useConfigStore();
const activeView = ref<ViewKey>('instances');
const currentViewComponent = computed(() => views[activeView.value]);
const currentTitle = computed(() => viewTitles[activeView.value]);

/**
 * 处理侧栏菜单选择，切换当前视图。
 * @param index 菜单项标识
 */
function handleMenuSelect(index: string): void {
  if (index === 'instances' || index === 'settings') {
    activeView.value = index;
  }
}

/** 刷新配置与实例数据 */
function handleRefresh(): void {
  void configStore.load();
}

// 「关闭窗口」询问对话框是否可见
const closeDialogVisible = ref(false);
// 本次关窗选择的行为（默认最小化到系统托盘）
const closeChoice = ref<CloseBehavior>('minimize');
// 是否记住本次选择（默认记住）
const closeRemember = ref(true);
// close-requested 事件的取消订阅函数
let unlistenCloseRequested: UnlistenFn | null = null;

/**
 * 响应后端「首次关闭窗口」事件：重置为默认选项后弹出询问对话框。
 * 对话框已打开时忽略重复事件（如连续点击关闭按钮）。
 */
function handleCloseRequested(): void {
  if (closeDialogVisible.value) {
    return;
  }
  closeChoice.value = 'minimize';
  closeRemember.value = true;
  closeDialogVisible.value = true;
}

/**
 * 确认关窗选择：请求后端按所选行为隐藏到托盘或退出应用；
 * 无论成败都关闭对话框（失败时提示错误，窗口保持打开，可再次关闭重试）。
 */
async function handleCloseConfirm(): Promise<void> {
  try {
    await resolveCloseRequest(closeChoice.value, closeRemember.value);
  } catch (err) {
    ElMessage.error(`处理关闭请求失败：${err instanceof Error ? err.message : String(err)}`);
  } finally {
    closeDialogVisible.value = false;
  }
}

onMounted(async () => {
  // 首次进入时加载配置并订阅后端事件
  void configStore.load();
  void configStore.initEventListeners();
  unlistenCloseRequested = await onCloseRequested(handleCloseRequested);
});

onUnmounted(() => {
  unlistenCloseRequested?.();
  unlistenCloseRequested = null;
});
</script>

<style scoped>
.app-layout {
  display: flex;
  width: 100%;
  height: 100vh;
  overflow: hidden;
}

.app-sidebar {
  display: flex;
  flex-direction: column;
  flex: 0 0 200px;
  width: 200px;
  background-color: #1f2937;
  color: #e5e7eb;
}

.brand {
  padding: 16px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.brand-title {
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0.5px;
}

.brand-subtitle {
  margin-top: 4px;
  font-size: 12px;
  color: #9ca3af;
}

.nav-menu {
  flex: 1;
  border-right: none;
}

.app-main {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  background-color: #f5f7fa;
}

.app-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 56px;
  height: 56px;
  padding: 0 16px;
  background-color: #ffffff;
  border-bottom: 1px solid #e5e7eb;
}

.toolbar-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.app-content {
  flex: 1;
  overflow: auto;
  padding: 16px;
}

/* 「关闭窗口」询问对话框内容 */
.close-request-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.close-request-tip {
  font-size: 13px;
  color: #606266;
}

.close-request-options {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 10px;
}
</style>
