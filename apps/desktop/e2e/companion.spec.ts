import { expect, test } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

async function addConnection(page: import("@playwright/test").Page) {
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-connections").click();
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存 Key")).toBeVisible();
}

// 陪玩只要选好有 Key 的 API 连接（不再要求能力自检；测试连接只是给用户手动确认）。
test("choosing a connection is enough: no self-check gate", async ({ page }) => {
  await page.goto("/?mockCompanionCheck=fail#/settings?onboarding=never");
  await addConnection(page);
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByRole("heading", { name: "琴音", exact: true })).toBeVisible();
  // 状态在「设置 → 怎么叫她」，API 连接在「设置 → 她怎么回答 → 她用的模型」（ADR 028）
  await openSettingsTab(page, "call");
  const companion = page.getByTestId("companion-settings");
  await expect(companion.getByTestId("companion-status")).toContainText("还没有选 API 连接");
  await openSettingsTab(page, "reply");
  await page.getByLabel("她用的 API 连接").click();
  await page.getByRole("option").nth(1).click();
  await openSettingsTab(page, "call");
  // 下一步缺的是文字识别模型，而不是自检
  await expect(companion.getByTestId("companion-status")).toContainText("文字识别模型");
  await expect(companion.getByTestId("companion-status")).not.toContainText("自检");
});

test("quick messages can be added, edited and removed, and online OCR shows the upload warning", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await addConnection(page);
  await openSettingsTab(page, "reply");
  await page.getByLabel("她用的 API 连接").click();
  await page.getByRole("option").nth(1).click();
  await openSettingsTab(page, "call");
  const companion = page.getByTestId("companion-settings");
  // 还没有文字识别模型：仍不就绪，提示去下载
  await expect(companion.getByTestId("companion-status")).toContainText("文字识别模型");

  await companion.getByRole("button", { name: "添加", exact: true }).click();
  await expect(companion.getByTestId("quick-message")).toHaveCount(1);
  const content = companion.getByLabel("快捷指令 1 内容");
  await expect(content).toHaveValue("帮我看看队友说了什么");
  await content.fill("帮我跟队友说先别打龙");
  await content.blur();
  await expect.poll(async () => page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).getSettings().then((s) => s.assistant.companion.quickMessages[0]?.text))).toBe("帮我跟队友说先别打龙");
  await companion.getByRole("button", { name: "删除", exact: true }).click();
  await expect(companion.getByTestId("quick-message")).toHaveCount(0);

  await page.evaluate(async () => (await import("/src/lib/ipc.ts")).updateSettings({ ocr: { activeModelId: "deepseek-ocr" } }));
  await openVoiceInput(page);
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("companion-upload-warning")).toContainText("上传");
  await expect(page.getByTestId("companion-status")).toHaveText("陪玩已就绪。");
});
