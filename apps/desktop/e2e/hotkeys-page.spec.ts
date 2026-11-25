// ADR 040：冲突只在后端查一次（游戏配置的聊天键、控制台触发键、Esc 也算，第 1 期）；「设置 → 热键」列出全部热键，
// 热键只在这一页改，别的页面只显示现在的键和“去改”（第 2 期第 3 步，D3）。浏览器预览里的后端是 ipc/hotkeysMock.ts
// （照 kotone_core::hotkey_table / hotkey_table_cmd.rs 写）；真正的规矩在 Rust 里有单元测试。
import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

const mockSettings = (page: Page) => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());

/** 一个带控制台通道的游戏配置（内置配置都不带，ADR 032：没实测的不带），触发键是默认的 F23。 */
async function addConsoleProfile(page: Page) {
  await page.evaluate(async () => {
    const mock = await import("/src/lib/ipc/browserMock.ts");
    await mock.saveProfile({
      id: "dota2", displayName: "Dota 2", processNames: ["dota2.exe"], windowTitlePatterns: [],
      openChatKey: "Enter", sendKey: "Enter", preOpenDelayMs: 20, prePasteDelayMs: 20, preSendDelayMs: 100,
      preferClipboardPaste: false, hotwords: [], consoleChat: { cfgDir: "game/dota/cfg", triggerKey: "F23" },
    });
  });
}

/** 像别的地方（她、别的窗口）改了设置一样：存进预览后端，界面的设置跟着更新。 */
async function patchMock(page: Page, patch: Record<string, unknown>) {
  await page.evaluate(async (p) => {
    const [{ updateSettings }, { settingsStore }] = await Promise.all([import("/src/lib/ipc.ts"), import("/src/lib/stores/ui.ts")]);
    settingsStore.set(await updateSettings(p as never));
  }, patch);
}

test("the call key can't take a game's chat key, the console trigger key or Esc", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await addConsoleProfile(page);
  await openSettingsTab(page, "hotkeys");
  const input = page.getByTestId("hotkey-input-kotone.call");
  for (const [key, conflict] of [
    ["Enter", "《League of Legends》的打开聊天框键"],
    ["Shift+Enter", "《League of Legends》的「所有人」频道键"],
    ["F23", "《Dota 2》的控制台触发键"],
    ["Escape", "Esc"],
  ]) {
    await input.fill(key);
    await input.press("Enter");
    await expect(page.getByText(`呼叫键与${conflict}（${key}）冲突，请换一个`, { exact: true })).toBeVisible();
    expect((await mockSettings(page)).assistant.hotkey, key).toBe("Ctrl+Backquote");
  }
  // 修饰键不同就不冲突：Ctrl+Enter 不是游戏的 Enter
  await input.fill("Ctrl+Enter");
  await input.press("Enter");
  await expect(page.getByTestId("hotkey-status-kotone.call")).toContainText("当前生效：Ctrl+Enter");
  // 后端那一道：绕过界面直接存冲突的键也不让存，说清楚和谁撞了
  const refused = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).setHotkey("kotone.stop", "CapsLock").then(() => "", (e: Error) => e.message));
  expect(refused).toBe("「CapsLock」已经是「语音输入热键」的热键：先在「设置 → 热键」把它换掉，或者换一个键。");
});

