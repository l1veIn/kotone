import { expect, test } from "@playwright/test";
import { openChat, openSettingsTab, openVoiceInput } from "./nav";

test("chat is home and keeps the draft across the role tabs and 设置; speech lives in 设置 and defaults to automatic", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1040, height: 1000 });
  await page.goto("/#/settings?onboarding=never");
  // 打开就是聊天（ADR 028），聊天从侧栏的对话进入，不再是角色页的一个分页
  await expect(page.getByTestId("assistant-tasks")).toBeVisible();
  await expect(page.getByTestId("assistant-nav-chat")).toHaveCount(0);
  await expect(page.getByTestId("assistant-settings")).toHaveCount(0);
  // 进行中的提醒与自动化只在这里显示概况（预览数据：1 条提醒等待中、1 个自动化运行中）
  await expect(page.getByTestId("assistant-open-summary")).toContainText("1 条提醒、1 个自动化");
  await page.getByTestId("assistant-request").fill("5 分钟后提醒我打野有闪");
  await page.screenshot({ path: testInfo.outputPath("assistant-tasks.png"), animations: "disabled" });
  // 角色页（角色 · 记忆 · 技能）：聊天藏起来但草稿还在
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-tasks")).not.toBeVisible();
  await expect(page.getByTestId("assistant-nav-identity")).toHaveAttribute("aria-current", "page");
  await openChat(page);
  await expect(page.getByTestId("assistant-request")).toHaveValue("5 分钟后提醒我打野有闪");
  // 她的设置在角色页的「交互」（原「设置 → 她怎么回答」），那里没有语音输入的语音设置
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("settings-reply")).toBeVisible();
  // 原来这一页上面是“她用的模型”（AI 连接）那一组，现在挪到了聊天页输入框旁，只留一行指过去
  await expect(page.getByTestId("assistant-settings")).toHaveCount(0);
  await expect(page.getByTestId("settings-reply-model-moved")).toBeVisible();
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("settings-reply.png"), animations: "disabled" });
  // 去设置转一圈回来，草稿还在
  await openChat(page);
  await expect(page.getByTestId("assistant-request")).toHaveValue("5 分钟后提醒我打野有闪");
  await page.getByTestId("assistant-open-summary").getByRole("button", { name: "在「自动化」里查看 →" }).click();
  await expect(page.getByTestId("automation-page")).toBeVisible();
  // 呼叫琴音在角色页的「唤醒」，声音与弹幕在「交互」（原「设置 → 怎么叫她 / 她怎么回答」，交接第十一节第 8 条）
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("settings-call")).toContainText("呼叫琴音");
  await openSettingsTab(page, "reply");
  const output = page.getByTestId("output-settings");
  // 默认“自动”：琴音用 DeepSeek 连接时用 DeepSeek 语音，否则系统语音（预览里没有 DeepSeek 连接）
  await expect(output.getByLabel("语音引擎", { exact: true })).toHaveAttribute("data-value", "auto");
  await expect(output.getByTestId("auto-speech-info")).toContainText("自动用 DeepSeek 语音");
  await expect(output.getByRole("button", { name: "下载语音模型", exact: true })).toHaveCount(0);
  await output.screenshot({ path: testInfo.outputPath("output-settings.png") });
});

test("呼叫琴音 has one key (default ~) and no input-mode choice; the chat offers a call button instead of push-to-talk", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 聊天输入框里是“呼叫琴音”，不是“按住说话”；浏览器预览里不可用
  await expect(page.getByTestId("call-kotone")).toContainText("呼叫琴音");
  await expect(page.getByTestId("call-kotone")).toBeDisabled();
  await expect(page.getByText("按住说话")).toHaveCount(0);
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("assistant-interaction")).toHaveCount(0);
  await expect(page.getByTestId("hotkey-input-assistant")).toHaveValue("Ctrl+Backquote");
  await expect(page.getByTestId("settings-call")).not.toContainText("陪玩热键");
  await expect(page.getByTestId("settings-call")).toContainText("呼叫键");
  // 聊天里的“去看看”跳到分组：琴音 → 交互 → 声音与弹幕；旧写法「设置 → 她怎么回答 → 声音与弹幕」也落到这里
  for (const where of ["琴音 → 交互 → 声音与弹幕", "设置 → 她怎么回答 → 声音与弹幕"]) {
    await page.getByTestId("settings-nav-game").click();
    await page.evaluate(async (w) => (await import("/src/lib/stores/navigation.ts")).openLocation(w), where);
    await expect(page.getByTestId("assistant-nav-reply"), where).toHaveAttribute("aria-current", "page");
    await expect(page.locator("#settings-reply-output"), where).toBeInViewport();
  }
});

