import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = (relative: string) => fileURLToPath(new URL(relative, import.meta.url));

// ADR 028「游戏陪玩」的交互流程：打开就是聊天页、一个 DeepSeek Key 打开全部功能、语音输入是子板块、
// 「启动」只属于本地语音识别。浏览器预览（dev:web）的 mock 后端，不接任何 AI。

const settingsOf = (page: Page) =>
  page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());

/** 页面上任何看得见的“启动”按钮（在线识别时一个都不应该有）。 */
const startButtons = (page: Page) => page.getByRole("button", { name: /启动/ }).filter({ visible: true });

test("1 · new user: one DeepSeek key → five things on DeepSeek → call key → chat with “你能做什么？”, never a start button", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=always");
  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding.getByTestId("onboarding-welcome")).toBeVisible();
  await expect(onboarding).toContainText("陪你打游戏的 AI 陪玩");
  await page.screenshot({ path: testInfo.outputPath("w1-welcome.png") });
  await page.getByTestId("onboarding-add-key").click();

  // 本地格式检查（照 DeepSeek Harness）：带引号 / NAME=value 当场拦下
  const input = page.getByTestId("onboarding-key-input");
  await input.fill("DEEPSEEK_API_KEY=sk-test");
  await page.getByTestId("onboarding-key-submit").click();
  await expect(page.getByTestId("onboarding-key-error")).toContainText("不要带引号、空格或 DEEPSEEK_API_KEY=");
  await input.fill("sk-test-123");
  await page.screenshot({ path: testInfo.outputPath("w2-key.png") });
  await page.getByTestId("onboarding-key-submit").click();

  // 已连接：五项都换成 DeepSeek；新用户的呼叫键默认 Ctrl + ~
  const switched = page.getByTestId("onboarding-switched");
  await expect(switched).toContainText("对话");
  for (const item of ["语音识别", "文字识别", "语音合成", "联网搜索"]) await expect(switched).toContainText(item);
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("Ctrl + ~");
  await page.screenshot({ path: testInfo.outputPath("w3-connected.png") });
  const saved = await settingsOf(page);
  expect(saved.activeModelId).toBe("deepseek-asr");
  expect(saved.ocr.activeModelId).toBe("deepseek-ocr");
  expect(saved.assistant.ttsModelId).toBe("deepseek-tts");
  expect(saved.search.activeModelId).toBe("deepseek-search");
  expect(saved.ui.firstRunCompleted).toBe(true);
  expect(saved.connections.find((c) => c.id === saved.assistant.connectionId)?.provider).toBe("deepseek");

  await page.getByTestId("onboarding-enter-chat").click();
  await expect(onboarding).toHaveCount(0);
  // 聊天页：输入框里放好“你能做什么？”，玩家自己点发送（D2）；它也是第一个起手提示
  await expect(page.getByTestId("assistant-request")).toHaveValue("你能做什么？");
  // 空白对话（浏览器预览里先点“新对话”，预览自带示例对话）：呼叫按钮 + 起手提示，没有本地识别的状态行
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("agent-starter").first()).toHaveText("你能做什么？");
  await expect(page.getByTestId("chat-call-block")).toBeVisible();
  await expect(page.getByTestId("chat-local-asr")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("w6-chat.png") });

  // 在线识别：标题栏、聊天页、语音输入、设置里都没有启动按钮
  await expect(page.getByTestId("titlebar-asr")).toHaveCount(0);
  await expect(startButtons(page)).toHaveCount(0);
  await page.getByTestId("settings-nav-general").click();
  await expect(page.getByTestId("asr-card")).toHaveAttribute("data-asr-state", "online-ready");
  await expect(startButtons(page)).toHaveCount(0);
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-system").click();
  await expect(startButtons(page)).toHaveCount(0);
});

