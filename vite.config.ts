import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// Tauri 开发模式约定：固定端口 1420，方便 tauri.conf.json 中 devUrl 指向
export default defineConfig({
  plugins: [vue()],
  // 清屏会干扰 Rust 侧日志输出，关闭
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri 目录由 cargo 自行监听，避免 vite 重复监听
      ignored: ['**/src-tauri/**'],
    },
  },
});
