<template>
  <el-dialog
    v-model="visible"
    :title="dialogTitle"
    width="640px"
    :close-on-click-modal="false"
  >
    <el-form ref="formRef" :model="form" :rules="rules" label-width="96px">
      <el-form-item label="目标地址" prop="target">
        <el-input v-model="form.target" placeholder="如 https://api.openai.com" clearable />
      </el-form-item>
      <el-form-item label="监听端口" prop="httpPort">
        <el-input-number
          v-model="form.httpPort"
          :min="1"
          :max="65535"
          :step="1"
          controls-position="right"
        />
      </el-form-item>
      <el-form-item label="协议" prop="protocol">
        <el-select v-model="form.protocol" class="protocol-select">
          <el-option label="OpenAI" value="openai" />
          <el-option label="Anthropic" value="anthropic" />
        </el-select>
      </el-form-item>
      <el-form-item label="推理强度">
        <el-input
          v-model="form.reasoningEffort"
          placeholder="如 max / high，留空则透明转发"
          clearable
        />
      </el-form-item>
      <el-form-item label="静态请求头">
        <div class="headers-editor">
          <div v-for="(row, index) in headerRows" :key="index" class="header-row">
            <el-input v-model="row.key" placeholder="Header 名称" />
            <el-input v-model="row.value" placeholder="Header 值" />
            <el-button type="danger" plain :icon="Delete" @click="removeHeaderRow(index)" />
          </div>
          <el-button plain :icon="Plus" @click="addHeaderRow">添加请求头</el-button>
        </div>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="configStore.saving" @click="handleSave">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { computed, reactive, ref, toRefs, watch } from 'vue';
import { Delete, Plus } from '@element-plus/icons-vue';
import { ElMessage, type FormInstance, type FormRules } from 'element-plus';
import { useConfigStore } from '../stores/config';
import type { Protocol, ProxyInstance } from '../types';

// 请求头编辑行的结构
interface HeaderRow {
  key: string;
  value: string;
}

// 对话框表单的数据结构
interface InstanceForm {
  target: string;
  httpPort: number;
  protocol: Protocol;
  reasoningEffort: string;
}

// 新建实例时的默认监听端口
const DEFAULT_PORT = 8787;

const visible = defineModel<boolean>({ required: true });
const props = defineProps<{ instance?: ProxyInstance | null }>();
const { instance } = toRefs(props);

const configStore = useConfigStore();
const formRef = ref<FormInstance>();
const form = reactive<InstanceForm>({
  target: '',
  httpPort: DEFAULT_PORT,
  protocol: 'openai',
  reasoningEffort: '',
});
// 静态请求头的动态编辑行
const headerRows = ref<HeaderRow[]>([]);

// 对话框标题：有实例为编辑，无实例为新建
const dialogTitle = computed(() => (instance.value ? '编辑实例' : '新建实例'));

/**
 * 校验目标地址必须为 http:// 或 https:// 开头。
 * @param _rule 校验规则（未使用）
 * @param value 当前字段值
 * @param callback 校验回调，传入 Error 表示校验失败
 */
function validateTarget(_rule: unknown, value: unknown, callback: (error?: Error) => void): void {
  const text = typeof value === 'string' ? value.trim() : '';
  if (!text) {
    callback(new Error('请输入目标地址'));
    return;
  }
  if (!/^https?:\/\//i.test(text)) {
    callback(new Error('目标地址必须以 http:// 或 https:// 开头'));
    return;
  }
  callback();
}

// 表单校验规则
const rules: FormRules = {
  target: [
    { required: true, message: '请输入目标地址', trigger: 'blur' },
    { validator: validateTarget, trigger: 'blur' },
  ],
  httpPort: [{ required: true, message: '请输入 1-65535 之间的监听端口', trigger: 'change' }],
  protocol: [{ required: true, message: '请选择协议', trigger: 'change' }],
};

/** 把传入实例的数据（深拷贝请求头）写入本地表单，无实例时重置为默认值 */
function syncFormFromInstance(): void {
  const source = instance.value;
  form.target = source?.target ?? '';
  form.httpPort = source?.httpPort ?? DEFAULT_PORT;
  form.protocol = source?.protocol ?? 'openai';
  form.reasoningEffort = source?.reasoningEffort ?? '';
  headerRows.value = Object.entries(source?.headers ?? {}).map(([key, value]) => ({
    key,
    value,
  }));
  // 再次打开时清除上一次的校验状态
  formRef.value?.clearValidate();
}

watch(visible, (opened) => {
  if (opened) {
    syncFormFromInstance();
  }
});

/** 新增一行请求头 */
function addHeaderRow(): void {
  headerRows.value.push({ key: '', value: '' });
}

/**
 * 删除指定行的请求头。
 * @param index 行下标
 */
function removeHeaderRow(index: number): void {
  headerRows.value.splice(index, 1);
}

/** 收集请求头编辑行，key 为空的行会被忽略 */
function collectHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  headerRows.value.forEach((row) => {
    const key = row.key.trim();
    if (key) {
      headers[key] = row.value;
    }
  });
  return headers;
}

/** 校验表单后把实例写入配置并保存 */
async function handleSave(): Promise<void> {
  const currentForm = formRef.value;
  if (!currentForm) {
    return;
  }
  const valid = await currentForm.validate().catch(() => false);
  if (!valid) {
    return;
  }

  const current = configStore.config;
  if (!current) {
    ElMessage.error('配置尚未加载，无法保存');
    return;
  }

  const effort = form.reasoningEffort.trim();
  const payload: ProxyInstance = {
    id: props.instance?.id ?? crypto.randomUUID(),
    // 新建实例默认未启用（保存后不会自动启动，需手动打开启用开关）
    enabled: props.instance?.enabled ?? false,
    target: form.target.trim(),
    httpPort: Math.trunc(form.httpPort),
    protocol: form.protocol,
    reasoningEffort: effort ? effort : null,
    headers: collectHeaders(),
  };

  const proxies = props.instance
    ? current.proxies.map((item) => (item.id === payload.id ? payload : item))
    : [...current.proxies, payload];

  await configStore.save({
    version: current.version,
    closeBehavior: current.closeBehavior,
    closeBehaviorConfirmed: current.closeBehaviorConfirmed,
    proxies,
  });
  visible.value = false;
}
</script>

<style scoped>
.protocol-select {
  width: 100%;
}

.headers-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
}

.header-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
