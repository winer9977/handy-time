import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  server: {
    // ★ 必须显式给端口：tauri.conf.json 的 `build.devUrl` 指 http://localhost:1420，
    // 而 vite 默认起在 5173 —— 两边对不上时 `npm run tauri:dev` 会一直卡在
    // "Waiting for your frontend dev server to start on http://localhost:1420/"，
    // 应用根本不启动（实测就是这个现象，日志里只有循环的 Warn）。
    // strictPort：端口被占时直接报错，而不是悄悄换一个端口再卡住。
    port: 1420,
    strictPort: true,
  },
})
