import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// ADR 051 英雄联盟玩家的 happy path：不管从哪存的 DeepSeek Key，她都换成 DeepSeek，文字识别、语音合成、联网搜索跟着换；
// 聊天页最上面的“准备好了吗”一行一样，全都 ✓ 就不显示。三种起点：新用户、0.1.x 用户（旧提醒助手用千问）、0.1.x 就配过 DeepSeek。
// 浏览器预览里装成 Windows、已经是管理员身份（管理员那一行也算进来）。

const settingsOf = (page: Page) => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());

/** 装成 Windows：默认是管理员身份（ADR 044 的说明框已经看过）。 */
async function open(page: Page, query: string, elevation: { elevated?: boolean; runAsAdminOnStart?: boolean } = {}) {
  await page.addInitScript((seed) => {
    (window as unknown as { __kotoneElevationSeed: unknown }).__kotoneElevationSeed = {
      status: { supported: true, elevated: seed.elevated ?? true, adminPromptRevision: 1 },
      settings: { adminPromptRevision: 1, runAsAdminOnStart: seed.runAsAdminOnStart ?? true },
    };
  }, elevation);
  await page.goto(`/${query}`);
}

/** “准备好了吗”里打 ✗ 的行。 */
async function failing(page: Page): Promise<string[]> {
  await expect(page.getByTestId("chat-ready")).toBeVisible();
  return page.locator('li[data-testid^="chat-ready-"][data-ok="no"]').evaluateAll((items) =>
    items.map((item) => item.getAttribute("data-testid")!.replace("chat-ready-", "")));
}

/** 全都 ✓：清单不显示，六样都查过。 */
async function expectAllReady(page: Page) {
  await expect(page.getByTestId("chat-ready")).toHaveCount(0);
  await expect(page.getByTestId("chat-ready-all")).toHaveAttribute("data-rows", "key model call admin lol ocr");
}

async function addDeepSeekInConnections(page: Page) {
  await openSettingsTab(page, "connections");
  await page.getByTestId("connection-preset-deepseek").click();
  const editor = page.getByTestId("connection-editor");
  await expect(editor.getByTestId("connection-deepseek-note")).toContainText("换成 DeepSeek");
  await expect(editor.getByTestId("connection-use-after-save")).toHaveCount(0);
  await editor.getByTestId("connection-api-key").fill("sk-test-123");
  await editor.getByTestId("connection-save").click();
  await expect(page.getByText(/换成了 DeepSeek/).first()).toBeVisible();
}

async function backToChat(page: Page) {
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("assistant-tasks")).toBeVisible();
}

test("a · new user: one key in onboarding → she and everything else on DeepSeek → nothing left on the checklist", async ({ page }, testInfo) => {
  await open(page, "#/settings?onboarding=always&mockModels=none");
  await page.getByTestId("onboarding-add-key").click();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("Ctrl + ~");
  await page.getByTestId("onboarding-enter-chat").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);

  const s = await settingsOf(page);
  expect(s.connections.find((c) => c.id === s.assistant.connectionId)?.provider).toBe("deepseek");
  expect(s.ocr.activeModelId).toBe("deepseek-ocr");
  expect(s.assistant.ttsModelId).toBe("deepseek-tts");
  expect(s.search.activeModelId).toBe("deepseek-search");
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-flash");
  await expectAllReady(page);
  await screenshot(page, { path: testInfo.outputPath("a-chat.png") });
});

test("b1 · 0.1.x user whose reminders used Qwen: the upgrade onboarding puts her on DeepSeek and leaves the reminders alone", async ({ page }) => {
  await open(page, "#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=qwen&mockModels=none");
  const before = await settingsOf(page);
  expect(before.assistant.connectionId).toBe("original-qwen");
  await page.getByTestId("onboarding-meet-companion").click();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-connected")).toBeVisible();
  await page.getByTestId("onboarding-call-key-recommended").click();
  await page.getByTestId("onboarding-enter-chat").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);

  const s = await settingsOf(page);
  expect(s.ui.legacyUpgrade).toBe("optedIn");
  expect(s.connections.find((c) => c.id === s.assistant.companionBindings?.connectionId)?.provider).toBe("deepseek");
  expect(s.assistant.companionBindings?.hotkey).toBe("Ctrl+Backquote");
  // 旧提醒助手照旧用千问；语音识别不动
  expect(s.assistant.connectionId).toBe("original-qwen");
  expect(s.assistant.hotkey).toBe("Alt+F9");
  expect(s.sttEngine).toBe(before.sttEngine);
  expect(s.ocr.activeModelId).toBe("deepseek-ocr");
  await backToChat(page);
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-flash");
  await expectAllReady(page);
});

