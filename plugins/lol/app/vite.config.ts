import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { viteSingleFile } from "vite-plugin-singlefile";

// 英雄联盟的 MCP App（ADR 039 D5）：打成一个自带脚本和样式的 HTML（MCP Apps 的资源就是一个 HTML，
// ext-apps 的示例也这样打），plugins/lol/build.rs 把 dist/index.html 编进插件。几个页面共用这一个打包，
// 插件给资源时插一行 <meta name="kotone:page">，App 按它选页面。
export default defineConfig({
  plugins: [svelte(), viteSingleFile()],
  build: {
    // 和 Kotone 主窗口一样：WebView2 / 较新的 Chromium（Claude Desktop 也是 Chromium）
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
});
