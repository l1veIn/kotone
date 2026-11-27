import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 第三方插件的“事件”（ADR 047 第 2 期，观察项）：插件设置里的一块（开关、第一次打开的确认、自己加一个、停下时的提示和重试），
// 插件卡片上的“停了”，自动化的“当……的时候”能选到它的事件、模板“一条龙跑完推一行”按 Kotone 预设找到玩家起的插件 id。
// 浏览器预览用 ipc/mcpServersMock（观察项的样子照后端 `watch::gate` 现算，不连任何服务器）。

/** 一个用着 Kotone 预设（绝区零一条龙）的第三方插件，玩家给它起名 longzhu。 */
function onedragon(extra: Record<string, unknown> = {}) {
  return {
    id: "longzhu",
    entry: { type: "http", url: "http://127.0.0.1:23001/mcp" },
    status: "connected",
    trusted: true,
    tools: [
      { name: "get_run_status", readOnly: true, title: "查询运行状态" },
      { name: "run_one_dragon", readOnly: false, title: "运行一条龙" },
    ],
    kotoneManifest: {
      source: "preset",
      preset: "zzz-onedragon",
      watchFrom: { run_finished: "preset", run_failed: "preset" },
      manifest: {
        games: ["zzz"],
        events: [
          { name: "run_finished", label: "一条龙跑完了", data: { from: "之前的状态", to: "现在的状态" } },
          { name: "run_failed", label: "一条龙出错了", data: { from: "之前的状态", to: "现在的状态" } },
        ],
        watches: [
          { event: "run_finished", tool: "get_run_status", everySeconds: 30, compare: "changed", field: "state", to: ["success"] },
          { event: "run_failed", tool: "get_run_status", everySeconds: 30, compare: "changed", field: "state", to: ["failed"] },
        ],
      },
    },
    ...extra,
  };
}

async function seed(page: Page, servers: unknown[]) {
  await page.addInitScript((list) => {
    (window as unknown as { __kotoneMcpSeed: unknown }).__kotoneMcpSeed = list;
  }, servers);
}

async function openEvents(page: Page) {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-longzhu-open").click();
  const risk = page.getByTestId("plugin-risk-dialog-longzhu");
  if (await risk.isVisible().catch(() => false)) await risk.getByTestId("plugin-risk-ok").click();
  await page.getByTestId("plugin-settings-button").click();
  const events = page.getByTestId("remote-plugin-events");
  await expect(events).toBeVisible();
  return events;
}

const row = (events: ReturnType<Page["getByTestId"]>, event: string) => events.locator(`[data-testid="remote-plugin-event"][data-event="${event}"]`);

test("the Events block lists watches and asks once before turning one on", async ({ page }, testInfo) => {
  await seed(page, [onedragon()]);
  const events = await openEvents(page);
  await expect(events.getByTestId("remote-plugin-event")).toHaveCount(2);
  const finished = row(events, "run_finished");
  await expect(finished).toHaveAttribute("data-state", "off");
  await expect(finished).toContainText("一条龙跑完了");
  await expect(finished).toContainText("longzhu.run_finished");
  await expect(finished.getByTestId("remote-plugin-event-from")).toHaveText("Kotone 带的");
  await expect(finished).toContainText("每 30 秒调一次 get_run_status，结果里的 state 变成 success 时算");
  await expect(finished.getByTestId("remote-plugin-event-status")).toHaveText("没开");
  await expect(events).toContainText("只交给「自动化」");

  // 第一次打开：先确认（说清楚调哪个工具、多久一次、挂在哪个游戏上、只给自动化）
  await finished.getByTestId("remote-plugin-event-switch").click();
  const confirm = page.getByTestId("remote-plugin-event-confirm");
  await expect(confirm).toBeVisible();
  await expect(confirm.getByTestId("remote-plugin-event-confirm-rule")).toContainText("Kotone 会每 30 秒调一次 get_run_status");
  await expect(confirm.getByTestId("remote-plugin-event-confirm-rule")).toContainText("这个工具要只读");
  await expect(confirm).toContainText("只在 zzz 开着时查");
  await expect(confirm).toContainText("longzhu.run_finished");
  await expect(confirm.getByTestId("remote-plugin-event-agree")).toHaveCount(0);
  await screenshot(page, { path: testInfo.outputPath("remote-watch-confirm.png"), animations: "disabled" });
  // 取消：还是关着
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  await expect(finished).toHaveAttribute("data-state", "off");
  await finished.getByTestId("remote-plugin-event-switch").click();
  await confirm.getByTestId("remote-plugin-event-confirm-on").click();
  await expect(confirm).toHaveCount(0);
  // 绝区零没开：等游戏开
  await expect(finished).toHaveAttribute("data-state", "waiting");
  await expect(finished.getByTestId("remote-plugin-event-status")).toHaveText("等游戏开");
  // 关了再开：确认过这份内容，不再问
  await finished.getByTestId("remote-plugin-event-switch").click();
  await expect(finished).toHaveAttribute("data-state", "off");
  await finished.getByTestId("remote-plugin-event-switch").click();
  await expect(page.getByTestId("remote-plugin-event-confirm")).toHaveCount(0);
  await expect(finished).toHaveAttribute("data-state", "waiting");
  await screenshot(page, { path: testInfo.outputPath("remote-watch-events.png"), animations: "disabled", fullPage: true });
});

