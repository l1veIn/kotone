import { join } from "node:path";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 聊天页右侧栏（ADR 031，照 DeepSeek Harness 桌面版与 Codex 内置浏览器）：标签“卡片”（网页卡片 + 后台在做的事，
// 按宽度 1–3 列）/“网页”（标签页、地址栏、网页）；左边缘拖动调宽、双击恢复；网页可全屏；网页的位置报给后端
// （网页由后端摆在窗口里）；要玩家登录时切到网页并出横幅；窄窗口默认收起、盖在聊天上。
// 用 @tauri-apps/api/mocks 模拟 browser_* / agent_* 命令与 kotone://browser、kotone://agent-event，不接任何 AI。

interface Call { command: string; args?: Record<string, unknown> }
interface Bounds { x: number; y: number; width: number; height: number; visible: boolean }
interface Info { name: string; label: string; conversationId: string; status: string; summary?: string; startedAtMs: number; updatedAtMs: number; steps: number; question?: string }
declare global {
  interface Window {
    sidebarTest: {
      calls: Call[];
      browser(state: Record<string, unknown>): Promise<void>;
      subagent(info: Partial<Info> & { name: string }): Promise<void>;
      /** agent_subagent_messages 返回的过程。 */
      process: unknown[];
    };
  }
}

const T = 1_800_000_000_000;
const SHOTS = process.env.KOTONE_SIDEBAR_SHOTS;
/** 插件重构第 7 步的截图（实现记录用）：仓库的 target/plugin-shots/step7/；testDir 是 apps/desktop/e2e。 */
const step7 = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "..", "..", "target", "plugin-shots", "step7", `${name}.png`);
const MUSIC_TAB = {
  id: "t-music", url: "https://music.163.com/#/song?id=186016", title: "晴天 - 周杰伦 - 单曲 - 网易云音乐", site: "music.163.com",
  loading: false, canGoBack: true, canGoForward: false, audible: true, agent: true, fullscreenElement: false,
};
const GUIDE_TAB = {
  id: "t-guide", url: "https://www.bilibili.com/video/BV1xx411c7mD", title: "海克斯大乱斗剑姬出装 - 哔哩哔哩", site: "www.bilibili.com",
  loading: false, canGoBack: false, canGoForward: false, audible: false, agent: false, fullscreenElement: false,
};
const PLAYING = {
  open: true, url: MUSIC_TAB.url, title: MUSIC_TAB.title, site: MUSIC_TAB.site,
  loading: false, handover: null, busy: false, media: { playing: true, title: "晴天", artist: "周杰伦" },
  tabs: [MUSIC_TAB], activeTab: MUSIC_TAB.id, notice: null,
};