test("assistant preview submits to the assistant instead of offering game send", async ({ page }) => {
  await page.goto("/#/overlay");
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, purpose: "assistant", state: "preview", finalText: "5 分钟后提醒我打野有闪" }));
  });
  await expect(page.getByText("Ctrl + ~ 提交助手 · Esc 重说", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "发送", exact: true })).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("发送");
  await expect(page.getByText(/提交助手/).first()).toBeVisible();
});

test("calling 琴音 shows that she listens until you stop talking (no walkie-talkie)", async ({ page }) => {
  await page.goto("/#/overlay");
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, purpose: "assistant", state: "listening", partialText: "" }));
  });
  await expect(page.getByText("在听 · 说完自动提交（再按 Ctrl + ~ 立刻提交）", { exact: true })).toBeVisible();
  await expect(page.getByText(/松开/)).toHaveCount(0);
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, partialText: "打野有闪" }));
  });
  await expect(page.getByText(/^打野有闪/)).toBeVisible();
});

test("assistant preview labels itself and never pretends to run AI", async ({ page, baseURL }, testInfo) => {
  const origin = new URL(baseURL!).origin;
  const externalRequests: string[] = [];
  page.on("request", (request) => {
    // 本机回环上的（插件 MCP App 的沙盒代理页，预览里在 127.0.0.1 的另一个端口上）不算往外发
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin && !/^(127\.0\.0\.1|localhost)$/.test(new URL(request.url()).hostname)) externalRequests.push(request.url());
  });
  await page.goto("/#/settings?onboarding=never");
  const assistant = page.getByTestId("assistant-page");
  await expect(assistant.getByRole("heading", { name: "琴音", exact: true })).toBeVisible();
  await expect(assistant.getByText(/界面预览/)).toBeVisible();
  await assistant.getByTestId("assistant-request").fill("五分钟后提醒我打野有闪");
  await assistant.getByTestId("agent-send").click();
  await expect(assistant.getByTestId("agent-send-error")).toContainText("仅预览");
  // 草稿留着，示例对话不变
  await expect(assistant.getByTestId("assistant-request")).toHaveValue("五分钟后提醒我打野有闪");
  await expect(assistant.getByTestId("agent-user-message").first()).toHaveText("帮我做一个每秒按 Q 的自动化");
  expect(externalRequests).toEqual([]);
  await page.setViewportSize({ width: 1040, height: 1150 });
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("assistant-preview.png"), fullPage: true });
});

