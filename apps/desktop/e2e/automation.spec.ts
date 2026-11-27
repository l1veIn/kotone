import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 「自动化」页（ADR 013 §8、ADR 045）：浏览器预览用示例数据，不运行任何程序。动作只有程序一种：
// 新建先选模板（说一句 / 推送一条 / 让她做 / 空白程序）；▶ 只对用到游戏窗口的先倒数。

async function openAutomation(page: Page, library = true) {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  if (library) await page.getByTestId("automation-library").click();
  return page.getByTestId("automation-page");
}

test("activity home and library share live runs, reminders and history", async ({ page, baseURL }, testInfo) => {
  const origin = new URL(baseURL!).origin;
  const external: string[] = [];
  page.on("request", (request) => {
    // 本机回环上的（插件 MCP App 的沙盒代理页，预览里在 127.0.0.1 的另一个端口上）不算往外发
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin && !/^(127\.0\.0\.1|localhost)$/.test(new URL(request.url()).hostname)) external.push(request.url());
  });
  await page.setViewportSize({ width: 1040, height: 1000 });
  const automation = await openAutomation(page, false);
  await expect(automation.getByRole("heading", { name: "自动化", exact: true })).toBeVisible();
  await expect(automation.getByText(/界面预览/)).toBeVisible();
  // 没有“进行中 / 已保存”分页了
  await expect(page.getByTestId("automation-nav-open")).toHaveCount(0);
  await expect(automation.getByText("Ctrl+Shift+F12", { exact: true })).toBeVisible();

  await expect(automation.getByTestId("automation-active")).toContainText("每秒按 Q");
  await expect(automation.getByTestId("automation-pending")).toContainText("打野有闪了");
  await expect(automation.getByTestId("automation-history")).toContainText("变黑就按回车");
  await screenshot(page, { path: testInfo.outputPath("activity-live.png"), animations: "disabled" });
  await automation.getByTestId("automation-library").click();
  const items = automation.getByTestId("automation-item");
  await expect(items).toHaveCount(4);
  // 内置的“X 秒之后提醒我”排第一
  await expect(items.first()).toContainText("X 秒之后提醒我");
  await expect(items.first()).toContainText("内置");
  // 动作不再分种类：没有“播报”“程序”的标签；用到游戏窗口的写出来，会叫她的带个小标记
  await expect(automation.getByText("播报", { exact: true })).toHaveCount(0);
  const pressQ = items.filter({ hasText: "界面示例：连按某键" });
  await expect(pressQ.getByTestId("automation-item-window")).toHaveText("操作游戏窗口");
  await expect(items.first().getByTestId("automation-item-window")).toHaveCount(0);
  await expect(items.filter({ hasText: "英雄联盟进加载界面播报突破口" }).getByTestId("automation-item-asks")).toHaveText("会叫琴音");
  await expect(pressQ.getByTestId("automation-item-asks")).toHaveCount(0);
  // 运行中的自动化：■ 停止；定时的显示“每天 22:00 · 下次 …”
  await expect(pressQ.getByTestId("automation-item-running")).toBeVisible();
  await expect(pressQ.getByTestId("automation-item-stop")).toBeVisible();
  await expect(items.filter({ hasText: "界面示例：喝水提醒" }).getByTestId("automation-item-schedule")).toContainText(/每天 22:00 · 下次/);
  // 不属于任何自动化的提醒（她设的，临时的）排在最上面，也能停
  const loose = automation.getByTestId("automation-loose");
  await expect(loose).toHaveCount(1);
  await expect(loose).toContainText("界面示例：打野有闪了");
  await expect(loose).toContainText(/临时提醒/);
  await screenshot(page, { path: testInfo.outputPath("automation-list.png"), animations: "disabled", fullPage: true });

  // 停下：运行中的停掉，等待中的提醒留着（ADR 045 D6）
  await automation.getByTestId("automation-stop-all").click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "已停止 1 个自动化" })).toBeVisible();
  await expect(pressQ.getByTestId("automation-item-play")).toBeVisible();
  await expect(loose).toHaveCount(1);
  await expect(automation.getByTestId("automation-stop-all")).toBeDisabled();
  await loose.getByTestId("automation-stop").click();
  await expect(loose).toHaveCount(0);

  // 详情：做什么（用不用游戏窗口）、参数、定时、最近的执行
  await pressQ.getByTestId("automation-item-toggle").click();
  const details = pressQ.getByTestId("automation-details");
  await expect(details.getByTestId("automation-detail-does")).toHaveText("运行一段程序 · 会操作游戏窗口（▶ 先倒数，让你切到游戏）");
  await expect(details).toContainText("按键、间隔（毫秒）、次数");
  await expect(details).toContainText("不定时");
  await expect(details.getByTestId("automation-run")).toHaveCount(1);
  // 用到游戏窗口的 ▶ 先填参数、倒数几秒（切到游戏）
  await pressQ.getByTestId("automation-item-play").click();
  await expect(pressQ.getByTestId("automation-run-panel").getByLabel("倒数秒数")).toBeVisible();
  await pressQ.getByTestId("automation-run-panel").getByRole("button", { name: "取消", exact: true }).click();

  // 全部记录：提醒写回答方式，没有种类标签
  await automation.getByTestId("automation-library").click();
  await expect(automation.getByTestId("automation-active")).toHaveCount(0);
  await expect(automation.getByTestId("automation-pending")).toHaveCount(0);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0, { timeout: 10000 });
  await screenshot(page, { path: testInfo.outputPath("activity-idle.png"), animations: "disabled" });
  const history = automation.getByTestId("automation-history").getByTestId("automation-run");
  await expect(history).toHaveCount(3);
  const done = history.filter({ hasText: "界面示例：变黑就按回车" });
  await done.getByText("执行详情").click();
  await expect(done.getByText(/跑完 · 输入 1 次/)).toBeVisible();
  const reminder = history.filter({ hasText: "界面示例：打野有闪了" });
  await expect(reminder).toContainText("语音 + 弹幕");
  await expect(reminder).toContainText("已取消");
  expect(external).toEqual([]);
});

