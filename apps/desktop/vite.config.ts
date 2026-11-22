import { defineConfig, normalizePath } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { paraglideVitePlugin } from "@inlang/paraglide-js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// 应用版本单一来源：apps/desktop/package.json（与 tauri.conf.json / Cargo.toml 由
// release:bump 同步）。以 define 注入，dev:web / 打包均拿到同一版本，避免前端再硬编码。
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

// Tauri 开发环境约定：固定端口、不自动清屏
export default defineConfig({
  plugins: [
    // 界面文案（i18n）：messages/zh-CN.json → src/lib/paraglide（生成，不进 git）。
    // 插件从 node_modules 读，不从 CDN 拉（project.inlang/settings.json）。选项与 package.json 的 i18n 脚本一致。
    paraglideVitePlugin({
      project: "./project.inlang",
      outdir: "./src/lib/paraglide",
      strategy: ["baseLocale"],
      outputStructure: "locale-modules",
    }),
    svelte(),
    tailwindcss(),
  ],
  // shadcn-svelte 组件用 $lib 引用公共代码（与 tsconfig.json 的 paths 一致）
  resolve: {
    alias: {
      // 用正斜杠：Windows 反斜杠路径会让 Vite 以 /@fs/ 另起一份模块实例（store 不再共享）
      $lib: normalizePath(fileURLToPath(new URL("./src/lib", import.meta.url))),
    },
  },
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    // 不用 create-tauri-app 默认的 1420，避免和本机其它 Tauri 项目抢端口
    port: 17420,
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