test("assistant shares connections and saves voice/delivery settings with an independent conflict-checked hotkey", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  // 她用的模型（API 连接 + 模型）在聊天页输入框旁（2026-11-25 从「设置 → 她怎么回答」搬回来）：
  // 还没有填了 Key 的连接时，菜单里说一声，并能去「设置 → API 连接」
  await openChat(page);
  const model = page.getByTestId("composer-model");
  await expect(model).toHaveText("选模型");
  await model.click();
  await expect(page.getByTestId("composer-model-none")).toContainText("还没有填了 Key 的 API 连接");
  await page.getByTestId("composer-model-connections").click();
  await expect(page.getByTestId("advanced-nav-connections")).toHaveAttribute("aria-current", "page");
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存 Key")).toBeVisible();
  // 回到聊天，在模型菜单里选这条连接的模型：她用的连接跟着换
  await openChat(page);
  await model.click();
  await page.getByTestId("composer-model-menu").getByRole("menuitem").first().click();
  await expect.poll(async () => page.evaluate(async () => (await import("/src/lib/ipc/browserMock.ts")).getSettings().then((s) => s.assistant.connectionId))).not.toBe("");
  await expect(model).not.toHaveText("选模型");
  // 呼叫键在角色页的「唤醒」（原「设置 → 怎么叫她」）
  await openSettingsTab(page, "call");
  const talk = page.getByTestId("settings-call");
  await talk.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("CapsLock");
  await expect(page.getByText("呼叫键与语音输入热键（CapsLock）冲突，请换一个", { exact: true })).toBeVisible();
  await talk.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("Control+F9");
  await expect(talk.getByTestId("hotkey-input-assistant")).toHaveValue("Ctrl+F9");
  await expect(talk.getByTestId("hotkey-status-assistant")).toContainText("当前生效：Ctrl+F9");

  // 声音与弹幕在角色页的「交互」（原「设置 → 她怎么回答」）
  await openSettingsTab(page, "reply");
  const output = page.getByTestId("output-settings");
  await output.getByRole("checkbox", { name: "弹幕", exact: true }).uncheck();
  await expect(output.getByRole("checkbox", { name: "弹幕", exact: true })).not.toBeChecked();
  await output.getByLabel("播报音量").fill("65");
  await output.getByLabel("播报音量").dispatchEvent("change");
  await expect(output.getByText("65%", { exact: true })).toBeVisible();
  await output.getByLabel("播报语速").fill("2");
  await output.getByLabel("播报语速").dispatchEvent("change");
  await expect(output.getByText("+2", { exact: true })).toBeVisible();

  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant).toMatchObject({ hotkey: "Ctrl+F9", defaultDelivery: ["voice"], speechVolume: 65, speechRate: 2 });
  expect(saved.assistant.connectionId).not.toBe("");
  expect(saved.hotkey.key).toBe("CapsLock");

  // 语音输入的热键和呼叫键互相做冲突检测
  await openVoiceInput(page);
  const named = page.getByTestId("hotkey-input-post-process-toggle");
  await named.fill("Ctrl+F9");
  await named.press("Enter");
  await expect(page.getByText("文字处理开关热键与呼叫键（Ctrl+F9）冲突，请换一个", { exact: true })).toBeVisible();
});

test("settings modal traps Tab and restores focus after Escape", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await openVoiceInput(page, "processing");
  const trigger = page.getByTestId("manage-postprocess-pipelines");
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "管理流程" });
  const controls = dialog.locator("button:not(:disabled), input:not(:disabled)");
  await expect(controls.first()).toBeFocused();
  await controls.last().focus();
  await page.keyboard.press("Tab");
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(controls.last()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
});

test("calling her opens a call; how long it stays open and echo cancellation are set in 琴音 → 唤醒", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "call");
  const talk = page.getByTestId("settings-call");
  // ADR 033：缺省 30 秒没人说话挂断（旧配置没有这个字段）
  const idle = talk.getByTestId("call-idle");
  await expect(idle).toHaveAttribute("data-value", "30");
  await expect(idle).toContainText("30 秒（推荐）");
  await expect(talk.getByText("通话", { exact: true })).toBeVisible();
  await idle.click();
  await page.getByRole("option", { name: "不通话（一问一答）" }).click();
  await expect(idle).toHaveAttribute("data-value", "0");
  await expect(page.getByText("不通话：一问一答")).toBeVisible();
  let saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.callIdleSeconds).toBe(0);
  await idle.click();
  await page.getByRole("option", { name: "1 分钟" }).click();
  await expect(idle).toHaveAttribute("data-value", "60");
  saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.callIdleSeconds).toBe(60);
  // 回声消除缺省开，关掉存成 false
  const echo = talk.getByTestId("call-echo-cancellation");
  await expect(echo).toBeChecked();
  await echo.click();
  await expect(echo).not.toBeChecked();
  saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.echoCancellation).toBe(false);
  await talk.screenshot({ path: testInfo.outputPath("call-settings.png"), animations: "disabled" });
});
