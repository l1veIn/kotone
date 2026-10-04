import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    assistantVoiceTest: {
      calls: { command: string; sessionId?: string }[];
      holdBegin: boolean;
      holdEnd: boolean;
      resolveBegin(): Promise<void>;
      resolveEnd(): Promise<void>;
      state(state: string, purpose?: string): Promise<void>;
      runtime(phase: string): Promise<void>;
      mode(mode: "push-to-talk" | "dictation" | "one-shot"): Promise<void>;
    };
  }
}

async function openVoice(page: Page, mode: "push-to-talk" | "dictation" | "one-shot" = "push-to-talk", ready = true) {
  await page.route("**/__assistant-voice-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__assistant-voice-test#/settings?onboarding=never");
  await page.evaluate(async ({ mode, ready }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    let sequence = 0;
    let session: { sessionId: string; interactionMode: string } | undefined;
    let beginResolve: (() => Promise<void>) | undefined;
    let endResolve: (() => Promise<void>) | undefined;
    const runtime = { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null };
    const bridge = {
      calls: [] as { command: string; sessionId?: string }[], holdBegin: false, holdEnd: false,
      async resolveBegin() { await beginResolve?.(); beginResolve = undefined; },
      async resolveEnd() { await endResolve?.(); endResolve = undefined; },
      async state(state: string, purpose = "assistant") {
        await emit("kotone://state", { state, payload: { purpose, inputSource: "button", text: state === "preview" ? "5 分钟后提醒我打野有闪" : null } });
      },
      async runtime(phase: string) { runtime.phase = phase; await emit("kotone://runtime", runtime); },
      async mode(next: "push-to-talk" | "dictation" | "one-shot") {
        const settings = await browser.updateSettings({ assistant: { interactionMode: next } });
        const { settingsStore } = await import("/src/lib/stores/ui.ts");
        settingsStore.set(settings);
      },
    };
    Object.assign(window, { assistantVoiceTest: bridge });
    await browser.updateSettings({ assistant: { interactionMode: mode, connectionId: ready ? "test-ai" : "", hotkey: "" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      if (command.startsWith("assistant_voice_")) bridge.calls.push({ command, sessionId: args?.sessionId });
      if (command === "assistant_voice_begin") {
        const snapshot = { sessionId: `voice-${++sequence}`, interactionMode: (await browser.getSettings()).assistant.interactionMode };
        const begin = async () => { session = snapshot; await bridge.state("listening"); return snapshot; };
        if (!bridge.holdBegin) return begin();
        return new Promise((resolve) => { beginResolve = async () => { resolve(await begin()); }; });
      }
      if (command === "assistant_voice_cancel") { await bridge.state("idle"); return; }
      if (command === "assistant_voice_end" || command === "assistant_voice_confirm") {
        if (args.sessionId !== session?.sessionId) throw new Error("过期会话");
        if (command === "assistant_voice_end" && session.interactionMode === "dictation") { await bridge.state("preview"); return; }
        await bridge.state("processing");
        if (!bridge.holdEnd) { await bridge.state("success"); return; }
        return new Promise<void>((resolve) => { endResolve = async () => { resolve(); }; });
      }
      if (command === "get_runtime_status") return { ...runtime };
      if (command === "list_connections") return ready ? [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }] : [];
      if (command === "list_reminders") return [];
      if (command === "list_speech_voices") return [];
      if (command === "get_speech_model_status") return { modelId: "fixture", installed: false, downloading: false, downloadedBytes: 0, totalBytes: 0, phase: "idle", error: null, modelDir: "fixture" };
      if (command === "get_startup_options") return { onboarding: "never" };
      if (command === "update_settings") return browser.updateSettings(args.patch);
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      return typeof browser[method] === "function" ? browser[method]() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { mode, ready });
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-voice-button")).toBeVisible();
}

async function commands(page: Page) {
  return page.evaluate(() => window.assistantVoiceTest.calls.map((call) => call.command));
}

test("voice entry sits in the create card, works without a shortcut and exposes setup and stopped guidance", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 900 });
  await openVoice(page);
  const button = page.getByTestId("assistant-voice-button");
  await expect(button).toBeEnabled();
  await expect(page.getByTestId("assistant-voice-hotkey")).toHaveText("设置快捷键");
  await page.screenshot({ path: testInfo.outputPath("assistant-voice-entry.png"), animations: "disabled" });
  await page.getByTestId("assistant-voice-hotkey").click();
  await expect(page.getByTestId("assistant-nav-settings")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-interaction").getByRole("button")).toHaveCount(3);
  await expect(page.getByTestId("assistant-mode-solo")).toHaveCount(0);
  await page.getByTestId("assistant-nav-tasks").click();
  await page.evaluate(() => window.assistantVoiceTest.runtime("stopped"));
  await expect(button).toBeDisabled();
  await expect(page.getByTestId("assistant-voice-input")).toContainText("请先启动 Kotone");
  await page.evaluate(async () => {
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.update((settings) => settings && ({ ...settings, assistant: { ...settings.assistant, hotkey: "Ctrl+F9" } }));
  });
  await page.setViewportSize({ width: 638, height: 900 });
  await expect(button).toBeInViewport();
  const hotkey = page.getByTestId("assistant-voice-hotkey");
  await expect(hotkey).toHaveText("快捷键：Ctrl+F9");
  const hint = page.locator("#assistant-voice-hint");
  const hotkeyBox = (await hotkey.boundingBox())!;
  const hintBox = (await hint.boundingBox())!;
  expect(Math.abs(hotkeyBox.y - hintBox.y)).toBeLessThan(2);
  await page.screenshot({ path: testInfo.outputPath("assistant-voice-entry-narrow.png"), animations: "disabled" });
  await page.evaluate(async () => {
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.update((settings) => settings && ({ ...settings, assistant: { ...settings.assistant, hotkey: "Ctrl+Alt+Shift+Meta+F12" } }));
  });
  await expect(hotkey).toHaveAttribute("title", "快捷键：Ctrl+Alt+Shift+Meta+F12");
  const longHotkeyBox = (await hotkey.boundingBox())!;
  expect(Math.abs(longHotkeyBox.y - (await hint.boundingBox())!.y)).toBeLessThan(2);
  expect(longHotkeyBox.x).toBeGreaterThan((await page.locator("#assistant-input-title").boundingBox())!.x + 60);
  await expect(hint).toBeInViewport();
  expect(await commands(page)).toEqual([]);
});

test("quick push-to-talk release waits for begin and ends the exact session once", async ({ page }) => {
  await openVoice(page);
  await page.evaluate(() => { window.assistantVoiceTest.holdBegin = true; });
  const button = page.getByTestId("assistant-voice-button");
  await button.hover();
  await page.mouse.down();
  await page.mouse.up();
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin"]);
  await page.evaluate(() => window.assistantVoiceTest.resolveBegin());
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_end"]);
  expect(await page.evaluate(() => window.assistantVoiceTest.calls[1].sessionId)).toBe("voice-1");
  await expect(button).toHaveAccessibleName("按住说话");
});

test("pointer capture releases outside the button and keyboard hold ignores repeats", async ({ page }) => {
  await openVoice(page);
  const button = page.getByTestId("assistant-voice-button");
  await button.hover();
  await page.mouse.down();
  await expect(button).toHaveAccessibleName("松开提交");
  await page.mouse.move(20, 20);
  await page.mouse.up();
  await expect(button).toHaveAccessibleName("按住说话");
  await button.focus();
  await page.keyboard.down("Space");
  await page.keyboard.down("Space");
  await expect(button).toHaveAccessibleName("松开提交");
  await page.keyboard.up("Space");
  await expect(button).toHaveAccessibleName("按住说话");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect.poll(() => commands(page)).toEqual(Array(3).fill(["assistant_voice_begin", "assistant_voice_end"]).flat());
});

test("focus loss during pending begin cancels that token and never submits the released recording", async ({ page }) => {
  await openVoice(page);
  await page.evaluate(() => { window.assistantVoiceTest.holdBegin = true; });
  await page.getByTestId("assistant-voice-button").hover();
  await page.mouse.down();
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await page.mouse.up();
  await page.evaluate(() => window.assistantVoiceTest.resolveBegin());
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_cancel"]);
  expect(await page.evaluate(() => window.assistantVoiceTest.calls[1].sessionId)).toBe("voice-1");
  await expect(page.getByTestId("assistant-voice-button")).toHaveAccessibleName("按住说话");
});

test("dictation previews recognized text and confirms only once, with a cancel that bypasses pending AI", async ({ page }) => {
  await openVoice(page, "dictation");
  const button = page.getByTestId("assistant-voice-button");
  await button.click();
  await expect(button).toHaveAccessibleName("结束录音");
  await button.click();
  await expect(button).toHaveAccessibleName("确认创建");
  await expect(page.getByTestId("assistant-voice-transcript")).toHaveText("5 分钟后提醒我打野有闪");
  await page.evaluate(() => { window.assistantVoiceTest.holdEnd = true; });
  await button.click();
  await expect(button).toBeDisabled();
  await page.getByRole("button", { name: "取消语音输入", exact: true }).click();
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_end", "assistant_voice_confirm", "assistant_voice_cancel"]);
  await expect(button).toHaveAccessibleName("语音输入");
  await page.evaluate(() => window.assistantVoiceTest.resolveEnd());
  await expect(button).toHaveAccessibleName("语音输入");
});

test("one-shot supports manual finish and relinquishes ownership after automatic completion", async ({ page }) => {
  await openVoice(page, "one-shot");
  const button = page.getByTestId("assistant-voice-button");
  await button.press("Enter");
  await expect(button).toHaveAccessibleName("结束录音");
  await button.click();
  await expect(button).toHaveAccessibleName("语音输入");
  await button.click();
  await page.evaluate(async () => { await window.assistantVoiceTest.state("transcribing"); await window.assistantVoiceTest.state("processing"); });
  await expect(button).toBeDisabled();
  await page.evaluate(() => window.assistantVoiceTest.state("success"));
  await expect(button).toBeEnabled();
  expect(await commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_end", "assistant_voice_begin"]);
});

test("switching sections and unmounting cancel owned sessions even while an end request is waiting", async ({ page }) => {
  await openVoice(page, "dictation");
  await page.getByTestId("assistant-request").fill("保留文字草稿");
  await page.getByTestId("assistant-voice-button").click();
  await expect(page.getByTestId("assistant-request")).toBeDisabled();
  await page.getByTestId("assistant-nav-settings").click();
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_cancel"]);
  await page.getByTestId("assistant-nav-tasks").click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("保留文字草稿");
  await page.evaluate(async () => { await window.assistantVoiceTest.mode("one-shot"); window.assistantVoiceTest.holdEnd = true; });
  await page.getByTestId("assistant-voice-button").click();
  await expect(page.getByTestId("assistant-voice-button")).toHaveAccessibleName("结束录音");
  await page.getByTestId("assistant-voice-button").click();
  await expect(page.getByTestId("assistant-voice-button")).toBeDisabled();
  await page.getByTestId("settings-nav-general").click();
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin", "assistant_voice_cancel", "assistant_voice_begin", "assistant_voice_end", "assistant_voice_cancel"]);
  await page.evaluate(() => window.assistantVoiceTest.resolveEnd());
});

test("existing regular and hotkey sessions cannot be taken over", async ({ page }) => {
  await openVoice(page);
  const button = page.getByTestId("assistant-voice-button");
  await page.evaluate(() => window.assistantVoiceTest.state("listening", "dictation"));
  await expect(button).toBeDisabled();
  await expect(page.getByTestId("assistant-voice-input")).toContainText("请先结束当前输入");
  await page.evaluate(() => window.assistantVoiceTest.state("listening", "assistant"));
  await expect(button).toBeDisabled();
  await expect(page.getByTestId("assistant-voice-input")).toContainText("助手语音尚未结束");
  await expect(page.getByRole("button", { name: "取消语音输入", exact: true })).toHaveCount(0);
  expect(await commands(page)).toEqual([]);
});

test("missing AI disables voice", async ({ page }) => {
  await openVoice(page, "push-to-talk", false);
  await expect(page.getByTestId("assistant-voice-button")).toBeDisabled();
  await expect(page.getByTestId("assistant-voice-input")).toContainText("请先选择 AI 连接");
});

test("Space and Enter bound as global hotkeys never also activate the voice button", async ({ page }) => {
  await openVoice(page, "dictation");
  await page.evaluate(async () => {
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.update((settings) => settings && ({ ...settings, hotkey: { ...settings.hotkey, key: "Space" }, assistant: { ...settings.assistant, hotkey: "Ctrl+Enter" } }));
  });
  const button = page.getByTestId("assistant-voice-button");
  await button.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Control+Enter");
  expect(await commands(page)).toEqual([]);
  await expect(page.getByTestId("assistant-voice-hotkey")).toHaveText("快捷键：Ctrl+Enter");
  await page.keyboard.press("Enter");
  await expect.poll(() => commands(page)).toEqual(["assistant_voice_begin"]);
});

for (const style of ["capsule", "card"] as const) {
  test(`button-owned ${style} overlay uses cross-window source and sends preview confirmation back to the assistant page`, async ({ page }) => {
    await openVoice(page);
    await page.evaluate(async (style) => {
      const browser = await import("/src/lib/ipc/browserMock.ts");
      await browser.updateSettings({ overlay: { style, clickThrough: false } });
      window.location.hash = "#/overlay";
    }, style);
    await page.evaluate(() => window.assistantVoiceTest.state("listening"));
    await expect(page.getByText("助手收音中 · 松开按钮提交", { exact: true })).toBeVisible();
    await page.evaluate(() => window.assistantVoiceTest.state("preview"));
    await expect(page.getByText("请回到提醒助手确认 · Esc 取消", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: /提交助手/ })).toHaveCount(0);
    await expect(page.locator("body")).not.toContainText("助手热键 提交助手");
  });
}
