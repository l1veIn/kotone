import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "node:fs";

// 应用版本单一来源：apps/desktop/package.json（与 tauri.conf.json / Cargo.toml 由
// release:bump 同步）。以 define 注入，dev:web / 打包均拿到同一版本，避免前端再硬编码。
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

// Tauri 开发环境约定：固定端口、不自动清屏
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri 需要监听本机地址
    host: "127.0.0.1",
    watch: {
      // 不要监听 src-tauri 与 workspace 共享的 target/（cargo 编译产物，避免触发前端
      // reload，且 rustc 写入 DLL 时 chokidar watch 会 EBUSY 崩溃）
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
  build: {
    // Tauri 使用较新的 WebView2 / Chromium 内核
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
