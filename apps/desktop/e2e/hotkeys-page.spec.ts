// ADR 040 第 1 期：冲突只在后端查一次（游戏配置的聊天键、控制台触发键、Esc 也算），
// 「设置 → 热键」一页只读列出全部热键。浏览器预览里的后端是 ipc/hotkeysMock.ts（照 kotone_core::hotkey_table 写）；
// 真正的冲突规矩在 Rust 里有单元测试（crates/kotone-core/src/hotkey_table_tests.rs）。
import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab } from "./nav";

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

/** 像别的地方（她、别的页面）改了设置一样：存进预览后端，界面的设置跟着更新。 */
async function patchMock(page: Page, patch: Record<string, unknown>) {
  await page.evaluate(async (p) => {
    const [{ updateSettings }, { settingsStore }] = await Promise.all([import("/src/lib/ipc.ts"), import("/src/lib/stores/ui.ts")]);
    settingsStore.set(await updateSettings(p as never));
  }, patch);
}

test("the call key can't take a game's chat key, the console trigger key or Esc", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await addConsoleProfile(page);
  await openSettingsTab(page, "call");
  const input = page.getByTestId("hotkey-input-assistant");
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
  await expect(page.getByTestId("hotkey-status-assistant")).toContainText("当前生效：Ctrl+Enter");
});

test("设置 → 热键 lists every hotkey with its source; fixed keys are read-only", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await patchMock(page, { assistant: { companion: { quickMessages: [{ id: "q1", label: "", text: "集合打小龙", hotkey: "F8" }] } } });
  await openSettingsTab(page, "hotkeys");
  const list = page.getByTestId("hotkeys-page");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(10);
  const expectRow = async (id: string, name: string, source: string, key: string) => {
    const row = page.getByTestId(`hotkeys-row-${id}`);
    await expect(row, id).toContainText(name);
    await expect(page.getByTestId(`hotkeys-source-${id}`), id).toHaveText(source);
    await expect(page.getByTestId(`hotkeys-key-${id}`), id).toHaveText(key);
  };
  await expectRow("assistant", "呼叫键", "Kotone", "Ctrl + ~");
  await expectRow("automation-stop", "停下热键", "Kotone", "Ctrl + Shift + F12");
  await expectRow("runtime-toggle", "启动 / 停止本地语音识别热键", "Kotone", "未设置");
  await expectRow("escape", "Esc", "固定", "Escape");
  await expectRow("record", "语音输入热键", "语音输入", "CapsLock");
  await expectRow("channel-cycle", "频道切换热键", "语音输入", "Shift + CapsLock");
  await expectRow("resend-last", "重发最近一条热键", "语音输入", "未设置");
  await expectRow("quick-message-1", "快捷指令 1 热键", "自己加的", "F8");
  await expect(page.getByTestId("hotkeys-row-quick-message-1")).toContainText("集合打小龙");
  await expect(page.getByTestId("hotkeys-row-escape")).toContainText("取消这次录音");
  // Esc 不能改：没有“去改”；游戏的聊天键不进表；0.2 新用户没有提醒助手热键
  await expect(page.getByTestId("hotkeys-goto-escape")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-row-escape")).toContainText("不能改");
  await expect(page.locator("[data-testid^='hotkeys-row-game.']")).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-row-legacy-reminder")).toHaveCount(0);
  await expect(list.locator("[data-conflict='true']")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("hotkeys-page.png"), fullPage: true, animations: "disabled" });

  // 搜索：名字、键、来源
  const search = page.getByTestId("hotkeys-search");
  await search.fill("caps");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(2);
  await expect(page.getByTestId("hotkeys-row-record")).toBeVisible();
  await expect(page.getByTestId("hotkeys-row-channel-cycle")).toBeVisible();
  await search.fill("自己加的");
  await expect(list.locator("[data-testid^='hotkeys-row-']")).toHaveCount(1);
  await expect(page.getByTestId("hotkeys-row-quick-message-1")).toBeVisible();
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
  for (const id of ["automation-stop", "assistant", "resend-last", "runtime-toggle", "escape", "post-process-toggle"]) await red(id);
  await expect(page.getByTestId("hotkeys-row-record")).toHaveAttribute("data-conflict", "false");
  await expect(page.getByTestId("hotkeys-conflict-automation-stop")).toHaveText(
    "和《League of Legends》的打开聊天框键、《通用（任意前台窗口）》的打开聊天框键、《Dota 2》的打开聊天框键冲突");
  await expect(page.getByTestId("hotkeys-conflict-assistant")).toHaveText("和重发最近一条热键冲突");
  await expect(page.getByTestId("hotkeys-conflict-resend-last")).toHaveText("和呼叫键冲突");
  await expect(page.getByTestId("hotkeys-conflict-runtime-toggle")).toHaveText("和Esc冲突");
  await expect(page.getByTestId("hotkeys-conflict-escape")).toHaveText("和启动 / 停止本地语音识别热键冲突");
  await expect(page.getByTestId("hotkeys-conflict-post-process-toggle")).toHaveText("和《Dota 2》的控制台触发键冲突");
  await page.screenshot({ path: testInfo.outputPath("hotkeys-conflicts.png"), fullPage: true, animations: "disabled" });
  // 改掉以后（别的页面改了键）表跟着变
  await page.getByTestId("hotkeys-goto-resend-last").click();
  const resend = page.getByTestId("hotkey-input-resend-last");
  await resend.fill("Ctrl+F7");
  await resend.press("Enter");
  await expect(page.getByTestId("hotkey-status-resend-last")).toContainText("Ctrl+F7");
  await openSettingsTab(page, "hotkeys");
  await expect(page.getByTestId("hotkeys-row-assistant")).toHaveAttribute("data-conflict", "false");
});

test("每行的“去改”跳到现在改它的那个页面", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await patchMock(page, { assistant: { companion: { quickMessages: [{ id: "q1", label: "", text: "集合", hotkey: "F8" }] } } });
  const go = async (id: string) => {
    await openSettingsTab(page, "hotkeys");
    await page.getByTestId(`hotkeys-goto-${id}`).click();
  };
  await go("assistant");
  await expect(page.getByTestId("assistant-nav-call")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("hotkey-input-assistant")).toBeVisible();
  await go("quick-message-1");
  await expect(page.getByTestId("assistant-nav-call")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("hotkey-input-quick-message-1")).toBeVisible();
  await go("automation-stop");
  await expect(page.getByTestId("advanced-nav-system")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("hotkey-input-automation-stop")).toBeVisible();
  await go("runtime-toggle");
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  await go("record");
  await expect(page.getByTestId("voice-tab-general")).toHaveAttribute("aria-current", "page");
  await go("channel-cycle");
  await expect(page.getByTestId("voice-tab-general")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("hotkey-input-channel-cycle")).toBeVisible();
});
