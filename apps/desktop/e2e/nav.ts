// ADR 028 的界面结构下常用的跳转（不是测试文件，供各 spec 引用）。
import { expect, type Page } from "@playwright/test";

/** 「设置」的某个分页：陪玩组 call / reply / permissions / usage，通用组 models / connections / sound / tuning / appearance / system。 */
export async function openSettingsTab(page: Page, tab: string) {
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId(`advanced-nav-${tab}`).click();
  await expect(page.getByTestId(`advanced-nav-${tab}`)).toHaveAttribute("aria-current", "page");
}

/** 「语音输入」（插件的 App，ADR 038：左侧栏「插件 → 语音输入」）的某个分页（输入 = general、文字处理 = processing、历史 = history）。 */
export async function openVoiceInput(page: Page, tab: "general" | "processing" | "history" = "general") {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-voice-input-open").click();
  if (tab !== "general") await page.getByTestId(`voice-tab-${tab}`).click();
  await expect(page.getByTestId(`voice-tab-${tab}`)).toHaveAttribute("aria-current", "page");
}

/** 回到聊天：侧栏对话列表里的当前对话（浏览器预览有示例对话）。 */
export async function openChat(page: Page) {
  await page.getByTestId("sidebar-conversations").getByTestId("agent-conversation").first().click();
  await expect(page.getByTestId("assistant-tasks")).toBeVisible();
}

/** 某个插件的 App（ADR 038）：左侧栏「插件」→ 点它的卡片。返回 App 的容器。 */
export async function openPlugin(page: Page, id: string) {
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId(`plugins-card-${id}-open`).click();
  const pluginPage = page.locator(`[data-testid="plugin-page"][data-plugin-id="${id}"]`);
  await expect(pluginPage).toBeVisible();
  // 插件的 App 从「插件」点进来：左侧栏亮的是「插件」
  await expect(page.getByTestId("settings-nav-plugins")).toHaveAttribute("aria-current", "page");
  return pluginPage;
}
