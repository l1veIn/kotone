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
  // CI（GitHub 的 windows-latest，4 个逻辑核）开 2 个：Playwright 不设时默认就是逻辑核的一半
  // （https://playwright.dev/docs/api/class-testconfig#test-config-workers ），https://playwright.dev/docs/test-parallel 的例子
  // 也是 `process.env.CI ? 2 : undefined`（https://playwright.dev/docs/ci 求稳建议 1 个）。一个 worker 一个 Chrome，加上 vite，
  // 2 个就快吃满 4 个核：CI 上 4 个只比 1 个快 14%（8.9 / 10.4 分钟，run 45665909267），每个测试却慢到 7–19 秒。
  // 本机只给 4 个逻辑核（start /affinity F）冷 vite 全量，1 / 2 / 3 / 4 个是 5.4 / 3.6 / 3.3 / 3.3 分钟，
  // 单个测试中位数 1.2 / 1.7 / 2.4 / 3.4 秒：再多开只是让每个测试更慢。
  // 本地默认 1 个，不和玩家、别的构建抢 CPU，要快就加 `--workers=4`
  workers: process.env.CI ? 2 : 1,
  // 多个 worker 时先用一个页面把 vite 的冷启动走完再开跑（见 e2e/warm-up.ts）
  globalSetup: "./e2e/warm-up.ts",
  // CI 上失败的测试再跑一次（照 Playwright 官方模板 `npm init playwright` 的 `retries: process.env.CI ? 2 : 0`
  // 和文档 https://playwright.dev/docs/test-retries ，这里只重试 1 次）：重试才过的算 flaky，CI 不挂，但单独列出来——
  // list 每次重试都单独打一行（带 “retry #1”），最后的汇总里有 “N flaky” 和这几个测试的名字、第一次失败的错误。
  // 本地不重试，失败就是失败。
  retries: process.env.CI ? 1 : 0,
  // 单个测试的超时：4 个 worker 时 4 个 Chrome 加 vite 挤 GitHub 的 4 个核，本机约 2 秒的测试在 CI 上要 7–19 秒，
  // 冷一点的 page.goto 就能把 30 秒用完（run 45662156258，onboarding.spec.ts:161）。CI 放到 60 秒，本地还是默认的 30 秒。
  timeout: process.env.CI ? 60_000 : 30_000,
  // expect 的超时：CI 上 15 秒，本地还是默认的 5 秒。CI 上挤的时候，点一下以后页面里的变化也能慢过 5 秒——run 45665909267
  // 的 locale.spec.ts 点了 English 以后整页重新加载，两次都在等 `<html lang="en">` 的 5 秒上挂了（那个 spec 现在先等 load）。
  // 只在 CI 上放宽：代价是真坏了的测试在 CI 上每个 expect 多等 10 秒才报；本地照旧 5 秒就报。
  expect: { timeout: process.env.CI ? 15_000 : 5_000 },
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
