import { expect, test } from "@playwright/test";
import { openChat, openSettingsTab, openVoiceInput } from "./nav";

async function addConnection(page: import("@playwright/test").Page) {
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-connections").click();
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存 Key")).toBeVisible();
}

/** 她用的连接在聊天页输入框旁的“她用的模型”菜单里选（交接第十一节第 8 条，原「设置 → 她怎么回答」）：选第一个模型。 */
async function chooseConnection(page: import("@playwright/test").Page) {
  await openChat(page);
  await page.getByTestId("composer-model").click();
  await page.getByTestId("composer-model-menu").getByRole("menuitem").first().click();
  await expect(page.getByTestId("composer-model")).not.toHaveText("选模型");
}

// 陪玩只要选好有 Key 的 API 连接（不再要求能力自检；测试连接只是给用户手动确认）。
test("choosing a connection is enough: no self-check gate", async ({ page }) => {
  await page.goto("/?mockCompanionCheck=fail#/settings?onboarding=never");
  await addConnection(page);
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByRole("heading", { name: "琴音", exact: true })).toBeVisible();
  // 状态在角色页的「唤醒」，她用的连接在聊天页输入框旁的模型菜单（交接第十一节第 8 条）
  await openSettingsTab(page, "call");
  const companion = page.getByTestId("companion-settings");
  await expect(companion.getByTestId("companion-status")).toContainText("还没有选 API 连接");
  await chooseConnection(page);
  await openSettingsTab(page, "call");
  // 下一步缺的是文字识别模型，而不是自检
  await expect(companion.getByTestId("companion-status")).toContainText("文字识别模型");
  await expect(companion.getByTestId("companion-status")).not.toContainText("自检");
});

test("quick commands moved to 设置 → 热键, and online OCR shows the upload warning", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await addConnection(page);
  await chooseConnection(page);
  await openSettingsTab(page, "call");
  const companion = page.getByTestId("companion-settings");
  // 还没有文字识别模型：仍不就绪，提示去下载
  await expect(companion.getByTestId("companion-status")).toContainText("文字识别模型");

  // 快捷指令在「设置 → 热键」里加（ADR 040 第 3 期）：这里只有一句话和“去加”
  await expect(companion.getByTestId("quick-message")).toHaveCount(0);
  await expect(companion).toContainText("设置 → 热键");
  await expect(companion.getByTestId("companion-quick-goto")).toBeVisible();

  await page.evaluate(async () => (await import("/src/lib/ipc.ts")).updateSettings({ ocr: { activeModelId: "deepseek-ocr" } }));
  await openVoiceInput(page);
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("companion-upload-warning")).toContainText("上传");
  await expect(page.getByTestId("companion-status")).toHaveText("陪玩已就绪。");
});
