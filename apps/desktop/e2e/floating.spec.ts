import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 悬浮窗（ADR 043 D6；原来的说话气泡 #/companion 和消息提示 #/message-popup 并进了 #/floating）。用 @tauri-apps/api/mocks
// 模拟 floating_* 命令和 kotone://floating、kotone://bubble、kotone://push 事件：后端怎么决定显示什么、什么时候收、摆在哪
// 由 Rust 单元测试（floating::model、floating::place、screen_corner）管，这里看页面照着画、量好大小报上去、点和拖交给后端。
// 小窗里的卡片是真的英雄联盟 MCP App（vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。

interface Bridge {
  calls: { command: string; args?: Record<string, unknown> }[];
  view(view: Record<string, unknown>): Promise<void>;
  her(snapshot: Record<string, unknown>): Promise<void>;
  push(event: unknown): Promise<void>;
  /** 对话里那张开局战绩，`opening(name)` 是同一个 key、我那一行改了名字的版本。 */
  opening(name?: string): unknown;
  line(key: string, cells: { text: string; tone?: string }[]): unknown;
}
declare global {
  interface Window { floatingTest: Bridge }
}

// 测试页是 Playwright 现给的（route.fulfill），Chrome 按“本地网络访问”的规矩挡掉它去连回环地址上的 MCP App 代理；
// 这个文件里关掉这项检查（真的 Kotone 窗口里页面本来就在本机）。
test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const OPENING = "opening:9100000";
const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round9/${name}.png`, import.meta.url));

/** 打开悬浮窗的页面（窗口大小照后端第一次摆的大小，后面页面按内容报大小）。 */
async function open(page: Page, size = { width: 420, height: 640 }) {
  await page.setViewportSize(size);
  await page.route("**/__floating-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__floating-test#/floating");
  await page.evaluate(async ({ T, OPENING }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    const { sampleMatch } = await import("/src/lib/ipc/lolMock.ts");
    // i18n-ignore-start：示例对话
    const opening = (name?: string) => {
      const data = sampleMatch();
      if (name) data.allies[0] = { ...data.allies[0], name, riotId: `${name}#1001` };
      return {
        role: "notice", text: "英雄联盟 推送：开局战绩", atMs: T - 30_000,
        push: { plugin: "lol", pluginName: "英雄联盟", trigger: "opening", title: "开局战绩", key: OPENING,
          card: { title: "开局战绩", ui: { uri: "ui://kotone/lol/match" }, structuredContent: data }, ...(name ? { updatedAtMs: T } : {}) },
      };
    };
    const line = (key: string, cells: { text: string; tone?: string }[]) => ({
      role: "notice", text: "英雄联盟 推送：海克斯三选一", atMs: T - 60_000,
      push: { plugin: "lol", pluginName: "英雄联盟", trigger: "augment-pick", title: "海克斯三选一", key, line: { cells }, said: "中间那个" },
    });
    const conversation = {
      version: 1, id: "c1", title: "这局", createdAtMs: T - 600_000, updatedAtMs: T,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 600_000 },
        { role: "assistant", text: "在的。", atMs: T - 599_000 },
        line("augment:1", [{ text: "可选", tone: "ok" }, { text: "首选", tone: "good" }, { text: "可刷新", tone: "bad" }]),
        line("augment:2", [{ text: "首选", tone: "good" }, { text: "可选", tone: "ok" }, { text: "可选", tone: "ok" }]),
        opening(),
      ],
    };
    // i18n-ignore-end
    const bridge = {
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      opening, line,
      async view(view: Record<string, unknown>) {
        await emit("kotone://floating", { her: false, lines: [], mini: null, corner: "bottom-right", maxHeight: 420, ...view });
      },
      async her(snapshot: Record<string, unknown>) {
        await emit("kotone://bubble", { heard: "", askSeconds: null, askStartedMs: null, corner: "bottom-right", label: "", text: "", ...snapshot });
      },
      async push(event: unknown) { await emit("kotone://push", event); },
    };
    Object.assign(window, { floatingTest: bridge });
    mockWindows("floating");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      if (command.startsWith("floating_") || command === "auto_card_open") bridge.calls.push({ command, args: a });
      switch (command) {
        case "floating_current": return null;
        case "bubble_snapshot": return null;
        case "agent_conversation": return (a as { id: string }).id === "c1" ? structuredClone(conversation) : null;
        case "agent_held_cards": return [];
        case "plugins_list": return plugins.listPlugins();
        case "plugin_app": return plugins.pluginApp((a as { uri: string }).uri);
        case "plugin_resource": return plugins.pluginResource((a as { plugin: string }).plugin, (a as { uri: string }).uri);
        case "plugin_call": return plugins.pluginCall((a as { plugin: string }).plugin, (a as { tool: string }).tool, (a as { arguments: Record<string, unknown> }).arguments);
      }
      if (command.startsWith("plugin:") || command.startsWith("floating_") || command === "auto_card_open") return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, OPENING });
  await expect(page.getByTestId("floating")).toBeAttached();
}