test("the built-in reminder asks what and when, then waits under its own item", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 900 });
  const automation = await openAutomation(page);
  const remind = automation.getByTestId("automation-item").filter({ hasText: "X 秒之后提醒我" });
  await remind.getByTestId("automation-item-play").click();
  const panel = remind.getByTestId("automation-run-panel");
  await expect(panel).toContainText("几秒之后");
  await expect(panel).toContainText("缺省：60");
  // 不用游戏窗口：不倒数
  await expect(panel.getByLabel("倒数秒数")).toHaveCount(0);
  await panel.getByTestId("automation-run-start").click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "缺少参数 提醒什么" })).toBeVisible();
  await panel.getByLabel("X 秒之后提醒我 提醒什么").fill("打小龙");
  await panel.getByLabel("X 秒之后提醒我 提醒什么").blur();
  await panel.getByLabel("X 秒之后提醒我 几秒之后").fill("300");
  await panel.getByLabel("X 秒之后提醒我 几秒之后").blur();
  await panel.getByTestId("automation-run-start").click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "已安排" })).toBeVisible();
  // 排上的提醒挂在内置提醒自己那一项下面，不再出现在“临时提醒”里；▶ 还能再排一条
  const waiting = remind.getByTestId("automation-item-pending");
  await expect(waiting).toHaveCount(1);
  await expect(waiting).toContainText("打小龙");
  await expect(waiting).toContainText(/[45] 分 \d+ 秒后/);
  await expect(automation.getByTestId("automation-loose").filter({ hasText: "打小龙" })).toHaveCount(0);
  await expect(remind.getByTestId("automation-item-play")).toBeVisible();
  await screenshot(page, { path: testInfo.outputPath("automation-reminder-pending.png"), animations: "disabled" });
  // 单条取消；记录也在它自己那一项下
  await waiting.getByTestId("automation-item-pending-cancel").click();
  await expect(waiting).toHaveCount(0);
  await remind.getByTestId("automation-item-toggle").click();
  const runs = remind.getByTestId("automation-details").getByTestId("automation-run");
  await expect(runs).toHaveCount(1);
  await expect(runs).toContainText("打小龙");
  await expect(runs).toContainText("语音 + 弹幕");
  await expect(runs).toContainText("已取消");
});

