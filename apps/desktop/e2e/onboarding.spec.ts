import { expect, test } from "@playwright/test";

test("forced onboarding completes the full guided setup and can be reopened", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always");

  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding).toBeVisible();
  await page.getByRole("button", { name: "开始设置", exact: true }).click();

  await expect(page.getByTestId("onboarding-profile")).toBeVisible();
  await page.getByTestId("profile-lol").click();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  await expect(page.getByTestId("onboarding-model")).toBeVisible();
  await page.getByRole("button", { name: "下载推荐模型", exact: true }).click();
  await expect(page.getByText("✓ 已就绪", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  await expect(page.getByTestId("onboarding-hotkey")).toBeVisible();
  await page.getByTestId("mode-push-to-talk").click();
  await page.getByRole("button", { name: "去测试", exact: true }).click();

  await expect(page.getByTestId("onboarding-test")).toBeVisible();
  await page.getByRole("button", { name: "▶ 启动琴音", exact: true }).click();
  await expect(page.getByTestId("training-input")).toBeFocused();
  await expect(
    page.getByTestId("onboarding-test").getByText("✓ CapsLock 已注册", { exact: true }),
  ).toBeVisible();

  await page.getByRole("button", { name: "只测试文字发送", exact: true }).click();
  await expect(page.getByTestId("test-success")).toContainText("琴音测试发送");

  const finish = page.getByTestId("finish-onboarding");
  await expect(finish).toBeEnabled();
  await finish.click();
  await expect(onboarding).toBeHidden();

  await page.getByRole("button", { name: "高级", exact: true }).click();
  await page.getByRole("button", { name: "重新运行向导", exact: true }).click();
  await expect(onboarding).toBeVisible();
});

test("auto and never modes respect the persisted completed state", async ({ page }) => {
  await page.goto("/#/settings");
  await expect(page.getByTestId("onboarding")).toHaveCount(0);

  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
});

test("starting without a model routes the user to a recoverable download state", async ({
  page,
}) => {
  await page.goto("/#/settings");
  await page.getByRole("button", { name: "▶ 启动", exact: true }).click();

  await expect(page.getByRole("heading", { name: "高级", exact: true })).toBeVisible();
  await expect(page.getByText("未就绪（模型未下载）", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "下载", exact: true }).first()).toBeVisible();
});

test("a failed model download keeps an inline error and retry action", async ({ page }) => {
  await page.goto("/#/settings?mockDownload=fail");
  await page.getByRole("button", { name: "高级", exact: true }).click();
  await page.getByRole("button", { name: "下载", exact: true }).first().click();

  await expect(page.getByText(/下载失败：网络连接失败/)).toBeVisible();
  await expect(page.getByRole("button", { name: "重试", exact: true }).first()).toBeVisible();
});
