import { expect, test } from "@playwright/test";

// ADR 023：侧栏分“语音输入”“游戏陪玩”两组；语音输入仍是首页，顶部引导去用游戏陪玩；
// 「交流」集中了怎么叫琴音、她怎么回答、怎么替你跟队友说话；所有设置页内容宽度一致。
test("sidebar groups, homepage companion entry, talk page and one content width", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  const sidebar = page.getByTestId("app-sidebar");
  await expect(sidebar.getByTestId("sidebar-group-0")).toHaveText("语音输入");
  await expect(sidebar.getByTestId("sidebar-group-1")).toHaveText("游戏陪玩");
  await expect(sidebar.getByTestId("settings-nav-general")).toHaveText("语音输入");
  await expect(sidebar.getByTestId("settings-nav-assistant")).toHaveText("琴音");
  await expect(sidebar.getByTestId("settings-nav-talk")).toHaveText("交流");
  await expect(sidebar.getByTestId("settings-nav-game")).toHaveText("游戏");
  await expect(sidebar.getByTestId("settings-nav-advanced")).toHaveText("设置");

  // 首页仍是语音输入，有显眼的游戏陪玩入口
  await expect(page.getByTestId("general-page")).toBeVisible();
  await expect(page.getByTestId("companion-cta")).toContainText("试试游戏陪玩");
  await expect(page.getByTestId("general-page")).not.toContainText("声音与弹幕");
  await page.screenshot({ path: testInfo.outputPath("home.png") });

  await page.getByTestId("companion-cta-talk").click();
  const talk = page.getByTestId("talk-page");
  await expect(talk).toBeVisible();
  for (const text of ["你 → 琴音", "琴音 → 你", "琴音 → 队友", "陪玩热键", "语音唤醒", "对话气泡", "声音与弹幕", "游戏语音"]) {
    await expect(talk).toContainText(text);
  }
  await page.screenshot({ path: testInfo.outputPath("talk.png"), fullPage: true });

  await page.getByTestId("settings-nav-general").click();
  await page.getByTestId("companion-cta-open").click();
  await expect(page.getByTestId("assistant-nav-chat")).toHaveAttribute("aria-current", "page");

  // 所有设置页内容列一样宽（包括模型、连接这些以前更宽的页）
  const widths: number[] = [];
  for (const nav of ["general", "processing", "history", "talk", "game", "automation", "about"]) {
    await page.getByTestId(`settings-nav-${nav}`).click();
    widths.push(await page.locator("main .ui-scope").first().evaluate((el) => el.getBoundingClientRect().width));
  }
  await page.getByTestId("settings-nav-advanced").click();
  for (const tab of ["system", "models", "connections", "appearance"]) {
    await page.getByTestId(`advanced-nav-${tab}`).click();
    await expect(page.getByTestId(`advanced-nav-${tab}`)).toHaveAttribute("aria-current", "page");
    widths.push(await page.locator("main .ui-scope").first().evaluate((el) => el.getBoundingClientRect().width));
    if (tab === "models" || tab === "connections") await page.screenshot({ path: testInfo.outputPath(`settings-${tab}.png`) });
  }
  await page.screenshot({ path: testInfo.outputPath("settings-appearance.png") });
  expect(new Set(widths.map((w) => Math.round(w))).size).toBe(1);
});
