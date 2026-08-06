import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
/** dev:web 下关于页显示的版本来自 package.json（单一来源），断言与其保持一致 */
const pkg = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url), "utf8"),
) as { version: string };

test("the check-for-updates button is always visible and reports the latest version", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByRole("button", { name: "关于", exact: true }).click();

  const checkUpdates = page.getByTestId("check-updates");
  await expect(checkUpdates).toBeVisible();
  await expect(checkUpdates).toBeEnabled();

  await checkUpdates.click();
  await expect(
    page.getByText(`✓ 已是最新版本（v${pkg.version}）`, { exact: true }),
  ).toBeVisible();
  await expect(checkUpdates).toBeVisible();
  await expect(checkUpdates).toBeEnabled();
});
