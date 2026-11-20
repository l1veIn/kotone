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
  // 她的设置在「设置 → 她怎么回答」，那里没有语音输入的语音设置
  await openSettingsTab(page, "reply");
  await expect(page.getByTestId("assistant-settings")).toBeVisible();
  await expect(page.getByTestId("assistant-settings").getByTestId("speech-settings")).toHaveCount(0);
  await page.getByRole("main").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("assistant-settings.png"), animations: "disabled" });
  // 去设置转一圈回来，草稿还在
  await openChat(page);
  await expect(page.getByTestId("assistant-request")).toHaveValue("5 分钟后提醒我打野有闪");
  await page.getByTestId("assistant-open-summary").getByRole("button", { name: "在「自动化」里查看 →" }).click();
  await expect(page.getByTestId("automation-page")).toBeVisible();
  // 呼叫琴音在「设置 → 怎么叫她」，声音与弹幕在「设置 → 她怎么回答」（ADR 028 D11）
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
  await expect(page.getByTestId("hotkey-input-assistant")).toHaveValue("Backquote");
  await expect(page.getByTestId("settings-call")).not.toContainText("陪玩热键");
  await expect(page.getByTestId("settings-call")).toContainText("呼叫键");
  // 聊天里的“去看看”跳到分组：设置 → 她怎么回答 → 游戏语音
  await page.evaluate(async () => (await import("/src/lib/stores/navigation.ts")).openLocation("设置 → 她怎么回答 → 游戏语音"));
  await expect(page.getByTestId("advanced-nav-reply")).toHaveAttribute("aria-current", "page");
  await expect(page.locator("#settings-reply-teammates")).toBeInViewport();
});

test("assistant preview submits to the assistant instead of offering game send", async ({ page }) => {
  await page.goto("/#/overlay");
  await page.evaluate(async () => {
    const { appState } = await import("/src/lib/stores/state.ts");
    appState.update((state) => ({ ...state, purpose: "assistant", state: "preview", finalText: "5 分钟后提醒我打野有闪" }));
  });
  await expect(page.getByText("~ 提交助手 · Esc 重说", { exact: true })).toBeVisible();
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
  await expect(page.getByText("在听 · 说完自动提交（再按 ~ 立刻提交）", { exact: true })).toBeVisible();
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
    if (/^https?:/.test(request.url()) && new URL(request.url()).origin !== origin) externalRequests.push(request.url());
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
  // AI 连接在「设置 → 她怎么回答」
  await openSettingsTab(page, "reply");
  const reply = page.getByTestId("settings-reply");
  await reply.getByRole("button", { name: "管理连接 →", exact: true }).click();
  await expect(page.getByTestId("advanced-nav-connections")).toHaveAttribute("aria-current", "page");
  await page.getByTestId("connection-preset-dashscope").click();
  await page.getByTestId("connection-api-key").fill("sk-test-local-fixture");
  await page.getByTestId("connection-save").click();
  await expect(page.getByText("已保存密钥")).toBeVisible();
  await openSettingsTab(page, "reply");
  await reply.getByLabel("陪玩 AI 连接").click();
  await page.getByRole("option").nth(1).click();
  await expect(reply.getByRole("button", { name: "管理连接 →", exact: true })).toHaveCount(0);
  // 呼叫键在「设置 → 怎么叫她」
  await openSettingsTab(page, "call");
  const talk = page.getByTestId("settings-call");
  await talk.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("CapsLock");
  await expect(page.getByText("呼叫键与语音输入热键（CapsLock）冲突，请换一个", { exact: true })).toBeVisible();
  await talk.getByTestId("hotkey-capture-assistant").click();
  await page.keyboard.press("Control+F9");
  await expect(talk.getByTestId("hotkey-input-assistant")).toHaveValue("Ctrl+F9");
  await expect(talk.getByTestId("hotkey-status-assistant")).toContainText("当前生效：Ctrl+F9");

  // 声音与弹幕在「设置 → 她怎么回答」
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

test("after answering she keeps listening for a few seconds; how long is set in 设置 → 怎么叫她", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await openSettingsTab(page, "call");
  const talk = page.getByTestId("settings-call");
  // ADR 029 §五：缺省 8 秒（旧配置没有这个字段）
  const followUp = talk.getByTestId("call-follow-up");
  await expect(followUp).toHaveAttribute("data-value", "8");
  await expect(followUp).toContainText("8 秒（推荐）");
  await expect(talk.getByText("回答完接着听", { exact: true })).toBeVisible();
  await followUp.click();
  await page.getByRole("option", { name: "不接着听" }).click();
  await expect(followUp).toHaveAttribute("data-value", "0");
  await expect(page.getByText("回答完不再接着听")).toBeVisible();
  let saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.followUpSeconds).toBe(0);
  await followUp.click();
  await page.getByRole("option", { name: "12 秒" }).click();
  await expect(followUp).toHaveAttribute("data-value", "12");
  saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant.followUpSeconds).toBe(12);
  await talk.screenshot({ path: testInfo.outputPath("call-follow-up.png"), animations: "disabled" });
});
