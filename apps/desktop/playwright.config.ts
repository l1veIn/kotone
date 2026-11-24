import { defineConfig } from "@playwright/test";

export default defineConfig({
  // 本体的测试在 e2e/；每个插件的页面测试在 plugins/<id>/e2e/（ADR 038），一起跑
  projects: [
    { name: "desktop", testDir: "./e2e" },
    { name: "plugins", testDir: "../../plugins", testMatch: "*/e2e/**/*.spec.ts" },
  ],
  fullyParallel: false,
  workers: 1,
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