const calls = (page: Page, command: string) =>
  page.evaluate((command) => window.floatingTest.calls.filter((c) => c.command === command).map((c) => c.args ?? {}), command);
/** 最后一次报上去的大小（版本、宽、高）。 */
const lastLayout = async (page: Page) => (await calls(page, "floating_layout")).at(-1) as { revision: number; width: number; height: number } | undefined;

test("bubble: her line goes listening → thinking → speaking → asking → error, a stale snapshot is ignored, and the size is reported", async ({ page }) => {
  await open(page, { width: 376, height: 160 });
  const floating = page.getByTestId("floating");
  await expect(floating).toHaveAttribute("data-mode", "hidden");
  await expect(page.getByRole("status")).toHaveCount(0);
  await page.evaluate(() => window.floatingTest.view({ revision: 1, mode: "bubble", her: true }));
  const steps: [string, Record<string, unknown>, string | null][] = [
    ["listening", { revision: 1, phase: "listening", label: "在听…", heard: "帮我看看队友说了" }, "“帮我看看队友说了”"],
    ["thinking", { revision: 2, phase: "thinking", label: "正在看屏幕", text: "帮我看看队友说了什么" }, "“帮我看看队友说了什么”"],
    ["speaking", { revision: 3, phase: "speaking", label: "琴音", text: "队友说中路的人消失了，让你注意下路。" }, "队友说中路的人消失了，让你注意下路。"],
    ["asking", { revision: 4, phase: "asking", label: "等你回答", text: "要我帮你回一句“马上来”吗？", askSeconds: 8, askStartedMs: Date.now() }, "要我帮你回一句“马上来”吗？"],
    ["error", { revision: 5, phase: "error", label: "出错了", text: "AI 连接没响应，检查一下网络或 API Key。" }, "AI 连接没响应，检查一下网络或 API Key。"],
  ];
  for (const [phase, snap, text] of steps) {
    await page.evaluate((s) => window.floatingTest.her(s), snap);
    await expect(page.getByRole("status")).toHaveAttribute("data-phase", phase);
    await expect(page.getByTestId("companion-label")).toHaveText(String(snap.label));
    if (text) await expect(page.getByTestId("companion-text")).toHaveText(text);
  }
  // 迟到的旧序号不覆盖新内容
  await page.evaluate(() => window.floatingTest.her({ revision: 2, phase: "thinking", label: "在想…" }));
  await expect(page.getByRole("status")).toHaveAttribute("data-phase", "error");
  // 量好的大小报给后端：版本对得上，大小就是那一块（窗口跟着内容走）
  await expect.poll(async () => (await lastLayout(page))?.revision).toBe(1);
  const box = await floating.boundingBox();
  const layout = await lastLayout(page);
  expect(Math.abs((layout?.width ?? 0) - (box?.width ?? 0))).toBeLessThanOrEqual(1);
  expect(Math.abs((layout?.height ?? 0) - (box?.height ?? 0))).toBeLessThanOrEqual(1);
  expect(layout?.width).toBeLessThanOrEqual(376);
  // 右下角：头像在右边、贴着窗口的右下（等出场动画放大完）
  await expect.poll(async () => {
    const avatar = await page.locator(".avatar").boundingBox();
    return avatar ? 376 - (avatar.x + avatar.width) : 999;
  }).toBeLessThan(16);
  // 后端说收起：什么都不画
  await page.evaluate(() => window.floatingTest.view({ revision: 2, mode: "hidden", her: false }));
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(floating).toHaveAttribute("data-mode", "hidden");
});

