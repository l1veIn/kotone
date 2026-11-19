import { expect, test } from "@playwright/test";

test("forced onboarding completes the full guided setup and can be reopened", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always");

  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding).toBeVisible();
  await page.getByTestId("onboarding-start-voice").click();

  await expect(page.getByTestId("onboarding-profile")).toBeVisible();
  await page.getByTestId("profile-lol").click();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  await expect(page.getByTestId("onboarding-model")).toBeVisible();
  await page.getByRole("button", { name: "下载推荐模型", exact: true }).click();
  await expect(page.getByText("✓ 已就绪", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  await expect(page.getByTestId("onboarding-hotkey")).toBeVisible();
  // 到达第三步即主动检测，不依赖用户先点击「重新录入」。
  await expect(page.getByTestId("input-environment-ready")).toBeVisible();
  await page.getByTestId("mode-push-to-talk").click();
  await page.getByRole("button", { name: "重新录入", exact: true }).click();
  await page.keyboard.press("F9");
  await expect(page.getByTestId("onboarding-hotkey").getByText("F9", { exact: true })).toBeVisible();
  // 热键设好后说一句试试：字打进框里就算成了（真机上是语音输入打进去的）
  await page.getByRole("button", { name: "完成", exact: true }).click();
  await expect(page.getByTestId("onboarding-try")).toBeVisible();
  await expect(page.getByTestId("onboarding-try-status")).toContainText("等你开口");
  await page.getByTestId("onboarding-try-box").fill("打野来下路");
  await expect(page.getByTestId("onboarding-try-status")).toContainText("成了");
  await page.getByTestId("onboarding-try-done").click();
  await expect(onboarding).toBeHidden();
  await expect(page.getByTestId("general-page")).toBeVisible();

  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-system").click();
  await page.getByRole("button", { name: "重新运行向导", exact: true }).click();
  await expect(onboarding).toBeVisible();
});

test("with one DeepSeek key the player meets 琴音 and asks her the first question", async ({ page }, testInfo) => {
  // 刚装好：一个本地模型都没有，语音识别也用 DeepSeek
  await page.goto("/#/settings?onboarding=always&mockModels=none");
  await page.screenshot({ path: testInfo.outputPath("welcome.png") });
  await page.getByTestId("onboarding-start-ai").click();
  const key = page.getByTestId("onboarding-key");
  await expect(key).toBeVisible();
  await expect(page.getByTestId("onboarding-key-start")).toBeDisabled();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await expect(page.getByTestId("onboarding-key-input")).toHaveAttribute("type", "password");
  await page.screenshot({ path: testInfo.outputPath("key.png") });
  await page.getByTestId("onboarding-key-start").click();
  // 一个 Key：琴音的连接、语音识别、读屏都用 DeepSeek
  const hello = page.getByTestId("onboarding-hello");
  await expect(hello).toBeVisible();
  await expect(hello).toContainText("嗨琴音，你能干什么？");
  await expect(page.getByTestId("onboarding-hello-status")).toContainText("等你开口");
  await page.screenshot({ path: testInfo.outputPath("hello.png") });
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
  expect(settings.assistant.connectionId).toMatch(/^deepseek-/);
  expect(settings.sttEngine).toBe("deepseek-asr");
  expect(settings.ocr.activeModelId).toBe("deepseek-ocr");
  expect(settings.ui.firstRunCompleted).toBe(true);
  // 不方便说话：打字问她，进聊天并替玩家发出这句话
  await page.getByTestId("onboarding-hello-type").click();
  await expect(page.getByTestId("onboarding")).toBeHidden();
  await expect(page.getByTestId("assistant-nav-chat")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("agent-send-error")).toContainText("仅预览");
});

test("a key that does not work is explained and the player stays on the key page", async ({ page }) => {
  await page.goto("/#/settings?onboarding=always&mockCompanionCheck=fail");
  await page.getByTestId("onboarding-start-ai").click();
  await page.getByTestId("onboarding-key-input").fill("sk-wrong");
  await page.getByTestId("onboarding-key-start").click();
  await expect(page.getByTestId("onboarding-key-error")).toBeVisible();
  await expect(page.getByTestId("onboarding-key")).toBeVisible();
});

test("model setup confirms storage before download and keeps navigation in view", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto("/#/settings?onboarding=always");

  await page.getByTestId("onboarding-start-voice").click();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  const setup = page.getByTestId("model-setup-section");
  await expect(setup).toBeVisible();
  await expect(setup.getByTestId("model-storage-section")).toBeVisible();
  await expect(setup.getByTestId("model-download-section")).toBeVisible();
  await expect(setup.locator(":scope > div")).toHaveCount(2);
  await expect(setup.locator(":scope > div").first()).toHaveAttribute(
    "data-testid",
    "model-storage-section",
  );

  await expect(page.getByTestId("onboarding-model-footer")).toBeInViewport();
  await expect(page.getByRole("button", { name: "下一步", exact: true })).toBeInViewport();
});

test("the hotkey step proactively blocks setup when the input environment is rejected", async ({
  page,
}) => {
  await page.goto("/#/settings?onboarding=always&mockInputEnvironment=blocked");

  await page.getByTestId("onboarding-start-voice").click();
  await page.getByRole("button", { name: "下一步", exact: true }).click();
  await page.getByRole("button", { name: "下载推荐模型", exact: true }).click();
  await expect(page.getByText("✓ 已就绪", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "下一步", exact: true }).click();

  await expect(page.getByTestId("input-environment-blocked")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "已加入信任区，重新检测", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "重新录入", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "完成", exact: true })).toBeDisabled();
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

  await expect(page.getByRole("heading", { name: "设置", exact: true })).toBeVisible();
  // 0.1.5 起高级页只恒显 X-ASR 模型卡：未下载时展示模型行 + 下载按钮（可恢复状态）
  await expect(page.getByText("X-ASR 流式中英标点（int8，480ms 低延迟）", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "下载", exact: true }).first()).toBeVisible();
});

