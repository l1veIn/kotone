import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

// 引导（ADR 028）：新用户 欢迎 → 填 DeepSeek Key → 已连接、设呼叫键 → 聊天；“只想用语音输入？”进语音输入子板块的
// 四步引导；0.1.x 升级用户先看升级欢迎页。主路径的验收在 copilot-flow.spec.ts，这里补各种出错和边界情况。

const mockSettings = (page: Page) => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
const mockConnections = (page: Page) => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).listConnections());

/** 欢迎页 →“只想用语音输入？”→ 语音输入引导的第 2 步（识别）。 */
async function guideToModelStep(page: Page) {
  await page.getByTestId("onboarding-voice-only").click();
  await expect(page.getByTestId("voice-guide")).toBeVisible();
  await page.getByTestId("voice-guide-next").click();
  await expect(page.getByTestId("voice-guide-model")).toBeVisible();
}

test("voice-only path: the guide records a hotkey, the first sentence lands, and onboarding can be reopened", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always");
  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding).toBeVisible();
  await page.getByTestId("onboarding-voice-only").click();
  await expect(onboarding).toBeHidden();

  // 引导嵌在语音输入子板块顶部，不是弹窗
  await expect(page.getByTestId("voice-tab-general")).toHaveAttribute("aria-current", "page");
  await page.getByTestId("profile-lol").click();
  await page.getByTestId("voice-guide-next").click();
  await page.getByTestId("voice-guide-download").click();
  await expect(page.getByTestId("voice-guide-model").getByText("✓ 已就绪", { exact: true })).toBeVisible();
  await page.getByTestId("voice-guide-next").click();

  // 到第 3 步即主动检测输入环境，不依赖用户先点「重新录入」
  await expect(page.getByTestId("input-environment-ready")).toBeVisible();
  await page.getByTestId("mode-push-to-talk").click();
  await page.getByTestId("voice-guide-hotkey").getByRole("button", { name: "重新录入", exact: true }).click();
  await page.keyboard.press("F9");
  await expect(page.getByTestId("voice-guide-hotkey-value")).toHaveText("F9");
  await page.getByTestId("voice-guide-next").click();

  // 说一句试试：字打进框里就算成了（真机上是语音输入打进去的）
  await expect(page.getByTestId("voice-guide-try")).toContainText("F9");
  await expect(page.getByTestId("onboarding-try-status")).toContainText("等你开口");
  await page.getByTestId("onboarding-try-box").fill("打野来下路");
  await expect(page.getByTestId("onboarding-try-status")).toContainText("成了");
  await page.getByTestId("voice-guide-done").click();
  await expect(page.getByTestId("voice-guide")).toHaveCount(0);
  const saved = await mockSettings(page);
  expect(saved.hotkey).toEqual({ key: "F9", mode: "hold" });
  expect(saved.interactionMode).toBe("push-to-talk");
  expect(saved.activeProfileId).toBe("lol");

  // 引导可以重走：语音输入页底部，和「设置 → 系统」
  await page.getByTestId("voice-guide-open").click();
  await expect(page.getByTestId("voice-guide")).toBeVisible();
  await openSettingsTab(page, "system");
  await page.getByTestId("reopen-onboarding").click();
  await expect(page.getByTestId("onboarding-welcome")).toBeVisible();
});

test("with one DeepSeek key and no local model, everything runs on DeepSeek and “你能做什么？” waits in the composer", async ({ page }, testInfo) => {
  // 刚装好：一个本地模型都没有，语音识别也用 DeepSeek
  await page.goto("/#/settings?onboarding=always&mockModels=none");
  await page.screenshot({ path: testInfo.outputPath("welcome.png") });
  await page.getByTestId("onboarding-add-key").click();
  await expect(page.getByTestId("onboarding-key-step")).toBeVisible();
  await expect(page.getByTestId("onboarding-key-submit")).toBeDisabled();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await expect(page.getByTestId("onboarding-key-input")).toHaveAttribute("type", "password");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-connected")).toBeVisible();
  const settings = await mockSettings(page);
  expect(settings.assistant.connectionId).toMatch(/^deepseek-/);
  expect(settings.sttEngine).toBe("deepseek-asr");
  expect(settings.ocr.activeModelId).toBe("deepseek-ocr");
  expect(settings.ui.firstRunCompleted).toBe(true);
  // Key 只存进凭据库，不在设置里
  expect(JSON.stringify(settings)).not.toContain("sk-test-123");
  await page.getByTestId("onboarding-enter-chat").click();
  await expect(page.getByTestId("onboarding")).toBeHidden();
  // 只预填，不替玩家发出（D2）；玩家自己点发送（浏览器预览不接 AI）
  const composer = page.getByTestId("assistant-request");
  await expect(composer).toHaveValue("你能做什么？");
  await expect(page.getByTestId("agent-send-error")).toHaveCount(0);
  await page.getByTestId("agent-send").click();
  await expect(page.getByTestId("agent-send-error")).toContainText("仅预览");
});

