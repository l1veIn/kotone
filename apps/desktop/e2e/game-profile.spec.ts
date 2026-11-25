import { expect, test } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 「游戏适配」：允许自动化输入（allowAutomationInput，缺省开）与启动路径（launchPath）。
// 保存时整份 profile 原样带回：新字段和原有字段都不能丢。

test("game profile saves the automation-input switch and launch path without dropping other fields", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-game").click();
  await expect(page.getByRole("heading", { name: "游戏列表", exact: true })).toBeVisible();

  // 没写过这个字段的配置按“允许”显示
  const generic = page.getByTestId("game-profile-generic");
  await generic.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(generic.getByTestId("profile-allow-automation-input")).toHaveAttribute("aria-checked", "true");
  await expect(generic.getByTestId("profile-launch-path")).toHaveValue("");
  await generic.getByRole("button", { name: "收起", exact: true }).click();

  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const editor = lol.getByTestId("game-profile-editor");
  await expect(editor.getByTestId("profile-launch-path")).toHaveValue("C:\\Riot Games\\Riot Client\\RiotClientServices.exe");
  await editor.getByTestId("profile-allow-automation-input").click();
  await expect(editor.getByTestId("profile-allow-automation-input")).toHaveAttribute("aria-checked", "false");
  await editor.getByTestId("profile-launch-path").fill("steam://rungameid/570");
  await editor.getByTestId("profile-companion-notes").fill("聊天在左下角");
  await screenshot(editor, { path: testInfo.outputPath("game-profile-editor.png") });
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.getByTestId("game-profile-editor")).toHaveCount(0);
  await expect(lol).toContainText("不允许自动化输入");

  const saved = await page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).listProfiles().then((list) => list.find((p) => p.id === "lol")));
  expect(saved).toMatchObject({
    allowAutomationInput: false,
    launchPath: "steam://rungameid/570",
    companionNotes: "聊天在左下角",
    openChatKey: "Enter",
    sendKey: "Enter",
    preSendDelayMs: 100,
    icon: "lol-kotone.webp",
    hotwords: ["闪现", "大龙", "gank", "打野", "推塔", "回城"],
  });
  expect(saved?.channels).toHaveLength(2);

  // 再打开编辑器：读回的是刚保存的值
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(lol.getByTestId("profile-allow-automation-input")).toHaveAttribute("aria-checked", "false");
  await expect(lol.getByTestId("profile-launch-path")).toHaveValue("steam://rungameid/570");
});

test("auto-detect lists installed games, matching ones first, and fills the launch path", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-game").click();
  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const editor = lol.getByTestId("game-profile-editor");
  await editor.getByTestId("profile-detect-games").click();
  const list = editor.getByTestId("profile-detected-games");
  // 英雄联盟的配置：进程名对得上的国服排第一
  await expect(list.locator("button").first()).toContainText("英雄联盟（国服）");
  await expect(list).toContainText("原神");
  await screenshot(editor, { path: testInfo.outputPath("game-profile-detect.png") });
  await editor.getByTestId("profile-detected-tencent:lol").click();
  await expect(editor.getByTestId("profile-launch-path")).toHaveValue('"C:\\WeGameApps\\英雄联盟\\WeGameLauncher\\launcher.exe"');
  await expect(list).toHaveCount(0);
});

test("game profile saves who may operate the game (computer use)", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-game").click();
  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const editor = lol.getByTestId("game-profile-editor");
  // 没写过的配置按“先问我”显示
  const select = editor.getByTestId("profile-computer-use");
  await expect(select).toHaveAttribute("data-value", "ask");
  await select.click();
  await page.getByRole("option", { name: /一直允许/ }).click();
  await expect(select).toHaveAttribute("data-value", "allow");
  await screenshot(editor, { path: testInfo.outputPath("game-profile-computer-use.png") });
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  await expect(lol).toContainText("可直接操作");
  const saved = await page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).listProfiles().then((list) => list.find((p) => p.id === "lol")));
  expect(saved?.computerUse).toBe("allow");
  expect(saved?.hotwords).toHaveLength(6);
});

test("the game list detects installed games on first open, links their profiles and can re-detect", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(() => localStorage.removeItem("kotone.installedGames"));
  await page.getByTestId("settings-nav-game").click();
  const list = page.getByTestId("installed-games");
  await expect(list.getByTestId("installed-game")).toHaveCount(3);
  await expect(list.getByTestId("installed-games-elapsed")).toContainText("ms");
  const lol = list.locator('[data-game-id="tencent:lol"]');
  await expect(lol).toContainText("英雄联盟（国服）");
  await expect(lol).toContainText("腾讯");
  // 有对应的游戏配置：点“已适配”打开它的编辑
  await lol.getByTestId("installed-game-profile").click();
  await expect(page.getByTestId("game-profile-editor")).toBeVisible();
  await list.getByTestId("installed-games-detect").click();
  await expect(list.getByTestId("installed-game")).toHaveCount(3);
});