test("a failed model download keeps an inline error and retry action", async ({ page }) => {
  await page.goto("/#/settings?mockDownload=fail");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-models").click();
  await page.getByRole("button", { name: "下载", exact: true }).first().click();

  const guide = page.getByTestId("manual-download-dialog");
  await expect(guide).toBeVisible();
  await expect(guide.getByText(/网络连接失败/)).toBeVisible();
  await guide.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.getByTestId("manual-download-dialog")).toHaveCount(0);
  await expect(page.getByText(/下载失败：网络连接失败/)).toBeVisible();
  await expect(page.getByRole("button", { name: "重试", exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "手动下载", exact: true }).first()).toBeVisible();
});

test("a legacy upgrade starts with voice input and preserves the reminder entry", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending");
  const notice = page.getByTestId("legacy-upgrade-notice");
  await expect(notice.getByRole("heading")).toHaveText("语音输入法的用法没有变");
  await expect(page.getByTestId("onboarding")).toHaveCount(0);
  await notice.getByRole("button", { name: "继续只用语音输入法" }).click();
  await expect(notice).toHaveCount(0);
  await expect(page.getByTestId("settings-nav-assistant")).toHaveCount(0);
  await expect(page.getByTestId("home-call")).toHaveCount(0);
  await page.getByTestId("settings-nav-legacy-reminders").click();
  await expect(page.getByTestId("legacy-reminders-page")).toBeVisible();
  const settings = await page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
  expect(settings.ui.legacyUpgrade).toBe("dismissed");
  expect(settings.assistant.hotkey).toBe("");
});

test("an upgraded user explicitly picks a separate call key and keeps the old reminder settings", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured&mockModels=none");
  const getSettings = () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings());
  const original = await getSettings();
  await page.getByTestId("legacy-upgrade-notice").getByRole("button", { name: "认识琴音" }).click();
  await expect(page.getByTestId("onboarding-key")).toBeVisible();
  await page.getByTestId("onboarding-key-input").fill("sk-test-123");
  await page.getByTestId("onboarding-key-start").click();
  await expect(page.getByTestId("onboarding-call-key")).toContainText("游戏用 ~");
  await expect(page.getByTestId("onboarding-call-key-enable")).toBeDisabled();
  expect(original.assistant.hotkey).toBe("Alt+F9");
  expect((await getSettings()).connections).toEqual(original.connections);
  await page.getByTestId("onboarding-call-key-input").fill("Menu+F9");
  await page.getByTestId("onboarding-call-key-enable").click();
  await expect(page.getByRole("alert")).toContainText("提醒助手热键");
  expect((await getSettings()).connections).toEqual(original.connections);
  await page.getByTestId("onboarding-call-key-input").fill("Ctrl+Alt+F10");
  await page.getByTestId("onboarding-call-key-enable").click();
  await expect(page.getByTestId("onboarding-hello")).toBeVisible();
  const next = await getSettings();
  expect(next.ui.legacyUpgrade).toBe("optedIn");
  expect(next.assistant.hotkey).toBe(original.assistant.hotkey);
  expect(next.assistant.connectionId).toBe(original.assistant.connectionId);
  expect(next.assistant.companionBindings).toEqual({
    hotkey: "Ctrl+Alt+F10",
    connectionId: expect.stringMatching(/^deepseek-/),
  });
  expect(next.connections).toContainEqual(original.connections[0]);
  expect(next.sttEngine).toBe(original.sttEngine);
  expect(next.activeProfileId).toBe(original.activeProfileId);
  expect(next.hotkey).toEqual(original.hotkey);

  await page.getByTestId("onboarding-hello-done").click();
  await page.getByTestId("settings-nav-legacy-reminders").click();
  await expect(page.getByTestId("legacy-reminders-page")).toBeVisible();
  await page.getByTestId("assistant-nav-settings").click();
  await expect(page.getByRole("combobox", { name: "提醒助手 AI 连接" })).toContainText("原来的提醒连接");
  await expect(page.getByTestId("hotkey-input-legacy-reminder")).toHaveValue("Alt+F9");
});

test("legacy setup discards unconfirmed keys and removes tentative connections when validation fails", async ({ page }) => {
  await page.goto("/#/settings?mockLegacyUpgrade=pending&mockLegacyReminder=configured&mockCompanionCheck=fail");
  await expect(page.getByTestId("legacy-upgrade-notice")).toBeVisible();
  const listConnections = () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).listConnections());
  const original = await listConnections();
  const open = () => page.getByTestId("legacy-upgrade-notice").getByRole("button", { name: "认识琴音" }).click();
  const key = page.getByTestId("onboarding-key-input");
  await open();
  await key.fill("sk-cancel");
  await page.getByTestId("onboarding-key-start").click();
  await page.getByRole("button", { name: "先不启用" }).click();
  await expect(page.getByTestId("legacy-upgrade-notice")).toBeVisible();
  expect(await listConnections()).toEqual(original);

  await open();
  await expect(key).toHaveValue("");
  await key.fill("sk-invalid");
  await page.getByTestId("onboarding-key-start").click();
  await page.getByTestId("onboarding-call-key-input").fill("Ctrl+Alt+F10");
  await page.getByTestId("onboarding-call-key-enable").click();
  await expect(page.getByTestId("onboarding-key-error")).toBeVisible();
  expect(await listConnections()).toEqual(original);
});
