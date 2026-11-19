import { expect, test, type Page } from "@playwright/test";

async function showUpdateDialog(
  page: Page,
  state:
    | { phase: "available" | "backing-up" | "ready"; version: string; notes?: string }
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
  await expect(updateButton).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(laterButton).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(updateButton).toBeFocused();
  await laterButton.click();
  await expect(dialog).toBeHidden();

  await showUpdateDialog(page, { phase: "backing-up", version: "0.1.8" });
  await expect(dialog.getByRole("heading", { name: "正在备份设置和数据" })).toBeVisible();
  await expect(dialog.getByRole("button")).toHaveCount(0);

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

test("0.2 updates keep the legacy explanation short and link the full changelog", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(async () => {
    const { describeUpdate, updateDialogStore } = await import("/src/lib/updater.ts");
    const summary = describeUpdate({ version: "0.2.0", notes: "full release notes" });
    updateDialogStore.set({ ...summary, phase: "available" });
  });

  const notes = page.getByTestId("update-notes");
  await expect(notes).toContainText("语音输入法的用法没有变");
  await expect(notes).toContainText("不配置 API Key");
  await expect(notes).toContainText("手动重装 0.1.x");
  await expect(notes).not.toContainText("full release notes");
  expect((await notes.textContent())?.split("\n")).toHaveLength(3);
  await expect(page.getByRole("link", { name: "查看完整更新日志" })).toHaveAttribute(
    "href", "https://github.com/l1veIn/kotone/blob/v0.2.0/CHANGELOG.md",
  );
});

test("about page explains how to restore a backup and reinstall 0.1.13", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByRole("button", { name: "关于", exact: true }).click();
  await page.getByText("回到旧版本", { exact: true }).click();
  const rollback = page.getByTestId("rollback-section");
  await expect(rollback).toContainText("backup");
  await expect(rollback.getByRole("link", { name: "下载 0.1.13 安装包" })).toHaveAttribute(
    "href", "https://github.com/l1veIn/kotone/releases/download/v0.1.13/Kotone_0.1.13_x64-setup.exe",
  );
});
