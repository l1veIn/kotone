import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// Tauri 开发环境约定：固定端口、不自动清屏
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri 需要监听本机地址
    host: "127.0.0.1",
    watch: {
      // 不要监听 src-tauri，避免触发不必要的前端 reload
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    // Tauri 使用较新的 WebView2 / Chromium 内核
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
