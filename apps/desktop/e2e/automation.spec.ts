import { openSettingsTab } from "./nav";
import { expect, test } from "@playwright/test";

// 「自动化」页（ADR 013 §8）：浏览器预览用示例数据，不运行任何程序。

test("one list: play / stop, temporary reminders, schedules, details and the full history", async ({ page, baseURL }, testInfo) => {
  const origin = new URL(baseURL!).origin;
  const external: string[] = [];
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin) external.push(request.url());
  });
  await page.setViewportSize({ width: 1040, height: 1000 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");
  await expect(automation.getByRole("heading", { name: "自动化", exact: true })).toBeVisible();
  await expect(automation.getByText(/界面预览/)).toBeVisible();
  // 没有“进行中 / 已保存”分页了
  await expect(page.getByTestId("automation-nav-open")).toHaveCount(0);
  await expect(automation.getByText("Ctrl+Shift+F12", { exact: true })).toBeVisible();

  const items = automation.getByTestId("automation-item");
  await expect(items).toHaveCount(4);
  // 内置的“X 秒之后提醒我”排第一
  await expect(items.first()).toContainText("X 秒之后提醒我");
  await expect(items.first()).toContainText("内置");
  // 运行中的自动化：■ 停止；定时的显示“每天 22:00 · 下次 …”
  const pressQ = items.filter({ hasText: "界面示例：连按某键" });
  await expect(pressQ.getByTestId("automation-item-running")).toBeVisible();
  await expect(pressQ.getByTestId("automation-item-stop")).toBeVisible();
  await expect(items.filter({ hasText: "界面示例：喝水提醒" }).getByTestId("automation-item-schedule")).toContainText(/每天 22:00 · 下次/);
  // 不属于任何自动化的提醒（临时的）排在最上面，也能停
  const loose = automation.getByTestId("automation-loose");
  await expect(loose).toHaveCount(1);
  await expect(loose).toContainText("界面示例：打野有闪了");
  await expect(loose).toContainText(/临时提醒/);
  await page.screenshot({ path: testInfo.outputPath("automation-list.png"), animations: "disabled", fullPage: true });

  await pressQ.getByTestId("automation-item-stop").click();
  await expect(pressQ.getByTestId("automation-item-play")).toBeVisible();
  await loose.getByTestId("automation-stop").click();
  await expect(loose).toHaveCount(0);

  // 详情：说明、参数、定时、最近的执行
  await pressQ.getByTestId("automation-item-toggle").click();
  const details = pressQ.getByTestId("automation-details");
  await expect(details).toContainText("按键、间隔（毫秒）、次数");
  await expect(details).toContainText("不定时");
  await expect(details.getByTestId("automation-run")).toHaveCount(1);

  // 全部记录
  await automation.getByTestId("automation-history").locator("summary").first().click();
  const history = automation.getByTestId("automation-history").getByTestId("automation-run");
  await expect(history).toHaveCount(3);
  const done = history.filter({ hasText: "界面示例：变黑就按回车" });
  await expect(done).toContainText("跑完 · 输入 1 次");
  expect(external).toEqual([]);
});

test("the built-in reminder asks what and when, then waits as a temporary reminder", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");
  const remind = automation.getByTestId("automation-item").filter({ hasText: "X 秒之后提醒我" });
  await remind.getByTestId("automation-item-play").click();
  const panel = remind.getByTestId("automation-run-panel");
  await expect(panel).toContainText("几秒之后");
  await expect(panel).toContainText("缺省：60");
  await panel.getByTestId("automation-run-start").click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "缺少参数 提醒什么" })).toBeVisible();
  await panel.getByLabel("X 秒之后提醒我 提醒什么").fill("打小龙");
  await panel.getByLabel("X 秒之后提醒我 提醒什么").blur();
  await panel.getByLabel("X 秒之后提醒我 几秒之后").fill("300");
  await panel.getByLabel("X 秒之后提醒我 几秒之后").blur();
  await panel.getByTestId("automation-run-start").click();
  const loose = automation.getByTestId("automation-loose").filter({ hasText: "打小龙" });
  await expect(loose).toBeVisible();
  await expect(loose).toContainText(/[45] 分 \d+ 秒后/);
});

