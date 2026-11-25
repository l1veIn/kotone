import { defineConfig } from "@playwright/test";

export default defineConfig({
  // 本体的测试在 e2e/；每个插件的页面测试在 plugins/<id>/e2e/（ADR 038），一起跑
  projects: [
    { name: "desktop", testDir: "./e2e" },
    { name: "plugins", testDir: "../../plugins", testMatch: "*/e2e/**/*.spec.ts" },
  ],
  // 测试之间不共享状态（每个测试一个新的浏览器上下文，预览的假数据在它自己的 localStorage 里；没有固定端口、
  // 没有写同一个文件），同一个文件里的测试也能分到不同的 worker 上。
  fullyParallel: true,
  // CI（GitHub 的 Windows 机器 4 核）开 4 个；本地默认 1 个，不和玩家、别的构建抢 CPU，要快就加 `--workers=4`
  workers: process.env.CI ? 4 : 1,
  // 多个 worker 时先用一个页面把 vite 的冷启动走完再开跑（见 e2e/warm-up.ts）
  globalSetup: "./e2e/warm-up.ts",
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:17420",
    channel: "chrome",
    headless: true,
    viewport: { width: 900, height: 700 },
  },
  webServer: {
    command: "pnpm dev:web",
    url: "http://127.0.0.1:17420",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