test("players add their own event from a tool, with consent for tools not marked read-only, and can remove it", async ({ page }, testInfo) => {
  await seed(page, [onedragon({ gameRunning: true })]);
  const events = await openEvents(page);
  await events.getByTestId("remote-plugin-events-add").click();
  const dialog = page.getByTestId("remote-plugin-event-add");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("remote-plugin-event-add-label").fill("开始跑了");
  await dialog.getByTestId("remote-plugin-event-add-name").fill("run_started");
  await dialog.getByTestId("remote-plugin-event-add-field").fill("state");
  await dialog.getByTestId("remote-plugin-event-add-to").fill("running");
  await dialog.getByTestId("remote-plugin-event-add-every").fill("5");
  await expect(dialog.getByTestId("remote-plugin-event-add-rule")).toContainText("每 5 秒调一次 get_run_status，结果里的 state 变成 running 时算");
  // 换成没标只读的工具：要勾上同意才能加
  await dialog.getByTestId("remote-plugin-event-add-tool").click();
  await page.getByRole("option", { name: "run_one_dragon", exact: true }).click();
  await expect(dialog.getByTestId("remote-plugin-event-add-submit")).toBeDisabled();
  await dialog.getByTestId("remote-plugin-event-add-agree").check();
  await expect(dialog.getByTestId("remote-plugin-event-add-submit")).toBeEnabled();
  // 写错的参数不交
  await dialog.getByTestId("remote-plugin-event-add-args").fill("[1, 2]");
  await dialog.getByTestId("remote-plugin-event-add-submit").click();
  await expect(dialog.getByTestId("remote-plugin-event-add-error")).toContainText("JSON 对象");
  await dialog.getByTestId("remote-plugin-event-add-args").fill("{\"block\": false}");
  await screenshot(page, { path: testInfo.outputPath("remote-watch-add.png"), animations: "disabled" });
  await dialog.getByTestId("remote-plugin-event-add-submit").click();
  await expect(dialog).toHaveCount(0);
  // 加完就开（游戏开着：在查）
  const mine = row(events, "run_started");
  await expect(mine).toHaveAttribute("data-state", "polling");
  await expect(mine.getByTestId("remote-plugin-event-from")).toHaveText("你加的");
  await expect(mine).toContainText("longzhu.run_started");
  // 重名的不加
  await events.getByTestId("remote-plugin-events-add").click();
  await dialog.getByTestId("remote-plugin-event-add-label").fill("又一个");
  await dialog.getByTestId("remote-plugin-event-add-name").fill("run_finished");
  await dialog.getByTestId("remote-plugin-event-add-field").fill("state");
  await dialog.getByTestId("remote-plugin-event-add-submit").click();
  await expect(dialog.getByTestId("remote-plugin-event-add-error")).toContainText("已经有叫 run_finished 的事件了");
  await page.keyboard.press("Escape");
  // 只有自己加的能删
  await expect(row(events, "run_finished").getByTestId("remote-plugin-event-remove")).toHaveCount(0);
  await mine.getByTestId("remote-plugin-event-remove").click();
  await expect(mine).toHaveCount(0);
  await expect(events.getByTestId("remote-plugin-event")).toHaveCount(2);
});

