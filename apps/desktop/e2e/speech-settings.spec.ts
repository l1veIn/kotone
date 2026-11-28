import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";
import type { SpeechModelStatus } from "../src/lib/assistant";
import { screenshot } from "./screenshot.cjs";

declare global {
  interface Window {
    speechTest: {
      commands: string[];
      holdSnapshots: boolean;
      holdVoices: boolean;
      push(patch: Partial<SpeechModelStatus>): Promise<void>;
      fail(): Promise<void>;
      resolveSnapshots(): void;
      resolveVoices(): void;
      pendingSnapshots(): number;
      pendingVoices(): number;
    };
  }
}

async function openNativeSettings(page: Page) {
  // Real Svelte page and Tauri APIs; transport/model work stays inside this test fixture.
  await page.route("**/__speech-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__speech-test#/settings?onboarding=never");
  await page.evaluate(async () => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    type Status = { modelId: string; installed: boolean; downloading: boolean; downloadedBytes: number; totalBytes: number; phase: string; error: string | null; modelDir: string };
    let status: Status = { modelId: "kokoro-int8-multi-lang-v1_1", installed: false, downloading: false, downloadedBytes: 0, totalBytes: 100, phase: "idle", error: null, modelDir: "test-fixture" };
    let rejectDownload: ((error: string) => void) | undefined;
    const snapshots: ((value: Status) => void)[] = [];
    const voices: ((value: unknown) => void)[] = [];
    const bridge = {
      commands: [] as string[], holdSnapshots: false, holdVoices: false,
      async push(patch: Partial<Status>) {
        status = { ...status, ...patch };
        await emit("kotone://speech-model", status);
      },
      async fail() {
        await this.push({ phase: "error", downloading: false, error: "测试下载连接失败" });
        rejectDownload?.("测试下载连接失败");
      },
      resolveSnapshots() {
        for (const resolve of snapshots.splice(0)) resolve({ ...status, installed: true, downloading: false, phase: "installed", error: null });
      },
      resolveVoices() {
        for (const resolve of voices.splice(0)) resolve([{ id: "stale-voice", name: "过期音色" }]);
      },
      pendingSnapshots: () => snapshots.length,
      pendingVoices: () => voices.length,
    };
    Object.assign(window, { speechTest: bridge });
    await browser.updateSettings({ assistant: { speechEngine: "kokoro", voiceId: "sapi:test", kokoroVoiceId: "kokoro:zf_001" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      bridge.commands.push(command);
      if (command === "get_speech_model_status") {
        return bridge.holdSnapshots ? new Promise((resolve) => snapshots.push(resolve)) : structuredClone(status);
      }
      if (command === "download_speech_model") {
        await bridge.push({ phase: "downloading", downloading: true, downloadedBytes: 0, error: null });
        return new Promise((_resolve, reject) => { rejectDownload = reject; });
      }
      if (command === "cancel_speech_model_download") {
        await bridge.push({ phase: "cancelled", downloading: false, error: null });
        rejectDownload?.("下载已取消");
        return;
      }
      if (command === "list_speech_voices") {
        if (bridge.holdVoices) return new Promise((resolve) => voices.push(resolve));
        return (await browser.getSettings()).assistant.speechEngine === "system"
          ? [{ id: "system/sapi:test", name: "系统测试声音" }]
          : ["001", "003", "017"].map((id) => ({ id: `kokoro/zf_${id}`, name: `中文女声 ${id}` }));
      }
      if (command === "get_startup_options") return { onboarding: "never" };
      if (command === "update_settings") return browser.updateSettings(args.patch);
      // 她的声音（ADR 048）：服务商状态、Key 都走浏览器预览的那份（不联网）
      if (command === "speech_key_save") return browser.speechKeySave(args.modelId, args.key);
      if (command === "speech_key_clear") return browser.speechKeyClear(args.modelId);
      if (command === "speech_try_voice") return null;
      // 插件清单照预览的那份（语音输入是插件的 App，ADR 038：从「插件 → 语音输入」进）
      if (command === "plugins_list") return (await import("/src/lib/ipc/pluginsMock.ts")).listPlugins();
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      return typeof browser[method] === "function" ? browser[method]() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  });
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("speech-settings")).toBeVisible();
}

test("Kokoro offers three Chinese voices and switching engines preserves each voice selection", async ({ page }) => {
  await openNativeSettings(page);
  const speech = page.getByTestId("speech-settings");
  await speech.getByLabel("播报声音", { exact: true }).click();
  await expect(page.getByRole("option")).toHaveText(["中文女声 001", "中文女声 003", "中文女声 017"]);
  await page.getByRole("option", { name: "中文女声 017", exact: true }).click();
  await expect(speech.getByLabel("播报声音", { exact: true })).toHaveAttribute("data-value", "kokoro/zf_017");
  await speech.getByLabel("服务商", { exact: true }).click();
  await page.getByRole("option", { name: "Windows · 系统语音", exact: true }).click();
  await expect(speech.getByLabel("播报声音", { exact: true })).toHaveText("系统测试声音");
  await speech.getByLabel("播报声音", { exact: true }).click();
  await page.getByRole("option", { name: "系统默认声音", exact: true }).click();
  await speech.getByLabel("服务商", { exact: true }).click();
  await page.getByRole("option", { name: "Kokoro · 本地中文语音", exact: true }).click();
  await expect(speech.getByLabel("播报声音", { exact: true })).toHaveText("中文女声 017");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  // 落盘用旧写法，旧版本仍能使用这个声音
  expect(saved.assistant).toMatchObject({ speechEngine: "kokoro", kokoroVoiceId: "kokoro:zf_017", voiceId: "" });
});

test("browser preview clearly shows an uninstalled model, disables download and makes no external request", async ({ page, baseURL }, testInfo) => {
  const origin = new URL(baseURL!).origin;
  const external: string[] = [];
  page.on("request", (request) => {
    // 本机回环上的（插件 MCP App 的沙盒代理页，预览里在 127.0.0.1 的另一个端口上）不算往外发
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin && !/^(127\.0\.0\.1|localhost)$/.test(new URL(request.url()).hostname)) external.push(request.url());
  });
  await page.setViewportSize({ width: 1040, height: 1150 });
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("auto-speech-info")).toBeVisible();
  await page.getByLabel("服务商", { exact: true }).click();
  await page.getByRole("option", { name: "Kokoro · 本地中文语音", exact: true }).click();
  await expect(page.getByTestId("speech-model")).toContainText("尚未下载");
  await expect(page.getByRole("button", { name: "下载语音模型", exact: true })).toBeDisabled();
  const result = await page.evaluate(async () => {
    const speech = await import("/src/lib/assistant.ts");
    try { await speech.downloadSpeechModel(); } catch (error) { return String(error); }
  });
  expect(result).toContain("请在桌面应用中下载语音模型");
  expect(external).toEqual([]);
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await screenshot(page, { path: testInfo.outputPath("assistant-settings-top.png"), fullPage: true, animations: "disabled" });
  await page.getByTestId("danmaku-settings").scrollIntoViewIfNeeded();
  await screenshot(page, { path: testInfo.outputPath("assistant-settings-bottom.png"), fullPage: true, animations: "disabled" });
});