test("2 · “只想用语音输入？” → the four-step guide in the voice input board, first dictation without a key", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=always");
  await page.getByTestId("onboarding-voice-only").click();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await expect(page.getByTestId("voice-input")).toBeVisible();
  const guide = page.getByTestId("voice-guide");
  await expect(guide).toBeVisible();
  await page.getByTestId("profile-lol").click();
  await page.getByTestId("voice-guide-next").click();
  // 识别：不填 Key 只有本地识别；在线识别那一项写“需要 DeepSeek Key → 让 Kotone 帮我配置”
  await expect(page.getByTestId("voice-guide-online-option")).toContainText("需要 DeepSeek Key");
  await expect(page.getByTestId("voice-guide-next")).toBeDisabled();
  await page.getByTestId("voice-guide-download").click();
  await expect(page.getByTestId("voice-guide-next")).toBeEnabled();
  await page.screenshot({ path: testInfo.outputPath("w10-guide-model.png") });
  await page.getByTestId("voice-guide-next").click();
  await expect(page.getByTestId("input-environment-ready")).toBeVisible();
  await page.getByTestId("mode-push-to-talk").click();
  await page.getByTestId("voice-guide-next").click();
  // 说一句试试：本地识别先启动，然后在框里说出第一句
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-running");
  await page.getByTestId("onboarding-try-box").fill("打野来下路");
  await expect(page.getByTestId("onboarding-try-status")).toContainText("成了");
  await page.getByTestId("voice-guide-done").click();
  await expect(guide).toHaveCount(0);
  const saved = await settingsOf(page);
  expect(saved.ui.startPage).toBe("voiceInput");
  expect(saved.ui.firstRunCompleted).toBe(true);
  expect(saved.ui.autoStart).toBe(true);
  expect(saved.connections).toHaveLength(0);
  // 本地识别：启动 / 停止、启动快捷键、打开时自动启动都在识别卡片里
  await expect(page.getByTestId("asr-stop")).toBeVisible();
  await expect(page.getByTestId("asr-toggle-hotkey")).toBeVisible();
});

test("3 · 0.1.x config → upgrade welcome → 继续使用语音输入法 changes nothing and keeps landing on voice input", async ({ page }, testInfo) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured");
  const before = await settingsOf(page);
  const upgrade = page.getByTestId("onboarding-upgrade");
  await expect(upgrade).toBeVisible();
  await expect(upgrade).toContainText("Kotone 0.2：多了一位游戏陪玩");
  await expect(page.getByTestId("onboarding-upgrade-summary")).toContainText("CapsLock");
  await expect(page.getByTestId("onboarding-upgrade-summary")).toContainText("提醒助手照旧");
  // “继续使用语音输入法”是第一个、最醒目的按钮
  await expect(upgrade.getByRole("button").first()).toHaveText("继续使用语音输入法");
  await page.screenshot({ path: testInfo.outputPath("w4-upgrade.png") });
  await page.getByTestId("onboarding-keep-voice").click();

  await expect(page.getByTestId("voice-input")).toBeVisible();
  const after = await settingsOf(page);
  // 只多记了“看过、选了继续”和落地页，其余一字不差
  expect({ ...after, ui: { ...after.ui, legacyUpgrade: before.ui.legacyUpgrade, startPage: before.ui.startPage } }).toEqual(before);
  expect(after.ui.legacyUpgrade).toBe("dismissed");
  expect(after.ui.startPage).toBe("voiceInput");
  // 本地识别和启动按钮照旧；没有呼叫键、没有唤醒词
  await expect(page.getByTestId("asr-card")).toHaveAttribute("data-asr-state", /local-/);
  await expect(page.getByTestId("titlebar-asr")).toBeVisible();
  expect(after.assistant.companionBindings).toBeUndefined();
  expect(after.assistant.wake.enabled).toBe(false);
  // 完整侧栏（D4）：琴音、游戏适配、提醒助手都在；点聊天看到的是 Key 卡片和“回到语音输入”
  await expect(page.getByTestId("settings-nav-game")).toHaveText("游戏适配");
  await expect(page.getByTestId("settings-nav-legacy-reminders")).toBeVisible();
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("chat-unconfigured")).toBeVisible();
  await page.getByTestId("chat-back-to-voice").click();
  await expect(page.getByTestId("voice-input")).toBeVisible();

  // 下次打开：仍落在语音输入，不再弹引导
  await page.goto("/?mockLegacyUpgrade=dismissed#/settings");
  await expect(page.getByTestId("voice-input")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
});

test("4 · 0.1.x config → add a key → chat; voice input stays on local recognition with its start button", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured");
  const before = await settingsOf(page);
  await page.getByTestId("onboarding-meet-companion").click();
  await page.getByTestId("onboarding-key-input").fill("sk-legacy-1");
  await page.getByTestId("onboarding-key-submit").click();
  const switched = page.getByTestId("onboarding-switched");
  await expect(switched).toContainText("保持原来的设置");
  // 升级用户的呼叫键默认空，推荐 Ctrl + ~
  await expect(page.getByTestId("onboarding-call-key-value")).toHaveText("未设置");
  await page.getByTestId("onboarding-call-key-recommended").click();
  await page.getByTestId("onboarding-enter-chat").click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("你能做什么？");

  const after = await settingsOf(page);
  expect(after.ui.legacyUpgrade).toBe("optedIn");
  expect(after.ui.startPage).toBe("chat");
  expect(after.activeModelId).toBe(before.activeModelId);
  expect(after.sttEngine).toBe(before.sttEngine);
  expect(after.hotkey).toEqual(before.hotkey);
  // 0.1.x 的提醒键和连接原样保留；呼叫键和新连接写进 companionBindings
  expect(after.assistant.hotkey).toBe(before.assistant.hotkey);
  expect(after.assistant.connectionId).toBe(before.assistant.connectionId);
  expect(after.assistant.companionBindings?.hotkey).toBe("Ctrl+Backquote");
  expect(after.ocr.activeModelId).toBe("deepseek-ocr");
  // 本地识别的状态和启动按钮还在
  await expect(page.getByTestId("titlebar-asr")).toBeVisible();
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("chat-local-asr")).toBeVisible();
  await page.getByTestId("settings-nav-general").click();
  await expect(page.getByTestId("asr-start")).toBeVisible();
});

