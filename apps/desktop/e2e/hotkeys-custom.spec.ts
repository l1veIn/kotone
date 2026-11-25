// ADR 040 第 3 期第 3 步：「设置 → 热键」“自己加的”那一组——“加一个”就地展开表单（跟她说一句、运行自动化、调一个工具、
// 打开一个页面），录键，保存；调的工具要问的写清会做什么、按钮是“允许并保存”（D9：按的时候不再问）；她或外部 Agent 加的、
// 还没确认的那一行标“等你确认”，点“确认”看清调哪个工具带什么参数再“允许”；不能绑的不让存；运行自动化那行写自动化的名字。
// 浏览器预览里的后端是 ipc/hotkeysMock.ts（照 hotkey_bind.rs / hotkey_custom.rs 写）；真正的规矩在 Rust 里有单元测试，
// 真机见 scripts/manual-e2e/hotkey_tools_real.py。
import { expect, test, type Page } from "@playwright/test";
import { openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

type Binding = { key: string; id?: string; say?: string; tool?: string; args?: Record<string, unknown>; page?: string; confirmed?: string[]; label?: string };
const table = (page: Page) => page.evaluate(async () =>
  ((await import("/src/lib/ipc/browserMock.ts")).getSettings()).then((s) => s.hotkeys.bindings as Binding[]));
const entry = async (page: Page, id: string) => (await table(page)).find((b) => b.id === id);

async function open(page: Page) {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "hotkeys");
}

async function startAdding(page: Page) {
  await page.getByTestId("hotkeys-add").click();
  const form = page.getByTestId("custom-hotkey-form");
  await expect(form).toBeVisible();
  return form;
}

async function setKey(page: Page, key: string) {
  await page.getByTestId("custom-key").fill(key);
  await expect(page.getByTestId("custom-key-conflict")).toHaveCount(0);
}

async function pick(page: Page, testid: string, option: string) {
  await page.getByTestId(testid).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test("add a quick command, an automation and a page; the automation row says which automation", async ({ page }) => {
  await open(page);
  // 跟她说一句（快捷指令）：默认就是这一种
  let form = await startAdding(page);
  await expect(page.getByTestId("custom-kind-say")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("custom-save")).toBeDisabled();
  await page.getByTestId("custom-say").fill("帮我看看队友说了什么");
  await setKey(page, "F8");
  await expect(page.getByTestId("custom-save")).toHaveText("保存");
  await page.getByTestId("custom-save").click();
  await expect(form).toHaveCount(0);
  await expect(page.getByTestId("hotkeys-row-custom.qm-1")).toContainText("快捷指令「帮我看看队友说了什么」");
  expect(await entry(page, "qm-1")).toEqual({ key: "F8", say: "帮我看看队友说了什么", id: "qm-1" });
  await expect(page.getByTestId("hotkey-status-custom.qm-1")).toHaveText("当前生效：F8");

  // 运行自动化：选一个已保存的、填它的入参
  form = await startAdding(page);
  await page.getByTestId("custom-kind-automation").click();
  await pick(page, "custom-automation", "界面示例：连按某键");
  await form.getByLabel("次数").fill("5");
  await form.getByLabel("次数").blur();
  await page.getByTestId("custom-label").fill("连发");
  await setKey(page, "F9");
  await page.getByTestId("custom-save").click();
  await expect(form).toHaveCount(0);
  expect(await entry(page, "qm-2")).toMatchObject({ key: "F9", tool: "automation.run", args: { id: "preview-auto-1", args: { times: 5 } }, label: "连发" });
  await expect(page.getByTestId("hotkeys-row-custom.qm-2")).toContainText("连发");
  await expect(page.getByTestId("hotkeys-detail-custom.qm-2")).toHaveText("运行自动化「界面示例：连按某键」");

  // 打开一个页面
  form = await startAdding(page);
  await page.getByTestId("custom-kind-page").click();
  await pick(page, "custom-page", "设置 · 模型");
  await setKey(page, "F10");
  await page.getByTestId("custom-save").click();
  await expect(form).toHaveCount(0);
  expect(await entry(page, "qm-3")).toMatchObject({ key: "F10", page: "ui://kotone/settings/models" });

  // 运行的自动化删了：那一行说清楚
  await page.evaluate(async () => {
    const [{ saveCustomHotkey }, { settingsStore }] = await Promise.all([import("/src/lib/ipc.ts"), import("/src/lib/stores/ui.ts")]);
    settingsStore.set(await saveCustomHotkey({ key: "", tool: "automation.run", args: { id: "gone" } }, false));
  });
  await expect(page.getByTestId("hotkeys-detail-custom.qm-4")).toContainText("已经删了（gone）");

  // 删一条
  await page.getByTestId("hotkeys-remove-custom.qm-1").click();
  await expect(page.getByTestId("hotkeys-row-custom.qm-1")).toHaveCount(0);
  expect(await entry(page, "qm-1")).toBeUndefined();
});

test("calling a tool: 发到游戏聊天 is first; one that asks says what it will do and is saved with 允许并保存; editing keeps asking", async ({ page }, testInfo) => {
  await open(page);
  const form = await startAdding(page);
  await page.getByTestId("custom-kind-tool").click();
  const tools = form.locator("[role='option'][data-testid^='custom-tool-']");
  await expect(tools.first()).toHaveAttribute("data-testid", "custom-tool-game_chat.send");
  // 找一个第三方插件的工具（没标只读：要确认）
  await page.getByTestId("custom-tool-search").fill("echo");
  await expect(tools).toHaveCount(1);
  await expect(page.getByTestId("custom-tool-everything.echo")).toContainText("要确认");
  await page.getByTestId("custom-tool-everything.echo").click();
  await page.getByTestId("tool-arg-message").fill("集合打龙");
  await setKey(page, "F7");
  const willDo = page.getByTestId("custom-will-do");
  await expect(willDo).toContainText("这个热键会：第三方插件的操作。");
  await expect(willDo).toContainText('调 everything.echo，参数 {"message":"集合打龙"}');
  await expect(willDo).toContainText("按的时候不再问你");
  await expect(page.getByTestId("custom-save")).toHaveText("允许并保存");
  await page.getByTestId("custom-tool-search").fill("");
  await screenshot(form, { path: testInfo.outputPath("custom-hotkey-form.png"), animations: "disabled" });
  await page.getByTestId("custom-save").click();
  await expect(form).toHaveCount(0);
  expect(await entry(page, "qm-1")).toMatchObject({ tool: "everything.echo", args: { message: "集合打龙" }, confirmed: ["third-party-action"] });
  const row = page.getByTestId("hotkeys-row-custom.qm-1");
  await expect(row).toHaveAttribute("data-confirm", "false");
  await expect(page.getByTestId("hotkey-status-custom.qm-1")).toHaveText("当前生效：F7");

  // 改参数：照样写清、照样“允许并保存”（改了参数要重新确认）
  await page.getByTestId("hotkeys-edit-custom.qm-1").click();
  const edit = row.getByTestId("custom-hotkey-form");
  await expect(edit.getByTestId("tool-arg-message")).toHaveValue("集合打龙");
  await edit.getByTestId("tool-arg-message").fill("先别打龙");
  await expect(edit.getByTestId("custom-save")).toHaveText("允许并保存");
  await edit.getByTestId("custom-save").click();
  await expect(edit).toHaveCount(0);
  expect(await entry(page, "qm-1")).toMatchObject({ args: { message: "先别打龙" }, confirmed: ["third-party-action"] });
});

test("a tool set to 禁止 can't be bound; a key that clashes can't be saved", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(async () => (await import("/src/lib/ipc.ts")).updateSettings({ agent: { permissions: { mode: "default", rules: { "game_chat.send": "deny" } } } } as never));
  await openSettingsTab(page, "hotkeys");
  await startAdding(page);
  await page.getByTestId("custom-kind-tool").click();
  await expect(page.getByTestId("custom-tool-game_chat.send")).toContainText("不能绑");
  await page.getByTestId("custom-tool-game_chat.send").click();
  await expect(page.getByTestId("custom-refused")).toContainText("用户设置为禁止：先去她的「权限」里改");
  await page.getByTestId("custom-key").fill("F7");
  await expect(page.getByTestId("custom-save")).toBeDisabled();
  // 撞上别的热键：说和谁撞了，不让存
  await page.getByTestId("custom-kind-say").click();
  await page.getByTestId("custom-say").fill("集合");
  await page.getByTestId("custom-key").fill("CapsLock");
  await expect(page.getByTestId("custom-key-conflict")).toContainText("语音输入热键");
  await expect(page.getByTestId("custom-save")).toBeDisabled();
  await page.getByTestId("custom-cancel").click();
  await expect(page.getByTestId("custom-hotkey-form")).toHaveCount(0);
});