test("native download progress, failure, retry and cancellation remain actionable", async ({ page }) => {
  await openNativeSettings(page);
  const model = page.getByTestId("speech-model");
  await model.getByRole("button", { name: "下载语音模型", exact: true }).click();
  await page.evaluate(async () => window.speechTest.push({ downloadedBytes: 37 }));
  await expect(model.getByRole("progressbar")).toHaveAttribute("value", "37");
  await page.evaluate(async () => window.speechTest.push({ phase: "verifying", downloadedBytes: 100 }));
  await expect(model).toContainText("正在校验 · 100%");
  await page.evaluate(() => { window.speechTest.holdSnapshots = true; });
  await page.evaluate(async () => window.speechTest.fail());
  await expect(model).toContainText("安装失败");
  await expect(model.getByRole("alert")).toHaveText("测试下载连接失败");
  await page.waitForFunction(() => window.speechTest.pendingSnapshots() === 1);
  await model.getByRole("button", { name: "下载语音模型", exact: true }).click();
  await page.evaluate(() => { window.speechTest.holdSnapshots = false; window.speechTest.resolveSnapshots(); });
  await expect(model).toContainText("正在下载 · 0%");
  await model.getByRole("button", { name: "取消下载", exact: true }).click();
  await expect(model).toContainText("已取消下载");
  await expect(model.getByRole("button", { name: "下载语音模型", exact: true })).toBeEnabled();
  const commands = await page.evaluate(() => window.speechTest.commands);
  expect(commands.filter((command) => command === "download_speech_model")).toHaveLength(2);
  expect(commands.filter((command) => command === "cancel_speech_model_download")).toHaveLength(1);
});

test("leaving the page discards old model and voice responses while fresh events win over a snapshot", async ({ page }) => {
  await openNativeSettings(page);
  await openVoiceInput(page, "history");
  await page.evaluate(() => { window.speechTest.holdSnapshots = true; window.speechTest.holdVoices = true; });
  await openSettingsTab(page, "reply");
  // 「交互」在角色页（交接第十一节第 8 条）：从左侧栏进角色页先落在「角色」，那一页也读一次声音列表，所以等的声音请求可能是两个
  await page.waitForFunction(() => window.speechTest.pendingSnapshots() === 1 && window.speechTest.pendingVoices() >= 1);
  await openVoiceInput(page, "history");
  await page.evaluate(() => { window.speechTest.holdSnapshots = false; window.speechTest.holdVoices = false; });
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("speech-model")).toContainText("尚未下载");
  await page.evaluate(() => { window.speechTest.resolveSnapshots(); window.speechTest.resolveVoices(); });
  await expect(page.getByTestId("speech-model")).toContainText("尚未下载");
  await expect(page.getByLabel("播报声音", { exact: true })).toHaveText("中文女声 001");

  await openVoiceInput(page, "history");
  await page.evaluate(() => { window.speechTest.holdSnapshots = true; });
  await openSettingsTab(page, "reply");
  await page.waitForFunction(() => window.speechTest.pendingSnapshots() === 1);
  await page.evaluate(async () => {
    await window.speechTest.push({ phase: "downloading", downloading: true, downloadedBytes: 72 });
    window.speechTest.resolveSnapshots();
  });
  await expect(page.getByTestId("speech-model")).toContainText("正在下载 · 72%");
});