test("5 · online recognition is ready on launch: no start button on any page", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never&mockAsr=online");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  await expect(page.getByTestId("titlebar-asr")).toHaveCount(0);
  for (const nav of ["assistant", "game", "automation", "general", "advanced", "about"]) {
    await page.getByTestId(`settings-nav-${nav}`).click();
    await expect(startButtons(page), nav).toHaveCount(0);
  }
  for (const tab of ["processing", "history"]) {
    await page.getByTestId("settings-nav-general").click();
    await page.getByTestId(`voice-tab-${tab}`).click();
    await expect(startButtons(page), tab).toHaveCount(0);
  }
  for (const tab of ["call", "reply", "permissions", "usage", "models", "connections", "sound", "tuning", "appearance", "system"]) {
    await page.getByTestId("settings-nav-advanced").click();
    await page.getByTestId(`advanced-nav-${tab}`).click();
    await expect(startButtons(page), tab).toHaveCount(0);
  }
});

test("6 · local recognition not started: chat shows the state and starts it from the page", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never&mockAsr=local-ready&mockCompanion=ready");
  await page.getByTestId("sidebar-new-chat").click();
  const state = page.getByTestId("chat-local-asr");
  await expect(state).toContainText("本地语音识别 · 未启动");
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-stopped");
  // “点一下呼叫按钮会先启动本地识别再听”需要桌面版：见 agent-chat.spec.ts 里的同名场景。
  await page.screenshot({ path: testInfo.outputPath("w7-local-stopped.png") });
  await page.getByTestId("chat-local-asr-start").click();
  await expect(state).toHaveCount(0);
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-running");
  // 标题栏菜单：停止 / 去语音输入设置
  await page.getByTestId("titlebar-asr").click();
  await page.getByTestId("titlebar-asr-stop").click();
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-stopped");
  await page.getByTestId("titlebar-asr").click();
  await page.getByTestId("titlebar-asr-settings").click();
  await expect(page.getByTestId("asr-card")).toBeVisible();
});

test("7 · online ↔ local: the start UI appears and disappears, the start shortcut is kept", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never&mockAsr=online");
  await page.getByTestId("settings-nav-general").click();
  const card = page.getByTestId("asr-card");
  await expect(card).toHaveAttribute("data-asr-state", "online-ready");
  await expect(page.getByTestId("asr-switch-local")).toContainText("换成本地识别：免费、不联网、边说边出字");
  await page.screenshot({ path: testInfo.outputPath("w8-online.png") });
  // 下载并切换：切到本地后立即启动一次，“打开时自动启动”默认勾上（D9）
  await page.getByTestId("asr-switch-local-button").click();
  await expect(card).toHaveAttribute("data-asr-state", "local-running");
  await expect(page.getByTestId("titlebar-asr")).toHaveAttribute("data-asr", "local-running");
  expect((await settingsOf(page)).ui.autoStart).toBe(true);
  await page.getByTestId("hotkey-input-runtime-toggle").fill("Ctrl+Alt+K");
  await page.getByTestId("hotkey-input-runtime-toggle").press("Enter");
  await expect.poll(async () => (await settingsOf(page)).runtimeToggleHotkey).toBe("Ctrl+Alt+K");
  await page.screenshot({ path: testInfo.outputPath("w9-local.png") });

  for (let round = 0; round < 2; round++) {
    await page.getByTestId("asr-switch-online").click();
    await expect(card).toHaveAttribute("data-asr-state", "online-ready");
    await expect(page.getByTestId("titlebar-asr")).toHaveCount(0);
    await expect(page.getByTestId("asr-toggle-hotkey")).toHaveCount(0);
    expect((await settingsOf(page)).runtimeToggleHotkey).toBe("Ctrl+Alt+K");
    await page.getByTestId("asr-switch-local-button").click();
    await expect(card).toHaveAttribute("data-asr-state", "local-running");
    await expect(page.getByTestId("hotkey-input-runtime-toggle")).toHaveValue("Ctrl+Alt+K");
  }
});

