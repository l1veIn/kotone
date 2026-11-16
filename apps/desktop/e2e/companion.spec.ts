import { expect, test } from "@playwright/test";

async function addConnection(page: import("@playwright/test").Page) {
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-connections").click();
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存密钥")).toBeVisible();
}

// 陪玩必须有通过能力自检的 AI 连接：选连接时自动自检；没通过时热键不工作，只提示用户。
test("choosing a connection runs the self-check, and the companion stays blocked until it passes", async ({ page }) => {
  await page.goto("/?mockCompanionCheck=fail#/settings?onboarding=never");
  await addConnection(page);
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByRole("heading", { name: "琴音陪玩", exact: true })).toBeVisible();
  await page.getByTestId("assistant-nav-settings").click();
  const companion = page.getByTestId("companion-settings");
  await expect(companion.getByTestId("companion-status")).toContainText("还没有给琴音选 AI 连接");
  await page.getByLabel("陪玩 AI 连接").click();
  await page.getByRole("option").nth(1).click();
  await expect(companion.getByTestId("companion-check")).toContainText("未通过");
  await expect(companion.getByTestId("companion-status")).toContainText("没有通过能力自检");
});

test("quick messages can be added, edited and removed, and online OCR shows the upload warning", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await addConnection(page);
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  await page.getByLabel("陪玩 AI 连接").click();
  await page.getByRole("option").nth(1).click();
  const companion = page.getByTestId("companion-settings");
  await expect(companion.getByTestId("companion-check")).toContainText("已通过");
  // 自检通过但还没有文字识别模型：仍不就绪，提示去下载
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
  await page.getByTestId("assistant-nav-chat").click();
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByTestId("companion-upload-warning")).toContainText("上传");
  await expect(page.getByTestId("companion-status")).toHaveText("陪玩已就绪。");
});
