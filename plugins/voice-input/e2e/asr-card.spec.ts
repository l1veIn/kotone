// 语音输入页的“语音识别”卡片（交接第十一节第 6 条）：语音识别归本体，语音输入只是借来用。
// 卡片只显示现在用的是哪个（本地模型的名字 / 在线服务的名字）和准备好没有，加一个“去 设置 → 模型 换”的链接；
// 这里没有切换、下载、启动按钮——这些在「设置 → 模型 → 语音识别」（那边的检查在 apps/desktop/e2e 的 copilot-flow、onboarding）。
// 由 apps/desktop 的 Playwright 一起跑（plugins 项目）；浏览器预览里的识别是模拟的（ipc/browserMock.ts）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

/** 截图放在仓库的 target/plugin-shots/round7/（实现记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round7", `${name}.png`);

/** 语音输入（插件的 App）：左侧栏「插件」→ 点它的卡片，落在“输入”页。 */
async function openVoiceInput(page: Page) {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-voice-input-open").click();
  await expect(page.getByTestId("voice-tab-general")).toHaveAttribute("aria-current", "page");
}

/** 卡片上不该有的按钮：切换、下载、启动 / 停止、重试（链接“去 设置 → 模型 换”是唯一能点的）。 */
const actionButtons = (page: Page) =>
  page.getByTestId("asr-card").getByRole("button", { name: /切换|下载|启动|停止|重试|换成|换回/ });

test("online: the voice input card only shows the current speech recognition and links to 设置 → 模型", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 760 });
  await page.goto("/#/settings?onboarding=never&mockAsr=online");
  await openVoiceInput(page);
  const card = page.getByTestId("asr-card");
  await expect(card).toHaveAttribute("data-asr-state", "online-ready");
  await expect(page.getByTestId("asr-model-name")).toHaveText("DeepSeek 语音识别 · 在线");
  await expect(page.getByTestId("asr-status")).toHaveText("打开 Kotone 就能用");
  await expect(actionButtons(page)).toHaveCount(0);
  await expect(card.getByRole("button")).toHaveCount(1);
  await expect(page.getByTestId("asr-open-models")).toHaveText("去 设置 → 模型 换");
  // 原来卡片上的切换、下载、启动热键都不在这里了
  for (const id of ["asr-switch-local", "asr-switch-local-button", "asr-switch-online", "asr-download-active", "asr-all-models", "asr-start", "asr-toggle-hotkey"]) {
    await expect(page.getByTestId(id), id).toHaveCount(0);
  }
  await card.scrollIntoViewIfNeeded();
  await screenshot(card, { path: shot(testInfo, "voice-input-asr-card-online") });

  // 链接打开「设置 → 模型 → 语音识别」：那里能换、能下载
  await page.getByTestId("asr-open-models").click();
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("model-category-asr")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("models-asr-runtime")).toHaveAttribute("data-asr-state", "online-ready");
  await expect(page.getByTestId("model-list")).toBeVisible();
  await expect(page.getByTestId("model-download-x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05")).toBeVisible();
});

test("local: the card shows the local model's name and state (not downloaded / not started), still with no buttons", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 760 });
  // 默认：本地 X-ASR，还没下载
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page);
  const card = page.getByTestId("asr-card");
  await expect(card).toHaveAttribute("data-asr-state", "local-stopped");
  await expect(page.getByTestId("asr-model-name")).toHaveText("X-ASR 流式中英标点 · 本地");
  await expect(page.getByTestId("asr-status")).toHaveText("本地模型还没下载");
  await expect(actionButtons(page)).toHaveCount(0);
  await card.scrollIntoViewIfNeeded();
  await screenshot(card, { path: shot(testInfo, "voice-input-asr-card") });

  // 本地模型下好了、没启动：卡片写“未启动”，启动按钮在设置里
  await page.goto("/?case=local-ready#/settings?onboarding=never&mockAsr=local-ready");
  await openVoiceInput(page);
  await expect(card).toHaveAttribute("data-asr-state", "local-stopped");
  await expect(page.getByTestId("asr-status")).toHaveText("未启动");
  await expect(actionButtons(page)).toHaveCount(0);
  await page.getByTestId("asr-open-models").click();
  await expect(page.getByTestId("model-category-asr")).toHaveAttribute("aria-selected", "true");
  // 启动 / 停止、启动热键、打开时自动启动都在这里（从语音输入页的卡片搬来）
  const runtime = page.getByTestId("models-asr-runtime");
  await expect(runtime).toHaveAttribute("data-asr-state", "local-stopped");
  await expect(page.getByTestId("asr-toggle-hotkey")).toBeVisible();
  await screenshot(runtime, { path: shot(testInfo, "settings-models-asr-runtime") });
  await page.getByTestId("asr-start").click();
  await expect(runtime).toHaveAttribute("data-asr-state", "local-running");
  // 回到语音输入：卡片跟着变成“已启动”
  await openVoiceInput(page);
  await expect(card).toHaveAttribute("data-asr-state", "local-running");
  await expect(page.getByTestId("asr-status")).toContainText("已启动");
});