test("a key that does not work is explained, leaves nothing behind, and “稍后配置” lands on the chat key card", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always&mockCompanionCheck=fail");
  await page.getByTestId("onboarding-add-key").click();
  await page.getByTestId("onboarding-key-input").fill("sk-wrong");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-key-error")).toContainText("模型列表里没有");
  await expect(page.getByTestId("onboarding-key-step")).toBeVisible();
  // 测试没过：刚建的连接删掉，什么都没换
  expect(await mockConnections(page)).toEqual([]);
  const settings = await mockSettings(page);
  expect(settings.assistant.connectionId).toBe("");
  expect(settings.ocr.activeModelId).toBe("");
  // 返回欢迎页，或稍后配置：进聊天页，页上就是 Key 卡片，引导记为完成（D14）
  await page.getByTestId("onboarding-key-back").click();
  await expect(page.getByTestId("onboarding-welcome")).toBeVisible();
  await page.getByTestId("onboarding-later").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("chat-unconfigured")).toBeVisible();
  await expect(page.getByTestId("chat-key-input")).toHaveAttribute("type", "password");
  expect((await mockSettings(page)).ui.firstRunCompleted).toBe(true);
});

test("the voice guide refuses a non-English model folder until a valid one is picked", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always");
  // 旧配置或手改过的中文路径：sherpa 读不了
  await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).updateSettings({ models: { dir: "D:\\游戏模型" } }));
  await guideToModelStep(page);
  const setup = page.getByTestId("model-setup-section");
  await expect(setup.getByTestId("models-dir-path")).toContainText("D:\\游戏模型");
  await expect(setup.getByTestId("models-dir-error")).toContainText("纯英文");
  await expect(page.getByTestId("voice-guide-download")).toBeDisabled();
  await expect(page.getByTestId("voice-guide-next")).toBeDisabled();
  // 换成英文路径（浏览器预览里用 prompt 代替目录选择框）
  page.once("dialog", (dialog) => void dialog.accept("D:\\KotoneModels"));
  await setup.getByRole("button", { name: "更改", exact: true }).click();
  await expect(setup.getByTestId("models-dir-path")).toContainText("D:\\KotoneModels");
  await expect(setup.getByTestId("models-dir-error")).toHaveCount(0);
  await expect(page.getByTestId("voice-guide-download")).toBeEnabled();
});

test("a failed model download in the guide opens the manual-download dialog and keeps a retry", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always&mockDownload=fail");
  await guideToModelStep(page);
  await page.getByTestId("voice-guide-download").click();
  const guide = page.getByTestId("manual-download-dialog");
  await expect(guide).toBeVisible();
  await expect(guide.getByText(/网络连接失败/)).toBeVisible();
  await guide.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(guide).toHaveCount(0);
  await expect(page.getByTestId("voice-guide-model")).toContainText("网络连接失败");
  await expect(page.getByTestId("voice-guide-download")).toHaveText("重试下载");
  await expect(page.getByTestId("voice-guide-next")).toBeDisabled();
});

test("the hotkey step proactively blocks setup when the input environment is rejected", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always&mockInputEnvironment=blocked");
  await guideToModelStep(page);
  await page.getByTestId("voice-guide-download").click();
  await expect(page.getByTestId("voice-guide-model").getByText("✓ 已就绪", { exact: true })).toBeVisible();
  await page.getByTestId("voice-guide-next").click();

  await expect(page.getByTestId("input-environment-blocked")).toBeVisible();
  await expect(page.getByRole("button", { name: "重新检测", exact: true })).toBeVisible();
  await expect(page.getByTestId("voice-guide-hotkey").getByRole("button", { name: "重新录入", exact: true })).toBeDisabled();
  await expect(page.getByTestId("voice-guide-next")).toBeDisabled();
  // 仍然继续（风险自担）
  await page.getByRole("button", { name: "仍然继续", exact: true }).click();
  await expect(page.getByTestId("voice-guide-next")).toBeEnabled();
});