test("a watch that stopped after repeated failures shows the reason here and on the card, and retries", async ({ page }, testInfo) => {
  await seed(page, [onedragon({ gameRunning: true, stoppedWatches: { run_finished: "连续 5 次没查成，停下了：后端未就绪" } })]);
  const events = await openEvents(page);
  const finished = row(events, "run_finished");
  await finished.getByTestId("remote-plugin-event-switch").click();
  await page.getByTestId("remote-plugin-event-confirm-on").click();
  await expect(finished).toHaveAttribute("data-state", "stopped");
  await expect(finished.getByTestId("remote-plugin-event-status")).toHaveText("停了：连续 5 次没查成，停下了：后端未就绪");
  await screenshot(page, { path: testInfo.outputPath("remote-watch-stopped.png"), animations: "disabled", fullPage: true });
  // 插件卡片上也说
  await page.getByTestId("plugin-page-back").click();
  await expect(page.getByTestId("plugins-card-longzhu-status")).toHaveText("事件“一条龙跑完了”停了");
  await page.getByTestId("plugins-card-longzhu-open").click();
  await page.getByTestId("plugin-settings-button").click();
  await row(events, "run_finished").getByTestId("remote-plugin-event-retry").click();
  await expect(row(events, "run_finished")).toHaveAttribute("data-state", "polling");
  await page.getByTestId("plugin-page-back").click();
  await expect(page.getByTestId("plugins-card-longzhu-status")).toHaveText("连上了");
});

test("automations can pick third-party events, and the OneDragon template finds the plugin by its preset", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  await seed(page, [onedragon()]);
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  await page.getByTestId("automation-library").click();
  const automation = page.getByTestId("automation-page");
  await automation.getByTestId("automation-new").click();
  const templates = automation.getByTestId("automation-template");
  await expect(templates).toHaveCount(5);
  const template = automation.locator('[data-template="onedragon-push"]');
  await expect(template).toContainText("一条龙跑完推一行");
  await template.click();
  const editor = automation.getByTestId("automation-editor");
  // 触发照模板填好，事件名换成玩家起的插件 id
  await expect(editor.getByTestId("automation-trigger-enabled")).toBeChecked();
  await expect(editor.getByTestId("automation-trigger-event")).toHaveAttribute("data-value", "longzhu.run_finished");
  await expect(editor.getByTestId("automation-trigger-event")).toContainText("一条龙跑完了（第三方插件 longzhu）");
  // 事件还没打开：说清楚为什么不会发生、去哪打开
  await expect(editor.getByTestId("automation-trigger-off")).toContainText("longzhu插件的“一条龙跑完了”没打开");
  await screenshot(page, { path: testInfo.outputPath("remote-watch-template.png"), animations: "disabled", fullPage: true });
  await editor.getByLabel("自动化名字").fill("一条龙推一行");
  await editor.getByTestId("automation-save").click();
  const saved = automation.getByTestId("automation-item").filter({ hasText: "一条龙推一行" });
  await expect(saved).toBeVisible();

  // 选“任何游戏”时不挂在游戏上的才列；选了它挂的游戏（这里预览没有绝区零的游戏配置，就用模板带的）才有它的事件
  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="push"]').click();
  await editor.getByTestId("automation-trigger-enabled").check();
  await editor.getByTestId("automation-trigger-event").click();
  await expect(page.getByRole("option", { name: /第三方插件 longzhu/ })).toHaveCount(0);
});

test("without a plugin using the OneDragon preset the template is not offered", async ({ page }) => {
  await seed(page, [{ ...onedragon(), kotoneManifest: { ...onedragon().kotoneManifest, preset: null, source: "server" } }]);
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  await page.getByTestId("automation-library").click();
  const automation = page.getByTestId("automation-page");
  await automation.getByTestId("automation-new").click();
  await expect(automation.getByTestId("automation-template")).toHaveCount(4);
});