async function openChat(page: Page, options: { empty?: boolean } = {}) {
  await page.route("**/__sidebar-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__sidebar-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, empty }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const main = {
      version: 1, id: "c-main", title: "陪我打一局", createdAtMs: T - 60_000, updatedAtMs: T - 50_000,
      messages: [
        { role: "user", text: "陪我打一局", source: "voice", atMs: T - 60_000 },
        { role: "assistant", text: "好呀，开始吧。", atMs: T - 59_000 },
      ] as unknown[],
      subagents: [] as Info[],
    };
    const state = {
      current: empty ? null : "c-main", running: null as string | null, notReadyReason: null,
      conversations: empty ? [] : [{ id: "c-main", title: main.title, createdAtMs: main.createdAtMs, updatedAtMs: main.updatedAtMs, messageCount: 2 }],
    };
    let browserState: Record<string, unknown> = { open: false, url: null, title: null, site: null, loading: false, handover: null, media: null, busy: false, tabs: [], activeTab: null };
    const extensions: { id: string; name: string; enabled: boolean; path: string }[] = [];
    const runtime = { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null, asrLocal: false, error: null };
    const bridge = {
      calls: [] as Call[],
      process: [] as unknown[],
      async browser(next: Record<string, unknown>) {
        browserState = { ...browserState, ...next };
        await emit("kotone://browser", browserState);
      },
      async subagent(partial: Partial<Info> & { name: string }) {
        const old = main.subagents.find((s) => s.name === partial.name);
        const info = { label: "", conversationId: "c-task", status: "running", startedAtMs: T, steps: 0, ...old, ...partial, updatedAtMs: (old?.updatedAtMs ?? T) + 1 } as Info;
        if (info.status !== "waiting") delete info.question;
        main.subagents = old ? main.subagents.map((s) => (s.name === info.name ? info : s)) : [...main.subagents, info];
        await emit("kotone://agent-event", { conversationId: "c-main", event: { kind: "subagent", info } });
      },
    };
    Object.assign(window, { sidebarTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      if (command.startsWith("browser_") || command.startsWith("agent_subagent_")) bridge.calls.push({ command, args: a });
      switch (command) {
        case "browser_state": return structuredClone(browserState);
        case "browser_set_bounds": case "browser_control": case "browser_handover_done": case "browser_clear_data": return null;
        case "browser_navigate": case "browser_tab_new": case "browser_tab_close": case "browser_tab_select": return null;
        case "browser_extensions": return structuredClone(extensions);
        case "browser_extension_add": {
          const path = String(a.path);
          extensions.push({ id: `ext-${extensions.length + 1}`, name: path.split(/[\\/]/).pop() ?? path, enabled: true, path });
          return null;
        }
        case "browser_extension_toggle": { const e = extensions.find((x) => x.id === a.id); if (e) e.enabled = !!a.enabled; return null; }
        case "browser_extension_remove": extensions.splice(extensions.findIndex((x) => x.id === a.id), 1); return null;
        case "plugin:dialog|open": return "D:\\extensions\\ublock-origin";
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return a.id === "c-main" ? structuredClone(main) : null;
        case "agent_send": return { status: "started", conversationId: "c-main" };
        case "agent_subagent_messages": return { messages: structuredClone(bridge.process) };
        case "agent_subagent_stop": case "agent_subagent_continue": case "agent_subagent_answer": return null;
        case "usage_conversation": return null;
        case "automation_runs": return { running: 0, pending: 0, maxRunning: 2, runs: [] };
        case "get_runtime_status": return structuredClone(runtime);
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "list_connection_presets": return [];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, empty: !!options.empty });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  // 对话读进来之后再发事件（不然 subagent 事件到的时候还没有当前对话）
  if (!options.empty) await expect(page.getByTestId("agent-answer").last()).toHaveText("好呀，开始吧。");
}

const calls = (page: Page, command: string) =>
  page.evaluate((command) => window.sidebarTest.calls.filter((c) => c.command === command).map((c) => c.args ?? null), command);
const lastBounds = async (page: Page) => ((await calls(page, "browser_set_bounds")).at(-1) ?? null) as Bounds | null;

test("the sidebar opens and closes and remembers it; task and browser cards follow their state", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  const sidebar = page.getByTestId("right-sidebar");
  const toggle = page.getByTestId("right-sidebar-toggle");
  // 宽窗口默认开着；没打开网页时告诉玩家可以放歌、上网查
  await expect(sidebar).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("sidebar-browser-card")).toHaveAttribute("data-browser-state", "closed");
  await expect(page.getByTestId("sidebar-browser-hint")).toHaveText("让她上网查东西，页面会在这里出现。");
  await expect(page.getByTestId("sidebar-tasks-empty")).toBeVisible();

  // 收起并记住：重新打开还是收起的
  await toggle.click();
  await expect(sidebar).toHaveCount(0);
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebar"))).toBe("closed");
  // 真的重新打开（同一个地址只差 # 时浏览器不会重新加载）
  await page.goto("about:blank");
  await openChat(page);
  await expect(page.getByTestId("right-sidebar")).toHaveCount(0);
  await page.getByTestId("right-sidebar-toggle").click();
  await expect(page.getByTestId("right-sidebar")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebar"))).toBe("open");

  // 后台在做的事：进行中 → 等确认（允许 / 拒绝）→ 做完带总结
  await page.evaluate(() => window.sidebarTest.subagent({ name: "potion", label: "写喝药脚本", status: "running", steps: 2 }));
  const task = page.locator('[data-testid="sidebar-task"][data-task-name="potion"]');
  await expect(task.getByTestId("sidebar-task-label")).toHaveText("写喝药脚本");
  await expect(task.getByTestId("sidebar-task-status")).toHaveText("进行中 · 第 2 步");
  await task.getByTestId("sidebar-task-stop").click();
  expect(await calls(page, "agent_subagent_stop")).toEqual([{ conversationId: "c-main", name: "potion" }]);
  await page.evaluate(() => window.sidebarTest.subagent({ name: "potion", status: "waiting", question: "要删掉 notes.txt，可以吗？" }));
  await expect(task.getByTestId("sidebar-task-ask")).toContainText("要删掉 notes.txt，可以吗？");
  await task.getByTestId("sidebar-task-allow").click();
  expect(await calls(page, "agent_subagent_answer")).toEqual([{ conversationId: "c-main", name: "potion", answer: "allow-once" }]);
  await page.evaluate(() => window.sidebarTest.subagent({ name: "potion", status: "done", summary: "已保存为自动化《自动喝药》。" }));
  await expect(task).toHaveAttribute("data-task-status", "done");
  await expect(task.getByTestId("sidebar-task-summary")).toHaveText("已保存为自动化《自动喝药》。");

  // 网页卡片：网页里在放什么（只读：放歌在音乐页，ADR 034），她在操作
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  const card = page.getByTestId("sidebar-browser-card");
  await expect(card).toHaveAttribute("data-browser-state", "open");
  await expect(card.getByTestId("sidebar-browser-site")).toHaveText("网易云音乐");
  await expect(card.getByTestId("sidebar-browser-media")).toHaveText("正在放《晴天》- 周杰伦");
  await expect(card.getByTestId("sidebar-browser-pause")).toHaveCount(0);
  await expect(card.getByTestId("sidebar-browser-next")).toHaveCount(0);
  await page.evaluate(() => window.sidebarTest.browser({ busy: true, media: { playing: false, title: "晴天", artist: "周杰伦" } }));
  await expect(card.getByTestId("sidebar-browser-busy")).toHaveText("她在操作");
  await expect(card.getByTestId("sidebar-browser-media")).toHaveText("暂停了：《晴天》- 周杰伦");
  await expect(page.getByTestId("assistant-chat")).not.toContainText(/子代理|子会话|子网页视图|CDP|工具调用/);

  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-${theme}.png` : testInfo.outputPath(`right-sidebar-${theme}.png`), animations: "disabled" });
  }
});

test("the web tab reports where the page goes; the cards tab, dialogs and leaving the chat hide it", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  // 网页卡片上的“展开” = 切到“网页”标签
  await page.getByTestId("sidebar-browser-expand").click();
  const panel = page.getByTestId("browser-panel");
  await expect(panel).toBeVisible();
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "web");
  await expect(page.getByTestId("sidebar-tab-web")).toHaveAttribute("aria-selected", "true");
  await expect(panel.getByTestId("browser-address")).toHaveValue(MUSIC_TAB.url);

  // 报的是占位区域在窗口里的位置，可见
  const slot = page.getByTestId("browser-panel-slot");
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);
  const box = (await slot.boundingBox())!;
  const reported = (await lastBounds(page))!;
  expect(Math.abs(reported.x - box.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(reported.y - box.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(reported.width - box.width)).toBeLessThanOrEqual(1);
  expect(Math.abs(reported.height - box.height)).toBeLessThanOrEqual(1);
  expect(reported.width).toBeGreaterThanOrEqual(380);
  expect(reported.height).toBeGreaterThan(300);
  expect(reported.x + reported.width).toBeLessThanOrEqual(1440);
  expect(reported.y + reported.height).toBeLessThanOrEqual(860);
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-expanded-${theme}.png` : testInfo.outputPath(`right-sidebar-expanded-${theme}.png`), animations: "disabled" });
  }

  // 界面缩放（外观 → 缩放，body 上的 CSS zoom）：报的仍是窗口里的实际位置
  await page.evaluate(() => document.body.style.setProperty("zoom", "1.25"));
  await expect.poll(async () => {
    const b = await lastBounds(page);
    const real = await slot.evaluate((el) => el.getBoundingClientRect().toJSON());
    return b && Math.abs(b.x - Math.round(real.x)) <= 1 && Math.abs(b.width - Math.round(real.width)) <= 1;
  }).toBe(true);
  const zoomed = (await lastBounds(page))!;
  expect(zoomed.x + zoomed.width).toBeLessThanOrEqual(1440 + 1);
  await page.evaluate(() => document.body.style.removeProperty("zoom"));

  // 窗口变了就重新报
  await page.setViewportSize({ width: 1280, height: 760 });
  await expect.poll(async () => { const b = await lastBounds(page); return b ? b.y + b.height : 0; }).toBeLessThanOrEqual(760);
  expect((await lastBounds(page))!.x + (await lastBounds(page))!.width).toBeLessThanOrEqual(1280);

  // 有对话框盖在界面上：先藏起网页；关掉再出现
  await page.evaluate(() => {
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.id = "fake-dialog";
    document.body.appendChild(dialog);
  });
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
  await page.evaluate(() => document.getElementById("fake-dialog")?.remove());
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);

  // 回到卡片：不可见；再切到网页又可见
  await panel.getByTestId("browser-panel-collapse").click();
  await expect(panel).toHaveCount(0);
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "cards");
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
  await page.getByTestId("sidebar-tab-web").click();
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);

  // 地址栏：后退（能后退）、前进（不能前进时不可点）、刷新
  await expect(page.getByTestId("browser-panel-forward")).toBeDisabled();
  await page.getByTestId("browser-panel-back").click();
  await page.getByTestId("browser-panel-reload").click();
  expect((await calls(page, "browser_control")).map((c) => c?.action)).toEqual(["back", "reload"]);

  // 离开聊天页：不可见
  await openSettingsTab(page, "permissions");
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
});

