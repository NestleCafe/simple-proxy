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
            已启用 {{ configStore.enabledCount }} / {{ configStore.instances.length }}
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
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, type Component } from 'vue';
import { Connection, Refresh, Setting } from '@element-plus/icons-vue';
import InstancesView from './views/InstancesView.vue';
// 日志页入口暂时隐藏（日志已在实例页下方实时展示），恢复时取消注释下一行
// import LogsView from './views/LogsView.vue';
import SettingsView from './views/SettingsView.vue';
import { useConfigStore } from './stores/config';

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

onMounted(() => {
  // 首次进入时加载配置并订阅后端事件
  void configStore.load();
  void configStore.initEventListeners();
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
</style>
