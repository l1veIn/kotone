import { expect, test } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { openChat, openSettingsTab, openVoiceInput } from "./nav";
import { screenshot } from "./screenshot.cjs";

/** 设置重排的截图（交接第十一节第 8 条）放在仓库的 target/plugin-shots/round7/。 */
const round7 = (name: string) => fileURLToPath(new URL(`../../../target/plugin-shots/round7/${name}.png`, import.meta.url));

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
// 唤醒、交互、权限、用量在角色页（2026-11-25 从「设置」的陪玩组搬来，交接第十一节第 8 条）；「设置」只剩一排；所有设置页内容宽度一致。
test("chat is home; sidebar groups; companion settings live on her page; one content width", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  const sidebar = page.getByTestId("app-sidebar");
  await expect(sidebar.getByTestId("sidebar-group-0")).toHaveText("游戏陪玩");
  // 语音输入搬进了插件（ADR 038）：左侧栏没有单独一项、也没有“语音输入”一组，从「插件」进
  await expect(sidebar.getByTestId("sidebar-group-1")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-assistant")).toHaveText("琴音");
  await expect(sidebar.getByTestId("settings-nav-game")).toHaveText("游戏列表");
  await expect(sidebar.getByTestId("settings-nav-automation")).toHaveText("自动化");
  await expect(sidebar.getByTestId("settings-nav-general")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-plugins")).toHaveText("插件");
  await expect(sidebar.getByTestId("settings-nav-processing")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-history")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-talk")).toHaveCount(0);
  await expect(sidebar.getByTestId("settings-nav-advanced")).toHaveText("设置");
  // 对话列表在分组上面
  const listBox = await sidebar.getByTestId("sidebar-conversations").boundingBox();
  const groupBox = await sidebar.getByTestId("sidebar-group-0").boundingBox();
  expect(listBox!.y).toBeLessThan(groupBox!.y);
  await screenshot(page, { path: testInfo.outputPath("home.png") });

  // 角色页的分页：角色 · 记忆 · 技能 · 唤醒 · 交互 · 权限 · 用量（玩家原话：“现在这个页面有角色、记忆、技能，再加上唤醒、交互、权限、用量”）
  await sidebar.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-nav-identity")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-nav").getByRole("button")).toHaveText(["角色", "记忆", "技能", "唤醒", "交互", "权限", "用量"]);
  await expect(page.getByTestId("assistant-nav-settings")).toHaveCount(0);
  await expect(page.getByTestId("assistant-nav-chat")).toHaveCount(0);
  await expect(page.getByTestId("assistant-open-settings")).toHaveCount(0);
  for (const [tab, texts] of [
    ["call", ["呼叫琴音", "呼叫键", "快捷指令", "语音唤醒"]],
    ["reply", ["气泡", "声音与弹幕", "回答方式", "她用的模型在聊天页输入框旁边选"]],
    ["permissions", ["她怎么做事"]],
    ["usage", ["用量与费用"]],
  ] as const) {
    await page.getByTestId(`assistant-nav-${tab}`).click();
    for (const text of texts) await expect(page.getByTestId(`settings-${tab}`), `${tab} · ${text}`).toContainText(text);
  }
  // 选模型不在「交互」里了（在聊天页输入框旁）
  await page.getByTestId("assistant-nav-reply").click();
  await expect(page.getByTestId("settings-reply-model-moved")).toBeVisible();
  await expect(page.getByTestId("settings-reply").getByTestId("assistant-model")).toHaveCount(0);
  await expect(page.getByTestId("settings-reply").getByLabel("她用的 API 连接")).toHaveCount(0);
  for (const tab of ["identity", "memory", "skills", "call", "reply", "permissions", "usage"] as const) {
    await page.getByTestId(`assistant-nav-${tab}`).click();
    await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
    await screenshot(page, { path: round7(`role-${tab}`), animations: "disabled" });
  }

  // 「设置」只剩一排（不写组名），说明下面一行指向她的页面
  await sidebar.getByTestId("settings-nav-advanced").click();
  await expect(page.getByTestId("advanced-nav").getByRole("button")).toHaveText(["模型", "API 连接", "声音", "热键", "调参", "外观", "系统", "浏览器"]);
  await expect(page.getByTestId("advanced-group-陪玩")).toHaveCount(0);
  await expect(page.getByTestId("advanced-group-通用")).toHaveCount(0);
  await expect(page.getByTestId("advanced-companion-moved")).toContainText("唤醒、交互、权限和用量在琴音自己的页面里");
  await screenshot(page, { path: round7("settings"), animations: "disabled" });
  await page.getByTestId("advanced-open-companion").click();
  await expect(page.getByTestId("assistant-nav-call")).toHaveAttribute("aria-current", "page");

  // 所有设置页内容列一样宽（包括模型、连接这些以前更宽的页）
  const widths: number[] = [];
  for (const nav of ["game", "automation", "about"]) {
    await page.getByTestId(`settings-nav-${nav}`).click();
    widths.push(await page.locator("main .ui-scope").first().evaluate((el) => el.getBoundingClientRect().width));
  }
  for (const tab of ["general", "processing", "history"] as const) {
    await openVoiceInput(page, tab);
    widths.push(await page.locator("main .ui-scope").last().evaluate((el) => el.getBoundingClientRect().width));
  }
  for (const tab of ["system", "models", "connections", "appearance", "call", "reply"]) {
    await openSettingsTab(page, tab);
    // 唤醒、交互在角色页：量那一页的内容列（聊天藏着也挂在页面里）
    const column = tab === "call" || tab === "reply" ? page.getByTestId(`assistant-${tab}-tab`) : page.locator("main .ui-scope").first();
    widths.push(await column.evaluate((el) => el.getBoundingClientRect().width));
    if (tab === "models" || tab === "connections") await screenshot(page, { path: testInfo.outputPath(`settings-${tab}.png`) });
  }
  await screenshot(page, { path: testInfo.outputPath("settings-appearance.png") });
  expect(new Set(widths.map((w) => Math.round(w))).size).toBe(1);
});

