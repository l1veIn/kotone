import { expect, test } from "@playwright/test";

test("exports a privacy-safe diagnostic package from the advanced page", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");

  await page.getByRole("button", { name: "高级", exact: true }).click();
  await page.getByTestId("advanced-nav-system").click();

  await expect(
    page.getByText("不包含录音、识别文本和热词，可安全分享给测试群管理员"),
  ).toBeVisible();
  await expect(page.getByText("记录诊断信息", { exact: true })).toBeVisible();

  await page.getByTestId("diagnostics-export").click();

  await expect(
    page.getByText("诊断包已导出（最近 24 小时）：KT-MOCK"),
  ).toBeVisible();
});

test("clear diagnostics asks for confirmation and can stop recording", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByRole("button", { name: "高级", exact: true }).click();
  await page.getByTestId("advanced-nav-system").click();

  await page.getByTestId("diagnostics-clear").click();
  await expect(page.getByTestId("diagnostics-clear-dialog")).toBeVisible();
  await page.getByTestId("diagnostics-stop-recording").check();
  await page.getByTestId("diagnostics-clear-confirm").click();
  await expect(page.getByText("本地诊断信息已清除")).toBeVisible();
});