test("a task in the sidebar opens in place to show its result and steps; a small button jumps to the chat card", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate((T) => {
    window.sidebarTest.process = [
      { role: "user", text: "<player-request>帮我放首歌</player-request>\n在右侧栏的浏览器里放《雨爱》。", source: "agent-task:1", atMs: T },
      { role: "assistant", text: "先打开网易云搜一下。", toolCalls: [{ id: "t1", name: "browser.open", arguments: '{"url":"https://music.163.com"}' }], atMs: T + 1000 },
      { role: "tool", callId: "t1", name: "browser.open", content: '{"site":"music.163.com"}', isError: false, untrusted: true, atMs: T + 2000 },
    ];
  }, T);
  await page.evaluate(() => window.sidebarTest.subagent({ name: "song", label: "放歌", status: "running", steps: 1 }));
  const task = page.locator('[data-testid="sidebar-task"][data-task-name="song"]');
  const toggle = task.getByTestId("sidebar-task-toggle");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  const body = task.getByTestId("sidebar-task-body");
  await expect(body.getByTestId("task-card-step-task")).toHaveText("交代：在右侧栏的浏览器里放《雨爱》。");
  await expect(body.getByTestId("task-card-step-text")).toHaveText("先打开网易云搜一下。");
  await expect(body.getByTestId("agent-tool-card")).toHaveCount(1);
  expect(await calls(page, "agent_subagent_messages")).toContainEqual({ conversationId: "c-main", name: "song" });
  // 多一步就重新读
  const before = (await calls(page, "agent_subagent_messages")).length;
  await page.evaluate(() => window.sidebarTest.subagent({ name: "song", steps: 2 }));
  await expect.poll(async () => (await calls(page, "agent_subagent_messages")).length).toBeGreaterThan(before);
  // 做完：先看结果再看过程
  await page.evaluate(() => window.sidebarTest.subagent({ name: "song", status: "done", summary: "放上了《雨爱》- 杨丞琳。" }));
  await expect(body.getByTestId("task-card-summary-full")).toHaveText("放上了《雨爱》- 杨丞琳。");
  await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-task-open.png` : testInfo.outputPath("right-sidebar-task-open.png"), animations: "disabled" });
  await expect(task.getByTestId("sidebar-task-reveal")).toHaveAttribute("title", "在聊天里看");
  // 点开的任务在卡片网格里占满一行；收起后回到一格
  await expect(task).toHaveAttribute("data-open", "true");
  const cardsBox = (await page.getByTestId("sidebar-cards").boundingBox())!;
  expect((await task.boundingBox())!.width).toBeGreaterThan(cardsBox.width - 2);
  await toggle.click();
  await expect(task).toHaveAttribute("data-open", "false");
});

test("a browser that could not open says why and offers a retry", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  const reason = "浏览器没能打开：WebView2 报错 0x80070057（参数错误）";
  await page.evaluate((error) => window.sidebarTest.browser({ open: false, error }), reason);
  const card = page.getByTestId("sidebar-browser-card");
  await expect(card).toHaveAttribute("data-browser-state", "error");
  await expect(card.getByTestId("browser-error-text")).toHaveText(reason);
  await expect(card.getByTestId("sidebar-browser-hint")).toHaveCount(0);
  await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-error.png` : testInfo.outputPath("right-sidebar-error.png"), animations: "disabled" });
  await card.getByTestId("browser-error-retry").click();
  expect(await calls(page, "browser_control")).toEqual([{ action: "reload" }]);
  // 打开过的网页后来坏了：卡片和展开的网页顶上都说
  await page.evaluate(({ playing, error }) => window.sidebarTest.browser({ ...playing, error }), { playing: PLAYING, error: "浏览器没有回应" });
  await expect(card).toHaveAttribute("data-browser-state", "error");
  await expect(card.getByTestId("browser-error-text")).toHaveText("浏览器没有回应");
  await card.getByTestId("sidebar-browser-expand").click();
  await expect(page.getByTestId("browser-panel").getByTestId("browser-error-text")).toHaveText("浏览器没有回应");
  await page.evaluate(() => window.sidebarTest.browser({ error: null }));
  await expect(page.getByTestId("browser-error")).toHaveCount(0);
});