test("a hotkey she or an external agent added that needs asking waits for the player's 确认", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 外部 Agent 经 settings.update 加的（补丁里写的 confirmed 后端不算；这里干脆没写）。真窗口里各窗口收到
  // kotone://settings-changed 刷新设置，这里直接放进界面的设置
  await page.evaluate(async () => {
    const [ipc, { settingsStore }] = await Promise.all([import("/src/lib/ipc.ts"), import("/src/lib/stores/ui.ts")]);
    const current = await ipc.getSettings();
    settingsStore.set(await ipc.updateSettings({ hotkeys: { bindings: [...current.hotkeys.bindings,
      { key: "F11", tool: "settings.update", args: { patch: { language: "en" } }, id: "ext" }] } } as never));
  });
  await openSettingsTab(page, "hotkeys");
  const row = page.getByTestId("hotkeys-row-custom.ext");
  await expect(row).toHaveAttribute("data-confirm", "true");
  await expect(page.getByTestId("hotkeys-waiting-custom.ext")).toHaveText("等你确认");
  await expect(page.getByTestId("hotkey-status-custom.ext")).toContainText("要先在「设置 → 热键」确认一次：会修改设置（settings.update）");
  await page.getByTestId("hotkeys-confirm-custom.ext").click();
  const box = page.getByTestId("hotkeys-confirm-box-custom.ext");
  await expect(box).toContainText("这个热键会：会修改设置（settings.update）。");
  await expect(box).toContainText('调 settings.update，参数 {"patch":{"language":"en"}}');
  await page.getByTestId("hotkeys-allow-custom.ext").click();
  await expect(row).toHaveAttribute("data-confirm", "false");
  await expect(page.getByTestId("hotkeys-waiting-custom.ext")).toHaveCount(0);
  expect(await entry(page, "ext")).toMatchObject({ confirmed: ["writes-settings"] });
  await expect(page.getByTestId("hotkey-status-custom.ext")).toHaveText("当前生效：F11");
});