// 设置重排（交接第十一节第 8 条）：旧地址都还认，落到新位置——她的 ui.open（ui://kotone/settings/call 等）、
// 旧对话里的“去看看”（「设置 → 怎么叫她 → 呼叫键」）、新写法（「琴音 → 唤醒 → 呼叫键」）。
test("old settings addresses for wake-up, interaction, permissions and usage land on her page", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  for (const [section, tab] of [["call", "call"], ["reply", "reply"], ["permissions", "permissions"], ["usage", "usage"]] as const) {
    // 从聊天页打开：她带你过去（玩家在别的页面时先放进右侧栏等玩家点，见 ui-open.spec.ts）
    await openChat(page);
    expect(await page.evaluate((uri) => (window as unknown as { kotoneUiOpen: (e: unknown) => string | null }).kotoneUiOpen({ uri }), `ui://kotone/settings/${section}`)).toBe("shown");
    await expect(page.getByTestId(`assistant-nav-${tab}`), section).toHaveAttribute("aria-current", "page");
    await expect(page.getByTestId(`settings-${tab}`), section).toBeVisible();
  }
  // 设置里还在的分页照旧
  await openChat(page);
  expect(await page.evaluate(() => (window as unknown as { kotoneUiOpen: (e: unknown) => string | null }).kotoneUiOpen({ uri: "ui://kotone/settings/models" }))).toBe("shown");
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  for (const [where, tab, anchor] of [
    ["设置 → 怎么叫她 → 呼叫键", "call", "settings-call-hotkey"],
    ["琴音 → 唤醒 → 语音唤醒", "call", "settings-call-wake"],
    ["琴音 → 设置 → 快捷指令", "call", "companion-title"],
    ["设置 → 她怎么回答 → 气泡", "reply", "settings-reply-bubble"],
    ["琴音 → 交互 → 声音与弹幕", "reply", "settings-reply-output"],
    ["设置 → 权限", "permissions", ""],
    ["琴音 → 用量", "usage", ""],
  ] as const) {
    await page.getByTestId("settings-nav-game").click();
    await page.evaluate(async (w) => (await import("/src/lib/stores/navigation.ts")).openLocation(w), where);
    await expect(page.getByTestId(`assistant-nav-${tab}`), where).toHaveAttribute("aria-current", "page");
    if (anchor) await expect(page.locator(`#${anchor}`), where).toBeInViewport();
  }
  // 「她用的模型」的旧位置都落到聊天页
  for (const where of ["设置 → 她怎么回答 → 她用的模型", "琴音 → 设置 → AI 连接", "聊天页 → 她用的模型"]) {
    expect(await page.evaluate(async (w) => (await import("/src/lib/stores/navigation.ts")).parseLocation(w), where), where)
      .toEqual({ page: "assistant", section: "chat", anchor: "chat-model" });
  }
});