test("a handover opens the sidebar, expands the page and asks the player to log in", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await page.addInitScript(() => { try { localStorage.setItem("kotone.chat.rightSidebar", "closed"); } catch { /* ignore */ } });
  await openChat(page);
  await expect(page.getByTestId("right-sidebar")).toHaveCount(0);
  await page.evaluate(() => window.sidebarTest.browser({
    open: true, url: "https://music.163.com/#/login", title: "网易云音乐", site: "music.163.com", handover: { reason: "网易云要登录一下" },
  }));
  await expect(page.getByTestId("right-sidebar")).toBeVisible();
  await expect(page.getByTestId("browser-panel")).toBeVisible();
  const banner = page.getByTestId("browser-handover");
  await expect(banner).toHaveText(/网易云要登录一下：在下面的页面里登录（网易云可以用手机扫码），好了点「好了」/);
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);
  // 横幅在网页上面，不被盖住
  const bannerBox = (await banner.boundingBox())!;
  const b = (await lastBounds(page))!;
  expect(bannerBox.y + bannerBox.height).toBeLessThanOrEqual(b.y + 1);
  await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-handover.png` : testInfo.outputPath("right-sidebar-handover.png"), animations: "disabled" });
  await page.getByTestId("browser-handover-done").click();
  expect(await calls(page, "browser_handover_done")).toEqual([{}]);
  await page.evaluate(() => window.sidebarTest.browser({ handover: null }));
  await expect(banner).toHaveCount(0);
});

test("narrow window: the sidebar starts closed, opens over the chat and never scrolls sideways; 来首歌 fills the composer", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 720 });
  await openChat(page, { empty: true });
  await expect(page.getByTestId("right-sidebar")).toHaveCount(0);
  await page.getByTestId("right-sidebar-toggle").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar).toBeVisible();
  // 临时打开不记
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebar"))).toBeNull();
  const width = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(width).toBeLessThanOrEqual(0);
  const chat = page.getByTestId("assistant-chat");
  expect(await chat.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
  // 展开网页时占满聊天区，也不横向滚动
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  await page.getByTestId("sidebar-browser-expand").click();
  await expect(page.getByTestId("browser-panel")).toBeVisible();
  expect(await chat.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
  await page.getByTestId("browser-panel-collapse").click();
  await page.getByTestId("right-sidebar-backdrop").click({ position: { x: 8, y: 200 } });
  await expect(sidebar).toHaveCount(0);

  // 起手提示“来首歌”只放进输入框，让玩家接着写歌名
  await page.getByTestId("agent-starter").filter({ hasText: "来首歌" }).click();
  await expect(page.getByTestId("assistant-request")).toHaveValue("来首歌：");
});

test("drag the divider to resize the sidebar (remembered), double-click to reset; cards flow into 1–3 columns", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1700, height: 900 });
  await openChat(page);
  const sidebar = page.getByTestId("right-sidebar");
  const cards = page.getByTestId("sidebar-cards");
  await expect(sidebar).toHaveAttribute("data-width", "420");
  await expect(cards).toHaveAttribute("data-columns", "1");
  for (const name of ["a", "b", "c", "d"]) {
    await page.evaluate((name) => window.sidebarTest.subagent({ name, label: `后台的事 ${name}`, status: "done", summary: "做完了。" }), name);
  }
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);

  const drag = async (dx: number) => {
    const handle = (await page.getByTestId("right-sidebar-resize").boundingBox())!;
    const x = handle.x + handle.width / 2;
    const y = handle.y + handle.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + dx / 2, y, { steps: 4 });
    await page.mouse.move(x + dx, y, { steps: 4 });
    await page.mouse.up();
  };
  // 往左拖变宽：两列
  await drag(-240);
  await expect.poll(async () => Number(await sidebar.getAttribute("data-width"))).toBeGreaterThan(640);
  await expect(cards).toHaveAttribute("data-columns", "2");
  // 再宽：三列
  await drag(-260);
  await expect(cards).toHaveAttribute("data-columns", "3");
  const wide = Number(await sidebar.getAttribute("data-width"));
  expect(Number(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebarWidth")))).toBe(wide);
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-cards-wide-${theme}.png` : testInfo.outputPath(`right-sidebar-cards-wide-${theme}.png`), animations: "disabled" });
  }
  // 聊天区至少留 400：拖不过头
  await drag(-2000);
  const chatBox = (await page.getByTestId("chat-column").boundingBox())!;
  expect(chatBox.width).toBeGreaterThanOrEqual(399);
  // 记住：重新打开还是这个宽度
  const kept = Number(await sidebar.getAttribute("data-width"));
  await page.goto("about:blank");
  await openChat(page);
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-width", String(kept));
  // 双击恢复默认
  await page.getByTestId("right-sidebar-resize").dblclick();
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-width", "420");
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebarWidth"))).toBe("420");
  // 键盘：分隔线获得焦点后左右键调宽
  await page.getByTestId("right-sidebar-resize").focus();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-width", "436");

  // 网页标签里拖动时网页跟着报位置
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  await page.getByTestId("sidebar-tab-web").click();
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);
  await drag(-120);
  await expect.poll(async () => {
    const b = await lastBounds(page);
    const real = await page.getByTestId("browser-panel-slot").evaluate((el) => el.getBoundingClientRect().toJSON());
    return b && b.visible && Math.abs(b.x - Math.round(real.x)) <= 1 && Math.abs(b.width - Math.round(real.width)) <= 1;
  }).toBe(true);
});