test("saved automations can be created with params and a schedule, edited and deleted", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");
  await expect(automation.getByTestId("automation-item")).toHaveCount(4);

  await automation.getByTestId("automation-new").click();
  const editor = automation.getByTestId("automation-editor");
  await editor.getByLabel("自动化名字").fill("每隔一会按 E");
  await editor.getByTestId("automation-source").fill("every(args.interval, function() key(args.key) end, { times = 3 })");
  await editor.getByRole("button", { name: "添加参数", exact: true }).click();
  await editor.getByRole("button", { name: "添加参数", exact: true }).click();
  await expect(editor.getByTestId("automation-param")).toHaveCount(2);
  await editor.getByLabel("参数 1 名").fill("interval");
  await editor.getByLabel("参数 1 名").blur();
  await editor.getByLabel("参数 1 显示名").fill("间隔");
  await editor.getByLabel("参数 1 显示名").blur();
  await editor.getByLabel("参数 2 名").fill("key");
  await editor.getByLabel("参数 2 名").blur();
  await editor.getByLabel("参数 2 类型").click();
  await page.getByRole("option", { name: "按键", exact: true }).click();
  await expect(editor.getByTestId("automation-param").nth(1)).toContainText("没有缺省值 = 运行时必须填");
  // 定时：工作日 09:30
  await editor.getByTestId("automation-schedule-enabled").check();
  await editor.getByLabel("定时方式").click();
  await page.getByRole("option", { name: "工作日（周一到周五）", exact: true }).click();
  await editor.getByTestId("automation-schedule-time").fill("09:30");
  await expect(editor.getByTestId("automation-schedule")).toContainText("30 9 * * 1-5");
  await editor.getByLabel("定时 key").fill("E");
  await editor.getByLabel("定时 key").blur();
  await page.screenshot({ path: testInfo.outputPath("automation-editor.png"), fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("automation-item").filter({ hasText: "每隔一会按 E" });
  await expect(saved).toBeVisible();
  await expect(saved.getByTestId("automation-item-schedule")).toContainText(/工作日 09:30 · 下次/);
  await saved.getByTestId("automation-item-toggle").click();
  await expect(saved.getByTestId("automation-details")).toContainText("间隔、key");
  await expect(saved.getByTestId("automation-details")).toContainText("30 9 * * 1-5");
  await saved.getByTestId("automation-edit").click();
  await expect(editor.getByLabel("自动化名字")).toHaveValue("每隔一会按 E");
  await expect(editor.getByTestId("automation-schedule-time")).toHaveValue("09:30");
  await editor.getByLabel("自动化名字").fill("每隔一会按某键");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const renamed = automation.getByTestId("automation-item").filter({ hasText: "每隔一会按某键" });
  await expect(renamed).toBeVisible();
  // 编辑完回到列表，详情还开着
  await expect(renamed.getByTestId("automation-details")).toBeVisible();
  await renamed.getByTestId("automation-delete").click();
  await renamed.getByRole("button", { name: "确认删除", exact: true }).click();
  await expect(automation.getByTestId("automation-item")).toHaveCount(4);
});

test("game events: the LoL template is off, and an 'ask her' automation saves with its trigger (ADR 036)", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");

  // 内置模板：每次花一次 AI 调用，默认关（D8）——列表里不显示触发，详情里写“已暂停”
  const template = automation.getByTestId("automation-item").filter({ hasText: "英雄联盟进加载界面播报突破口" });
  await expect(template).toContainText("内置");
  await expect(template).toContainText("让琴音做");
  await expect(template.getByTestId("automation-item-trigger")).toHaveCount(0);
  await template.getByTestId("automation-item-toggle").click();
  await expect(template.getByTestId("automation-detail-trigger")).toHaveText("已暂停（League of Legends · 进入加载 · 每局一次）");
  // “让她做”不交给她当工具
  await expect(template.getByTestId("saved-automation-agent")).toBeHidden();

  await automation.getByTestId("automation-new").click();
  const editor = automation.getByTestId("automation-editor");
  await editor.getByLabel("自动化名字").fill("进对局一分钟提醒买眼");
  await editor.getByRole("button", { name: "让琴音做", exact: true }).click();
  await editor.getByTestId("automation-prompt").fill("提醒我买真眼，一句话");
  await expect(editor.getByTestId("automation-prompt-cost")).toContainText("每局最多 3 次");
  await expect(editor.getByRole("button", { name: "添加参数", exact: true })).toHaveCount(0);
  await editor.getByTestId("automation-trigger-enabled").check();
  await editor.getByTestId("automation-trigger-game").click();
  await page.getByRole("option", { name: "League of Legends", exact: true }).click();
  await editor.getByTestId("automation-trigger-phase").click();
  await page.getByRole("option", { name: "对局中", exact: true }).click();
  await editor.getByTestId("automation-trigger-hold").fill("60");
  // 英雄联盟专用的事带“哪一边”
  await editor.getByTestId("automation-trigger-event").click();
  await page.getByRole("option", { name: "三杀以上", exact: true }).click();
  await expect(editor.getByTestId("automation-trigger-team")).toBeVisible();
  await expect(editor.getByTestId("automation-trigger-phase")).toHaveCount(0);
  await editor.getByTestId("automation-trigger-event").click();
  await page.getByRole("option", { name: "阶段变化", exact: true }).click();
  await expect(editor.getByTestId("automation-trigger-team")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("automation-trigger.png"), fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("automation-item").filter({ hasText: "进对局一分钟提醒买眼" });
  await expect(saved.getByTestId("automation-item-trigger")).toHaveText("League of Legends · 进入对局中满 60 秒 · 每局一次");
  await saved.getByTestId("automation-item-toggle").click();
  await expect(saved.getByTestId("automation-details")).toContainText("让琴音做：提醒我买真眼，一句话");
  await saved.getByTestId("automation-edit").click();
  await expect(editor.getByTestId("automation-trigger-hold")).toHaveValue("60");
  await expect(editor.getByTestId("automation-prompt")).toHaveValue("提醒我买真眼，一句话");
});

test("a quick command can run a saved automation with its own args instead of asking the AI", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 快捷指令在「设置 → 怎么叫她」（ADR 028）
  await openSettingsTab(page, "call");
  const companion = page.getByTestId("companion-settings");
  await expect(companion.getByRole("heading", { name: "快捷指令", exact: true })).toBeVisible();
  await companion.getByRole("button", { name: "添加", exact: true }).click();
  await companion.getByLabel("快捷指令 1 类型").click();
  await page.getByRole("option", { name: "运行自动化", exact: true }).click();
  await expect(companion.getByLabel("快捷指令 1 自动化")).toHaveText("界面示例：连按某键");
  await expect(companion.getByLabel("快捷指令 1 内容")).toHaveCount(0);
  await companion.getByLabel("快捷指令 1 次数").fill("5");
  await companion.getByLabel("快捷指令 1 次数").blur();
  await expect.poll(async () => page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).getSettings().then((s) => s.assistant.companion.quickMessages[0])))
    .toMatchObject({ automationId: "preview-auto-1", args: { times: 5 } });
  // 换回交给 AI：清掉自动化与入参，补回一句话
  await companion.getByLabel("快捷指令 1 类型").click();
  await page.getByRole("option", { name: "交给 AI 的一句话", exact: true }).click();
  await expect(companion.getByLabel("快捷指令 1 内容")).toHaveValue("帮我看看队友说了什么");
  await expect.poll(async () => page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).getSettings().then((s) => s.assistant.companion.quickMessages[0].automationId)))
    .toBe("");
});
