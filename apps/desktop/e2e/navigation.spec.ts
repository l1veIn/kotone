import { expect, test } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

// ADR 028：麦克风和“录音时降低扬声器音量”是语音输入和呼叫共用的，放在「设置 → 声音」。
test("speaker volume reduction is off by default and saves from the shared sound settings", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "sound");
  const lowerVolume = page.getByRole("switch", { name: "录音时降低扬声器音量" });
  await expect(lowerVolume).toBeVisible();
  await expect(lowerVolume).not.toBeChecked();
  await lowerVolume.click();
  await expect(lowerVolume).toBeChecked();
  await openVoiceInput(page, "history");
  await openSettingsTab(page, "sound");
  await expect(lowerVolume).toBeChecked();
  await lowerVolume.click();
  await expect(lowerVolume).not.toBeChecked();
  // 语音输入页只留一行共享设置，指向这里
  await openVoiceInput(page);
  await expect(page.getByTestId("voice-shared")).toContainText("设置 → 声音");
});

// ADR 028：打开就是聊天页；侧栏上面是新对话和对话列表，底部固定「游戏陪玩」分组和一项「语音输入」；
// 怎么叫她、她怎么回答、权限、用量搬进「设置」的陪玩组；所有设置页内容宽度一致。
test("chat is home; sidebar groups; companion settings live in 设置; one content width", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const sidebar = page.getByTestId("app-sidebar");
  await expect(sidebar.getByTestId("sidebar-group-0")).toHaveText("游戏陪玩");
  await expect(sidebar.getByTestId("sidebar-group-1")).toHaveText("语音输入");
  await expect(sidebar.getByTestId("settings-nav-assistant")).toHaveText("琴音");
  await expect(sidebar.getByTestId("settings-nav-game")).toHaveText("游戏列表");
  await expect(sidebar.getByTestId("settings-nav-automation")).toHaveText("自动化");
  await expect(sidebar.getByTestId("settings-nav-general")).toHaveText("语音输入");
  await expect(sidebar.getByTestId("settings-nav-processing")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-history")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-talk")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-advanced")).toHaveText("设置");
  // 对话列表在分组上面
  const listBox = await sidebar.getByTestId("sidebar-conversations").boundingBox();
  const groupBox = await sidebar.getByTestId("sidebar-group-0").boundingBox();
  expect(listBox!.y).toBeLessThan(groupBox!.y);
  await page.screenshot({ path: testInfo.outputPath("home.png") });

  // 角色页只留 角色 · 记忆 · 技能
  await sidebar.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-nav-identity")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-nav-settings")).toHaveCount(0);
  await expect(page.getByTestId("assistant-nav-chat")).toHaveCount(0);

  // 原「琴音 → 设置」在「设置」的陪玩组
  await page.getByTestId("assistant-open-settings").click();
  await expect(page.getByTestId("advanced-group-陪玩")).toBeVisible();
  await expect(page.getByTestId("advanced-group-通用")).toBeVisible();
  for (const [tab, texts] of [
    ["call", ["呼叫琴音", "呼叫键", "快捷指令", "语音唤醒"]],
    ["reply", ["AI 连接", "对话气泡", "声音与弹幕", "替你跟队友说话", "游戏语音"]],
    ["permissions", ["Agent"]],
    ["usage", ["用量与费用"]],
  ] as const) {
    await page.getByTestId(`advanced-nav-${tab}`).click();
    for (const text of texts) await expect(page.getByTestId(`settings-${tab}`), `${tab} · ${text}`).toContainText(text);
  }
  await page.getByTestId("advanced-nav-call").click();
  await page.screenshot({ path: testInfo.outputPath("settings-call.png"), fullPage: true });

  // 所有设置页内容列一样宽（包括模型、连接这些以前更宽的页）
  const widths: number[] = [];
  for (const nav of ["general", "game", "automation", "about"]) {
    await page.getByTestId(`settings-nav-${nav}`).click();
    widths.push(await page.locator("main .ui-scope").first().evaluate((el) => el.getBoundingClientRect().width));
  }
  for (const tab of ["processing", "history"] as const) {
    await openVoiceInput(page, tab);
    widths.push(await page.locator("main .ui-scope").last().evaluate((el) => el.getBoundingClientRect().width));
  }
  for (const tab of ["system", "models", "connections", "appearance", "call", "reply"]) {
    await openSettingsTab(page, tab);
    widths.push(await page.locator("main .ui-scope").first().evaluate((el) => el.getBoundingClientRect().width));
    if (tab === "models" || tab === "connections") await page.screenshot({ path: testInfo.outputPath(`settings-${tab}.png`) });
  }
  await page.screenshot({ path: testInfo.outputPath("settings-appearance.png") });
  expect(new Set(widths.map((w) => Math.round(w))).size).toBe(1);
});
