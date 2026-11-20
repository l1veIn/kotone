import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab } from "./nav";

// 聊天页右侧栏（ADR 031，照 DeepSeek Harness 桌面版）：网页卡片 + 后台在做的事；网页展开时把占位区域的位置报给后端
// （网页由后端摆在窗口里）；要玩家登录时自动展开并出横幅；窄窗口默认收起、盖在聊天上。
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
    };
  }
}

const T = 1_800_000_000_000;
const SHOTS = process.env.KOTONE_SIDEBAR_SHOTS;
const PLAYING = {
  open: true, url: "https://music.163.com/#/song?id=186016", title: "晴天 - 周杰伦 - 单曲 - 网易云音乐", site: "music.163.com",
  loading: false, handover: null, busy: false, media: { playing: true, title: "晴天", artist: "周杰伦" },
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
    let browserState: Record<string, unknown> = { open: false, url: null, title: null, site: null, loading: false, handover: null, media: null, busy: false };
    const runtime = { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "测试引擎", modelId: "fixture", interactionMode: "dictation", stage: null, asrLocal: false, error: null };
    const bridge = {
      calls: [] as Call[],
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
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return a.id === "c-main" ? structuredClone(main) : null;
        case "agent_send": return { status: "started", conversationId: "c-main" };
        case "agent_subagent_messages": return { messages: [] };
        case "agent_subagent_stop": case "agent_subagent_continue": case "agent_subagent_answer": return null;
        case "usage_conversation": return null;
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
  await expect(page.getByTestId("sidebar-browser-hint")).toHaveText("让她放首歌，或者让她上网查东西，页面会在这里出现。");
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

  // 网页卡片：在放歌（播放 / 暂停、下一首），她在操作
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  const card = page.getByTestId("sidebar-browser-card");
  await expect(card).toHaveAttribute("data-browser-state", "open");
  await expect(card.getByTestId("sidebar-browser-site")).toHaveText("网易云音乐");
  await expect(card.getByTestId("sidebar-browser-media")).toHaveText("正在放《晴天》- 周杰伦");
  await card.getByTestId("sidebar-browser-pause").click();
  await card.getByTestId("sidebar-browser-next").click();
  expect(await calls(page, "browser_control")).toEqual([{ action: "pause" }, { action: "next" }]);
  await page.evaluate(() => window.sidebarTest.browser({ busy: true, media: { playing: false, title: "晴天", artist: "周杰伦" } }));
  await expect(card.getByTestId("sidebar-browser-busy")).toHaveText("她在操作");
  await expect(card.getByTestId("sidebar-browser-media")).toHaveText("暂停了：《晴天》- 周杰伦");
  await expect(card.getByTestId("sidebar-browser-play")).toBeVisible();
  await expect(page.getByTestId("assistant-chat")).not.toContainText(/子代理|子会话|子网页视图|CDP|工具调用/);

  for (const theme of ["light", "dark"] as const) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await page.screenshot({ path: SHOTS ? `${SHOTS}/right-sidebar-${theme}.png` : testInfo.outputPath(`right-sidebar-${theme}.png`), animations: "disabled" });
  }
});

test("the expanded browser reports where the page goes; collapsing, dialogs and leaving the chat hide it", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await page.evaluate((s) => window.sidebarTest.browser(s), PLAYING);
  await page.getByTestId("sidebar-browser-expand").click();
  const panel = page.getByTestId("browser-panel");
  await expect(panel).toBeVisible();
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-expanded", "true");
  await expect(panel.getByTestId("browser-panel-site")).toHaveText("网易云音乐");

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
    await page.screenshot({ path: SHOTS ? `${SHOTS}/right-sidebar-expanded-${theme}.png` : testInfo.outputPath(`right-sidebar-expanded-${theme}.png`), animations: "disabled" });
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

  // 收起网页：不可见；再展开又可见
  await panel.getByTestId("browser-panel-collapse").click();
  await expect(panel).toHaveCount(0);
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
  await page.getByTestId("sidebar-browser-expand").click();
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(true);

  // 顶部一条：后退、刷新
  await page.getByTestId("browser-panel-back").click();
  await page.getByTestId("browser-panel-reload").click();
  expect((await calls(page, "browser_control")).map((c) => c?.action)).toEqual(["back", "reload"]);

  // 离开聊天页：不可见
  await openSettingsTab(page, "permissions");
  await expect.poll(async () => (await lastBounds(page))?.visible).toBe(false);
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
  await page.screenshot({ path: SHOTS ? `${SHOTS}/right-sidebar-handover.png` : testInfo.outputPath("right-sidebar-handover.png"), animations: "disabled" });
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

test("settings: clearing the browser login asks first", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openChat(page);
  await openSettingsTab(page, "permissions");
  const section = page.getByTestId("browser-data-settings");
  await expect(section).toContainText("内置浏览器");
  await section.getByTestId("browser-clear-data").click();
  await expect(page.getByTestId("browser-clear-dialog")).toBeVisible();
  expect(await calls(page, "browser_clear_data")).toEqual([]);
  await page.getByTestId("browser-clear-confirm").click();
  await expect(page.getByTestId("browser-clear-dialog")).toHaveCount(0);
  expect(await calls(page, "browser_clear_data")).toEqual([{}]);
});