test("speech engine: automatic by default, DeepSeek or an explicit system choice are saved distinctly", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "reply");
  const engine = page.getByLabel("服务商", { exact: true });
  await expect(engine).toHaveAttribute("data-value", "auto");
  // “自动”写明现在用的是哪一家、怎么计费（预览里她没有连接：系统语音，不花钱）
  await expect(page.getByTestId("auto-speech-info")).toContainText("Windows 系统语音");
  await expect(page.getByTestId("speech-billing")).toContainText("不花钱");
  const saved = async () => (await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings())).assistant;

  await engine.click();
  await page.getByRole("option", { name: "在线 · DeepSeek 语音", exact: true }).click();
  await expect(page.getByTestId("online-speech-info")).toBeVisible();
  await expect(page.getByTestId("speech-billing")).toContainText("DeepSeek");
  expect((await saved()).ttsModelId).toBe("deepseek-tts");

  await engine.click();
  await page.getByRole("option", { name: "Windows · 系统语音", exact: true }).click();
  await expect(page.getByTestId("system-speech-info")).toBeVisible();
  expect(await saved()).toMatchObject({ ttsModelId: "local", speechEngine: "system" });

  await engine.click();
  await page.getByRole("option", { name: "自动（跟她用的模型走）", exact: true }).click();
  await expect(page.getByTestId("auto-speech-info")).toBeVisible();
  expect(await saved()).toMatchObject({ ttsModelId: "", speechEngine: "system" });
});

// ADR 048：选服务商 → 就地填 Key（先试一次，被拒就不存）→ 选声音；同一家 API 连接的 Key 直接用；预览里不往外发请求。
test("MiniMax: key is checked before saving, voices come from the provider and a same-provider connection key is reused", async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  const external: string[] = [];
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin && !/^(127\.0\.0\.1|localhost)$/.test(new URL(request.url()).hostname)) external.push(request.url());
  });
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "reply");
  const speech = page.getByTestId("speech-settings");
  await speech.getByLabel("服务商", { exact: true }).click();
  await page.getByRole("option", { name: "在线 · MiniMax", exact: true }).click();
  await expect(speech.getByTestId("speech-billing")).toContainText("Speech 2.8 Turbo");
  const key = speech.getByTestId("speech-key-input");
  await key.fill("bad");
  await speech.getByRole("button", { name: "保存", exact: true }).click();
  await expect(speech.getByTestId("speech-key-check")).toContainText("被拒");
  await key.fill("sk-good");
  await speech.getByRole("button", { name: "保存", exact: true }).click();
  await expect(speech.getByTestId("speech-key-check")).toContainText("可以用了 · 国内站");
  await expect(speech.getByTestId("speech-key-source")).toContainText("已保存");
  // Key 不进设置
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(JSON.stringify(settings)).not.toContain("sk-good");
  // 声音列表是这一家的（MiniMax 有 Key 时含复刻的声音），默认少女音色
  const voice = speech.getByLabel("播报声音", { exact: true });
  await expect(voice).toHaveAttribute("data-value", "minimax/female-shaonv");
  await voice.click();
  await page.getByRole("option", { name: "我的声音（复刻）· my-voice-01", exact: true }).click();
  await expect(voice).toHaveAttribute("data-value", "minimax/my-voice-01");
  expect((await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings())).modelConfigs["minimax-tts"]).toMatchObject({ voice: "my-voice-01" });
  // 清掉自己填的，换成同一家 API 连接的 Key
  await speech.getByRole("button", { name: "清掉", exact: true }).click();
  await expect(key).toBeVisible();
  await page.evaluate(async () => {
    const ipc = await import("/src/lib/ipc.ts");
    await ipc.upsertConnection({ id: "mm-cn", displayName: "MiniMax（国内）", kind: "remote", provider: "minimax-cn", baseUrl: "https://api.minimaxi.com/v1", model: "MiniMax-M3" }, "sk-connection");
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.set(await ipc.getSettings());
  });
  await expect(speech.getByTestId("speech-key-source")).toContainText("用的是 API 连接「MiniMax（国内）」的 Key");
  expect(external).toEqual([]);
});
