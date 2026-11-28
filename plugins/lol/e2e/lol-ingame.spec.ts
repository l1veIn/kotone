// 英雄联盟对局里的两张推送卡片（ADR 043 D7 的 2026-11-26 晚修订、B 轮第 2 条）：
// - 海克斯三选一：玩家说“不需要单独给海克斯做一个UI，就直接推卡片就行啊，就像战绩一样推送，卡片上写清楚三个海克斯推荐程度就行”。
//   卡片（插件的 MCP App 的一页 `ui://kotone/lol/augment-pick`）从左到右三张：第几个、图标、名字、评级、首选 / 可选 / 可刷新 / 没数据；
//   刷新了同一个 key 原地更新（App 不重新加载），选完了标“选好了”；窄到悬浮窗的小窗（约 360 px）三张还是一排。
//   2026-11-27：“现在的海斗有时候会是四选一”——四选一时四格（标题“海克斯四选一”），小窗里四格也一排、名字和评级看得清。
// - 你阵亡时：对面每个人的等级、KDA、装备（按装备总价排），上次阵亡以后新出的描一圈；窄的时候装备换到下一行。
// 和 lol-pushes.spec.ts 一样：@tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件，插件的命令交给浏览器预览的
// pluginsMock，卡片里是真的 App（vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。
import { expect, test, type Frame, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

test.use({ launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks"] } });

const T = 1_800_000_000_000;
const PICK_KEY = "augment:9100001:1800000000000";
const DEATH_KEY = "death:9100001";
const AUGMENT_PICK = "ui://kotone/lol/augment-pick";
const ENEMY_BUILD = "ui://kotone/lol/enemy-build";

const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round-b", `${name}.png`);

interface Bridge {
  push(event: unknown): Promise<void>;
  /** 三选一卡片那条 Notice：`middle` 中间那张（刷新了换掉；`four` 是四选一：多一张、首选在第四个），`ended` 选完了。 */
  pick(middle?: "bees" | "unread" | "four", ended?: boolean): unknown;
  /** 阵亡卡片那条 Notice：第几次阵亡；`atMs` 是它在对话里的时间（带提醒的更新挪到最后时换成挪的那一刻）。 */
  death(count?: number, atMs?: number): unknown;
}
declare global {
  interface Window {
    lolIngameTest: Bridge;
  }
}

async function open(page: Page, width = 1280) {
  await page.setViewportSize({ width, height: 900 });
  await page.route("**/__lol-ingame-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__lol-ingame-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, PICK_KEY, DEATH_KEY, AUGMENT_PICK, ENEMY_BUILD }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    // i18n-ignore-start：示例对话和卡片的数据（插件推来的样子，plugins/lol/src/pushes/augment_pick.rs、alerts.rs）
    const icon = (file: string) => `/lol-game-data/assets/ASSETS/UX/Cherry/Augments/Icons/${file}_small.png`;
    const left = { id: 2, name: "鼓声暴起 低音轰鸣", rarity: "prismatic", icon: icon("Drums"), grade: "A", verdict: "ok", few: false };
    const bees = { id: 4, name: "弹跳蜜蜂", rarity: "silver", icon: icon("Bees"), grade: "C", verdict: "reroll", few: true };
    const right = { id: 3, name: "终极九头蛇", rarity: "prismatic", icon: icon("Hydra"), grade: "S", verdict: "best", few: false };
    const glacier = { id: 6, name: "冰川之心", rarity: "gold", icon: icon("Glacier"), grade: "B", verdict: "reroll", few: false };
    const pick = (middle: "bees" | "unread" | "four" = "bees", ended = false) => {
      const four = middle === "four";
      return {
        role: "notice", text: "英雄联盟 推送：海克斯三选一", atMs: T - 30_000,
        push: {
          plugin: "lol", pluginName: "英雄联盟", trigger: "augment-pick", title: "海克斯三选一", key: PICK_KEY, said: four ? "选第四个" : "选第三个",
          card: { title: four ? "海克斯四选一" : "海克斯三选一", ui: { uri: AUGMENT_PICK }, structuredContent: {
            champion: { id: 39, short: "艾瑞莉娅" },
            count: four ? 4 : 3,
            slots: four ? [left, bees, glacier, right] : [left, middle === "bees" ? bees : { unread: true }, right],
            best: four ? 3 : 2, weak: false, source: "aramhex", patch: "16.23", ended,
          } },
          ...(middle === "bees" && !ended ? {} : { updatedAtMs: T }),
        },
      };
    };
    const item = (id: number, name: string, isNew = false) => ({ id, name, icon: `/lol-game-data/assets/ASSETS/Items/Icons2D/${id}.png`, new: isNew });
    const enemy = (id: number, short: string, level: number, k: number, d: number, a: number, gold: number, items: unknown[], dead = false) =>
      ({ champion: { id, short }, level, kills: k, deaths: d, assists: a, gold, items, dead, hidden: false });
    const death = (count = 1, atMs = T - 20_000) => ({
      role: "notice", text: "英雄联盟 推送：对面的装备", atMs,
      push: {
        plugin: "lol", pluginName: "英雄联盟", trigger: "my-death", title: "对面的装备", key: DEATH_KEY,
        card: { title: "对面的装备", ui: { uri: ENEMY_BUILD }, structuredContent: {
          deaths: count, at: "12:34", respawn: 18, goldKnown: true,
          me: { champion: { id: 39, short: "艾瑞莉娅" }, level: 11, kills: 4, deaths: count, assists: 6, gold: 6100 },
          enemies: [
            enemy(157, "亚索", 13, 9, 2, 3, 9800, [item(3026, "守护天使", count > 1), item(3031, "无尽之刃"), item(3006, "狂战士胫甲")]),
            enemy(112, "维克托", 12, 5, 3, 7, 8200, [item(3157, "中娅沙漏", count > 1), item(3020, "法师之靴"), item(2503, "黯炎火炬")], true),
            enemy(89, "蕾欧娜", 11, 1, 4, 12, 6400, [item(3075, "荆棘之甲"), item(3047, "铁板靴")]),
            enemy(163, "塔莉垭", 11, 3, 5, 4, 5600, [item(3916, "湮灭宝珠", count > 1), item(1052, "增幅典籍")]),
            enemy(22, "艾希", 10, 2, 6, 5, 4100, [item(3006, "狂战士胫甲")]),
          ],
        } },
        ...(count === 1 ? {} : { updatedAtMs: T }),
      },
    });
    const conversation = {
      version: 1, id: "c-1", title: "这局", createdAtMs: T - 86_400_000, updatedAtMs: T - 20_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 86_400_000 },
        { role: "assistant", text: "在的。", atMs: T - 86_399_000 },
        pick(),
        death(),
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c-1", running: null, notReadyReason: null,
      conversations: [{ id: "c-1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    Object.assign(window, { lolIngameTest: { async push(event: unknown) { await emit("kotone://push", event); }, pick, death } });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
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
  }, { T, PICK_KEY, DEATH_KEY, AUGMENT_PICK, ENEMY_BUILD });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);

async function appFrame(page: Page, which: string): Promise<Frame> {
  const pattern = new RegExp(`/app/lol/${which}$`);
  await expect.poll(() => page.frames().filter((f) => pattern.test(f.url())).length, { timeout: 15_000 }).toBe(1);
  return page.frames().find((f) => pattern.test(f.url()))!;
}

test("the augment pick is a card in the chat: three augments left to right, each with its grade and verdict, the top pick ringed", async ({ page }, testInfo) => {
  await open(page);
  const card = pushAt(page, PICK_KEY);
  await expect(card.getByTestId("push-title")).toHaveText("海克斯三选一");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(`[data-testid="mcp-app-frame"][data-uri="${AUGMENT_PICK}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  const app = await appFrame(page, "augment-pick");
  const root = app.getByTestId("lol-app-augment-pick");
  await expect(root).toHaveAttribute("data-ended", "false");
  await expect(root).toContainText("你用 艾瑞莉娅");
  const slots = app.getByTestId("lol-augment-pick-slot");
  await expect(slots).toHaveCount(3);
  expect(await slots.evaluateAll((els) => els.map((e) => e.getAttribute("data-verdict")))).toEqual(["ok", "reroll", "best"]);
  await expect(slots.nth(0)).toContainText("鼓声暴起 低音轰鸣");
  await expect(slots.nth(0)).toContainText("可选");
  await expect(slots.nth(1)).toContainText("可刷新");
  await expect(slots.nth(1)).toContainText("样本少");
  await expect(slots.nth(2)).toContainText("终极九头蛇");
  await expect(slots.nth(2)).toContainText("首选");
  await expect(slots.nth(2)).toContainText("S");
  // 和游戏里一样从左到右排开，每格写第几个（和她说的“选第三个”对得上）；首选（第三个）用墨线圈出来，别的没有
  await expect(app.getByTestId("lol-augment-pick-cols")).toHaveAttribute("data-count", "3");
  expect(await slots.locator(".place").allInnerTexts()).toEqual(["第一个", "第二个", "第三个"]);
  const boxes = await Promise.all([0, 1, 2].map((i) => slots.nth(i).boundingBox()));
  expect(boxes[0]!.x).toBeLessThan(boxes[1]!.x);
  expect(boxes[1]!.x).toBeLessThan(boxes[2]!.x);
  expect(Math.abs(boxes[0]!.y - boxes[2]!.y)).toBeLessThan(2);
  await expect(slots.nth(2)).toHaveAttribute("data-best", "true");
  expect(await slots.nth(2).evaluate((e) => getComputedStyle(e).boxShadow)).not.toBe("none");
  expect(await slots.nth(0).evaluate((e) => getComputedStyle(e).boxShadow)).toBe("none");
  // 来源标签：aramhex-data（许可证要求的署名），不写胜率数字
  await expect(app.getByTestId("lol-source-tag")).toHaveAttribute("data-source", "aramhex");
  await expect(root).toContainText("aramhex-data");
  expect(await root.innerText()).not.toMatch(/\d+(\.\d+)?%/);
  await screenshot(page, { path: shot(testInfo, "chat-augment-pick-card") });
});

test("a reroll updates the same augment card in place (the App is not reloaded); when the cards are gone it reads 选好了", async ({ page }) => {
  await open(page);
  await pushAt(page, PICK_KEY).scrollIntoViewIfNeeded();
  const app = await appFrame(page, "augment-pick");
  const slots = app.getByTestId("lol-augment-pick-slot");
  await expect(slots.nth(1)).toHaveAttribute("data-verdict", "reroll");
  await app.evaluate(() => { (window as unknown as { __mark: number }).__mark = 7; });
  // 刷新了中间那张，新的认不出：同一个 key 再推一次
  await page.evaluate(async (key) => {
    const t = window.lolIngameTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "augment-pick", key, held: false, alert: false, message: t.pick("unread") });
  }, PICK_KEY);
  await expect(slots.nth(1)).toHaveAttribute("data-verdict", "unread", { timeout: 10_000 });
  await expect(slots.nth(1)).toContainText("没认出");
  await expect(slots.nth(2)).toHaveAttribute("data-verdict", "best");
  expect(await app.evaluate(() => (window as unknown as { __mark?: number }).__mark)).toBe(7);
  await expect(page.locator(`[data-testid="push-item"][data-key="${PICK_KEY}"]`)).toHaveCount(1);
  // 选完了：最后一次标“选好了”
  await page.evaluate(async (key) => {
    const t = window.lolIngameTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "augment-pick", key, held: false, alert: false, message: t.pick("unread", true) });
  }, PICK_KEY);
  const root = app.getByTestId("lol-app-augment-pick");
  await expect(root).toHaveAttribute("data-ended", "true", { timeout: 10_000 });
  await expect(root).toContainText("选好了");
});

test("the augment card fits the floating window's mini window (about 360 px): three columns stay in one row", async ({ page }, testInfo) => {
  await open(page);
  await pushAt(page, PICK_KEY).scrollIntoViewIfNeeded();
  const app = await appFrame(page, "augment-pick");
  const slots = app.getByTestId("lol-augment-pick-slot");
  await expect(slots).toHaveCount(3);
  await app.evaluate(() => { document.body.style.width = "360px"; });
  const root = app.getByTestId("lol-app-augment-pick");
  expect(await root.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  const boxes = await Promise.all([0, 1, 2].map((i) => slots.nth(i).boundingBox()));
  expect(Math.abs(boxes[0]!.y - boxes[2]!.y)).toBeLessThan(2);
  const frame = (await root.boundingBox())!;
  expect(boxes[2]!.x + boxes[2]!.width).toBeLessThanOrEqual(frame.x + frame.width + 1);
  // 长名字两行以内，不撑出格子
  for (let i = 0; i < 3; i += 1) {
    expect(await slots.nth(i).evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  }
  await screenshot(page, { path: shot(testInfo, "chat-augment-pick-narrow") });
});

test("a four-choice augment pick is four columns in one row (title 海克斯四选一), still readable in the 360 px mini window", async ({ page }, testInfo) => {
  await open(page);
  await pushAt(page, PICK_KEY).scrollIntoViewIfNeeded();
  const app = await appFrame(page, "augment-pick");
  const slots = app.getByTestId("lol-augment-pick-slot");
  await expect(slots).toHaveCount(3);
  // 认成了四张：同一个 key 再推一次，原地换成四格
  await page.evaluate(async (key) => {
    const t = window.lolIngameTest;
    await t.push({ kind: "updated", conversationId: "c-1", plugin: "lol", trigger: "augment-pick", key, held: false, alert: false, message: t.pick("four") });
  }, PICK_KEY);
  await expect(slots).toHaveCount(4, { timeout: 10_000 });
  await expect(app.getByTestId("lol-augment-pick-cols")).toHaveAttribute("data-count", "4");
  await expect(pushAt(page, PICK_KEY).getByTestId("push-title")).toHaveText("海克斯四选一");
  const root = app.getByTestId("lol-app-augment-pick");
  await expect(root).toContainText("海克斯四选一");
  expect(await slots.evaluateAll((els) => els.map((e) => e.getAttribute("data-verdict")))).toEqual(["ok", "reroll", "reroll", "best"]);
  expect(await slots.locator(".place").allInnerTexts()).toEqual(["第一个", "第二个", "第三个", "第四个"]);
  await expect(slots.nth(2)).toContainText("冰川之心");
  await expect(slots.nth(3)).toHaveAttribute("data-best", "true");
  await screenshot(page, { path: shot(testInfo, "chat-augment-pick-four") });
  // 小窗（约 360 px）：四格还是一排、不溢出；每格的名字、评级、判断都在格子里看得见
  for (const width of [360, 320]) {
    await app.evaluate((w) => { document.body.style.width = `${w}px`; }, width);
    expect(await root.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    const boxes = await Promise.all([0, 1, 2, 3].map((i) => slots.nth(i).boundingBox()));
    for (let i = 1; i < 4; i += 1) {
      expect(Math.abs(boxes[i]!.y - boxes[0]!.y)).toBeLessThan(2);
      expect(boxes[i]!.x).toBeGreaterThan(boxes[i - 1]!.x + boxes[i - 1]!.width - 1);
    }
    const frame = (await root.boundingBox())!;
    expect(boxes[3]!.x + boxes[3]!.width).toBeLessThanOrEqual(frame.x + frame.width + 1);
    for (let i = 0; i < 4; i += 1) {
      const slot = slots.nth(i);
      expect(await slot.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      const cell = boxes[i]!;
      for (const part of [".name", ".grade", ".verdict"]) {
        const box = (await slot.locator(part).boundingBox())!;
        expect(box.width, `${width}px 第 ${i + 1} 格的 ${part}`).toBeGreaterThan(8);
        expect(box.x).toBeGreaterThanOrEqual(cell.x - 0.5);
        expect(box.x + box.width).toBeLessThanOrEqual(cell.x + cell.width + 0.5);
      }
      // 名字一行至少放得下五个字（中文海克斯名常见四到六个字，长的两行以内）
      const room = await slot.evaluate((el) => {
        const style = getComputedStyle(el);
        const font = parseFloat(getComputedStyle(el.querySelector(".name")!).fontSize);
        return (el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)) / font;
      });
      expect(room, `${width}px 第 ${i + 1} 格`).toBeGreaterThanOrEqual(5);
    }
    if (width === 360) await screenshot(page, { path: shot(testInfo, "chat-augment-pick-four-narrow") });
  }
});

test("when you die: one card with each enemy's level, KDA and items ordered by item value; what they bought since your last death is ringed", async ({ page }, testInfo) => {
  await open(page);
  const card = pushAt(page, DEATH_KEY);
  await expect(card.getByTestId("push-title")).toHaveText("对面的装备");
  await card.scrollIntoViewIfNeeded();
  const app = await appFrame(page, "enemy-build");
  const root = app.getByTestId("lol-app-enemy-build");
  await expect(root).toContainText("你第 1 次阵亡 · 12:34 · 18 秒后复活");
  const rows = app.getByTestId("lol-enemy-build-row");
  await expect(rows).toHaveCount(5);
  await expect(rows.first()).toContainText("亚索");
  await expect(rows.first()).toContainText("9/2/3");
  await expect(rows.first()).toContainText("9.8k");
  await expect(rows.nth(1)).toContainText("阵亡中");
  await expect(app.getByTestId("lol-enemy-build-me")).toContainText("你：11 级 · 4/1/6 · 装备 6.1k");
  await expect(root).toContainText("按身上装备的总价排");
  await expect(app.locator('[data-new="true"]')).toHaveCount(0);
  // 第二次阵亡：带提醒的更新，这张挪到对话最后（时间换成挪的那一刻，卡片重新加载），新出的三件描一圈
  await page.evaluate(async ({ key, at }) => {
    const t = window.lolIngameTest;
    await t.push({ kind: "moved", conversationId: "c-1", plugin: "lol", trigger: "my-death", key, held: false, alert: true, message: t.death(2, at) });
  }, { key: DEATH_KEY, at: T });
  await expect.poll(() => app.isDetached(), { timeout: 10_000 }).toBe(true);
  const moved = await appFrame(page, "enemy-build");
  const movedRoot = moved.getByTestId("lol-app-enemy-build");
  await expect(movedRoot).toHaveAttribute("data-deaths", "2", { timeout: 10_000 });
  await expect(moved.locator('[data-new="true"]')).toHaveCount(3);
  expect(await moved.locator('[data-new="true"]').first().evaluate((e) => getComputedStyle(e).boxShadow)).not.toBe("none");
  await expect(page.locator(`[data-testid="push-item"][data-key="${DEATH_KEY}"]`)).toHaveCount(1);
  await expect(page.getByTestId("push-item").last()).toHaveAttribute("data-key", DEATH_KEY);
  await screenshot(page, { path: shot(testInfo, "chat-enemy-build-card") });
  // 窄（悬浮窗的小窗约 360 px）：装备换到名字下面一行，不横着溢出
  await moved.evaluate(() => { document.body.style.width = "360px"; });
  expect(await movedRoot.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  const first = moved.getByTestId("lol-enemy-build-row").first();
  // 换行由页面里量宽度的那一下决定，机器忙时晚一帧：等它排好再比
  await expect
    .poll(async () => {
      const name = await first.locator(".name").boundingBox();
      const items = await first.locator(".items").boundingBox();
      return items!.y - name!.y;
    })
    .toBeGreaterThan(4);
  await screenshot(page, { path: shot(testInfo, "chat-enemy-build-narrow") });
});
