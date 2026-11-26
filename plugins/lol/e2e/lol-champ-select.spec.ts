// 选英雄卡片的完整内容和一键导入（ADR 043 B 轮）：峡谷的卡片随着选人的进度长出几块——预选 / 禁用时“可以禁的”、
// 轮到你时“你的英雄池”、锁定以后“对线”（OP.GG）、我方 3 个人选了就有“阵容”、有英雄就有“我的英雄”（符文、出装、加点、
// 召唤师技能）和“一键导入”；海斗是 aramhex-data 的最好的海克斯、核心三件、召唤师技能。同一张卡片（同一个 key）原地更新。
// 一键导入调只给页面的 `lol.import`：“操作客户端”关着时不调，按钮写为什么、点了去插件设置；开着时写进客户端，卡片上列出
// 每一样写成了什么。宽的（聊天）和窄的（悬浮窗的小窗约 360 px）都看。
// 和 lol-pushes.spec.ts 一样：@tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件，卡片里是真的 App（先 build:apps）。
import { expect, test, type Frame, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const KEY = "champ-select:9200001";
const CHAMP_SELECT = "ui://kotone/lol/champ-select";
type Stage = "planning" | "banning" | "waiting" | "picking" | "locked";

const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "feed-and-orb-b", `${name}.png`);

interface Bridge {
  calls: { plugin: string; tool: string }[];
  push(event: unknown): Promise<void>;
  card(mode: "rift" | "mayhem", stage: Stage | null, control: boolean, ended?: boolean): unknown;
}
declare global {
  interface Window {
    lolCsTest: Bridge;
  }
}

async function open(page: Page, options: { mode?: "rift" | "mayhem"; stage?: Stage; control?: boolean; width?: number } = {}) {
  await page.setViewportSize({ width: options.width ?? 1280, height: 900 });
  await page.route("**/__lol-cs-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__lol-cs-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, KEY, CHAMP_SELECT, options }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    // 和 pluginsMock 用同一份 browserMock（共用的开发服务器热更新过 browserMock 时，pluginsMock 引的是带 ?t= 的那一份，
    // 不用同一份的话这里改的开关它看不到）
    const source = await (await fetch("/src/lib/ipc/pluginsMock.ts")).text();
    const browserUrl = /from\s+"(\/src\/lib\/ipc\/browserMock\.ts[^"]*)"/.exec(source)?.[1] ?? "/src/lib/ipc/browserMock.ts";
    const browser = await import(/* @vite-ignore */ browserUrl);
    const { sampleChampSelect } = await import("/src/lib/ipc/lolStatsMock.ts");
    // i18n-ignore-start：示例对话
    const card = (mode: "rift" | "mayhem", stage: Stage | null, control: boolean, ended = false) => ({
      role: "notice", text: "英雄联盟 推送：选英雄", atMs: T - 30_000,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "champ-select", title: "选英雄", key: KEY,
        card: {
          title: "选英雄", ui: { uri: CHAMP_SELECT },
          structuredContent: sampleChampSelect(mode, "艾瑞莉娅", ended, mode === "rift" ? { stage: stage ?? "planning", control } : { mine: true, control }),
        },
      },
    });
    const conversation = {
      version: 1, id: "c-1", title: "这局", createdAtMs: T - 86_400_000, updatedAtMs: T - 20_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        card(options.mode ?? "rift", options.stage ?? "planning", options.control ?? false),
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c-1", running: null, notReadyReason: null,
      conversations: [{ id: "c-1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    const bridge = {
      calls: [] as { plugin: string; tool: string }[],
      async push(event: unknown) {
        await emit("kotone://push", event);
      },
      card,
    };
    Object.assign(window, { lolCsTest: bridge });
    await browser.updateSettings({
      assistant: { connectionId: "test-ai" },
      ...(options.control ? { plugins: { lol: { sources: { control: true } } } } : {}),
    } as never);
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      if (command === "plugin_call") bridge.calls.push({ plugin: (a as { plugin: string }).plugin, tool: (a as { tool: string }).tool });
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversation);
        case "agent_held_cards": return [];
        case "automation_runs": return { runs: [] };
        case "usage_conversation": return null;
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
        case "plugins_list": return plugins.listPlugins();
        case "plugin_app": return plugins.pluginApp((a as { uri: string }).uri);
        case "plugin_resource": return plugins.pluginResource((a as { plugin: string }).plugin, (a as { uri: string }).uri);
        case "plugin_call": return plugins.pluginCall((a as { plugin: string }).plugin, (a as { tool: string }).tool, (a as { arguments: Record<string, unknown> }).arguments);
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, KEY, CHAMP_SELECT, options });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