test("bubble: a three-cell push line with tones; clicking it opens Kotone at it, × asks the backend to close, dragging goes to the backend", async ({ page }) => {
  await open(page, { width: 376, height: 160 });
  await page.evaluate(() => window.floatingTest.her({ revision: 7, phase: "speaking", label: "琴音", text: "中间那个。" }));
  await page.evaluate(() => window.floatingTest.view({
    revision: 3, mode: "bubble", her: true,
    lines: [{ id: "lol:augment:1", plugin: "lol", pluginName: "英雄联盟", key: "augment:1", conversationId: "c1", untilDone: true, // i18n-ignore
      cells: [{ text: "可选", tone: "ok" }, { text: "首选", tone: "good" }, { text: "可刷新", tone: "bad" }] }], // i18n-ignore
  }));
  const line = page.getByTestId("floating-line");
  await expect(line).toHaveAttribute("data-id", "lol:augment:1");
  await expect(line.getByTestId("floating-cell")).toHaveText(["可选", "首选", "可刷新"]);
  expect(await line.getByTestId("floating-cell").evaluateAll((cells) => cells.map((c) => c.getAttribute("data-tone")))).toEqual(["ok", "good", "bad"]);
  await expect(page.getByTestId("floating-her")).toBeVisible();
  await screenshot(page.getByTestId("floating"), { path: shot("floating-bubble"), omitBackground: true });

  // 鼠标进出告诉后端（在上面时不收）
  await line.hover();
  await expect.poll(async () => (await calls(page, "floating_hover")).at(-1)).toEqual({ inside: true });
  // 点这一行：打开 Kotone、定位到这一条
  await line.click();
  await expect.poll(() => calls(page, "auto_card_open")).toEqual([{ conversationId: "c1", key: "augment:1" }]);
  // 按下就交给后端拖（只是点一下后端不动它）
  expect((await calls(page, "floating_drag")).length).toBeGreaterThanOrEqual(1);
  // ×：交给后端收起
  await page.getByTestId("floating-close").click();
  await expect.poll(() => calls(page, "floating_close")).toHaveLength(1);
  expect(await calls(page, "auto_card_open")).toHaveLength(1);
  await page.mouse.move(370, 5);
  await page.mouse.move(-10, -10);
  await expect.poll(async () => (await calls(page, "floating_hover")).at(-1)).toEqual({ inside: false });
});