test("b2 · 0.1.x user who is on the old Qwen: adding DeepSeek in 「API 连接」 switches her; picking Qwen again is respected", async ({ page }, testInfo) => {
  await open(page, "#/settings?onboarding=never&mockLegacyUpgrade=optedIn&mockLegacyReminder=qwen");
  // 起点：她用的是旧提醒助手那条千问（那位老用户的情形）
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "qwen-turbo");
  expect(await failing(page)).toEqual(["key", "ocr"]);
  await screenshot(page, { path: testInfo.outputPath("b2-before.png") });
  const before = await settingsOf(page);

  await addDeepSeekInConnections(page);
  const s = await settingsOf(page);
  expect(s.connections.find((c) => c.id === s.assistant.companionBindings?.connectionId)?.provider).toBe("deepseek");
  expect(s.assistant.companionBindings?.hotkey).toBe("Ctrl+Backquote");
  expect(s.assistant.connectionId).toBe("original-qwen");
  expect(s.sttEngine).toBe(before.sttEngine);
  expect(s.ocr.activeModelId).toBe("deepseek-ocr");
  expect(s.assistant.ttsModelId).toBe("deepseek-tts");
  expect(s.search.activeModelId).toBe("deepseek-search");
  await expect(page.locator('[data-testid^="connection-card-deepseek-"]')).toContainText("琴音在用");

  await backToChat(page);
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-flash");
  await expectAllReady(page);

  // 玩家自己在“她用的模型”里换回千问：照他的，不换回去；清单里多一行“换成 DeepSeek”
  await page.getByTestId("composer-model").click();
  await page.getByTestId("composer-model-original-qwen-qwen-turbo").click();
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "qwen-turbo");
  expect(await failing(page)).toEqual(["model"]);
  expect((await settingsOf(page)).assistant.companionBindings?.connectionId).toBe("original-qwen");
  await page.getByTestId("chat-ready-model-fix").click();
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-flash");
  await expectAllReady(page);
});

test("b3 · 0.1.x user on Qwen fills the key from the checklist: her connection changes, the reminder one does not", async ({ page }) => {
  await open(page, "#/settings?onboarding=never&mockLegacyUpgrade=optedIn&mockLegacyReminder=qwen");
  expect(await failing(page)).toEqual(["key", "ocr"]);
  await page.getByTestId("chat-ready-key-fix").click();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-submit").click();
  // 已经启用她的：呼叫键照旧
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("Ctrl + ~");
  await page.getByTestId("onboarding-enter-chat").click();
  const s = await settingsOf(page);
  expect(s.connections.find((c) => c.id === s.assistant.companionBindings?.connectionId)?.provider).toBe("deepseek");
  expect(s.assistant.connectionId).toBe("original-qwen");
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-flash");
  await expectAllReady(page);
});

test("b4 · 0.1.x user who kept voice input adds DeepSeek in 「API 连接」: she is switched on, only the call key is left", async ({ page }) => {
  await open(page, "#/settings?mockLegacyUpgrade=dismissed&mockLegacyReminder=qwen");
  await expect(page.getByTestId("voice-input")).toBeVisible();
  await addDeepSeekInConnections(page);
  const s = await settingsOf(page);
  expect(s.ui.legacyUpgrade).toBe("optedIn");
  expect(s.connections.find((c) => c.id === s.assistant.companionBindings?.connectionId)?.provider).toBe("deepseek");
  expect(s.assistant.connectionId).toBe("original-qwen");
  expect(s.assistant.hotkey).toBe("Alt+F9");
  await backToChat(page);
  // 升级用户的呼叫键默认空：清单里只剩这一行，按钮去角色页的「唤醒」
  expect(await failing(page)).toEqual(["call"]);
  await page.getByTestId("chat-ready-call-fix").click();
  await expect(page.getByTestId("assistant-nav-call")).toHaveAttribute("aria-current", "page");
});