test("new automations start from a template: say, push, ask her, blank program", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  const automation = await openAutomation(page);
  await automation.getByTestId("automation-new").click();
  const templates = automation.getByTestId("automation-template");
  await expect(templates).toHaveCount(4);
  await expect(templates.nth(0)).toContainText("说一句");
  await expect(templates.nth(1)).toContainText("推送一条");
  await expect(templates.nth(2)).toContainText("让琴音做");
  await expect(templates.nth(2)).toContainText("每局最多 3 次");
  await expect(templates.nth(3)).toContainText("空白程序");
  // 按键类不另做模板：复制内置的按键程序
  await expect(automation.getByTestId("automation-template-keys")).toContainText("复制");
  await screenshot(page, { path: testInfo.outputPath("automation-templates.png"), animations: "disabled", fullPage: true });

  // 说一句：只填参数，程序折叠着
  await automation.locator('[data-template="say"]').click();
  const editor = automation.getByTestId("automation-editor");
  await expect(editor.getByTestId("automation-template-note")).toContainText("说一句");
  await expect(editor.getByTestId("automation-source")).toHaveCount(0);
  await expect(editor.getByTestId("automation-source-preview")).toHaveText("say(args.text)");
  await expect(editor.getByLabel("说什么", { exact: true })).toHaveValue("该看小地图了");
  await editor.getByLabel("自动化名字").fill("看小地图");
  await editor.getByLabel("说什么", { exact: true }).fill("该看一眼小地图了");
  await editor.getByLabel("说什么", { exact: true }).blur();
  await screenshot(page, { path: testInfo.outputPath("automation-editor-say.png"), animations: "disabled", fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const say = automation.getByTestId("automation-item").filter({ hasText: "看小地图" });
  await expect(say).toBeVisible();
  await expect(say.getByTestId("automation-item-window")).toHaveCount(0);
  // 不用游戏窗口：▶ 不倒数，直接运行
  await say.getByTestId("automation-item-play").click();
  await expect(say.getByTestId("automation-run-panel")).toContainText("缺省：该看一眼小地图了");
  await expect(say.getByTestId("automation-run-panel").getByLabel("倒数秒数")).toHaveCount(0);
  await say.getByTestId("automation-run-start").click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "已开始" })).toBeVisible();
  await say.getByTestId("automation-item-toggle").click();
  await expect(say.getByTestId("automation-detail-does")).toHaveText("运行一段程序 · 不操作游戏窗口（▶ 马上开始）");
  await expect(say.getByTestId("automation-run")).toHaveCount(1);
  // 编辑：程序折叠，展开能看到（也能收起）
  await say.getByTestId("automation-edit").click();
  await expect(editor.getByLabel("说什么", { exact: true })).toHaveValue("该看一眼小地图了");
  await editor.getByTestId("automation-source-toggle").click();
  await expect(editor.getByTestId("automation-source")).toHaveValue("say(args.text)");
  await expect(editor.getByTestId("automation-param")).toHaveCount(1);
  await editor.getByRole("button", { name: "取消", exact: true }).click();

  // 推送一条
  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="push"]').click();
  await expect(editor.getByTestId("automation-source-preview")).toHaveText("push(args.text)");
  await expect(editor.getByLabel("推送什么", { exact: true })).toHaveValue("记得买真眼");
  await editor.getByLabel("自动化名字").fill("买眼");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  await expect(automation.getByTestId("automation-item").filter({ hasText: "买眼" })).toBeVisible();

  // 让她做：会叫她，写清楚花 AI 调用；不交给她当工具
  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="ask"]').click();
  await expect(editor.getByTestId("automation-ask-cost")).toContainText("每局最多 3 次");
  await expect(editor.getByTestId("automation-source-preview")).toHaveText("ask(args.request)");
  await editor.getByLabel("自动化名字").fill("问问局势");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const ask = automation.getByTestId("automation-item").filter({ hasText: "问问局势" });
  await expect(ask.getByTestId("automation-item-asks")).toHaveText("会叫琴音");
  await ask.getByTestId("automation-item-toggle").click();
  await expect(ask.getByTestId("automation-detail-does")).toContainText("会叫琴音：每运行一次花一次 AI 调用");
  await expect(ask.getByTestId("saved-automation-agent")).toBeHidden();

  // 空白程序：直接写源码
  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="blank"]').click();
  await expect(editor.getByTestId("automation-source")).toHaveValue(/-- 在这里写程序/);
  await expect(editor.getByTestId("automation-param")).toHaveCount(0);
  await editor.getByLabel("自动化名字").fill("喝水");
  await editor.getByTestId("automation-source").fill('toast("该喝水了")');
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const blank = automation.getByTestId("automation-item").filter({ hasText: "喝水" }).filter({ hasNotText: "界面示例" });
  await expect(blank).toBeVisible();
  await blank.getByTestId("automation-item-toggle").click();
  await blank.getByTestId("automation-edit").click();
  // 没有参数的直接展开源码
  await expect(editor.getByTestId("automation-source")).toHaveValue('toast("该喝水了")');
  await editor.getByRole("button", { name: "取消", exact: true }).click();
  await expect(automation.getByTestId("automation-item")).toHaveCount(8);
});

test("a built-in key program can be copied and changed into your own", async ({ page }) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  const automation = await openAutomation(page);
  const pressQ = automation.getByTestId("automation-item").filter({ hasText: "界面示例：连按某键" });
  await pressQ.getByTestId("automation-item-toggle").click();
  await pressQ.getByTestId("automation-copy").click();
  const editor = automation.getByTestId("automation-editor");
  await expect(editor.getByRole("heading", { name: "新建自动化" })).toBeVisible();
  await expect(automation.getByTestId("automation-template")).toHaveCount(0);
  await expect(editor.getByLabel("自动化名字")).toHaveValue("界面示例：连按某键（副本）");
  await expect(editor.getByTestId("automation-source-preview")).toHaveText(/^every\(args\.interval/);
  await editor.getByLabel("按键", { exact: true }).fill("E");
  await editor.getByLabel("按键", { exact: true }).blur();
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const copy = automation.getByTestId("automation-item").filter({ hasText: "（副本）" });
  await expect(copy.getByTestId("automation-item-window")).toHaveText("操作游戏窗口");
  await expect(automation.getByTestId("automation-item")).toHaveCount(5);
});