// ---------- 8 · 设置说明里的每个位置都能跳过去（含旧写法） ----------

const guideSource = readFileSync(here("../../../crates/kotone-core/src/settings_guide.rs"), "utf8");
/** settings_guide.rs 里 `f(path, label, page, …)` 的第三个参数。 */
const guidePages = [...new Set([...guideSource.matchAll(/\bf\(\s*"[^"]*",\s*"[^"]*",\s*"([^"]*)"/g)].map((m) => m[1]))];

test("8 · every location in settings_guide (and the old wording) opens the right place", async ({ page }) => {
  expect(guidePages.length).toBeGreaterThan(20);
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const locations = [...new Set(guidePages.flatMap((where) => where.split(/，或/).map((part) => part.trim())))];
  const old = ["琴音 → 设置 → 呼叫", "琴音 → 设置 → AI 连接", "琴音 → 设置 → 游戏语音", "琴音 → 设置 → 权限", "琴音 → 交流",
    "设置 → 其他热键", "设置 → 音效", "文字处理", "历史记录", "琴音 → 设置",
    // 术语表 v2 改名的小节：新名字和旧名字（以前的对话里引用着）都能跳过去
    "设置 → 她怎么回答 → 她用的模型", "设置 → 她怎么回答 → 气泡", "设置 → 她怎么回答 → 回答方式",
    "设置 → 她怎么回答 → AI 连接", "设置 → 她怎么回答 → 对话气泡"];
  for (const where of [...locations, ...old]) {
    const target = await page.evaluate(async (where) => {
      const nav = await import("/src/lib/stores/navigation.ts");
      const parsed = nav.parseLocation(where);
      nav.openLocation(where);
      return parsed;
    }, where);
    expect(target, where).not.toBeNull();
    if (!target) continue;
    if (target.page === "advanced") {
      await expect(page.getByTestId(`advanced-nav-${target.tab}`), where).toHaveAttribute("aria-current", "page");
    } else if (target.page === "assistant") {
      await expect(page.getByTestId(`assistant-nav-${target.section}`), where).toHaveAttribute("aria-current", "page");
    } else if (target.page === "general" || target.page === "processing" || target.page === "history") {
      await expect(page.getByTestId(`voice-tab-${target.page}`), where).toHaveAttribute("aria-current", "page");
    } else if (target.page === "sidebar-pages") {
      // 「左侧栏 → ⋯」= 全部页面（ADR 034）：打开对话框，看完关掉
      await expect(page.getByTestId("sidebar-pages-dialog"), where).toBeVisible();
      await page.getByTestId("sidebar-pages-done").click();
      await expect(page.getByTestId("sidebar-pages-dialog"), where).toHaveCount(0);
    } else if (target.page === "plugin") {
      // 插件的页面（ADR 038）：左侧栏「插件」组里那一项
      await expect(page.getByTestId(`settings-nav-plugin:${target.id}`), where).toHaveAttribute("aria-current", "page");
    } else if (target.page !== "legacy-reminders") {
      // 提醒助手只有 0.1.x 升级用户才有，这里只核对能解析出来
      await expect(page.getByTestId(`settings-nav-${target.page}`), where).toHaveAttribute("aria-current", "page");
    }
    if ("anchor" in target && target.anchor) {
      const id = target.anchor.startsWith("models-") ? "models-category" : target.anchor;
      await expect(page.locator(`[id="${id}"]`), where).toHaveCount(1);
    }
  }
});

// ---------- 9 · 界面上不再出现的说法 ----------

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(svelte|ts|rs|json)$/.test(name) ? [path] : [];
  });
}

test("9 · banned wording never appears in the UI sources", () => {
  const banned = [/右上角「启动」/, /陪玩热键/, /启动 Kotone 后生效/, /启动引擎|停止引擎/, /停止所有自动化/, /点「启动」开始说话/, /需手动点「启动」/, /标题栏「重启生效」/];
  // 界面文字搬进了 messages/zh-CN 的目录（Paraglide）：一条消息一行，按行查就是查每条的值
  const roots = [here("../src"), here("../src-tauri/src"), here("../messages/zh-CN")];
  const hits: string[] = [];
  for (const file of roots.flatMap(sourceFiles)) {
    const lines = readFileSync(file, "utf8").split(/\r?\n/);
    lines.forEach((line, index) => {
      if (/^\s*(\/\/|\*|\/\*)/.test(line)) return; // 注释里可以讲历史
      for (const pattern of banned) if (pattern.test(line)) hits.push(`${file}:${index + 1}: ${line.trim()}`);
    });
  }
  expect(hits).toEqual([]);
});