// 控制台通道（ADR 032 决定 3）：只有声明了 consoleChat 的配置才有开关；内置配置都不带。
test("console chat switch shows only for profiles that declare it and reports the game folder", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(async () => {
    const ipc = await import("/src/lib/ipc.ts");
    await ipc.saveProfile({
      id: "dota2", displayName: "Dota 2", processNames: ["dota2.exe"], windowTitlePatterns: [],
      openChatKey: "Enter", sendKey: "Enter", preOpenDelayMs: 20, prePasteDelayMs: 20, preSendDelayMs: 100,
      preferClipboardPaste: false, hotwords: [],
      channels: [
        { id: "team", displayName: "队伍", default: true, consoleCommand: "say_team" },
        { id: "all", displayName: "所有人", openChatKey: "Shift+Enter", consoleCommand: "say" },
      ],
      consoleChat: { cfgDir: "game/dota/cfg" },
    });
  });
  await page.getByTestId("settings-nav-game").click();

  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(lol.getByTestId("game-profile-editor")).toBeVisible();
  await expect(lol.getByTestId("game-profile-console-chat")).toHaveCount(0);
  await lol.getByRole("button", { name: "收起", exact: true }).click();

  const dota = page.getByTestId("game-profile-dota2");
  await dota.getByRole("button", { name: "编辑", exact: true }).click();
  const section = dota.getByTestId("game-profile-console-chat");
  await expect(section).toContainText("autoexec.cfg");
  await expect(section).toContainText("封号风险");
  const toggle = section.getByTestId("profile-console-chat");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(section.getByTestId("profile-console-chat-status")).toContainText("game\\dota\\cfg");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(section.getByTestId("profile-console-chat-status")).toContainText("已打开");
  await expect(section.getByTestId("profile-console-chat-status")).toContainText("F23");
  await screenshot(section, { path: testInfo.outputPath("console-chat.png") });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  // 开关不改 profile：保存的配置里没有多出别的东西
  const saved = await page.evaluate(async () =>
    (await import("/src/lib/ipc.ts")).listProfiles().then((list) => list.find((p) => p.id === "dota2")));
  expect(saved?.consoleChat).toEqual({ cfgDir: "game/dota/cfg" });
});

// ADR 035 D5 → ADR 038：英雄联盟的配置里有英雄联盟插件给的一节：“读客户端数据”开关（默认开、写着风险），
// 改的是插件的设置 plugins.lol.sources.client，不随“保存”走。
test("the League profile shows the plugin's client-data switch with its risk note and saves plugins.lol.sources.client", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-game").click();

  // 别的游戏没有这个开关
  const generic = page.getByTestId("game-profile-generic");
  await generic.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(generic.getByTestId("game-profile-lol-data")).toHaveCount(0);
  // 通用配置没有进程名，不算一个游戏：没有游戏事件
  await expect(generic.getByTestId("game-profile-events")).toHaveCount(0);
  await generic.getByRole("button", { name: "收起", exact: true }).click();

  const lol = page.getByTestId("game-profile-lol");
  await lol.getByRole("button", { name: "编辑", exact: true }).click();
  const box = lol.getByTestId("game-profile-lol-data");
  await expect(box).toContainText("读客户端数据");
  await expect(box).toContainText("可能违反腾讯的用户协议");
  const toggle = box.getByTestId("profile-lol-data");
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await screenshot(box, { path: testInfo.outputPath("lol-data-switch.png") });

  // 游戏事件（ADR 036）：通用的加英雄联盟局内的；浏览器预览里没有游戏在跑
  const events = lol.getByTestId("game-profile-events");
  await expect(events.getByTestId("game-profile-events-now")).toHaveText("现在没在跑");
  await expect(events.getByTestId("game-profile-events-list")).toContainText("阶段变化");
  await expect(events.getByTestId("game-profile-events-list")).toContainText("拿了大龙");

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(settings.plugins?.lol).toEqual({ sources: { client: false } });
  // 读客户端数据关了：只剩通用的事
  await expect(events.getByTestId("game-profile-events-list")).not.toContainText("拿了大龙");
  await expect(events).toContainText("“读客户端数据”关着");
});
