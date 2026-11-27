import { expect, test, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// 悬浮窗（ADR 043 D6，2026-11-26 晚修订）：角上是她的头像 + 一圈进度条，旁边是气泡（简单的）或聊天小窗（复杂的；就是聊天页：
// 同一个 AgentChat、buildTurns、ChatTurn）。用 @tauri-apps/api/mocks 模拟 floating_* / agent_* 命令和 kotone://floating、
// kotone://bubble、kotone://push、kotone://agent-event 事件：后端怎么决定显示不显示、进度条怎么走、摆在哪由 Rust 单元测试
// （floating::model、floating::place、screen_corner、bubble）管；这里看页面照着画、按 shape.ts 的规则选气泡还是小窗、
// 量好大小（和选的样子）报上去、点和拖交给后端。小窗里的卡片是真的英雄联盟 MCP App（vite.config.ts 的 mcpAppsDevServer 给，
// test:e2e 先打包）。

interface Bridge {
  calls: { command: string; args?: Record<string, unknown> }[];
  view(view: Record<string, unknown>): Promise<void>;
  her(snapshot: Record<string, unknown>): Promise<void>;
  push(event: unknown): Promise<void>;
  agent(event: unknown): Promise<void>;
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

/** 示例对话的时间（在过去：“这一次来的”按后端给的 `since` 分）。 */
const T = 1_700_000_000_000;
const OPENING = "opening:9100000";
const shot = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/roundB/${name}.png`, import.meta.url));

/** 空白页（不加载应用）：测纯函数。 */
async function blank(page: Page) {
  await page.route("**/__floating-blank", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body></body></html>',
  }));
  await page.goto("/__floating-blank");
}

/** 打开悬浮窗的页面（窗口大小照后端第一次摆的大小，后面页面按内容报大小）。 */
async function open(page: Page, size = { width: 460, height: 640 }) {
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
        role: "notice", text: "英雄联盟 推送：开局战绩", atMs: T - 500,
        push: { plugin: "lol", pluginName: "英雄联盟", trigger: "opening", title: "开局战绩", key: OPENING,
          card: { title: "开局战绩", ui: { uri: "ui://kotone/lol/match" }, structuredContent: data }, ...(name ? { updatedAtMs: T + 5_000 } : {}) },
      };
    };
    const line = (key: string, cells: { text: string; tone?: string }[]) => ({
      role: "notice", text: "英雄联盟 推送：海克斯三选一", atMs: T - 60_000,
      push: { plugin: "lol", pluginName: "英雄联盟", trigger: "augment-pick", title: "海克斯三选一", key, line: { cells }, said: "中间那个" },
    });
    const conversation = {
      version: 1, id: "c1", title: "这局", createdAtMs: T - 600_000, updatedAtMs: T,
      messages: [
        { role: "user", text: "在吗", source: "voice", atMs: T - 600_000 },
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
        await emit("kotone://floating", { shown: true, her: false, lines: [], mini: null, since: T, ring: null, corner: "bottom-right", maxHeight: 420, ...view });
      },
      async her(snapshot: Record<string, unknown>) {
        await emit("kotone://bubble", { heard: "", askSeconds: null, askStartedMs: null, corner: "bottom-right", label: "", text: "", ...snapshot });
      },
      async push(event: unknown) { await emit("kotone://push", event); },
      async agent(event: unknown) { await emit("kotone://agent-event", { conversationId: "c1", event }); },
    };
    Object.assign(window, { floatingTest: bridge });
    mockWindows("floating");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      if (command.startsWith("floating_") || ["auto_card_open", "agent_send", "assistant_call"].includes(command)) bridge.calls.push({ command, args: a });
      switch (command) {
        case "floating_current": return null;
        case "bubble_snapshot": return null;
        case "agent_state": return { current: "c1", running: null, conversations: [], notReadyReason: null, permissions: { mode: "default", rules: {} } };
        case "agent_conversation": return (a as { id: string }).id === "c1" ? structuredClone(conversation) : null;
        case "agent_held_cards": return [];
        case "agent_send": return { status: "started", conversationId: "c1" };
        case "assistant_call": return null;
        case "get_runtime_status": return { phase: "running", restartNeeded: false, engineId: "fixture", engineName: "fixture",
          modelId: "fixture", interactionMode: "dictation", stage: null, asrLocal: false, error: null };
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
/** 最后一次报上去的大小（版本、宽、高、是不是小窗）。 */
const lastLayout = async (page: Page) =>
  (await calls(page, "floating_layout")).at(-1) as
    { revision: number; width: number; height: number; mini: boolean; boxes: [number, number, number, number][] } | undefined;
/** 点 (x, y)（相对整块内容的左上角）落在报上去的哪一块挡鼠标的地方里（-1 = 都不在：透明，点得到下面的游戏）。 */
const hitAt = (boxes: [number, number, number, number][], x: number, y: number) =>
  boxes.findIndex(([bx, by, w, h]) => bx <= x && x < bx + w && by <= y && y < by + h);
const ringLeft = async (page: Page) => Number(await page.getByTestId("floating-ring").getAttribute("data-left"));

test("the rule: bubble for one simple thing, the mini chat for cards, tool calls, long answers and several at once; fresh turns; the ring", async ({ page }) => {
  await blank(page);
  const results = await page.evaluate(async () => {
    const { floatingShape, freshTurns, complexTurn, ringFraction, LONG_TEXT } = await import("/src/routes/floating/shape.ts");
    const { buildTurns } = await import("/src/lib/agentChat/timeline.ts");
    type Msg = Parameters<typeof buildTurns>[0][number];
    const since = 1_000_000;
    // i18n-ignore-start：示例对话
    const her = (phase: string, text = "") => ({ revision: 1, phase, label: "", text, heard: "", askSeconds: null, askStartedMs: null, corner: "bottom-right" }) as never;
    const ask = (at: number): Msg => ({ role: "user", text: "放首歌", source: "voice", atMs: at });
    const say = (at: number, text: string, written = ""): Msg => ({ role: "assistant", text: "", atMs: at,
      toolCalls: [{ id: `s${at}`, name: "speak", arguments: JSON.stringify({ text, written }) }] });
    const tool = (at: number, name: string): Msg => ({ role: "assistant", text: "", atMs: at, toolCalls: [{ id: `t${at}`, name, arguments: "{}" }] });
    const answer = (at: number, text: string): Msg => ({ role: "assistant", text, atMs: at });
    const card = (at: number): Msg[] => [tool(at, "lol.scout"), { role: "tool", callId: `t${at}`, name: "lol.scout", content: "{}", isError: false, untrusted: true, atMs: at + 1,
      display: { ui: { uri: "ui://kotone/lol/match" } } }];
    const push = (at: number, key: string, updatedAtMs?: number): Msg => ({ role: "notice", text: "推送", atMs: at,
      push: { plugin: "lol", pluginName: "英雄联盟", trigger: "t", title: "t", key, line: { cells: [{ text: "x" }] }, ...(updatedAtMs ? { updatedAtMs } : {}) } });
    const long = "队友说中路的人消失了，让你注意下路，对面打野刚刚在上路露过头，大概二十秒以后会到下路，先往塔下退一点，等你们中单过来再打。";
    // i18n-ignore-end
    const old = [ask(since - 600_000), answer(since - 599_000, "在的。")];
    const shape = (messages: Msg[], herSnap: unknown = null, lines = 0, popped = false, running = false) => {
      const turns = buildTurns(messages, null, running);
      return floatingShape({ popped, her: herSnap as never, lines, fresh: freshTurns(turns, since, null) });
    };
    const turnsOf = (messages: Msg[]) => buildTurns(messages, null, false);
    return {
      longIsLong: [...long].length > LONG_TEXT,
      listening: shape(old, her("listening")),
      short: shape([...old, ask(since + 1_000), say(since + 2_000, "好的。")], her("speaking", "好的。")),
      longSpoken: shape([...old, ask(since + 1_000)], her("speaking", long)),
      longInTurn: shape([...old, ask(since + 1_000), say(since + 2_000, long)]),
      written: shape([...old, ask(since + 1_000), say(since + 2_000, "好了。", "1. 无尽之刃\n2. 疾射火炮")]),
      toolCall: shape([...old, ask(since + 1_000), tool(since + 2_000, "music.play"), say(since + 3_000, "放了。")]),
      narration: shape([...old, ask(since + 1_000), tool(since + 2_000, "danmaku"), tool(since + 2_500, "reply.ask")]),
      shortText: shape([...old, ask(since + 1_000), answer(since + 2_000, "好的，马上。")]),
      longText: shape([...old, ask(since + 1_000), answer(since + 2_000, long)]),
      multiLine: shape([...old, ask(since + 1_000), answer(since + 2_000, "一\n二")]),
      toolCard: shape([...old, ask(since + 1_000), ...card(since + 2_000)]),
      oldToolCallIgnored: shape([...old, tool(since - 500_000, "music.play"), ask(since + 1_000)], her("thinking")),
      popped: shape(old, null, 0, true),
      oneLine: shape([...old, push(since - 100, "lv6")], null, 1),
      herAndLine: shape([...old, push(since - 100, "lv6")], her("speaking", "好的。"), 1),
      twoLines: shape([...old, push(since - 100, "a"), push(since + 100, "b")], null, 2),
      twoTalks: shape([...old, ask(since + 1_000), say(since + 2_000, "好的。"), ask(since + 9_000)], her("listening")),
      fresh: freshTurns(turnsOf([...old, push(since - 1_500, "slack"), push(since - 5_000, "stale"), push(since - 900_000, "updated", since + 3_000),
        push(since - 900_000, "focus")]), since, "lol:focus").map((t) => t.push?.key),
      activeFresh: freshTurns(buildTurns(old, null, true), since, null).length,
      notShown: freshTurns(turnsOf(old), 0, null).length,
      complexOld: complexTurn(turnsOf([ask(1), tool(2, "game.look")])[0]),
      ring: [
        ringFraction({ totalMs: 6_000, leftMs: 6_000, running: true, atMs: 0 }, 3_000),
        ringFraction({ totalMs: 6_000, leftMs: 3_000, running: false, atMs: 0 }, 99_000),
        ringFraction({ totalMs: 6_000, leftMs: 1_000, running: true, atMs: 0 }, 5_000),
        ringFraction(null, 0),
      ],
    };
  });
  expect(results).toEqual({
    longIsLong: true,
    listening: "bubble", short: "bubble", longSpoken: "mini", longInTurn: "mini", written: "mini", toolCall: "mini",
    narration: "bubble", shortText: "bubble", longText: "mini", multiLine: "mini", toolCard: "mini", oldToolCallIgnored: "bubble",
    popped: "mini", oneLine: "bubble", herAndLine: "mini", twoLines: "mini", twoTalks: "mini",
    fresh: ["slack", "updated", "focus"], activeFresh: 1, notShown: 0, complexOld: true,
    ring: [0.5, 0.5, 0, 1],
  });
});

test("bubble: her avatar in the corner and her line listening → thinking → speaking → asking → error; a stale snapshot is ignored; size and shape are reported", async ({ page }) => {
  await open(page, { width: 400, height: 160 });
  const floating = page.getByTestId("floating");
  await expect(floating).toHaveAttribute("data-mode", "hidden");
  await expect(page.getByTestId("floating-avatar")).toHaveCount(0);
  await page.evaluate(() => window.floatingTest.view({ revision: 1, her: true, since: Date.now(),
    ring: { totalMs: 6_000, leftMs: 6_000, running: false, atMs: Date.now() } }));
  // i18n-ignore-start：示例
  const steps: [string, Record<string, unknown>, string | null][] = [
    ["listening", { revision: 1, phase: "listening", label: "在听…", heard: "帮我看看队友说了" }, "“帮我看看队友说了”"],
    ["thinking", { revision: 2, phase: "thinking", label: "正在看屏幕", text: "帮我看看队友说了什么" }, "“帮我看看队友说了什么”"],
    ["speaking", { revision: 3, phase: "speaking", label: "琴音", text: "队友说中路的人消失了，注意下路。" }, "队友说中路的人消失了，注意下路。"],
    ["asking", { revision: 4, phase: "asking", label: "等你回答", text: "要我帮你回一句“马上来”吗？", askSeconds: 8, askStartedMs: Date.now() }, "要我帮你回一句“马上来”吗？"],
    ["error", { revision: 5, phase: "error", label: "出错了", text: "AI 连接没响应，检查一下网络或 API Key。" }, "AI 连接没响应，检查一下网络或 API Key。"],
  ];
  // i18n-ignore-end
  for (const [phase, snap, text] of steps) {
    await page.evaluate((s) => window.floatingTest.her(s), snap);
    await expect(page.getByRole("status")).toHaveAttribute("data-phase", phase);
    await expect(page.getByTestId("floating-avatar")).toHaveAttribute("data-phase", phase);
    await expect(page.getByTestId("companion-label")).toHaveText(String(snap.label));
    if (text) await expect(page.getByTestId("companion-text")).toHaveText(text);
  }
  await expect(floating).toHaveAttribute("data-mode", "bubble");
  // 迟到的旧序号不覆盖新内容
  await page.evaluate(() => window.floatingTest.her({ revision: 2, phase: "thinking", label: "在想…" })); // i18n-ignore
  await expect(page.getByRole("status")).toHaveAttribute("data-phase", "error");
  // 量好的大小报给后端：版本对得上、说的是气泡，大小就是那一块（窗口跟着内容走）
  await expect.poll(async () => (await lastLayout(page))?.revision).toBe(1);
  expect((await lastLayout(page))?.mini).toBe(false);
  const box = await floating.boundingBox();
  const layout = await lastLayout(page);
  expect(Math.abs((layout?.width ?? 0) - (box?.width ?? 0))).toBeLessThanOrEqual(1);
  expect(Math.abs((layout?.height ?? 0) - (box?.height ?? 0))).toBeLessThanOrEqual(1);
  expect(layout?.width).toBeLessThanOrEqual(400);
  // 右下角：头像在右边、贴着窗口的右下（等出场动画放大完）
  await expect.poll(async () => {
    const avatar = await page.getByTestId("floating-avatar").boundingBox();
    return avatar ? Math.round(Math.max(400 - (avatar.x + avatar.width), 160 - (avatar.y + avatar.height))) : 999;
  }).toBeLessThanOrEqual(8);
  // 她说完了，那边收起（快照变成 hidden）：后端还留着她那一行时接着画最后那句
  await page.evaluate(() => window.floatingTest.her({ revision: 6, phase: "hidden" }));
  await expect(page.getByRole("status")).toHaveAttribute("data-phase", "error");
  // 后端说收起：什么都不画
  await page.evaluate(() => window.floatingTest.view({ revision: 2, shown: false, her: false }));
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(page.getByTestId("floating-avatar")).toHaveCount(0);
  await expect(floating).toHaveAttribute("data-mode", "hidden");
});

test("bubble: one push line; the ring runs, hovering stops it and tells the backend, the backend hides it when the ring completes; click opens Kotone, × and drag go to the backend", async ({ page }) => {
  await open(page, { width: 440, height: 160 });
  await page.evaluate(() => window.floatingTest.view({
    revision: 3, since: Date.now(), ring: { totalMs: 6_000, leftMs: 6_000, running: true, atMs: Date.now() },
    lines: [{ id: "lol:augment:1", plugin: "lol", pluginName: "英雄联盟", key: "augment:1", conversationId: "c1", untilDone: false, // i18n-ignore
      cells: [{ text: "可选", tone: "ok" }, { text: "首选", tone: "good" }, { text: "可刷新", tone: "bad" }] }], // i18n-ignore
  }));
  const line = page.getByTestId("floating-line");
  await expect(line).toHaveAttribute("data-id", "lol:augment:1");
  await expect(line.getByTestId("floating-cell")).toHaveText(["可选", "首选", "可刷新"]);
  expect(await line.getByTestId("floating-cell").evaluateAll((cells) => cells.map((c) => c.getAttribute("data-tone")))).toEqual(["ok", "good", "bad"]);
  await expect(page.getByTestId("floating")).toHaveAttribute("data-mode", "bubble");
  await expect(page.getByTestId("floating-avatar")).toBeVisible();
  // 进度条在走
  const ring = page.getByTestId("floating-ring");
  await expect(ring).toHaveAttribute("data-running", "true");
  await expect.poll(() => ringLeft(page)).toBeLessThan(0.95);
  await screenshot(page.getByTestId("floating"), { path: shot("floating-bubble"), omitBackground: true });

  // 鼠标停上去：这里先停住，告诉后端
  await line.hover();
  await expect.poll(async () => (await calls(page, "floating_hover")).at(-1)).toEqual({ inside: true });
  await expect(ring).toHaveAttribute("data-running", "false");
  const held = await ringLeft(page);
  await page.waitForTimeout(400);
  expect(await ringLeft(page)).toBe(held);
  // 点这一行：打开 Kotone、定位到这一条；按下就交给后端拖（只是点一下后端不动它）
  await line.click();
  await expect.poll(() => calls(page, "auto_card_open")).toEqual([{ conversationId: "c1", key: "augment:1" }]);
  expect((await calls(page, "floating_drag")).length).toBeGreaterThanOrEqual(1);
  // ×：交给后端收起
  await page.getByTestId("floating-close").click();
  await expect.poll(() => calls(page, "floating_close")).toHaveLength(1);
  expect(await calls(page, "auto_card_open")).toHaveLength(1);
  // 鼠标移开：告诉后端，接着走（后端那一版：至少还剩 2 秒）
  await page.mouse.move(-10, -10);
  await expect.poll(async () => (await calls(page, "floating_hover")).at(-1)).toEqual({ inside: false });
  await page.evaluate(() => window.floatingTest.view({ revision: 4, since: Date.now() - 5_000,
    ring: { totalMs: 6_000, leftMs: 900, running: true, atMs: Date.now() },
    lines: [{ id: "lol:augment:1", plugin: "lol", pluginName: "英雄联盟", key: "augment:1", conversationId: "c1", untilDone: false, cells: [{ text: "首选", tone: "good" }] }] })); // i18n-ignore
  await expect(ring).toHaveAttribute("data-running", "true");
  await expect.poll(() => ringLeft(page)).toBe(0);
  // 走完了：后端整个收起
  await page.evaluate(() => window.floatingTest.view({ revision: 5, shown: false, ring: null }));
  await expect(page.getByTestId("floating-avatar")).toHaveCount(0);
  await expect(page.getByTestId("floating-line")).toHaveCount(0);
});

test("mini chat: a popped card shows the conversation's latest turns with the chat page's components; height follows it; an in-place update keeps the place; the card header opens Kotone", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.floatingTest.view({
    revision: 10, maxHeight: 360, ring: { totalMs: 10_000, leftMs: 10_000, running: true, atMs: Date.now() },
    mini: { conversationId: "c1", focus: "lol:opening:9100000", key: "opening:9100000" },
  }));
  const mini = page.getByTestId("floating-mini");
  await expect(mini).toBeVisible();
  await expect(page.getByTestId("floating")).toHaveAttribute("data-mode", "mini");
  await expect(page.getByTestId("floating-avatar")).toBeVisible();
  const turns = page.getByTestId("floating-turn");
  await expect(turns).toHaveCount(4);
  expect(await turns.evaluateAll((els) => els.map((e) => e.getAttribute("data-push")))).toEqual(["", "lol:augment:1", "lol:augment:2", `lol:${OPENING}`]);
  // 聊天页的组件：玩家的话、她的回答、推送的一行（三格带颜色）、卡片
  await expect(mini.getByTestId("agent-user-message")).toHaveText("在吗");
  await expect(mini.getByTestId("agent-answer")).toHaveText("在的。");
  await expect(turns.nth(1).getByTestId("push-cell")).toHaveText(["可选", "首选", "可刷新"]);
  const card = turns.nth(3);
  await expect(card.getByTestId("push-title")).toHaveText("开局战绩");
  await expect(card.locator('[data-testid="plugin-card"][data-uri="ui://kotone/lol/match"]')).toBeVisible();
  // 紧凑样子：没有每一轮上面的小头像和名字、没有页脚
  await expect(mini.getByTestId("kotone-avatar")).toHaveCount(0);
  await expect(mini.getByTestId("agent-turn-footer")).toHaveCount(0);
  // 报上去的是小窗
  await expect.poll(async () => (await lastLayout(page))?.mini).toBe(true);

  // 高度跟着这一次来的那张卡片走，最高 360 - 上面那一行和边框；它从上面开始看（更早的往回翻）
  const list = page.getByTestId("floating-list");
  await expect.poll(async () => (await list.boundingBox())?.height ?? 0).toBeGreaterThan(72);
  expect((await list.boundingBox())!.height).toBeLessThanOrEqual(360 - 34 + 1);
  await expect.poll(async () => {
    const [l, c] = await Promise.all([list.boundingBox(), card.boundingBox()]);
    return l && c ? Math.round(c.y - l.y) : null;
  }).toBeLessThanOrEqual(6);
  // 往回翻：滚轮
  await list.hover();
  await page.mouse.wheel(0, -2_000);
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(turns.nth(0)).toBeInViewport();
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
  await expect(app.getByTestId("lol-match-card")).toHaveAttribute("data-surface", "floating");
  await expect(app.getByTestId("lol-source-tag")).toHaveCount(0);
  await expect(app.getByTestId("lol-card-row")).toHaveCount(10);
  await expect(turns).toHaveCount(4);
  expect(await turns.evaluateAll((els) => els.map((e) => e.getAttribute("data-push")))).toEqual(["", "lol:augment:1", "lol:augment:2", `lol:${OPENING}`]);
  // 新的一条推送：放在最后面
  await page.evaluate(async () => {
    const t = window.floatingTest;
    await t.push({ kind: "posted", conversationId: "c1", plugin: "lol", trigger: "augment-pick", key: "augment:3", held: true, alert: false,
      message: t.line("augment:3", [{ text: "首选", tone: "good" }]) }); // i18n-ignore
  });
  await expect(turns).toHaveCount(5);
  await expect(turns.nth(4)).toHaveAttribute("data-push", "lol:augment:3");
  await card.scrollIntoViewIfNeeded();
  await screenshot(page.getByTestId("floating"), { path: shot("floating-mini-card"), omitBackground: true });

  // 卡片上面那一行：打开 Kotone、定位到这张（不在小窗里开右侧栏）
  await card.getByTestId("plugin-card-open").click();
  await expect.poll(() => calls(page, "auto_card_open")).toEqual([{ conversationId: "c1", key: OPENING }]);
  // 小窗上面那一行的“打开 Kotone”：定位到弹出的那张；×
  await page.getByTestId("floating-open").click();
  await expect.poll(() => calls(page, "auto_card_open")).toHaveLength(2);
  expect((await calls(page, "auto_card_open"))[1]).toEqual({ conversationId: "c1", key: OPENING });
  await page.getByTestId("floating-close").click();
  await expect.poll(() => calls(page, "floating_close")).toHaveLength(1);
  // 小窗 360 + 头像 56 - 重叠的 18 + 两边 8。
  await expect.poll(async () => (await lastLayout(page))?.width).toBe(414);
  // 挡鼠标的只有两块：小窗、右下角的头像（各带着阴影那 8）；头像上面那一条透明的不挡（窗口只留这两块，点得到下面的游戏）
  const layout = (await lastLayout(page))!;
  expect(layout.boxes).toHaveLength(2);
  const [rootBox, miniBox, avatarBox] = await Promise.all([
    page.getByTestId("floating").boundingBox(), mini.boundingBox(), page.getByTestId("floating-avatar").boundingBox()]);
  const rel = (b: { x: number; y: number; width: number; height: number }, fx: number, fy: number) =>
    [b.x - rootBox!.x + b.width * fx, b.y - rootBox!.y + b.height * fy] as const;
  expect(hitAt(layout.boxes, ...rel(miniBox!, 0.5, 0.5))).toBeGreaterThanOrEqual(0);
  expect(hitAt(layout.boxes, ...rel(avatarBox!, 0.5, 0.5))).toBeGreaterThanOrEqual(0);
  expect(hitAt(layout.boxes, ...rel(miniBox!, 0.02, 0.02))).toBeGreaterThanOrEqual(0);
  expect(layout.height - avatarBox!.height).toBeGreaterThan(100);
  const strip = rel(avatarBox!, 0.5, 0);
  expect(hitAt(layout.boxes, strip[0], strip[1] - 40)).toBe(-1);
  expect(hitAt(layout.boxes, strip[0], 4)).toBe(-1);
});

test("the bubble folds into the avatar before the mini unfolds; the composer sends to that chat and takes focus only on request", async ({ page }) => {
  await open(page);
  const floating = page.getByTestId("floating");
  await page.evaluate(() => window.floatingTest.view({
    revision: 1,
    lines: [{ id: "lol:enemy-6", plugin: "lol", pluginName: "英雄联盟", key: "enemy-6", conversationId: "c1", untilDone: false,
      cells: [{ text: "对面艾希" }, { text: "6 级了，有大招" }] }], // i18n-ignore
  }));
  await expect(floating).toHaveAttribute("data-surface", "bubble");
  await expect(floating).toHaveAttribute("data-unfolded", "true");
  await floating.evaluate((el) => {
    const stages: string[] = [];
    const capture = () => {
      const stage = `${el.dataset.surface}:${el.dataset.unfolded}`;
      if (stages.at(-1) !== stage) stages.push(stage);
    };
    capture();
    new MutationObserver(capture).observe(el, { attributes: true, attributeFilter: ["data-surface", "data-unfolded"] });
    Object.assign(window.floatingTest, { stages });
  });
  await page.evaluate(() => window.floatingTest.view({ revision: 2, mini: { conversationId: "c1", focus: "lol:opening:9100000", key: "opening:9100000" } }));
  const input = page.getByTestId("floating-composer-input");
  const mic = page.getByTestId("floating-composer-mic");
  const send = page.getByTestId("floating-composer-send");
  await expect(floating).toHaveAttribute("data-surface", "mini");
  await expect(floating).toHaveAttribute("data-unfolded", "true");
  const stages = await page.evaluate(() => (window.floatingTest as Bridge & { stages: string[] }).stages);
  expect(stages).toEqual(["bubble:true", "bubble:false", "mini:false", "mini:true"]);
  await expect(input).toBeVisible();
  await expect(input).not.toBeFocused();
  await expect(mic).toBeEnabled();
  await expect(send).toHaveCount(0);
  expect(await calls(page, "floating_input_focus")).toEqual([]);

  await mic.click();
  await expect.poll(() => calls(page, "assistant_call")).toHaveLength(1);
  expect(await calls(page, "floating_input_focus")).toEqual([]);
  await input.click();
  await expect.poll(() => calls(page, "floating_input_focus")).toEqual([{ active: true }]);
  await input.fill("看看我的战绩"); // i18n-ignore
  await expect(send).toBeVisible();
  await input.dispatchEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true });
  expect(await calls(page, "agent_send")).toEqual([]);
  await input.press("Enter");
  await expect.poll(() => calls(page, "agent_send")).toEqual([{ text: "看看我的战绩", conversationId: "c1" }]); // i18n-ignore
  await expect(input).toHaveValue("");
  await input.press("Escape");
  await expect.poll(async () => (await calls(page, "floating_input_focus")).at(-1)).toEqual({ active: false });

  await page.evaluate(() => window.floatingTest.view({ revision: 3, shown: false }));
  await expect(floating).toHaveAttribute("data-unfolded", "false");
  await expect(page.getByTestId("floating-avatar")).toHaveCount(0);
  await expect.poll(() => lastLayout(page)).toMatchObject({ revision: 3, boxes: [] });

  await page.evaluate(() => window.floatingTest.view({
    revision: 4, corner: "bottom-left", mini: { conversationId: "c1", focus: "lol:opening:9100000", key: "opening:9100000" },
  }));
  await expect.poll(async () => {
    const [avatar, button] = await Promise.all([page.getByTestId("floating-avatar").boundingBox(), mic.boundingBox()]);
    return avatar && button ? button.x - (avatar.x + avatar.width) : -1;
  }).toBeGreaterThanOrEqual(6);
  await page.evaluate(() => window.floatingTest.view({
    revision: 5, corner: "top-right", mini: { conversationId: "c1", focus: "lol:opening:9100000", key: "opening:9100000" },
  }));
  await input.fill("draft for c1");
  await page.evaluate(() => window.floatingTest.view({
    revision: 6, mini: { conversationId: "c1", focus: "lol:augment:1", key: "augment:1" },
  }));
  await expect(page.getByTestId("floating-mini")).toHaveAttribute("data-focus", "lol:augment:1");
  await expect(input).toHaveValue("draft for c1");
  await page.evaluate(() => window.floatingTest.view({
    revision: 7, mini: { conversationId: "c2", focus: "lol:augment:2", key: "augment:2" },
  }));
  await expect(page.getByTestId("floating-mini")).toHaveAttribute("data-focus", "lol:augment:2");
  await expect(input).toHaveValue("");
  await expect(input).not.toBeFocused();
  await expect(send).toHaveCount(0);
  await expect.poll(async () => (await calls(page, "floating_input_focus")).at(-1)).toEqual({ active: false });
  await page.getByTestId("floating-close").click();
  await expect.poll(() => calls(page, "floating_close")).toHaveLength(1);
});

test("bubble: a one-cell push line (an interpreter translation, up to 40 characters) wraps inside 360 px; a line of several cells stays on one row; the hit boxes are the avatar and the bubble", async ({ page }) => {
  await open(page, { width: 460, height: 200 });
  // i18n-ignore-start：示例
  const long = "队友说他们的打野刚刚从上路的河道绕到了我们的蓝区，小心中路那边的视野，先别压线。";
  // i18n-ignore-end
  await page.evaluate((text) => window.floatingTest.view({
    revision: 21, since: Date.now(), ring: { totalMs: 6_000, leftMs: 6_000, running: true, atMs: Date.now() },
    lines: [{ id: "interpreter:lines:1", plugin: "interpreter", pluginName: "同传", key: "lines:1", conversationId: "c1", untilDone: false, // i18n-ignore
      cells: [{ text }] }],
  }), long);
  const line = page.getByTestId("floating-line");
  const cell = line.getByTestId("floating-cell");
  await expect(cell).toHaveText(long);
  await expect.poll(() => line.evaluate((el) => getComputedStyle(el.parentElement!).transform)).toBe("none");
  const one = await line.boundingBox();
  expect(one!.width).toBeLessThanOrEqual(360);
  // 折成了几行：比一行高（一行 18 + 上下 2 = 22）、整句都在框里（没有横着溢出）
  expect((await cell.boundingBox())!.height).toBeGreaterThanOrEqual(2 * 18 + 4);
  expect(await line.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await expect.poll(async () => (await lastLayout(page))?.revision).toBe(21);
  const layout = (await lastLayout(page))!;
  expect(layout.mini).toBe(false);
  expect(layout.width).toBeLessThanOrEqual(360 + 56 - 18 + 16);
  expect(layout.boxes).toHaveLength(2);
  await screenshot(page.getByTestId("floating"), { path: shot("floating-bubble-wrapped"), omitBackground: true });

  // 分了几格的一行：照旧一行，格子不折
  await page.evaluate(() => window.floatingTest.view({
    revision: 22, since: Date.now(), ring: { totalMs: 6_000, leftMs: 6_000, running: true, atMs: Date.now() },
    lines: [{ id: "lol:lv6:1", plugin: "lol", pluginName: "英雄联盟", key: "lv6:1", conversationId: "c1", untilDone: false, // i18n-ignore
      cells: [{ text: "对面寒冰" }, { text: "6 级了，有大招" }] }], // i18n-ignore
  }));
  await expect(line.getByTestId("floating-cell")).toHaveText(["对面寒冰", "6 级了，有大招"]); // i18n-ignore
  const heights = await line.getByTestId("floating-cell").evaluateAll((cells) => cells.map((c) => c.getBoundingClientRect().height));
  for (const h of heights) expect(h).toBeLessThanOrEqual(24);
  const tops = await line.getByTestId("floating-cell").evaluateAll((cells) => cells.map((c) => Math.round(c.getBoundingClientRect().top)));
  expect(new Set(tops).size).toBe(1);
});

test("mini chat: her voice turn starts as a bubble and switches to the mini chat when she uses a tool; it shows her turn like the chat page and stays a mini chat after she is done", async ({ page }) => {
  await open(page);
  const now = await page.evaluate(() => Date.now());
  await page.evaluate((now) => window.floatingTest.view({ revision: 20, her: true, since: now,
    ring: { totalMs: 6_000, leftMs: 6_000, running: false, atMs: now } }), now);
  await page.evaluate(() => window.floatingTest.her({ revision: 30, phase: "listening", label: "在听…", heard: "放首晴天" })); // i18n-ignore
  const floating = page.getByTestId("floating");
  await expect(floating).toHaveAttribute("data-mode", "bubble");
  await expect(page.getByTestId("floating-ring")).toHaveAttribute("data-running", "false");
  // 她这一轮：玩家的话记进对话，她开始想（还是气泡）
  await page.evaluate(async (now) => {
    const t = window.floatingTest;
    await t.agent({ kind: "appended", message: { role: "user", text: "放首晴天", source: "voice", atMs: now + 1_000 } }); // i18n-ignore
    await t.agent({ kind: "step", step: 1 });
    await t.her({ revision: 31, phase: "thinking", label: "在想…", text: "放首晴天" }); // i18n-ignore
  }, now);
  await expect(page.getByRole("status")).toHaveAttribute("data-phase", "thinking");
  await expect(floating).toHaveAttribute("data-mode", "bubble");
  // 用了工具：换成聊天小窗
  await page.evaluate(async (now) => {
    const t = window.floatingTest;
    await t.agent({ kind: "appended", message: { role: "assistant", text: "", atMs: now + 2_000,
      toolCalls: [{ id: "t1", name: "music.play", arguments: '{"query":"晴天"}' }] } }); // i18n-ignore
    await t.agent({ kind: "toolCall", call: { id: "t1", name: "music.play", arguments: '{"query":"晴天"}' } }); // i18n-ignore
  }, now);
  await expect(floating).toHaveAttribute("data-mode", "mini");
  const mini = page.getByTestId("floating-mini");
  const tool = mini.locator('[data-testid="agent-tool-card"][data-tool-name="music.play"]');
  await expect(tool).toBeVisible();
  await expect(mini.getByTestId("agent-user-message").last()).toHaveText("放首晴天");
  await expect.poll(async () => (await lastLayout(page))?.mini).toBe(true);
  // 工具做完、她说了一句（和聊天页一样：带喇叭的一句话）
  await page.evaluate(async (now) => {
    const t = window.floatingTest;
    await t.agent({ kind: "appended", message: { role: "tool", callId: "t1", name: "music.play", content: '{"status":"ok"}', isError: false, untrusted: false, atMs: now + 3_000 } });
    await t.agent({ kind: "appended", message: { role: "assistant", text: "", atMs: now + 3_500,
      toolCalls: [{ id: "t2", name: "speak", arguments: '{"text":"放了，晴天。"}' }] } }); // i18n-ignore
    await t.her({ revision: 32, phase: "speaking", label: "琴音", text: "放了，晴天。" }); // i18n-ignore
  }, now);
  await expect(tool).toHaveAttribute("data-tool-status", "completed");
  await expect(mini.getByTestId("agent-spoken")).toContainText("放了，晴天。");
  // 这一次来的从玩家那句开始：最下面是她说的那句，上面能翻回更早的
  const last = page.getByTestId("floating-turn").last();
  await expect(last).toBeInViewport();
  await screenshot(page.getByTestId("floating"), { path: shot("floating-mini-tool"), omitBackground: true });
  // 她这一轮结束、那边收起：还是小窗（这一次出现里不来回跳），进度条接着走
  await page.evaluate(async (now) => {
    const t = window.floatingTest;
    await t.agent({ kind: "idle" });
    await t.her({ revision: 33, phase: "hidden" });
    await t.view({ revision: 21, her: false, since: now, ring: { totalMs: 10_000, leftMs: 10_000, running: true, atMs: Date.now() } });
  }, now);
  await expect(floating).toHaveAttribute("data-mode", "mini");
  await expect(page.getByTestId("floating-ring")).toHaveAttribute("data-running", "true");
  // 收起以后再出来：从头算（一句提醒是气泡）
  await page.evaluate(() => window.floatingTest.view({ revision: 22, shown: false, ring: null }));
  await expect(floating).toHaveAttribute("data-mode", "hidden");
  await page.evaluate((now) => window.floatingTest.view({ revision: 23, since: now + 100_000, ring: { totalMs: 6_000, leftMs: 6_000, running: true, atMs: Date.now() },
    lines: [{ id: "lol:lv6:1", plugin: "lol", pluginName: "英雄联盟", key: "lv6:1", conversationId: "c1", untilDone: false, cells: [{ text: "对面中单 6 级了" }] }] }), now); // i18n-ignore
  await expect(floating).toHaveAttribute("data-mode", "bubble");
  await expect(page.getByTestId("floating-line")).toBeVisible();
});

test("settings 交互 → 悬浮窗: show, pop up the mini chat for pushes, where it sits, reset after a drag, danmaku", async ({ page }) => {
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
  // 说明照新的样子：头像旁边是气泡或聊天小窗
  await expect(page.getByText(/角上是她的头像.*聊天小窗/)).toBeVisible();
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
