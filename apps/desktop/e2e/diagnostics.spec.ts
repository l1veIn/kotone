import { expect, test } from "@playwright/test";

test("exports a privacy-safe diagnostic package from the advanced page", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");

  await page.getByRole("button", { name: "高级", exact: true }).click();

  await expect(
    page.getByText("不包含录音、识别文本和热词，可安全分享给测试群管理员"),
  ).toBeVisible();

  await page.getByRole("button", { name: "导出诊断包" }).click();

  await expect(page.getByText("诊断包已导出：KT-MOCK")).toBeVisible();
});