test("auto and never modes respect the persisted state", async ({ page }) => {
  // 每次换 ?search 都是一次真正的重新打开（只改 #hash 不会重新加载页面，mock 状态也不会重置）。
  // 新用户已完成引导（mock 默认）：auto 不弹
  await page.goto("/?case=auto#/settings");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await page.goto("/?case=never#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  // 0.1.x 用户：还没选过时 auto 弹升级欢迎（即使 firstRunCompleted 为真）；never 不弹，落在语音输入
  await page.goto("/?mockLegacyUpgrade=pending#/settings");
  await expect(page.getByTestId("onboarding-upgrade")).toBeVisible();
  await page.goto("/?mockLegacyUpgrade=pending&case=never#/settings?onboarding=never");
  await expect(page.getByTestId("voice-input")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  // 选过“继续使用语音输入法”之后：不再弹，落在语音输入
  await page.goto("/?mockLegacyUpgrade=dismissed#/settings");
  await expect(page.getByTestId("voice-input")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
});

test("a local model that is not downloaded is shown with a recoverable download instead of a start button", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page);
  // 语音输入页的卡片只说“还没下载”，下载和启动在「设置 → 模型 → 语音识别」（交接第十一节第 6 条）
  const card = page.getByTestId("asr-card");
  await expect(card).toHaveAttribute("data-asr-state", "local-stopped");
  await expect(card).toContainText("还没下载");
  await expect(card.getByRole("button", { name: /下载|启动/ })).toHaveCount(0);
  await page.getByTestId("asr-open-models").click();
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("model-category-asr")).toHaveAttribute("aria-selected", "true");
  const runtime = page.getByTestId("models-asr-runtime");
  await expect(runtime).toContainText("还没下载");
  await expect(page.getByTestId("asr-start")).toBeDisabled();
  // X-ASR 行带下载按钮；下好以后就能启动
  await expect(page.getByText("X-ASR 流式中英标点（int8，480ms 低延迟）", { exact: true })).toBeVisible();
  const xAsr = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05";
  await page.getByTestId(`model-download-${xAsr}`).click();
  await expect(page.getByTestId(`model-row-${xAsr}`)).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("asr-start")).toBeEnabled();
  await page.getByTestId("asr-start").click();
  await expect(runtime).toHaveAttribute("data-asr-state", "local-running");
});

test("a failed model download keeps an inline error and retry action", async ({ page }) => {
  await page.goto("/#/settings?mockDownload=fail");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-models").click();
  await page.getByRole("button", { name: "下载", exact: true }).first().click();

  const guide = page.getByTestId("manual-download-dialog");
  await expect(guide).toBeVisible();
  await expect(guide.getByText(/网络连接失败/)).toBeVisible();
  await guide.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.getByTestId("manual-download-dialog")).toHaveCount(0);
  await expect(page.getByText(/下载失败：网络连接失败/)).toBeVisible();
  await expect(page.getByRole("button", { name: "重试", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "手动下载", exact: true }).first()).toBeVisible();
});

test("a legacy upgrade that keeps voice input preserves the reminder entry and stays quiet", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending");
  const upgrade = page.getByTestId("onboarding-upgrade");
  await expect(upgrade.getByRole("heading")).toHaveText("Kotone 0.2：多了一位游戏陪玩");
  await expect(page.getByTestId("onboarding-welcome")).toHaveCount(0);
  await page.getByTestId("onboarding-keep-voice").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await expect(page.getByTestId("voice-input")).toBeVisible();
  // 侧栏完整（D4），但角色没启用：对话列表是空的说明，点聊天是 Key 卡片
  await expect(page.getByTestId("sidebar-conversations-empty")).toBeVisible();
  await expect(page.getByTestId("settings-nav-game")).toHaveText("游戏适配");
  await page.getByTestId("settings-nav-legacy-reminders").click();
  await expect(page.getByTestId("legacy-reminders-page")).toBeVisible();
  // 还没启用她：「设置」里不指向唤醒、交互这些，角色页也没有这四个分页（以前是「设置」里没有陪玩组）
  await page.getByTestId("settings-nav-advanced").click();
  await expect(page.getByTestId("advanced-companion-moved")).toHaveCount(0);
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-nav-identity")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-nav-call")).toHaveCount(0);
  await page.getByTestId("settings-nav-advanced").click();
  const settings = await mockSettings(page);
  expect(settings.ui.legacyUpgrade).toBe("dismissed");
  expect(settings.assistant.hotkey).toBe("");
});

