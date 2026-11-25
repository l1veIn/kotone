import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 游戏事件由插件登记（插件重构第 7 步，ADR 036 D4）：自动化编辑里英雄联盟的事来自英雄联盟插件的清单（[[events]]），
// 名字用插件的文案；插件关着时不在选择里；已经保存的不删、不报废，列表和编辑里说清楚“英雄联盟插件关着，这个触发不会发生”。

async function setLol(page: Page, enabled: boolean) {
  await page.evaluate(async (on) => (await import("/src/lib/plugins.ts")).setPluginEnabled("lol", on), enabled);
}

test("LoL events come from the plugin; with the plugin off they leave the picker and saved triggers say why", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1100 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-automation").click();
  const automation = page.getByTestId("automation-page");

  // 插件开着：选英雄联盟，事件里有插件登记的“拿了小龙”，带“哪一边”
  await automation.getByTestId("automation-new").click();
  const editor = automation.getByTestId("automation-editor");
  await editor.getByLabel("自动化名字").fill("对面拿龙提醒我");
  await editor.getByRole("button", { name: "让琴音做", exact: true }).click();
  await editor.getByTestId("automation-prompt").fill("提醒我对面拿了龙");
  await editor.getByTestId("automation-trigger-enabled").check();
  await editor.getByTestId("automation-trigger-game").click();
  await page.getByRole("option", { name: "League of Legends", exact: true }).click();
  await editor.getByTestId("automation-trigger-event").click();
  await page.getByRole("option", { name: "拿了小龙", exact: true }).click();
  await editor.getByTestId("automation-trigger-team").click();
  await page.getByRole("option", { name: "对面", exact: true }).click();
  await expect(editor.getByTestId("automation-trigger-off")).toHaveCount(0);
  await editor.getByRole("button", { name: "保存", exact: true }).click();

  const saved = automation.getByTestId("automation-item").filter({ hasText: "对面拿龙提醒我" });
  await expect(saved.getByTestId("automation-item-trigger")).toHaveText("League of Legends · 拿了小龙（对面） · 每局一次");
  await expect(saved.getByTestId("automation-item-trigger-off")).toHaveCount(0);

  // 关掉英雄联盟插件：保存过的还在，说清楚不会发生
  await setLol(page, false);
  await expect(saved.getByTestId("automation-item-trigger")).toHaveText("League of Legends · 拿了小龙（对面） · 每局一次");
  await expect(saved.getByTestId("automation-item-trigger-off")).toHaveText("英雄联盟插件关着，这个触发不会发生；到「插件」里打开以后照常触发。");

  // 编辑：当前选的还显示（带原因），提示在下面；照样能保存
  await saved.getByTestId("automation-item-toggle").click();
  await saved.getByTestId("automation-edit").click();
  await expect(editor.getByTestId("automation-trigger-event")).toContainText("拿了小龙（英雄联盟插件关着）");
  await expect(editor.getByTestId("automation-trigger-off")).toHaveText("英雄联盟插件关着，这个触发不会发生；到「插件」里打开以后照常触发。");
  await editor.getByTestId("automation-trigger-event").click();
  // 别的英雄联盟的事不在选择里
  await expect(page.getByRole("option", { name: "推掉一座塔", exact: true })).toHaveCount(0);
  await expect(page.getByRole("option", { name: "阶段变化", exact: true })).toHaveCount(1);
  await page.keyboard.press("Escape");
  await screenshot(page, { path: testInfo.outputPath("plugin-events-off.png"), fullPage: true });
  await editor.getByLabel("自动化名字").fill("对面拿龙提醒我（改）");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  const renamed = automation.getByTestId("automation-item").filter({ hasText: "对面拿龙提醒我（改）" });
  await expect(renamed.getByTestId("automation-item-trigger")).toHaveText("League of Legends · 拿了小龙（对面） · 每局一次");

  // 新建：插件关着时英雄联盟只剩通用的事
  await automation.getByTestId("automation-new").click();
  await editor.getByTestId("automation-trigger-enabled").check();
  await editor.getByTestId("automation-trigger-game").click();
  await page.getByRole("option", { name: "League of Legends", exact: true }).click();
  await editor.getByTestId("automation-trigger-event").click();
  await expect(page.getByRole("option", { name: "一局结束", exact: true })).toHaveCount(1);
  await expect(page.getByRole("option", { name: "拿了小龙", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // 打开回来：又能选，保存过的不再提示
  await setLol(page, true);
  await editor.getByTestId("automation-trigger-event").click();
  await expect(page.getByRole("option", { name: "拿了小龙", exact: true })).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(renamed.getByTestId("automation-item-trigger-off")).toHaveCount(0);
});