test("tabs: the choice is remembered; a handover switches to the web tab", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  // 没打开网页时“网页”也能点：有地址栏，可以自己输入
  await page.getByTestId("sidebar-tab-web").click();
  await expect(page.getByTestId("browser-panel-empty")).toBeVisible();
  await expect(page.getByTestId("browser-address")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebarTab"))).toBe("web");
  await page.getByTestId("sidebar-tab-cards").click();
  expect(await page.evaluate(() => localStorage.getItem("kotone.chat.rightSidebarTab"))).toBe("cards");
  // 卡片标签上显示在跑的任务数
  await page.evaluate(() => window.sidebarTest.subagent({ name: "x", label: "查版本", status: "running", steps: 1 }));
  await expect(page.getByTestId("sidebar-tab-cards-count")).toHaveText("1");
  // 要登录：切到网页
  await page.evaluate((s) => window.sidebarTest.browser({ ...s, handover: { reason: "网易云要登录一下" } }), PLAYING);
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "web");
  await expect(page.getByTestId("browser-handover")).toBeVisible();
  // 记住：重新打开还在网页标签
  await page.goto("about:blank");
  await openChat(page);
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "web");
});

test("browser tabs and address bar: select, close, new tab, navigate, stop; dismissible notice", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate(({ s, guide }) => window.sidebarTest.browser({ ...s, tabs: [...s.tabs, guide] }), { s: PLAYING, guide: GUIDE_TAB });
  await page.getByTestId("sidebar-tab-web").click();
  const tabs = page.getByTestId("browser-tab");
  await expect(tabs).toHaveCount(2);
  await expect(page.getByTestId("sidebar-tab-web-count")).toHaveText("2");
  const music = page.locator('[data-testid="browser-tab"][data-tab-id="t-music"]');
  const guide = page.locator('[data-testid="browser-tab"][data-tab-id="t-guide"]');
  await expect(music).toHaveAttribute("data-active", "true");
  await expect(music.getByTestId("browser-tab-title")).toHaveText(MUSIC_TAB.title);
  await expect(music.getByTestId("browser-tab-agent")).toHaveText("她在用");
  await expect(music.getByTestId("browser-tab-audible")).toBeVisible();
  await expect(guide.getByTestId("browser-tab-agent")).toHaveCount(0);
  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-web-${theme}.png` : testInfo.outputPath(`right-sidebar-web-${theme}.png`), animations: "disabled" });
  }

  // 选另一个标签页、关掉它、新开一个
  await guide.getByTestId("browser-tab-select").click();
  await guide.hover();
  await guide.getByTestId("browser-tab-close").click();
  await page.getByTestId("browser-tab-new").click();
  expect(await calls(page, "browser_tab_select")).toEqual([{ id: "t-guide" }]);
  expect(await calls(page, "browser_tab_close")).toEqual([{ id: "t-guide" }]);
  expect(await calls(page, "browser_tab_new")).toEqual([{}]);
  // 新开标签页后地址栏获得焦点，直接输入：网址或搜索词都交给后端
  const address = page.getByTestId("browser-address");
  await expect(address).toBeFocused();
  await address.fill("晴天 周杰伦");
  await address.press("Enter");
  expect(await calls(page, "browser_navigate")).toEqual([{ url: "晴天 周杰伦" }]);
  // Esc：放弃输入，回到当前网址
  await address.click();
  await address.fill("bilibili.com");
  await address.press("Escape");
  await expect(address).toHaveValue(MUSIC_TAB.url);
  expect((await calls(page, "browser_navigate")).length).toBe(1);

  // 加载中：刷新变成停止；能前进时前进可点
  await page.evaluate(({ s, tab }) => window.sidebarTest.browser({ ...s, loading: true, tabs: [{ ...tab, loading: true, canGoForward: true }] }), { s: PLAYING, tab: MUSIC_TAB });
  await page.getByTestId("browser-panel-stop").click();
  await page.getByTestId("browser-panel-forward").click();
  expect((await calls(page, "browser_control")).map((c) => c?.action)).toEqual(["stop", "forward"]);

  // 提示（例如下载被拒绝）：可以关掉；新的提示再出现
  await page.evaluate((s) => window.sidebarTest.browser({ ...s, notice: "这个网页想下载文件，Kotone 暂时不支持下载。" }), PLAYING);
  await expect(page.getByTestId("browser-notice-text")).toHaveText("这个网页想下载文件，Kotone 暂时不支持下载。");
  await page.getByTestId("browser-notice-dismiss").click();
  await expect(page.getByTestId("browser-notice")).toHaveCount(0);
  await page.evaluate((s) => window.sidebarTest.browser({ ...s, notice: "又一个下载被拒绝了。" }), PLAYING);
  await expect(page.getByTestId("browser-notice-text")).toHaveText("又一个下载被拒绝了。");
});

test("fullscreen fills the window (button, Esc) and follows the page's own fullscreen", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  await page.getByTestId("sidebar-tab-web").click();
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);
  const sidebarSlot = (await lastBounds(page))!;

  await page.getByTestId("browser-panel-fullscreen").click();
  const full = page.getByTestId("browser-fullscreen");
  await expect(full).toBeVisible();
  // 报的是铺满窗口的区域（顶上只留一条窄栏）
  await expect.poll(async () => (await lastBounds(page))?.x).toBe(0);
  const fb = (await lastBounds(page))!;
  expect(fb.visible).toBe(true);
  expect(fb.width).toBeGreaterThanOrEqual(1439);
  expect(fb.y).toBeLessThan(50);
  expect(fb.y + fb.height).toBeGreaterThanOrEqual(859);
  await expect(full.getByTestId("browser-fullscreen-site")).toHaveText("网易云音乐");
  await screenshot(page, { path: SHOTS ? `${SHOTS}/right-sidebar-fullscreen.png` : testInfo.outputPath("right-sidebar-fullscreen.png"), animations: "disabled" });
  // Esc 退出，回到侧栏里的位置
  await page.keyboard.press("Escape");
  await expect(full).toHaveCount(0);
  await expect.poll(async () => (await lastBounds(page))?.x).toBe(sidebarSlot.x);

  // 页面自己要全屏（视频的全屏按钮）：自动进入；页面退出全屏时跟着退出
  await page.evaluate(({ s, tab }) => window.sidebarTest.browser({ ...s, tabs: [{ ...tab, fullscreenElement: true }] }), { s: PLAYING, tab: MUSIC_TAB });
  await expect(full).toBeVisible();
  await page.evaluate(({ s, tab }) => window.sidebarTest.browser({ ...s, tabs: [{ ...tab, fullscreenElement: false }] }), { s: PLAYING, tab: MUSIC_TAB });
  await expect(full).toHaveCount(0);
  // 按钮退出
  await page.getByTestId("browser-panel-fullscreen").click();
  await page.getByTestId("browser-fullscreen-exit").click();
  await expect(full).toHaveCount(0);
});

test("settings: browser extensions — add an unpacked folder, turn it off, remove it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await openSettingsTab(page, "browser");
  const section = page.getByTestId("browser-extensions-settings");
  // 浏览器里装的叫“扩展”，不叫插件（插件是她身上的本事，ADR 038；docs/glossary.md）
  await expect(section.getByRole("heading")).toHaveText("扩展");
  await expect(section).toContainText("只支持已解压的扩展文件夹");
  await expect(section).not.toContainText("插件");
  await expect(section.getByTestId("browser-extensions-empty")).toBeVisible();
  // 选文件夹（对话框）→ 加进去
  await section.getByTestId("browser-extension-add").click();
  await expect.poll(() => calls(page, "browser_extension_add")).toEqual([{ path: "D:\\extensions\\ublock-origin" }]);
  const ext = section.getByTestId("browser-extension");
  await expect(ext.getByTestId("browser-extension-name")).toHaveText("ublock-origin");
  const toggle = ext.getByTestId("browser-extension-toggle");
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect.poll(() => calls(page, "browser_extension_toggle")).toEqual([{ id: "ext-1", enabled: false }]);
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await ext.getByTestId("browser-extension-remove").click();
  await expect.poll(() => calls(page, "browser_extension_remove")).toEqual([{ id: "ext-1" }]);
  await expect(section.getByTestId("browser-extensions-empty")).toBeVisible();
});

test("settings: clearing the browser login asks first", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await openSettingsTab(page, "browser");
  const section = page.getByTestId("browser-data-settings");
  await expect(section).toContainText("内置浏览器");
  await section.getByTestId("browser-clear-data").click();
  await expect(page.getByTestId("browser-clear-dialog")).toBeVisible();
  expect(await calls(page, "browser_clear_data")).toEqual([]);
  await page.getByTestId("browser-clear-confirm").click();
  await expect(page.getByTestId("browser-clear-dialog")).toHaveCount(0);
  expect(await calls(page, "browser_clear_data")).toEqual([{}]);
});

test("settings: the browser has its own section; switching it off hides the web tab and saves browser.enabled", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  await expect(page.getByTestId("sidebar-tab-web")).toBeVisible();
  await expect(page.getByTestId("sidebar-browser-card")).toHaveAttribute("data-browser-state", "open");
  await page.getByTestId("sidebar-tab-web").click();
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);

  // 权限页不再放浏览器的东西；「设置 → 浏览器」（通用组最后一页；插件不在设置里）：开关在最上面，下面是清除登录、扩展
  await openSettingsTab(page, "permissions");
  await expect(page.getByTestId("browser-data-settings")).toHaveCount(0);
  await openSettingsTab(page, "browser");
  const nav = page.getByTestId("advanced-nav-browser");
  await expect(nav).toHaveText("浏览器");
  const tabs = await page.locator('[data-testid^="advanced-nav-"]').evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
  expect(tabs[tabs.length - 1]).toBe("advanced-nav-browser");
  expect(tabs).not.toContain("advanced-nav-plugins");
  const settings = page.getByTestId("settings-browser");
  const toggle = settings.getByTestId("browser-enabled");
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(settings.getByTestId("browser-data-settings")).toContainText("关掉后右侧栏不再有“网页”");
  const switchBox = (await toggle.boundingBox())!;
  const clearBox = (await settings.getByTestId("browser-clear-data").boundingBox())!;
  const extensionsBox = (await settings.getByTestId("browser-extensions-settings").boundingBox())!;
  expect(switchBox.y).toBeLessThan(clearBox.y);
  expect(clearBox.y).toBeLessThan(extensionsBox.y);
  await expect(settings.getByTestId("browser-extensions-settings").getByRole("heading")).toHaveText("扩展");
  await expect(settings).not.toContainText("插件");
  await page.mouse.move(0, 0);
  await screenshot(page, { path: step7(testInfo, "browser-settings"), animations: "disabled" });

  // 关掉：存成 browser.enabled = false
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  const saved = () => page.evaluate(async () => {
    const mock = await import("/src/lib/ipc/browserMock.ts");
    return (await mock.getSettings()).browser?.enabled;
  });
  await expect.poll(saved).toBe(false);

  // 回到聊天：右侧栏没有“网页”标签和网页卡片，网页不摆出来（停在“网页”标签上的也回到“卡片”）
  await page.getByTestId("sidebar-conversations").getByTestId("agent-conversation").first().click();
  await expect(page.getByTestId("right-sidebar")).toBeVisible();
  await expect(page.getByTestId("sidebar-tab-web")).toHaveCount(0);
  await expect(page.getByTestId("sidebar-browser-card")).toHaveCount(0);
  await expect(page.getByTestId("browser-panel")).toHaveCount(0);
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "cards");
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
  // 要玩家登录的事件也不再把右侧栏切到网页
  await page.evaluate(() => window.sidebarTest.browser({ handover: { reason: "网易云要登录一下" } }));
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "cards");
  await expect(page.getByTestId("browser-handover")).toHaveCount(0);

  // 再打开：都回来
  await openSettingsTab(page, "browser");
  await page.getByTestId("browser-enabled").click();
  await expect.poll(saved).toBe(true);
  await page.getByTestId("sidebar-conversations").getByTestId("agent-conversation").first().click();
  await expect(page.getByTestId("sidebar-tab-web")).toBeVisible();
  await page.getByTestId("sidebar-tab-cards").click();
  await expect(page.getByTestId("sidebar-browser-card")).toBeVisible();
});
