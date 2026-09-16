<template>
  <div class="settings-view">
    <el-card class="settings-card" shadow="never">
      <template #header>
        <span class="card-title">关闭窗口行为</span>
      </template>
      <el-radio-group v-model="closeBehavior" class="behavior-group">
        <div v-for="option in behaviorOptions" :key="option.value" class="behavior-item">
          <el-radio :value="option.value">{{ option.label }}</el-radio>
          <div class="behavior-desc">{{ option.desc }}</div>
        </div>
      </el-radio-group>
      <div class="card-actions">
        <el-button
          type="primary"
          :loading="configStore.saving"
          :disabled="!configStore.config"
          @click="handleSave"
        >
          保存
        </el-button>
      </div>
    </el-card>

    <el-card class="settings-card" shadow="never">
      <template #header>
        <span class="card-title">关于</span>
      </template>
      <el-descriptions :column="1" size="small">
        <el-descriptions-item label="应用名称">simple-proxy</el-descriptions-item>
        <el-descriptions-item label="版本">2.0.0</el-descriptions-item>
        <el-descriptions-item label="说明">
          面向 AI Agent 场景的轻量反向代理，支持自定义请求头与推理强度注入。
        </el-descriptions-item>
      </el-descriptions>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { useConfigStore } from '../stores/config';
import type { CloseBehavior } from '../types';

// 关窗行为选项（value 与后端 closeBehavior 取值保持一致）
const behaviorOptions: { value: CloseBehavior; label: string; desc: string }[] = [
  {
    value: 'minimize',
    label: '最小化到系统托盘（默认）',
    desc: '点击窗口关闭按钮时隐藏窗口并在系统托盘常驻，代理继续转发；可通过托盘菜单「显示窗口」恢复。',
  },
  {
    value: 'exit',
    label: '直接退出应用',
    desc: '点击关闭按钮时直接退出应用并停止全部代理实例。',
  },
];

const configStore = useConfigStore();

// 本地表单值：保存前不影响后端配置
const closeBehavior = ref<CloseBehavior>('minimize');

// 同步后端配置到本地表单（覆盖初次加载与保存后刷新两种情况）
watch(
  () => configStore.config?.closeBehavior,
  (value) => {
    if (value) {
      closeBehavior.value = value;
    }
  },
  { immediate: true },
);

/**
 * 保存关窗行为：沿用当前配置的版本号与实例列表，仅替换 closeBehavior。
 * store 内部已处理成功/失败提示与配置刷新，保存结束后按钮自动恢复可用。
 */
async function handleSave(): Promise<void> {
  const current = configStore.config;
  if (!current) {
    ElMessage.warning('配置尚未加载完成，请稍后重试');
    return;
  }
  await configStore.save({
    version: current.version,
    closeBehavior: closeBehavior.value,
    proxies: current.proxies,
  });
}
</script>

<style scoped>
.settings-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
  width: 100%;
  max-width: 720px;
}

.card-title {
  font-size: 14px;
  font-weight: 600;
  color: #303133;
}

.behavior-group {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 14px;
}

.behavior-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.behavior-desc {
  margin-left: 24px;
  font-size: 12px;
  line-height: 1.6;
  color: #909399;
}

.card-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px solid #f2f3f5;
}
</style>