test("saved automations can be created with params and a schedule, edited and deleted", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  const automation = await openAutomation(page);
  await expect(automation.getByTestId("automation-item")).toHaveCount(4);

  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="blank"]').click();
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
  await screenshot(page, { path: testInfo.outputPath("automation-editor.png"), fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("automation-item").filter({ hasText: "每隔一会按 E" });
  await expect(saved).toBeVisible();
  await expect(saved.getByTestId("automation-item-schedule")).toContainText(/工作日 09:30 · 下次/);
  // 程序里有 key()：用到游戏窗口
  await expect(saved.getByTestId("automation-item-window")).toBeVisible();
  await saved.getByTestId("automation-item-toggle").click();
  await expect(saved.getByTestId("automation-details")).toContainText("间隔、key");
  await expect(saved.getByTestId("automation-details")).toContainText("30 9 * * 1-5");
  await saved.getByTestId("automation-edit").click();
  await expect(editor.getByLabel("自动化名字")).toHaveValue("每隔一会按 E");
  await expect(editor.getByTestId("automation-schedule-time")).toHaveValue("09:30");
  // 有参数：程序折叠，参数在上面直接填（没有缺省值的标 *）
  await expect(editor.getByTestId("automation-fill")).toContainText("间隔");
  await expect(editor.getByTestId("automation-fill")).toContainText(/key\s*\*/);
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
  const automation = await openAutomation(page);

  // 内置模板：每次花一次 AI 调用，默认关（D8）——列表里不显示触发，详情里写“已暂停”
  const template = automation.getByTestId("automation-item").filter({ hasText: "英雄联盟进加载界面播报突破口" });
  await expect(template).toContainText("内置");
  await expect(template.getByTestId("automation-item-asks")).toHaveText("会叫琴音");
  await expect(template.getByTestId("automation-item-trigger")).toHaveCount(0);
  await template.getByTestId("automation-item-toggle").click();
  await expect(template.getByTestId("automation-detail-trigger")).toHaveText("已暂停（League of Legends · 进入加载 · 每局一次）");
  // 会叫她的不交给她当工具
  await expect(template.getByTestId("saved-automation-agent")).toBeHidden();

  await automation.getByTestId("automation-new").click();
  await automation.locator('[data-template="ask"]').click();
  const editor = automation.getByTestId("automation-editor");
  await editor.getByLabel("自动化名字").fill("进对局一分钟提醒买眼");
  await editor.getByLabel("让她做什么", { exact: true }).fill("提醒我买真眼，一句话");
  await editor.getByLabel("让她做什么", { exact: true }).blur();
  await expect(editor.getByTestId("automation-ask-cost")).toContainText("每局最多 3 次");
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
  await screenshot(page, { path: testInfo.outputPath("automation-trigger.png"), fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("automation-item").filter({ hasText: "进对局一分钟提醒买眼" });
  await expect(saved.getByTestId("automation-item-trigger")).toHaveText("League of Legends · 进入对局中满 60 秒 · 每局一次");
  await saved.getByTestId("automation-item-toggle").click();
  await expect(saved.getByTestId("automation-detail-does")).toContainText("会叫琴音");
  await saved.getByTestId("automation-edit").click();
  await expect(editor.getByTestId("automation-trigger-hold")).toHaveValue("60");
  await expect(editor.getByLabel("让她做什么", { exact: true })).toHaveValue("提醒我买真眼，一句话");
});

test("activity controls and the schedule shortcut work in a narrow window", async ({ page }) => {
  await page.setViewportSize({ width: 460, height: 740 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByRole("button", { name: /进行中的自动化/ }).click();
  const home = page.getByTestId("automation-page");
  const active = home.getByTestId("automation-active");
  const pending = home.getByTestId("automation-pending");
  await active.getByRole("button", { name: /停止/ }).click();
  await expect(active).toHaveCount(0);
  await pending.getByRole("button", { name: /取消/ }).click();
  await expect(pending).toHaveCount(0);
  await home.getByRole("button", { name: /喝水提醒/ }).click();
  const item = home.getByTestId("automation-item").filter({ hasText: "界面示例：喝水提醒" });
  await expect(item.getByTestId("automation-item-toggle")).toBeFocused();
  await expect(item.getByTestId("automation-details")).toBeVisible();
});