test("mini window: the pushes of the conversation with the popped card on top, height follows it, an in-place update keeps the place, and the card header opens Kotone", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.floatingTest.view({
    revision: 10, mode: "mini", maxHeight: 360,
    mini: { conversationId: "c1", focus: "lol:opening:9100000", key: "opening:9100000" },
  }));
  const mini = page.getByTestId("floating-mini");
  await expect(mini).toBeVisible();
  const items = page.getByTestId("floating-item");
  await expect(items).toHaveCount(3);
  expect(await items.evaluateAll((els) => els.map((e) => e.getAttribute("data-key")))).toEqual(["augment:1", "augment:2", OPENING]);
  // 一行的记录是三格带颜色的字（聊天页同一个组件）
  await expect(items.nth(0).getByTestId("push-cell")).toHaveText(["可选", "首选", "可刷新"]);
  const card = items.nth(2);
  await expect(card.getByTestId("push-title")).toHaveText("开局战绩");
  await expect(card.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]')).toBeVisible();

  // 高度跟着定位的那张卡片走，最高 360 - 上面那一行和留白；它在列表最上面（更早的往回翻）
  const list = page.getByTestId("floating-list");
  await expect.poll(async () => (await list.boundingBox())?.height ?? 0).toBeGreaterThan(72);
  expect((await list.boundingBox())!.height).toBeLessThanOrEqual(360 - 44 + 1);
  await expect.poll(async () => {
    const [l, c] = await Promise.all([list.boundingBox(), card.boundingBox()]);
    return l && c ? Math.round(c.y - l.y) : null;
  }).toBeLessThanOrEqual(6);
  // 往回翻：滚轮
  await list.hover();
  await page.mouse.wheel(0, -400);
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(items.nth(0)).toBeInViewport();
  // 也有往前 / 往后翻的按钮（滚轮在不可聚焦的窗口里还没在真机上验证）
  await expect(page.getByTestId("floating-older")).toBeDisabled();
  await page.getByTestId("floating-newer").click();
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);

  // 同一个 key 再推一次：原地换掉，位置不动、不多一张
  const frame = card.locator('[data-testid="mcp-app-frame"]');
  await expect(frame).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await page.evaluate(async (OPENING) => {
    const t = window.floatingTest;
    await t.push({ kind: "updated", conversationId: "c1", plugin: "lol", trigger: "opening", key: OPENING, held: false, alert: false, message: t.opening("改过的名字") }); // i18n-ignore
  }, OPENING);
  await expect.poll(() => page.frames().some((f) => /\/app\/lol\/match$/.test(f.url()))).toBe(true);
  const app = page.frames().find((f) => /\/app\/lol\/match$/.test(f.url()))!;
  await expect(app.locator("body")).toContainText("改过的名字", { timeout: 10_000 });
  await expect(items).toHaveCount(3);
  expect(await items.evaluateAll((els) => els.map((e) => e.getAttribute("data-key")))).toEqual(["augment:1", "augment:2", OPENING]);
  // 新的一条推送：放在最后面
  await page.evaluate(async () => {
    const t = window.floatingTest;
    await t.push({ kind: "posted", conversationId: "c1", plugin: "lol", trigger: "augment-pick", key: "augment:3", held: true, alert: false,
      message: t.line("augment:3", [{ text: "首选", tone: "good" }]) }); // i18n-ignore
  });
  await expect(items).toHaveCount(4);
  await expect(items.nth(3)).toHaveAttribute("data-key", "augment:3");
  await card.scrollIntoViewIfNeeded();
  await screenshot(page.getByTestId("floating"), { path: shot("floating-mini"), omitBackground: true });

  // 卡片上面那一行：打开 Kotone、定位到这张（不在小窗里开右侧栏）
  await card.getByTestId("plugin-card-open").click();
  await expect.poll(() => calls(page, "auto_card_open")).toEqual([{ conversationId: "c1", key: OPENING }]);
  // 小窗上面那一行的“打开 Kotone”、×
  await page.getByTestId("floating-open").click();
  await expect.poll(() => calls(page, "auto_card_open")).toHaveLength(2);
  await page.getByTestId("floating-close").click();
  await expect.poll(() => calls(page, "floating_close")).toHaveLength(1);
  // 报上去的大小就是这一块（宽 380 + 两边 8）
  await expect.poll(async () => (await lastLayout(page))?.width).toBe(396);
});

