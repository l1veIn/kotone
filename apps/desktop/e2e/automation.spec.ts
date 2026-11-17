import { expect, test } from "@playwright/test";

// 「自动化」页（ADR 013 §8）：浏览器预览用示例数据，不运行任何程序。

test("in-progress lists reminders and automations together; cancel, stop-all and history work in preview", async ({ page }, testInfo) => {
  const external: string[] = [];
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && !request.url().startsWith("http://127.0.0.1:17420")) external.push(request.url());
  });
  await page.setViewportSize({ width: 1040, height: 1000 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");
  await expect(automation.getByRole("heading", { name: "自动化", exact: true })).toBeVisible();
  await expect(automation.getByText(/界面预览/)).toBeVisible();
  await expect(page.getByTestId("automation-nav-open")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("automation-nav-open")).toHaveText("进行中 2");
  const open = automation.getByTestId("automation-run");
  await expect(open).toHaveCount(2);
  // 按到期时间：运行中的自动化（已开始）在前，等待中的提醒在后
  await expect(open.nth(0)).toContainText("界面示例：每秒按 Q");
  await expect(open.nth(0)).toContainText("示例游戏 (game.exe)");
  await expect(open.nth(0)).toContainText(/已运行/);
  await expect(open.nth(1)).toContainText("界面示例：打野有闪了");
  await expect(open.nth(1)).toContainText(/分 .*秒后/);
  await expect(automation.getByText("Ctrl+Shift+F12", { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("automation-open.png"), animations: "disabled" });

  // 停止所有自动化：只停程序，提醒还在
  await automation.getByRole("button", { name: "停止所有自动化", exact: true }).click();
  await expect(open).toHaveCount(1);
  await expect(open.first()).toContainText("界面示例：打野有闪了");
  // 单独取消提醒
  await open.first().getByRole("button", { name: "取消", exact: true }).click();
  await expect(automation.getByText("没有进行中的提醒或自动化")).toBeVisible();

  await page.getByTestId("automation-nav-history").click();
  const history = automation.getByTestId("automation-run");
  await expect(history).toHaveCount(3);
  await expect(automation.getByTestId("automation-run").filter({ hasText: "界面示例：打野有闪了" })).toContainText("已取消");
  const done = automation.getByTestId("automation-run").filter({ hasText: "界面示例：变黑就按回车" });
  await expect(done).toContainText("跑完 · 输入 1 次");
  await done.getByText("日志（1）").click();
  await expect(done).toContainText("左上角变黑了");
  expect(external).toEqual([]);
});

test("saved automations can be created with params, run panel validates in preview, edited and deleted", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  await page.getByTestId("automation-nav-saved").click();
  const automation = page.getByTestId("automation-page");
  await expect(automation.getByTestId("saved-automation")).toHaveCount(2);

  await automation.getByRole("button", { name: "新建", exact: true }).click();
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
  // 换成按键后缺省值清空 = 必填
  await expect(editor.getByTestId("automation-param").nth(1)).toContainText("没有缺省值 = 运行时必须填");
  await page.screenshot({ path: testInfo.outputPath("automation-editor.png"), fullPage: true });
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("saved-automation").filter({ hasText: "每隔一会按 E" });
  await expect(saved).toBeVisible();
  await expect(saved).toContainText("参数：间隔、key");
  await expect(saved).toContainText("你创建");
  await saved.getByRole("button", { name: "运行", exact: true }).click();
  const panel = saved.getByTestId("automation-run-panel");
  await expect(panel).toContainText("缺省：1");
  await panel.getByLabel("每隔一会按 E key").fill("E");
  await panel.getByLabel("每隔一会按 E key").blur();
  await expect(panel.getByLabel("倒数秒数")).toHaveValue("3");
  await panel.getByRole("button", { name: "开始", exact: true }).click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "浏览器仅预览界面" })).toBeVisible();

  await saved.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(editor.getByLabel("自动化名字")).toHaveValue("每隔一会按 E");
  await editor.getByLabel("自动化名字").fill("每隔一会按某键");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const renamed = automation.getByTestId("saved-automation").filter({ hasText: "每隔一会按某键" });
  await expect(renamed).toBeVisible();
  await renamed.getByRole("button", { name: "删除", exact: true }).click();
  await renamed.getByRole("button", { name: "确认删除", exact: true }).click();
  await expect(automation.getByTestId("saved-automation")).toHaveCount(2);
});

test("a quick command can run a saved automation with its own args instead of asking the AI", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-talk").click();
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
