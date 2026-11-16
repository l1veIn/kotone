import { expect, test, type Page } from "@playwright/test";

async function showUpdateDialog(
  page: Page,
  state:
    | { phase: "available" | "ready"; version: string; notes?: string }
    | {
        phase: "downloading";
        version: string;
        notes?: string;
        downloadedBytes: number;
        totalBytes?: number;
      },
) {
  await page.evaluate(async (dialogState) => {
    const { updateDialogStore } = await import("/src/lib/updater.ts");
    updateDialogStore.set(dialogState);
  }, state);
}

test("the branded update dialog covers confirmation, progress and restart states", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=never");

  await showUpdateDialog(page, {
    phase: "available",
    version: "0.1.8",
    notes: "增加中文路径门禁。\n增加鼠标侧键支持。",
  });
  const dialog = page.getByTestId("update-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "发现琴音新版本" })).toBeVisible();
  await expect(page.getByTestId("update-notes")).toContainText("增加鼠标侧键支持");
  const laterButton = dialog.getByRole("button", { name: "稍后更新" });
  const updateButton = dialog.getByRole("button", { name: "立即更新" });
  await expect(laterButton).toBeVisible();
  const neverButton = dialog.getByRole("button", { name: "不再提醒" });
  await expect(updateButton).toBeFocused();
  // 焦点在弹窗内循环：立即更新 → 不再提醒 → 稍后更新 → 立即更新
  await page.keyboard.press("Tab");
  await expect(neverButton).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(laterButton).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(updateButton).toBeFocused();
  await laterButton.click();
  await expect(dialog).toBeHidden();

  await showUpdateDialog(page, { phase: "available", version: "0.1.8" });
  await dialog.getByRole("button", { name: "不再提醒" }).click();
  await expect(dialog).toBeHidden();

  await showUpdateDialog(page, {
    phase: "downloading",
    version: "0.1.8",
    downloadedBytes: 42,
    totalBytes: 100,
  });
  await expect(dialog.getByRole("heading", { name: "正在更新琴音" })).toBeVisible();
  await expect(page.getByTestId("update-progress")).toHaveText("42%");
  await expect(dialog.getByRole("button")).toHaveCount(0);

  await showUpdateDialog(page, { phase: "ready", version: "0.1.8" });
  await expect(dialog.getByRole("heading", { name: "更新已经准备好" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "立即重启" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("the about page can turn the startup update reminder back on", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(async () => {
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.set(await (await import("/src/lib/ipc.ts")).updateSettings({ ui: { updatePromptDisabled: true } }));
  });
  await page.getByRole("button", { name: "关于", exact: true }).click();
  const toggle = page.getByTestId("update-prompt-enabled");
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect
    .poll(async () => page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings().then((s) => s.ui.updatePromptDisabled)))
    .toBe(false);
  // 启动时的自动检查在关闭提醒后直接跳过（浏览器预览里本来就不联网）
  const skipped = await page.evaluate(async () => {
    const ipc = await import("/src/lib/ipc.ts");
    const { settingsStore } = await import("/src/lib/stores/ui.ts");
    settingsStore.set(await ipc.updateSettings({ ui: { updatePromptDisabled: true } }));
    const { checkForUpdates } = await import("/src/lib/updater.ts");
    return checkForUpdates({ auto: true });
  });
  expect(skipped.status).toBe("up-to-date");
});
