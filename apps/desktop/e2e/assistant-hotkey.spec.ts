import { expect, test } from "@playwright/test";
import { openChat, openSettingsTab } from "./nav";

// 热键都在「设置 → 热键」改（ADR 040 D3）：每一行是同一个可录、可打、可清的热键框。

test("the call key row in 设置 → 热键 is editable, captured and clearable like every other hotkey", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "hotkeys");
  const input = page.getByTestId("hotkey-input-kotone.call");
  const capture = page.getByTestId("hotkey-capture-kotone.call");
  const status = page.getByTestId("hotkey-status-kotone.call");
  // 呼叫琴音默认是 Ctrl + ~（单独的 ~ 是英雄联盟的“只攻击英雄”键）
  await expect(input).toHaveValue("Ctrl+Backquote");
  await expect(status).toHaveText("当前生效：Ctrl+Backquote");
  await input.fill("Alt+F8");
  await input.press("Enter");
  await expect(status).toHaveText("当前生效：Alt+F8");
  await capture.click();
  await expect(capture).toContainText("鼠标侧键");
  await page.keyboard.press("Escape");
  await expect(input).toHaveValue("Alt+F8");
  await expect(status).toHaveText("当前生效：Alt+F8");
  await capture.click();
  await page.evaluate(() => window.dispatchEvent(new MouseEvent("mousedown", {button: 3, ctrlKey: true})));
  await expect(input).toHaveValue("Ctrl+Mouse4");
  await expect(status).toHaveText("当前生效：Ctrl+Mouse4");
  await page.getByTestId("hotkey-clear-kotone.call").click();
  await expect(input).toHaveValue("");
  await expect(status).toHaveText("未设置");
  await input.fill("F24");
  await page.getByTestId("hotkey-save-kotone.call").click();
  await expect(status).toHaveText("当前生效：F24");
  // 离开再回来还在；唤醒页显示同一个键
  await openChat(page);
  await openSettingsTab(page, "hotkeys");
  await expect(input).toHaveValue("F24");
  await expect(status).toHaveText("当前生效：F24");
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("hotkey-link-key-kotone.call")).toHaveText("F24");
});

test("leaving the hotkeys page during capture cannot save a later key or leave capture active", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "hotkeys");
  await page.getByTestId("hotkey-capture-kotone.call").click();
  await openChat(page);
  await page.keyboard.press("F9");
  await openSettingsTab(page, "hotkeys");
  await expect(page.getByTestId("hotkey-input-kotone.call")).toHaveValue("Ctrl+Backquote");
  await page.getByTestId("hotkey-capture-voice-input.post-process-toggle").click();
  await page.keyboard.press("F10");
  await expect(page.getByTestId("hotkey-input-voice-input.post-process-toggle")).toHaveValue("F10");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.hotkey).toBe("Ctrl+Backquote");
  expect(saved.togglePostProcessingHotkey).toBe("F10");
  // 再录一次呼叫键照常
  await page.getByTestId("hotkey-capture-kotone.call").click();
  await page.keyboard.press("F11");
  await expect(page.getByTestId("hotkey-input-kotone.call")).toHaveValue("F11");
});

test("one capture owner receives input when another hotkey row tries to record", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "hotkeys");
  await page.getByTestId("hotkey-capture-voice-input.post-process-toggle").click();
  await page.getByTestId("hotkey-capture-voice-input.post-process-cycle").click();
  await expect(page.getByText("已有热键录入进行中，请先完成或按 Esc 取消", { exact: true })).toBeVisible();
  await page.keyboard.press("F9");
  await expect(page.getByTestId("hotkey-input-voice-input.post-process-toggle")).toHaveValue("F9");
  await expect(page.getByTestId("hotkey-input-voice-input.post-process-cycle")).toHaveValue("");
});

test("aborting while the native capture starts waits for cleanup before accepting a new owner", async ({ page }) => {
  await page.route("**/__capture-test", (route) => route.fulfill({ contentType: "text/html", body: "<!doctype html><title>Capture test</title>" }));
  await page.goto("/__capture-test");
  const result = await page.evaluate(async () => {
    const { mockIPC } = await import("/node_modules/@tauri-apps/api/mocks.js");
    let releaseStart!: () => void;
    let reportStart!: () => void;
    let reportCancel!: () => void;
    const gate = new Promise<void>((resolve) => { releaseStart = resolve; });
    const started = new Promise<void>((resolve) => { reportStart = resolve; });
    const cancelled = new Promise<void>((resolve) => { reportCancel = resolve; });
    let starts = 0;
    let cancels = 0;
    mockIPC((command) => {
      if (command === "start_hotkey_capture" && ++starts === 1) { reportStart(); return gate; }
      if (command === "cancel_hotkey_capture") { cancels++; reportCancel(); }
    }, { shouldMockEvents: true });
    const { captureHotkey, isHotkeyCaptureActive } = await import("/src/lib/hotkeyCapture.ts");
    const controller = new AbortController();
    const stale: unknown[] = [];
    const waiting = captureHotkey((event) => stale.push(event), { signal: controller.signal });
    await started;
    controller.abort();
    const blocked: unknown[] = [];
    await captureHotkey((event) => blocked.push(event));
    const heldUntilCleanup = isHotkeyCaptureActive();
    releaseStart();
    await waiting;
    await cancelled;
    await Promise.resolve();
    await Promise.resolve();
    const activeAfterAbort = isHotkeyCaptureActive();
    let completed!: (event: unknown) => void;
    const nextResult = new Promise((resolve) => { completed = resolve; });
    await captureHotkey(completed);
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("kotone://hotkey-capture", { combo: "Ctrl+F9" });
    return { stale, blocked, heldUntilCleanup, activeAfterAbort, cancels, next: await nextResult };
  });
  expect(result.stale).toEqual([]);
  expect(result.blocked).toMatchObject([{ kind: "error" }]);
  expect(result.heldUntilCleanup).toBe(true);
  expect(result.activeAfterAbort).toBe(false);
  expect(result.cancels).toBe(1);
  expect(result.next).toEqual({ kind: "combo", combo: "Ctrl+F9" });
});
