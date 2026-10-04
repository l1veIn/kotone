import { expect, test } from "@playwright/test";

test("assistant opens on tasks, keeps drafts across sections and defaults to system speech without a download", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1000 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-nav-tasks")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-tasks")).toBeVisible();
  await expect(page.getByTestId("assistant-settings")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "待办提醒 1" })).toBeVisible();
  await page.getByTestId("assistant-request").fill("5 分钟后提醒我打野有闪");
  await page.screenshot({ path: testInfo.outputPath("assistant-tasks.png"), animations: "disabled" });
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByTestId("assistant-tasks")).not.toBeVisible();
  await expect(page.getByTestId("assistant-settings")).toBeVisible();
  await expect(page.getByLabel("语音引擎", { exact: true })).toHaveAttribute("data-value", "system");
  await expect(page.getByTestId("system-speech-info")).toHaveText("无需下载。");
  await expect(page.getByRole("button", { name: "下载语音模型", exact: true })).toHaveCount(0);
  await expect(page.getByTestId("assistant-mode-push-to-talk")).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("assistant-settings.png"), animations: "disabled" });
  await page.getByTestId("assistant-nav-tasks").click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("5 分钟后提醒我打野有闪");
});

test("assistant interaction modes save independently of regular input and survive section changes", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  const original = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByTestId("assistant-mode-solo")).toHaveCount(0);
  for (const mode of ["dictation", "one-shot", "push-to-talk"]) {
    await page.getByTestId(`assistant-mode-${mode}`).click();
    await expect(page.getByTestId(`assistant-mode-${mode}`)).toHaveAttribute("aria-pressed", "true");
    const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
    expect(saved.assistant.interactionMode).toBe(mode);
    expect(saved.interactionMode).toBe(original.interactionMode);
    expect(saved.hotkey).toEqual(original.hotkey);
  }
  await page.getByTestId("assistant-mode-dictation").click();
  await expect(page.getByTestId("assistant-mode-dictation")).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("assistant-nav-tasks").click();
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByTestId("assistant-mode-dictation")).toHaveAttribute("aria-pressed", "true");
});

test("assistant preview submits to the assistant instead of offering game send", async ({ page }) => {
  await page.goto("/#/overlay");
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, purpose: "assistant", state: "preview", finalText: "5 分钟后提醒我打野有闪" }));
  });
  await expect(page.getByText("助手热键 提交助手 · Esc 重说", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "发送", exact: true })).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("发送");
  await expect(page.getByText(/提交助手/).first()).toBeVisible();
});

test("default assistant walkie-talkie overlay tells the user to release the assistant hotkey", async ({ page }) => {
  await page.goto("/#/overlay");
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, purpose: "assistant", state: "listening", partialText: "" }));
  });
  await expect(page.getByText("助手收音中 · 松开助手热键提交", { exact: true })).toBeVisible();
  await expect(page.getByText(/再按助手热键预览/)).toHaveCount(0);
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, partialText: "打野有闪" }));
  });
  await expect(page.getByText(/^打野有闪/)).toBeVisible();
});

test("assistant preview labels fixtures, cancels reminders and never pretends to run AI", async ({ page }, testInfo) => {
  const externalRequests: string[] = [];
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && !request.url().startsWith("http://127.0.0.1:17420")) externalRequests.push(request.url());
  });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  const assistant = page.getByTestId("assistant-page");
  await expect(assistant.getByRole("heading", { name: "提醒助手", exact: true })).toBeVisible();
  await expect(assistant.getByText(/界面预览/)).toBeVisible();
  await expect(assistant.getByText("界面示例：打野有闪了")).toBeVisible();
  await assistant.getByRole("button", { name: "添加示例提醒", exact: true }).click();
  const fixture = assistant.getByRole("listitem").filter({ hasText: "界面示例：该打小龙了" });
  await expect(fixture).toContainText(/分 .*秒后/);
  await fixture.getByRole("button", { name: "取消", exact: true }).click();
  await expect(assistant.getByRole("heading", { name: "待办提醒 1" })).toBeVisible();
  await assistant.getByText("最近记录（1）", { exact: true }).click();
  await expect(fixture).toContainText("已取消");
  await assistant.getByTestId("assistant-request").fill("五分钟后提醒我打野有闪");
  await assistant.getByRole("button", { name: "创建提醒", exact: true }).click();
  await expect(assistant.getByRole("alert")).toContainText("浏览器仅预览界面");
  await expect(assistant.getByRole("heading", { name: "待办提醒 1" })).toBeVisible();
  expect(externalRequests).toEqual([]);
  await page.setViewportSize({ width: 1040, height: 1150 });
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("assistant-preview.png"), fullPage: true });
});

test("assistant shares connections and saves voice/delivery settings with an independent conflict-checked hotkey", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  const assistant = page.getByTestId("assistant-page");
  await assistant.getByRole("button", { name: "管理连接 →", exact: true }).click();
  await expect(page.getByTestId("advanced-nav-connections")).toHaveClass(/bg-kotone-cyan/);
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存密钥")).toBeVisible();
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  await assistant.getByLabel("提醒助手 AI 连接").click();
  await page.getByRole("option").nth(1).click();
  await expect(assistant.getByRole("button", { name: "管理连接 →", exact: true })).toHaveCount(0);
  await assistant.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("CapsLock");
  await expect(page.getByText("助手快捷键与录制热键（CapsLock）冲突，请换一个", { exact: true })).toBeVisible();
  await assistant.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("Control+F9");
  await expect(assistant.getByTestId("hotkey-input-assistant")).toHaveValue("Ctrl+F9");
  await expect(assistant.getByTestId("hotkey-status-assistant")).toContainText("当前生效：Ctrl+F9");

  await assistant.getByRole("checkbox", { name: "弹幕", exact: true }).uncheck();
  await expect(assistant.getByRole("checkbox", { name: "弹幕", exact: true })).not.toBeChecked();
  await assistant.getByLabel("播报音量").fill("65");
  await assistant.getByLabel("播报音量").dispatchEvent("change");
  await expect(assistant.getByText("65%", { exact: true })).toBeVisible();
  await assistant.getByLabel("播报语速").fill("2");
  await assistant.getByLabel("播报语速").dispatchEvent("change");
  await expect(assistant.getByText("+2", { exact: true })).toBeVisible();

  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant).toMatchObject({ hotkey: "Ctrl+F9", defaultDelivery: ["voice"], speechVolume: 65, speechRate: 2 });
  expect(saved.assistant.connectionId).not.toBe("");
  expect(saved.hotkey.key).toBe("CapsLock");

  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-hotkeys").click();
  const named = page.getByTestId("hotkey-input-post-process-toggle");
  await named.fill("Ctrl+F9");
  await named.press("Enter");
  await expect(page.getByText("文字处理开关热键与提醒助手热键（Ctrl+F9）冲突，请换一个", { exact: true })).toBeVisible();
});

test("settings modal traps Tab and restores focus after Escape", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-processing").click();
  const trigger = page.getByTestId("manage-postprocess-pipelines");
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "管理流程" });
  const controls = dialog.locator("button:not(:disabled), input:not(:disabled)");
  await expect(controls.first()).toBeFocused();
  await controls.last().focus();
  await page.keyboard.press("Tab");
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(controls.last()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
});
