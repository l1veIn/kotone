import { expect, test } from "@playwright/test";

test("registered post-processors can be composed, tried out and edited", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-processing").click();

  await expect(page.getByRole("heading", { name: "文字处理" })).toBeVisible();
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
    "【对面打野在下路！】",
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
