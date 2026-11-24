import { defineConfig, normalizePath, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { paraglideVitePlugin } from "@inlang/paraglide-js";
import { existsSync, readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { fileURLToPath } from "node:url";

// 应用版本单一来源：apps/desktop/package.json（与 tauri.conf.json / Cargo.toml 由
// release:bump 同步）。以 define 注入，dev:web / 打包均拿到同一版本，避免前端再硬编码。
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

/**
 * 浏览器预览（dev:web、Playwright）里的插件 MCP App（ADR 039 D5）。桌面端由 Rust 的回环代理（src-tauri/src/app_proxy.rs）
 * 在 127.0.0.1 的随机端口上给；这里也在 127.0.0.1 的随机端口上起一个小服务，照它的路径给：
 * `/<十六进制密钥>/sandbox|app|assets/<插件>/…`（端口不同 = 源不同，主窗口的 CSP `frame-src http://127.0.0.1:*` 也照样管着）。
 * 预览的前端从 `/__kotone/app-proxy` 问到它的地址（src/lib/ipc/pluginsMock.ts）。沙盒代理页就是桌面端那一个
 * （app_sandbox.html）；App 是 plugins/<插件>/app/dist/index.html（先 `pnpm build:apps`，test:e2e 会先跑）；
 * 素材一律回一个 1×1 的透明 PNG（预览不上网）。
 */
function mcpAppsDevServer(): Plugin {
  const sandbox = fileURLToPath(new URL("./src-tauri/src/app_sandbox.html", import.meta.url));
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "base64");
  /** App 的 HTML（资源的内容）：dist 加上页面标记，和 plugins/lol/src/app.rs 一样。没打包时 null。 */
  const appHtml = (plugin: string, page: string): string | null => {
    const dist = fileURLToPath(new URL(`../../plugins/${plugin}/app/dist/index.html`, import.meta.url));
    if (!/^[a-z0-9-]+$/.test(plugin) || !/^[a-z0-9-]+$/.test(page) || !existsSync(dist)) return null;
    return readFileSync(dist, "utf8").replace("<head>", `<head><meta name="kotone:page" content="${page}">`);
  };
  const handle = (req: IncomingMessage, res: ServerResponse) => {
    const send = (status: number, type: string, body: string | Buffer, csp?: string) => {
      res.statusCode = status;
      res.setHeader("Content-Type", type);
      res.setHeader("Cache-Control", "no-store");
      res.setHeader("Access-Control-Allow-Origin", "*");
      if (csp) res.setHeader("Content-Security-Policy", csp);
      res.end(body);
    };
    const match = /^\/([0-9a-f]+)\/(sandbox|app|assets)\/([a-z0-9-]+)\/(.+)$/.exec((req.url ?? "").split("?")[0]);
    if (!match) return send(404, "text/plain; charset=utf-8", "not found");
    const [, key, kind, plugin, rest] = match;
    if (kind === "sandbox") {
      return send(200, "text/html; charset=utf-8", readFileSync(sandbox), "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'");
    }
    if (kind === "assets") return send(200, "image/png", png);
    const html = appHtml(plugin, rest);
    if (html === null) return send(404, "text/plain; charset=utf-8", "pnpm build:apps");
    const assets = `http://${req.headers.host}/${key}/assets/${plugin}/`;
    // 和 app_proxy.rs 的 app_csp 一样的样子（这里只放 CommunityDragon）
    return send(200, "text/html; charset=utf-8", html, `default-src 'none'; script-src 'unsafe-inline' https://raw.communitydragon.org; style-src 'unsafe-inline' https://raw.communitydragon.org; img-src data: blob: ${assets} https://raw.communitydragon.org; font-src data: ${assets}; media-src data: blob: ${assets}; connect-src ${assets}; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'`);
  };
  return {
    name: "kotone-mcp-apps-dev",
    configureServer(server) {
      const proxy = createServer(handle);
      let origin = "";
      proxy.listen(0, "127.0.0.1", () => {
        origin = `http://127.0.0.1:${(proxy.address() as AddressInfo).port}`;
      });
      server.httpServer?.on("close", () => proxy.close());
      server.middlewares.use("/__kotone/app-proxy", (_req, res) => {
        res.setHeader("Content-Type", "application/json");
        res.setHeader("Cache-Control", "no-store");
        res.end(JSON.stringify({ origin }));
      });
      // 资源的内容（`resources/read`）：同源给，主窗口的 CSP 不许连代理
      server.middlewares.use("/__kotone/app-html", (req, res) => {
        const [plugin = "", page = ""] = (req.url ?? "").replace(/^\//, "").split("/");
        const html = appHtml(plugin, page);
        res.statusCode = html === null ? 404 : 200;
        res.setHeader("Content-Type", "text/html; charset=utf-8");
        res.setHeader("Cache-Control", "no-store");
        res.end(html ?? "pnpm build:apps");
      });
    },
  };
}

// Tauri 开发环境约定：固定端口、不自动清屏
export default defineConfig({
  plugins: [
    // 界面文案（i18n）：messages/<locale>/*.json → src/lib/paraglide（生成，不进 git）。语言存在 localStorage，没选过 = 中文。
    // 插件从 node_modules 读，不从 CDN 拉（project.inlang/settings.json）。选项与 package.json 的 i18n 脚本一致。
    paraglideVitePlugin({
      project: "./project.inlang",
      outdir: "./src/lib/paraglide",
      strategy: ["localStorage", "baseLocale"],
      outputStructure: "locale-modules",
    }),
    svelte(),
    tailwindcss(),
    mcpAppsDevServer(),
  ],
  // shadcn-svelte 组件用 $lib 引用公共代码（与 tsconfig.json 的 paths 一致）
  resolve: {
    alias: {
      // 用正斜杠：Windows 反斜杠路径会让 Vite 以 /@fs/ 另起一份模块实例（store 不再共享）
      $lib: normalizePath(fileURLToPath(new URL("./src/lib", import.meta.url))),
      // 插件的页面（ADR 038：plugins/<id>/ui/，在 apps/desktop 外面）
      $plugins: normalizePath(fileURLToPath(new URL("../../plugins", import.meta.url))),
    },
    // plugins/<id>/ui 下的组件也用这里的依赖：从项目根解析，只有一份（否则往上找不到 node_modules，
    // 或者另起一份 svelte 运行时）
    dedupe: ["svelte", "@lucide/svelte", "bits-ui", "@tauri-apps/api", "@tauri-apps/plugin-dialog", "clsx", "tailwind-merge", "tailwind-variants"],
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