test("设置 → 热键 lists every hotkey with its source and edits them in place", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await patchMock(page, { assistant: { companion: { quickMessages: [{ id: "q1", label: "", text: "集合打小龙", hotkey: "F8" }] } } });
  await openSettingsTab(page, "hotkeys");
  const list = page.getByTestId("hotkeys-page");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(10);
  const expectRow = async (id: string, name: string, source: string, key: string) => {
    await expect(page.getByTestId(`hotkeys-row-${id}`), id).toContainText(name);
    await expect(page.getByTestId(`hotkeys-source-${id}`), id).toHaveText(source);
    await expect(page.getByTestId(`hotkey-input-${id}`), id).toHaveValue(key);
  };
  await expectRow("kotone.call", "呼叫键", "Kotone", "Ctrl+Backquote");
  await expectRow("kotone.stop", "停下热键", "Kotone", "Ctrl+Shift+F12");
  await expectRow("kotone.speech-toggle", "启动 / 停止本地语音识别热键", "Kotone", "");
  await expectRow("voice-input.talk", "语音输入热键", "语音输入", "CapsLock");
  await expectRow("voice-input.channel-cycle", "频道切换热键", "语音输入", "Shift+CapsLock");
  await expectRow("voice-input.resend-last", "重发最近一条热键", "语音输入", "");
  // 快捷指令搬进了热键表（第 3 期），行 id 是 `custom.<它的 id>`；没起名字时名字就是那句话
  await expectRow("custom.q1", "快捷指令「集合打小龙」", "自己加的", "F8");
  // Esc 不能改：只列出来，没有输入框、没有“去改”
  await expect(page.getByTestId("hotkeys-row-escape")).toContainText("取消这次录音");
  await expect(page.getByTestId("hotkeys-key-escape")).toHaveText("Escape");
  await expect(page.getByTestId("hotkeys-row-escape")).toContainText("不能改");
  await expect(page.getByTestId("hotkeys-row-escape").locator("input")).toHaveCount(0);
  await expect(page.locator("[data-testid^='hotkeys-row-game.']")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-row-legacy-reminder")).toHaveCount(0);
  await expect(list.locator("[data-conflict='true']")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("hotkeys-page.png"), fullPage: true, animations: "disabled" });

  // 在表里改：快捷指令的键（预览把快捷指令的旧字段当存放处），唤醒页跟着显示
  const quick = page.getByTestId("hotkey-input-custom.q1");
  await quick.fill("F7");
  await quick.press("Enter");
  await expect(page.getByTestId("hotkey-status-custom.q1")).toHaveText("当前生效：F7");
  expect((await mockSettings(page)).assistant.companion.quickMessages[0].hotkey).toBe("F7");
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("hotkey-link-key-custom.q1")).toHaveText("F7");

  // 恢复默认：只有 D4 的三个默认键有
  await openSettingsTab(page, "hotkeys");
  await expect(page.getByTestId("hotkeys-reset-kotone.call")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-reset-voice-input.channel-cycle")).toHaveCount(0);
  const call = page.getByTestId("hotkey-input-kotone.call");
  await call.fill("Alt+F8");
  await call.press("Enter");
  await expect(page.getByTestId("hotkey-status-kotone.call")).toHaveText("当前生效：Alt+F8");
  await page.getByTestId("hotkeys-reset-kotone.call").click();
  await expect(page.getByText("呼叫键恢复成默认的 Ctrl + ~", { exact: true })).toBeVisible();
  await expect(call).toHaveValue("Ctrl+Backquote");
  const saved = await mockSettings(page);
  expect(saved.assistant.hotkey).toBe("Ctrl+Backquote");
  expect(saved.hotkeys?.bindings).toContainEqual({ key: "Ctrl+Backquote", command: "kotone.call" });

  // 搜索：名字、键、来源
  const search = page.getByTestId("hotkeys-search");
  await search.fill("caps");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(2);
  await expect(page.getByTestId("hotkeys-row-voice-input.talk")).toBeVisible();
  await expect(page.getByTestId("hotkeys-row-voice-input.channel-cycle")).toBeVisible();
  await search.fill("自己加的");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(1);
  await search.fill("语音输入");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(5);
  await search.fill("没有这个键");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-empty")).toContainText("没有和“没有这个键”对得上的热键");

  // 聊天里的“去看看：设置 → 热键”落到这一页
  await page.getByTestId("settings-nav-game").click();
  await page.evaluate(async () => (await import("/src/lib/stores/navigation.ts")).openLocation("设置 → 热键"));
  await expect(page.getByTestId("advanced-nav-hotkeys")).toHaveAttribute("aria-current", "page");
});