test("settings 交互 → 悬浮窗: show, pop up the mini window for pushes, where it sits, reset after a drag, danmaku", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.route("**/__floating-settings-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__floating-settings-test#/settings?onboarding=never");
  // 设置走模拟的命令（和真的一样经 get_settings / update_settings；拖完后端写位置、发 kotone://settings-changed）
  await page.evaluate(async () => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const settings = structuredClone(await browser.getSettings()) as unknown as Record<string, unknown>;
    const merge = (base: Record<string, unknown>, patch: Record<string, unknown>) => {
      for (const [k, v] of Object.entries(patch)) {
        const cur = base[k];
        if (cur && v && typeof cur === "object" && typeof v === "object" && !Array.isArray(v)) merge(cur as Record<string, unknown>, v as Record<string, unknown>);
        else base[k] = v;
      }
    };
    const calls: string[] = [];
    Object.assign(window, {
      floatingSettings: {
        settings, calls,
        async dragged(position: unknown) {
          merge(settings, { assistant: { bubble: { position } } });
          await emit("kotone://settings-changed", structuredClone(settings));
        },
      },
    });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      calls.push(command);
      if (command === "get_settings") return structuredClone(settings);
      if (command === "update_settings") { merge(settings, (args as { patch: Record<string, unknown> }).patch); return structuredClone(settings); }
      if (command === "get_startup_options") return { onboarding: "never" };
      if (command.startsWith("plugin:") || command.startsWith("floating_")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  });
  type Stored = { assistant: { bubble: Record<string, unknown> }; agent: { autoCards: Record<string, unknown> } };
  const stored = () => page.evaluate(() => structuredClone((window as unknown as { floatingSettings: { settings: unknown } }).floatingSettings.settings) as Stored);
  await openSettingsTab(page, "reply");
  const section = page.locator("#bubble-settings");
  await expect(section).toContainText("悬浮窗");
  await expect(section).not.toContainText("对话气泡");
  await expect(section).not.toContainText("消息提示");
  await expect(page.getByTestId("push-settings")).toHaveCount(0);
  await expect(page.getByTestId("bubble-enabled")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("push-popup")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("bubble-danmaku")).toHaveAttribute("aria-checked", "false");
  // 没拖过：位置还原是灰的
  await expect(page.getByTestId("bubble-reset")).toBeDisabled();
  await page.getByLabel("悬浮窗放在哪个角").click();
  await page.getByRole("option", { name: "左下角" }).click();
  await expect.poll(async () => (await stored()).assistant.bubble.corner).toBe("bottom-left");
  expect((await stored()).assistant.bubble).toMatchObject({ enabled: true, corner: "bottom-left", offsetY: 0, alsoDanmaku: false });
  // 推送时弹出小窗：写 agent.autoCards.popup（旧的总开关原样带上）
  await page.getByTestId("push-popup").click();
  await expect.poll(async () => (await stored()).agent.autoCards).toEqual({ enabled: true, popup: false });
  // 关掉悬浮窗：推送时弹出小窗灰掉
  await page.getByTestId("bubble-enabled").click();
  await expect(page.getByTestId("push-popup")).toBeDisabled();
  await page.getByTestId("bubble-enabled").click();
  await expect(page.getByTestId("push-popup")).toBeEnabled();

  // 拖过（后端写的位置，发 settings-changed）：位置还原能点，点了清掉、再请后端照角摆一次
  await page.evaluate(() => (window as unknown as { floatingSettings: { dragged(p: unknown): Promise<void> } }).floatingSettings
    .dragged({ corner: "bottom-right", x: 9000, y: 8000 }));
  await expect(page.getByTestId("bubble-reset")).toBeEnabled();
  await page.getByTestId("bubble-enabled").scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot("floating-settings") });
  await page.getByTestId("bubble-reset").click();
  await expect.poll(async () => (await stored()).assistant.bubble.position ?? null).toBeNull();
  await expect(page.getByTestId("bubble-reset")).toBeDisabled();
  expect(await page.evaluate(() => (window as unknown as { floatingSettings: { calls: string[] } }).floatingSettings.calls)).toContain("floating_reposition");
});
