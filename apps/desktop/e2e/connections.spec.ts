import { expect, test, type Page } from "@playwright/test";

// 高级 → API 连接：按分组展示预置提供商，编辑地址 / 模型 / 密钥（只写），设为琴音的连接并做能力自检。

async function openConnections(page: Page, query = "") {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto(`/${query}#/settings?onboarding=never`);
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-connections").click();
  await expect(page.getByTestId("connections-section")).toBeVisible();
}

test("providers are grouped, searchable and every preset is listed", async ({ page }, testInfo) => {
  await openConnections(page);
  const providers = page.getByTestId("connection-providers");
  await expect(providers.locator('[data-testid^="connection-preset-"]')).toHaveCount(29);
  for (const label of ["推荐", "国内厂商", "海外厂商", "聚合平台", "自定义"]) {
    await expect(providers.getByText(label, { exact: true })).toBeVisible();
  }
  await expect(page.getByTestId("connections-section")).toContainText("给琴音用的主流大模型");
  await page.screenshot({ path: testInfo.outputPath("connections-list.png"), fullPage: false });
  await providers.getByLabel("搜索提供商或模型").fill("kimi");
  await expect(providers.getByTestId("connection-preset-moonshot-cn")).toBeVisible();
  await expect(providers.getByTestId("connection-preset-openai")).toHaveCount(0);
});

test("editing picks a listed model or a typed one, and the key is write-only", async ({ page }, testInfo) => {
  await openConnections(page);
  await page.getByTestId("connection-preset-deepseek").click();
  const editor = page.getByTestId("connection-editor");
  await expect(editor).toContainText("DeepSeek");
  await expect(editor.getByTestId("connection-model")).toHaveValue("deepseek-flash");
  await editor.getByRole("button", { name: "deepseek-v4-pro", exact: true }).click();
  await expect(editor.getByTestId("connection-model")).toHaveValue("deepseek-v4-pro");
  await editor.getByTestId("connection-api-key").fill("sk-local-fixture");
  await page.screenshot({ path: testInfo.outputPath("connections-editor.png") });
  await editor.getByTestId("connection-save").click();
  const card = page.locator('[data-testid^="connection-card-deepseek-"]');
  await expect(card).toContainText("已保存密钥");
  await expect(card).toContainText("deepseek-v4-pro");

  await card.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(editor.getByTestId("connection-api-key")).toHaveValue("");
  await expect(editor.getByTestId("connection-api-key")).toHaveAttribute("placeholder", "已保存，留空则不修改");
  await editor.getByTestId("connection-model").fill("my-own-model");
  await editor.getByTestId("connection-save").click();
  await expect(card).toContainText("my-own-model");
  await expect(card).toContainText("已保存密钥");
});

test("a connection becomes 琴音's and runs the capability self-check", async ({ page }, testInfo) => {
  await openConnections(page);
  await page.getByTestId("connection-preset-dashscope").click();
  const editor = page.getByTestId("connection-editor");
  await editor.getByTestId("connection-api-key").fill("sk-local-fixture");
  await editor.getByTestId("connection-use-after-save").check();
  await editor.getByTestId("connection-save").click();
  const card = page.locator('[data-testid^="connection-card-dashscope-"]');
  await expect(card).toContainText("琴音在用");
  await expect(card.getByTestId("connection-check-result")).toContainText("能力自检通过");
  await page.screenshot({ path: testInfo.outputPath("connections-check.png") });
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.assistant.connectionId).toMatch(/^dashscope-/);
});

test("a failed self-check is shown on the card, and deleting asks first", async ({ page }) => {
  await openConnections(page, "?mockCompanionCheck=fail");
  await page.getByTestId("connection-preset-zhipu").click();
  await page.getByTestId("connection-api-key").fill("sk-local-fixture");
  await page.getByTestId("connection-save").click();
  const card = page.locator('[data-testid^="connection-card-zhipu-"]');
  await card.getByTestId("connection-use").click();
  await expect(card.getByTestId("connection-check-result")).toContainText("能力自检没通过");
  await card.getByRole("button", { name: /^删除/ }).click();
  await expect(page.getByTestId("connection-delete-dialog")).toContainText("琴音正在用它");
  await page.getByTestId("connection-delete-confirm").click();
  await expect(card).toHaveCount(0);
});
