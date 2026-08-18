import { expect, test } from "@playwright/test";

test("registered post-processors can be composed, tried out and edited", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-processing").click();

  await expect(page.getByRole("heading", { name: "文字处理" })).toBeVisible();
  await expect(page.getByTestId("postprocess-pipeline-select")).toHaveAttribute(
    "data-value",
    "blocklist",
  );
  await page.getByTestId("manage-postprocess-pipelines").click();
  await page.getByTestId("add-postprocess-pipeline").click();
  await page.getByRole("button", { name: "完成" }).click();
  await expect(page.getByText("还没有处理步骤", { exact: true })).toBeVisible();

  await page.getByTestId("add-postprocess-step").click();
  await expect(page.getByTestId("processor-option-mock.append-exclamation")).toBeVisible();
  await expect(page.getByTestId("processor-option-mock.wrap-brackets")).toBeVisible();
  await page.getByTestId("processor-option-mock.append-exclamation").click();

  await page.getByTestId("add-postprocess-step").click();
  await page.getByTestId("processor-option-mock.wrap-brackets").click();

  const steps = page.locator('[data-testid^="postprocess-step-"]');
  await expect(steps).toHaveCount(2);
  await expect(steps.nth(0)).toContainText("句尾叹号");
  await expect(steps.nth(1)).toContainText("方括号包裹");

  await page.getByTestId("postprocess-tryout-run").click();
  await expect(page.getByTestId("postprocess-tryout-result")).toContainText(
    "【对面那个傻逼打野太牛逼了！】",
  );

  await steps.nth(0).getByTitle("下移").click();
  await expect(steps.nth(0)).toContainText("方括号包裹");
  await expect(steps.nth(1)).toContainText("句尾叹号");

  const secondToggle = steps.nth(1).getByRole("checkbox", { name: /启用.*句尾叹号/ });
  await expect(secondToggle).toBeChecked();
  await secondToggle.uncheck({ force: true });
  await expect(secondToggle).not.toBeChecked();

  await steps.nth(1).getByRole("button", { name: "移除" }).click();
  await expect(steps).toHaveCount(1);
  await expect(steps.nth(0)).toContainText("方括号包裹");
});

test("a discovered blocklist processor works by default and saves a custom CSV path", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-processing").click();

  await expect(page.getByTestId("postprocess-pipeline-select")).toHaveAttribute(
    "data-value",
    "blocklist",
  );

  const step = page.getByTestId("postprocess-step-builtin.blocklist-filter");
  const toggle = step.getByRole("checkbox", { name: "启用屏蔽词过滤" });
  await expect(toggle).toBeChecked();
  await expect(step).toContainText("自带一份默认词库");

  const tryoutInput = page.getByTestId("postprocess-tryout-input");
  await tryoutInput.fill("你真傻逼，这波牛逼");
  await page.getByTestId("postprocess-tryout-run").click();
  await expect(page.getByTestId("postprocess-tryout-result")).toContainText(
    "你真**，这波NB",
  );

  const csvPath = step.getByRole("textbox", { name: "自己的词库" });
  await csvPath.fill("C:\\Kotone\\blocklist.csv");
  await csvPath.blur();

  await expect(csvPath).toHaveValue("C:\\Kotone\\blocklist.csv");
  await expect(toggle).toBeChecked();
});

test("online polish is added best-effort after a saved connection", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-processing").click();
  await page.getByTestId("add-postprocess-step").click();
  const addDialog = page.getByRole("dialog", { name: "添加步骤" });
  await expect(addDialog).toContainText("AI 润色");
  await expect(addDialog).toContainText("需要联网");
  await page.getByTestId("processor-option-writing.openai-compat").click();

  const step = page.getByTestId("postprocess-step-writing.openai-compat");
  await expect(step.getByRole("checkbox", { name: "启用AI 润色" })).not.toBeChecked();
  await expect(step.locator('[data-testid^="postprocess-on-error-"]')).toHaveAttribute(
    "data-value",
    "best-effort",
  );
  await step.getByTestId("open-advanced-connections").click();
  await expect(page.getByTestId("advanced-nav-connections")).toHaveClass(/bg-kotone-cyan/);

  await page.getByTestId("connection-preset-dashscope").click();
  await expect(page.getByTestId("connection-editor")).toBeVisible();
  await page.getByTestId("connection-api-key").fill("sk-test");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存密钥")).toBeVisible();

  await page.getByTestId("settings-nav-processing").click();
  const polishStep = page.getByTestId("postprocess-step-writing.openai-compat");
  await polishStep.getByLabel("API 连接").click();
  await page.getByRole("option").nth(1).click();
  await expect(polishStep.getByRole("checkbox", { name: "启用AI 润色" })).toBeChecked();
});
