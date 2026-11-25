// 界面语言：设置 → 外观 → 语言。没选过是中文；换成 English 后整个窗口变英文，换回来恢复中文。
import { expect, test } from "@playwright/test";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

test("the interface is Chinese by default and can switch to English and back", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("settings-nav-advanced")).toContainText("设置");
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");

  await openSettingsTab(page, "appearance");
  await expect(page.getByTestId("appearance-locale-zh-CN")).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("appearance-locale-en").click();

  // 重新加载以后是英文（回到打开设置时的那一页）
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.getByTestId("settings-nav-advanced")).toContainText("Settings");
  await openSettingsTab(page, "appearance");
  await expect(page.getByTestId("appearance-locale-en")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Language", { exact: false }).first()).toBeVisible();
  // 英文界面里不该冒出目录里的中文（玩家说的命令、语言名除外）——抽查侧栏和设置分组
  await expect(page.getByTestId("advanced-nav-models")).toContainText("Models");
  await expect(page.getByTestId("advanced-companion-moved")).toContainText("Wake-up, interaction, permissions and usage");
  await screenshot(page, { path: testInfo.outputPath("settings-en.png") });

  await page.getByTestId("appearance-locale-zh-CN").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await expect(page.getByTestId("settings-nav-advanced")).toContainText("设置");
});
