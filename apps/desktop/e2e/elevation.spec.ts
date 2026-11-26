// 默认以管理员身份运行（ADR 044 D1）：第一次先在窗口里说明一句再弹 UAC；UAC 点了“否”、管理员身份的游戏在前面时
// 说清楚哪些用不了。浏览器预览里用 `window.__kotoneElevationSeed` 装成 Windows（见 browserMock.ts）。
import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab, openVoiceInput } from "./nav";

type Seed = {
  status?: Record<string, unknown>;
  settings?: { adminPromptRevision?: number; runAsAdminOnStart?: boolean };
};

async function open(page: Page, seed: Seed, onboarding = "never") {
  await page.addInitScript((value) => {
    (window as unknown as { __kotoneElevationSeed: unknown }).__kotoneElevationSeed = value;
  }, seed);
  await page.goto(`/#/settings?onboarding=${onboarding}`);
  await expect(page.getByTestId("app-frame")).toBeVisible();
}

/** 预览里的设置（browserMock.ts 的 `window.__kotoneMockSettings`）。 */
async function mockSettings(page: Page) {
  return page.evaluate(() => {
    const read = (window as unknown as { __kotoneMockSettings: () => { adminPromptRevision: number; runAsAdminOnStart: boolean } }).__kotoneMockSettings;
    const s = read();
    return { revision: s.adminPromptRevision, runAsAdmin: s.runAsAdminOnStart };
  });
}

async function restartCalls(page: Page) {
  return page.evaluate(() => (window as unknown as { __kotoneRestartAsAdminCalls?: number }).__kotoneRestartAsAdminCalls ?? 0);
}

const windows = { supported: true, elevated: false, adminPromptRevision: 1 };

test("没问过：先说明一句；「先不用」关掉开关、记下问过，不重开", async ({ page }) => {
  await open(page, { status: windows });
  const dialog = page.getByTestId("admin-explain-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("英雄联盟这类游戏是以管理员身份运行的");
  await expect(dialog).toContainText("呼叫键、热键和往游戏里打字");
  await dialog.getByRole("button", { name: "先不用" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("好的，Kotone 以普通权限运行", { exact: false })).toBeVisible();
  expect(await mockSettings(page)).toEqual({ revision: 1, runAsAdmin: false });
  expect(await restartCalls(page)).toBe(0);
  await openSettingsTab(page, "system");
  await expect(page.getByLabel("启动时以管理员身份运行")).not.toBeChecked();
});

test("没问过：「好」打开开关、记下问过、以管理员身份重开", async ({ page }) => {
  await open(page, { status: windows, settings: { runAsAdminOnStart: false } });
  const dialog = page.getByTestId("admin-explain-dialog");
  await dialog.getByRole("button", { name: "好", exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(await mockSettings(page)).toEqual({ revision: 1, runAsAdmin: true });
  expect(await restartCalls(page)).toBe(1);
});

test("已经是管理员身份时「好」只记下，不再重开", async ({ page }) => {
  await open(page, { status: { ...windows, elevated: true } });
  await page.getByTestId("admin-explain-dialog").getByRole("button", { name: "好", exact: true }).click();
  expect(await mockSettings(page)).toEqual({ revision: 1, runAsAdmin: true });
  expect(await restartCalls(page)).toBe(0);
});

test("说明盖在新手引导上面，点完接着引导", async ({ page }) => {
  await open(page, { status: windows }, "always");
  await expect(page.getByTestId("admin-explain-dialog")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toBeVisible();
  await page.getByTestId("admin-explain-dialog").getByRole("button", { name: "先不用" }).click();
  await expect(page.getByTestId("admin-explain-dialog")).toBeHidden();
  await expect(page.getByTestId("onboarding")).toBeVisible();
});

test("问过了（第 1 版）就不再说明；自己拨开关也算问过", async ({ page }) => {
  await open(page, { status: windows, settings: { adminPromptRevision: 1 } });
  await expect(page.getByTestId("settings-nav-advanced")).toBeVisible();
  await expect(page.getByTestId("admin-explain-dialog")).toHaveCount(0);

  // 没问过的人在「设置 → 系统」里自己拨了开关：记成现在的版本
  const fresh = await page.context().newPage();
  await open(fresh, { status: windows });
  await fresh.getByTestId("admin-explain-dialog").getByRole("button", { name: "先不用" }).click();
  await openSettingsTab(fresh, "system");
  await fresh.getByLabel("启动时以管理员身份运行").check({ force: true });
  await expect(fresh.getByText("已开启：以后打开 Kotone 时以管理员身份运行")).toBeVisible();
  expect(await mockSettings(fresh)).toEqual({ revision: 1, runAsAdmin: true });
});

test("浏览器预览（不是 Windows）里不说明", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("settings-nav-advanced")).toBeVisible();
  await expect(page.getByTestId("admin-explain-dialog")).toHaveCount(0);
  await expect(page.getByTestId("elevation-notice")).toHaveCount(0);
});

test("UAC 点了“否”、管理员身份的游戏在前面：说清楚哪些用不了，能以管理员身份重开", async ({ page }) => {
  await open(page, { status: { ...windows, notice: "foreground" }, settings: { adminPromptRevision: 1 } });
  const notice = page.getByTestId("elevation-notice");
  await expect(notice).toBeVisible();
  const text = notice.getByTestId("elevation-notice-text");
  await expect(text).toContainText("前面的游戏是以管理员身份运行的，Kotone 不是");
  for (const part of ["呼叫键", "语音输入热键", "停下热键和 Esc", "快捷指令", "一键喊话", "往游戏里打字"]) {
    await expect(text).toContainText(part);
  }
  await notice.getByRole("button", { name: "以管理员身份重开" }).click();
  await expect.poll(() => restartCalls(page)).toBe(1);
  await notice.getByRole("button", { name: "知道了" }).click();
  await expect(notice).toBeHidden();
});

test("打字真的被拦了：先说刚才被挡掉了", async ({ page }) => {
  await open(page, { status: { ...windows, notice: "blocked" }, settings: { adminPromptRevision: 1 } });
  await expect(page.getByTestId("elevation-notice-text")).toContainText("刚才往游戏里打字被 Windows 挡掉了");
});

test("已经是管理员身份就不说用不了", async ({ page }) => {
  await open(page, { status: { ...windows, elevated: true, notice: "foreground" }, settings: { adminPromptRevision: 1 } });
  await expect(page.getByTestId("settings-nav-advanced")).toBeVisible();
  await expect(page.getByTestId("elevation-notice")).toHaveCount(0);
});

test("语音输入页的横幅不再只说文字", async ({ page }) => {
  await open(page, { status: { ...windows, activeGameElevated: true }, settings: { adminPromptRevision: 1 } });
  await openVoiceInput(page);
  await expect(page.getByText("游戏是以管理员身份运行的，Kotone 不是")).toBeVisible();
  await expect(page.getByText("呼叫键和热键都收不到", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "以管理员身份重启" }).click();
  await expect.poll(() => restartCalls(page)).toBe(1);
});
