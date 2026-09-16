import { createApp } from 'vue';
import { createPinia } from 'pinia';
import ElementPlus from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import 'element-plus/dist/index.css';
import App from './App.vue';
import './style.css';

// 应用入口：注册 Pinia、Element Plus（中文语言包）并挂载根组件
const app = createApp(App);

app.use(createPinia());
app.use(ElementPlus, { locale: zhCn });
app.mount('#app');