async function appFrame(page: Page): Promise<Frame> {
  const pattern = /\/app\/lol\/champ-select$/;
  const card = page.locator(`[data-testid="push-item"][data-key="${KEY}"]`);
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(`[data-testid="mcp-app-frame"][data-uri="${CHAMP_SELECT}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  await expect.poll(() => page.frames().filter((f) => pattern.test(f.url())).length, { timeout: 15_000 }).toBe(1);
  return page.frames().find((f) => pattern.test(f.url()))!;
}

/** 同一个 key 再推一次：选人到了下一步（宿主原地更新）。 */
async function advance(page: Page, stage: Stage, control = false, ended = false) {
  await page.evaluate(async ({ KEY, stage, control, ended }) => {
    const t = window.lolCsTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "champ-select", key: KEY, held: false, alert: false,
      message: t.card("rift", stage, control, ended) });
  }, { KEY, stage, control, ended });
}

/** App 那一层窄到 360 px（悬浮窗的小窗）：不横着溢出。 */
async function narrow(app: Frame) {
  await app.evaluate(() => { document.body.style.width = "360px"; });
  const root = app.getByTestId("lol-app-champ-select");
  expect(await root.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
}

test("the Rift card grows with the draft: bans after the pre-pick, the pool at my turn, the lane matchup once locked — one card, updated in place", async ({ page }, testInfo) => {
  await open(page, { stage: "planning" });
  const app = await appFrame(page);
  const root = app.getByTestId("lol-app-champ-select");
  await expect(root).toHaveAttribute("data-stage", "planning");
  await expect(app.getByTestId("lol-cs-stage")).toHaveText("预选");
  // 预选：可以禁的（克制艾瑞莉娅的 3 个 + 这一路最强的），阵容的数，我的英雄（预选）
  const bans = app.getByTestId("lol-cs-ban");
  await expect(bans).toHaveCount(5);
  await expect(app.getByTestId("lol-cs-bans")).toContainText("上路：艾瑞莉娅最难打的、这个版本最强的");
  await expect(bans.first()).toHaveAttribute("data-reason", "counter");
  await expect(bans.first()).toContainText("你对它 44.6%");
  await expect(bans.nth(3)).toContainText("T0 · 胜率 52.8%");
  await expect(app.getByTestId("lol-cs-bans").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "qq101");
  const ally = app.getByTestId("lol-cs-comp-ally");
  await expect(ally).toContainText("我方 4 人");
  await expect(ally).toContainText("物理 4 · 魔法 0 · 混合 0");
  await expect(ally.getByTestId("lol-cs-comp-flag")).toHaveText(["全是物理伤害", "没有前排"]);
  await expect(app.getByTestId("lol-cs-comp").getByTestId("lol-source-tag")).toHaveAttribute("data-source", "client");
  const mine = app.getByTestId("lol-cs-mine");
  await expect(mine).toContainText("预选");
  await expect(mine.getByTestId("lol-cs-mine-runes").locator(".ci")).toHaveCount(9);
  await expect(mine.locator('[data-testid="lol-cs-mine-items"]')).toHaveCount(4);
  await expect(mine.getByTestId("lol-cs-mine-skills")).toContainText("主升 Q > E > W");
  await expect(mine.getByTestId("lol-cs-mine-spells")).toContainText("闪现 + 传送");
  await expect(app.getByTestId("lol-cs-pool")).toHaveCount(0);
  await expect(app.getByTestId("lol-cs-matchup")).toHaveCount(0);
  await app.evaluate(() => { (window as unknown as { __mark: number }).__mark = 3; });
  await screenshot(page, { path: shot(testInfo, "champ-select-planning-wide") });

  // 轮到我选：可以禁的没了，英雄池出来（对面已锁的英雄、好打 / 难打）
  await advance(page, "picking");
  await expect(root).toHaveAttribute("data-stage", "picking", { timeout: 10_000 });
  await expect(app.getByTestId("lol-cs-stage")).toHaveText("轮到你选");
  await expect(app.getByTestId("lol-cs-bans")).toHaveCount(0);
  const pool = app.getByTestId("lol-cs-pool-row");
  await expect(pool).toHaveCount(3);
  await expect(pool.first()).toContainText("盖伦");
  await expect(pool.first()).toContainText("最近 6 局 4 胜");
  await expect(pool.first()).toContainText("好打");
  await expect(pool.first()).toContainText("亚托克斯 53.8%");
  await expect(pool.nth(1)).toContainText("难打");
  await expect(pool.nth(2)).toContainText("和对面已锁的没有对位数据");
  await expect(app.getByTestId("lol-cs-comp-enemy")).toContainText("对面已锁 3 人");
  await narrow(app);
  await screenshot(page, { path: shot(testInfo, "champ-select-picking-narrow") });

  // 锁定：对线（OP.GG，对手是猜的），我的英雄不再写“预选”
  await advance(page, "locked");
  await expect(root).toHaveAttribute("data-stage", "locked", { timeout: 10_000 });
  await expect(app.getByTestId("lol-cs-pool")).toHaveCount(0);
  const matchup = app.getByTestId("lol-cs-matchup");
  await expect(matchup).toContainText("艾瑞莉娅 对 亚托克斯");
  await expect(matchup).toContainText("对手是猜的");
  await expect(matchup.getByTestId("lol-cs-matchup-lane")).toHaveText("亚托克斯");
  await expect(matchup).toContainText("均势");
  await expect(matchup.getByTestId("lol-source-tag")).toHaveAttribute("data-source", "opgg");
  await expect(mine).not.toContainText("预选");
  await narrow(app);
  await screenshot(page, { path: shot(testInfo, "champ-select-locked-narrow") });
  // 一直是同一张卡片，App 没重新加载
  expect(await app.evaluate(() => (window as unknown as { __mark?: number }).__mark)).toBe(3);
  await expect(page.getByTestId("push-item")).toHaveCount(1);

  // 选英雄结束了：导入按钮换成一句“结束了”
  await advance(page, "locked", false, true);
  await expect(root).toHaveAttribute("data-ended", "true", { timeout: 10_000 });
  await expect(app.getByTestId("lol-cs-import")).toContainText("选英雄结束了，不能导入了");
  await expect(app.getByTestId("lol-cs-import-button")).toHaveCount(0);
});

test("import with “Control the client” off: no tool call, the button says why and opens the plugin settings", async ({ page }, testInfo) => {
  await open(page, { stage: "locked", control: false });
  const app = await appFrame(page);
  const imp = app.getByTestId("lol-cs-import");
  await expect(imp).toHaveAttribute("data-control", "off");
  await expect(imp).toContainText("要先在插件设置里打开“操作客户端”（默认关）");
  await narrow(app);
  await screenshot(page, { path: shot(testInfo, "champ-select-import-off-narrow") });
  await app.getByTestId("lol-cs-import-button").click();
  // 不调工具（不写客户端、不替玩家打开开关），去插件设置
  await expect(page.locator('[data-testid="plugin-page"][data-plugin-id="lol"]')).toHaveAttribute("data-view", "settings", { timeout: 10_000 });
  expect((await page.evaluate(() => window.lolCsTest.calls)).filter((c) => c.tool === "lol.import")).toEqual([]);
});

test("import with “Control the client” on: one click calls lol.import and lists what was written", async ({ page }, testInfo) => {
  await open(page, { stage: "locked", control: true, width: 720 });
  const app = await appFrame(page);
  const imp = app.getByTestId("lol-cs-import");
  await expect(imp).toHaveAttribute("data-control", "on");
  await expect(imp).toContainText("符文页、出装方案、召唤师技能写进客户端");
  await app.getByTestId("lol-cs-import-button").click();
  await expect.poll(async () => (await page.evaluate(() => window.lolCsTest.calls)).filter((c) => c.tool === "lol.import"))
    .toEqual([{ plugin: "lol", tool: "lol.import" }]);
  const result = app.getByTestId("lol-cs-import-result");
  await expect(result).toHaveAttribute("data-status", "ok");
  await expect(result.locator("li")).toHaveText([
    "符文页「Kotone 亚索 中路」新建好了",
    "出装方案「Kotone 亚索 中路」写好了（5 组）",
    "召唤师技能换好了",
  ]);
  await screenshot(page, { path: shot(testInfo, "champ-select-import-done") });
});

test("the Mayhem card keeps the grades and adds my champion's best augments, core items and spells from aramhex-data", async ({ page }, testInfo) => {
  await open(page, { mode: "mayhem", control: true });
  const app = await appFrame(page);
  await expect(app.getByTestId("lol-champ-select-row")).toHaveCount(5);
  const mine = app.getByTestId("lol-cs-mine");
  await expect(mine).toHaveAttribute("data-source", "aramhex");
  await expect(mine.getByTestId("lol-cs-mine-augments")).toContainText("+4.2");
  await expect(mine.locator('[data-testid="lol-cs-mine-items"]')).toHaveCount(2);
  // 胜率在右边那一列（和峡谷一样）；标签只写“核心三件”“海克斯”，窄的时候不压在图标上
  await expect(mine.locator('[data-testid="lol-cs-mine-items"]').first()).toContainText("核心三件");
  await expect(mine.locator('[data-testid="lol-cs-mine-items"]').first().locator(".r")).toHaveText("57.3%");
  await expect(mine.getByTestId("lol-cs-mine-augments").locator(".k")).toHaveText("海克斯");
  await expect(mine.getByTestId("lol-cs-mine-runes")).toHaveCount(0);
  await expect(mine.getByTestId("lol-cs-import")).toContainText("这份数据没有符文");
  await expect(mine.getByTestId("lol-source-tag")).toHaveAttribute("data-source", "aramhex");
  // 海斗没有禁用、英雄池、对线
  await expect(app.getByTestId("lol-cs-bans")).toHaveCount(0);
  await expect(app.getByTestId("lol-cs-pool")).toHaveCount(0);
  await narrow(app);
  // 每一行的标签都在标签那一列里，不压到右边的图标上
  for (const row of await mine.locator(".row").all()) {
    const [k, next] = await row.evaluate((el) => {
      const kids = [...el.children] as HTMLElement[];
      return [kids[0].scrollWidth <= kids[0].clientWidth + 1, kids[0].getBoundingClientRect().right <= (kids[1]?.getBoundingClientRect().left ?? Infinity) + 1];
    });
    expect(k && next).toBe(true);
  }
  await screenshot(page, { path: shot(testInfo, "champ-select-mayhem-narrow") });
});