test("c · 0.1.x user who already had DeepSeek: she stays on it and the one missing piece is fixed in place", async ({ page }) => {
  await open(page, "#/settings?onboarding=never&mockLegacyUpgrade=optedIn&mockLegacyReminder=deepseek");
  await expect(page.getByTestId("composer-model")).toHaveAttribute("data-model", "deepseek-chat");
  // 0.1.x 没有文字识别：一键换成 DeepSeek 的
  expect(await failing(page)).toEqual(["ocr"]);
  await page.getByTestId("chat-ready-ocr-fix").click();
  await expectAllReady(page);
  const s = await settingsOf(page);
  expect(s.ocr.activeModelId).toBe("deepseek-ocr");
  expect(s.assistant.companionBindings?.connectionId).toBe("original-deepseek");
});

test("c2 · 0.1.x user who already had DeepSeek goes through the upgrade onboarding: ends on DeepSeek, nothing left", async ({ page }) => {
  await open(page, "#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=deepseek&mockModels=none");
  await page.getByTestId("onboarding-meet-companion").click();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-submit").click();
  await page.getByTestId("onboarding-call-key-recommended").click();
  await page.getByTestId("onboarding-enter-chat").click();
  const s = await settingsOf(page);
  expect(s.connections.find((c) => c.id === s.assistant.companionBindings?.connectionId)?.provider).toBe("deepseek");
  expect(s.assistant.connectionId).toBe("original-deepseek");
  await backToChat(page);
  await expectAllReady(page);
});

test("the administrator and LoL rows: not elevated and plugin off show ✗, each button fixes its row", async ({ page }) => {
  await open(page, "#/settings?onboarding=never", { elevated: false, runAsAdminOnStart: false });
  await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).updateSettings({ plugins: { lol: { enabled: false } } }));
  // 没有连接：聊天页是 Key 卡片，清单还不出现
  await expect(page.getByTestId("chat-key-input")).toBeVisible();
  await expect(page.getByTestId("chat-ready")).toHaveCount(0);
  await page.getByTestId("chat-key-input").fill("sk-test-123");
  await page.getByTestId("chat-key-submit").click();
  await page.getByTestId("onboarding-enter-chat").click();
  expect(await failing(page)).toEqual(["admin", "lol"]);
  await expect(page.getByTestId("chat-ready-lol")).toContainText("英雄联盟插件关着");

  await page.getByTestId("chat-ready-lol-fix").click();
  await expect(page.getByTestId("chat-ready-lol")).toHaveAttribute("data-ok", "yes");
  expect((await settingsOf(page)).plugins?.lol?.enabled).toBe(true);

  await page.getByTestId("chat-ready-admin-fix").click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __kotoneRestartAsAdminCalls?: number }).__kotoneRestartAsAdminCalls ?? 0)).toBe(1);
  expect((await settingsOf(page)).runAsAdminOnStart).toBe(true);
});

test("a DeepSeek key that fails its test in 「API 连接」 is kept but she is not switched", async ({ page }) => {
  await open(page, "?mockCompanionCheck=fail#/settings?onboarding=never&mockLegacyUpgrade=optedIn&mockLegacyReminder=qwen");
  await openSettingsTab(page, "connections");
  await page.getByTestId("connection-preset-deepseek").click();
  await page.getByTestId("connection-api-key").fill("sk-wrong");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText(/Key 没通过测试，琴音没换/).first()).toBeVisible();
  const card = page.locator('[data-testid^="connection-card-deepseek-"]');
  await expect(card.getByTestId("connection-check-result")).toContainText("模型列表里没有");
  const s = await settingsOf(page);
  expect(s.assistant.companionBindings?.connectionId).toBe("original-qwen");
  expect(s.ocr.activeModelId).toBe("");
});