test("an upgraded user explicitly picks a separate call key and keeps the old reminder settings", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured&mockModels=none");
  const original = await mockSettings(page);
  expect(original.assistant.hotkey).toBe("Alt+F9");
  await page.getByTestId("onboarding-meet-companion").click();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-connected")).toBeVisible();
  // 升级用户的呼叫键默认空、推荐 ~；不能和提醒助手热键相同
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("未设置");
  await page.getByTestId("onboarding-call-key-record").click();
  await page.keyboard.press("Alt+F9");
  await expect(page.getByTestId("onboarding-call-key-conflict")).toContainText("提醒助手热键");
  await expect(page.getByTestId("onboarding-enter-chat")).toBeDisabled();
  await page.getByTestId("onboarding-call-key-record").click();
  await page.keyboard.press("Control+Alt+F10");
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("Ctrl + Alt + F10");
  await page.getByTestId("onboarding-enter-chat").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);

  const next = await mockSettings(page);
  expect(next.ui.legacyUpgrade).toBe("optedIn");
  expect(next.assistant.hotkey).toBe(original.assistant.hotkey);
  expect(next.assistant.connectionId).toBe(original.assistant.connectionId);
  expect(next.assistant.companionBindings).toEqual({
    hotkey: "Ctrl+Alt+F10",
    connectionId: expect.stringMatching(/^deepseek-/),
  });
  // 第一次设的呼叫键写进「设置 → 热键」那张表（ADR 040 D3 的例外）
  expect(next.hotkeys?.bindings).toContainEqual({ key: "Ctrl+Alt+F10", command: "kotone.call" });
  expect(next.connections).toContainEqual(original.connections[0]);
  expect(next.sttEngine).toBe(original.sttEngine);
  expect(next.activeModelId).toBe(original.activeModelId);
  expect(next.activeProfileId).toBe(original.activeProfileId);
  expect(next.hotkey).toEqual(original.hotkey);

  // 提醒助手照旧：原来的连接和热键
  await page.getByTestId("settings-nav-legacy-reminders").click();
  await expect(page.getByTestId("legacy-reminders-page")).toBeVisible();
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByRole("combobox", { name: "提醒助手 API 连接" })).toContainText("原来的提醒连接");
  await expect(page.getByTestId("hotkey-input-legacy-reminder")).toHaveValue("Alt+F9");
});

test("a legacy key that fails the test leaves no connection behind and the user can still keep voice input", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured&mockCompanionCheck=fail");
  await expect(page.getByTestId("onboarding-upgrade")).toBeVisible();
  const original = await mockConnections(page);
  await page.getByTestId("onboarding-meet-companion").click();
  await page.getByTestId("onboarding-key-input").fill("sk-invalid");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-key-error")).toBeVisible();
  expect(await mockConnections(page)).toEqual(original);
  expect((await mockSettings(page)).ui.legacyUpgrade).toBe("pending");
  expect((await mockSettings(page)).assistant.companionBindings).toBeUndefined();
  // 返回升级欢迎页，或在 Key 页直接“继续使用语音输入法”
  await page.getByTestId("onboarding-key-back").click();
  await expect(page.getByTestId("onboarding-upgrade")).toBeVisible();
  await page.getByTestId("onboarding-meet-companion").click();
  await expect(page.getByTestId("onboarding-key-input")).toHaveValue("");
  await page.getByTestId("onboarding-key-later").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await expect(page.getByTestId("voice-input")).toBeVisible();
  expect((await mockSettings(page)).ui.legacyUpgrade).toBe("dismissed");
  expect(await mockConnections(page)).toEqual(original);
});
