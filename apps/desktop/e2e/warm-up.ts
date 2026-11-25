// Playwright 的 globalSetup（不是测试文件）：几个 worker 一起开跑之前，先用一个页面把 vite 的冷启动走完
// （依赖打包、主页面模块的第一次编译），再放 worker 进来。
//
// 为什么：CI 上每次都是冷的（没有 node_modules/.vite）。一个 worker 时第一个测试自己扛冷启动（CI 上约 11 秒）；
// 4 个 worker 一起打开页面，本机冷启动全量跑 6 次里有 2 次头 4 个测试一起卡满 30 秒超时（about 卡在 page.goto 等 load，
// agent-chat 卡在页面里 import 模块），后面的测试都正常。先单独加载一次、给足时间，就不让头一波测试替冷启动背锅。
// webServer 在 globalSetup 之前起好（Playwright 先跑插件的 setup，再跑 globalSetup）。
import { chromium, type FullConfig } from "@playwright/test";

export default async function warmUp(config: FullConfig) {
  if (config.workers <= 1) return; // 一个 worker：第一个测试自己扛，和以前一样
  const { baseURL, channel } = config.projects[0].use;
  const browser = await chromium.launch({ channel, headless: true });
  try {
    const page = await browser.newPage();
    await page.goto(`${baseURL}/#/settings?onboarding=never`, { waitUntil: "load", timeout: 120_000 });
  } finally {
    await browser.close();
  }
}