test("conflicts are marked red, including a game's chat key and Esc", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await addConsoleProfile(page);
  // 已经冲突着的旧配置（以前前端不查游戏的键和 Esc；或者别的地方改的）：表里标出来
  await patchMock(page, { automationStopHotkey: "Enter", resendLastHotkey: "Ctrl+Backquote", runtimeToggleHotkey: "Ctrl+Escape", togglePostProcessingHotkey: "F23" });
  await openSettingsTab(page, "hotkeys");
  const red = (id: string) => expect(page.getByTestId(`hotkeys-row-${id}`), id).toHaveAttribute("data-conflict", "true");
  for (const id of ["kotone.stop", "kotone.call", "voice-input.resend-last", "kotone.speech-toggle", "escape", "voice-input.post-process-toggle"]) await red(id);
  await expect(page.getByTestId("hotkeys-row-voice-input.talk")).toHaveAttribute("data-conflict", "false");
  await expect(page.getByTestId("hotkeys-conflict-kotone.stop")).toHaveText(
    "和《League of Legends》的打开聊天框键、《通用（任意前台窗口）》的打开聊天框键、《Dota 2》的打开聊天框键冲突");
  await expect(page.getByTestId("hotkeys-conflict-kotone.call")).toHaveText("和重发最近一条热键冲突");
  await expect(page.getByTestId("hotkeys-conflict-voice-input.resend-last")).toHaveText("和呼叫键冲突");
  await expect(page.getByTestId("hotkeys-conflict-kotone.speech-toggle")).toHaveText("和Esc冲突");
  await expect(page.getByTestId("hotkeys-conflict-escape")).toHaveText("和启动 / 停止本地语音识别热键冲突");
  await expect(page.getByTestId("hotkeys-conflict-voice-input.post-process-toggle")).toHaveText("和《Dota 2》的控制台触发键冲突");
  await page.screenshot({ path: testInfo.outputPath("hotkeys-conflicts.png"), fullPage: true, animations: "disabled" });
  // 在表里改掉：标红跟着消失
  const resend = page.getByTestId("hotkey-input-voice-input.resend-last");
  await resend.fill("Ctrl+F7");
  await resend.press("Enter");
  await expect(page.getByTestId("hotkey-status-voice-input.resend-last")).toContainText("Ctrl+F7");
  await expect(page.getByTestId("hotkeys-row-kotone.call")).toHaveAttribute("data-conflict", "false");
});

test("other pages show the current key and 去改 opens its row in 设置 → 热键", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await patchMock(page, { assistant: { companion: { quickMessages: [{ id: "q1", label: "", text: "集合", hotkey: "F8" }] } } });
  const goTo = async (id: string) => {
    await page.getByTestId(`hotkey-link-goto-${id}`).click();
    await expect(page.getByTestId("advanced-nav-hotkeys")).toHaveAttribute("aria-current", "page");
    await expect(page.getByTestId(`hotkeys-row-${id}`)).toHaveAttribute("data-focused", "true");
    await expect(page.getByTestId(`hotkey-input-${id}`)).toBeVisible();
  };
  // 唤醒：呼叫键、快捷指令的键
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("hotkey-link-key-kotone.call")).toHaveText("Ctrl + ~");
  await expect(page.getByTestId("settings-call").locator("input[aria-label='呼叫键']")).toHaveCount(0);
  await goTo("kotone.call");
  await openSettingsTab(page, "call");
  await goTo("custom.q1");
  // 设置 → 系统 → 停下
  await openSettingsTab(page, "system");
  await expect(page.getByTestId("hotkey-link-key-kotone.stop")).toHaveText("Ctrl + Shift + F12");
  await goTo("kotone.stop");
  // 设置 → 模型 → 语音识别（本地识别时）
  await openSettingsTab(page, "models");
  await goTo("kotone.speech-toggle");
  // 语音输入：说话、其他热键
  await openVoiceInput(page);
  await expect(page.getByTestId("hotkey-link-key-voice-input.talk")).toHaveText("CapsLock");
  await goTo("voice-input.talk");
  await openVoiceInput(page);
  await goTo("voice-input.resend-last");
});

test("the reminder assistant hotkey stays on its own page (D10)", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never&mockLegacyUpgrade=dismissed&mockLegacyReminder=configured");
  await openSettingsTab(page, "hotkeys");
  const row = page.getByTestId("hotkeys-row-legacy-reminder");
  await expect(row).toContainText("提醒助手热键");
  await expect(page.getByTestId("hotkeys-key-legacy-reminder")).toHaveText("Alt + F9");
  await expect(row.locator("input")).toHaveCount(0);
  // 只用语音输入的老用户没有呼叫键、停下热键
  await expect(page.getByTestId("hotkeys-row-kotone.call")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-row-kotone.stop")).toHaveCount(0);
  await page.getByTestId("hotkeys-goto-legacy-reminder").click();
  await expect(page.getByTestId("legacy-reminders-page")).toBeVisible();
});
