// 语音输入的推送卡片和识别记录（ADR 043 D10）：
// - 聊天里“发出去的话”（连着发的几句并在一张卡片里、原话另起一行小字）和“没发出去”（原因 + 重发）；点“重发”调插件给页面的
//   工具 voice-input.resend（带这一句的 id 和当时的频道），不打开右侧栏；推送原地更新以后卡片写“已重发到「…」”；
// - 插件页「设置 → 推送」列出这两条；
// - 识别记录的列表和详情写着发到了哪个游戏、哪个频道（旧记录没有就不写）。
// 聊天那一段用 @tauri-apps/api/mocks 模拟 agent_* 命令和 kotone://push 事件（照 apps/desktop/e2e/push.spec.ts），不接任何 AI；
// 插件的命令交给浏览器预览的 pluginsMock，只有 voice-input.resend 由这里回答并记下参数。真的发到游戏由 Rust 测试管
// （plugins/voice-input/src/tests.rs：经本体的入口、回到原来的窗口和频道、strict_target）。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { join } from "node:path";
import { screenshot } from "../../../apps/desktop/e2e/screenshot.cjs";

/** 截图放在仓库的 target/plugin-shots/round9/（实施记录用）；testDir 是 plugins/。 */
const shot = (testInfo: TestInfo, name: string) => join(testInfo.project.testDir, "..", "target", "plugin-shots", "round9", `${name}.png`);

interface Bridge {
  calls: { tool: string; args: Record<string, unknown> }[];
  push(event: unknown): Promise<void>;
  failed: { role: "notice"; atMs: number; text: string; push: Record<string, unknown> };
}
declare global {
  interface Window {
    voicePushTest: Bridge;
  }
}

const T = 1_800_000_000_000;
const SENT_KEY = `sent:s1:${T - 120_000}`;
const FAILED_KEY = "failed:s3";

async function openChat(page: Page) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.route("**/__voice-push-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__voice-push-test#/settings?onboarding=never");
  await page.evaluate(async ({ T, SENT_KEY, FAILED_KEY }) => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    const plugins = await import("/src/lib/ipc/pluginsMock.ts");
    // i18n-ignore-start：示例对话
    const push = (trigger: string, key: string, title: string, uri: string, data: unknown) => ({
      plugin: "voice-input", pluginName: "语音输入", trigger, title, key,
      card: { title, ui: { uri }, structuredContent: data },
    });
    const sent = {
      role: "notice" as const, text: "语音输入 推送：发出去的话", atMs: T - 120_000,
      push: push("sent", SENT_KEY, "发出去的话", "ui://kotone/voice-input/sent", {
        game: { id: "lol", name: "英雄联盟" }, channel: { id: "team", name: "队伍" },
        lines: [
          { text: "打野来中路", source: "打野来中路吧", atMs: T - 120_000 },
          { text: "小龙刷了", atMs: T - 60_000 },
        ],
      }),
    };
    const failedData = {
      id: "s3", text: "我去下路", reason: "Ctrl、Alt、Shift 或 Win 一直按着（等了 1 秒），这次没发：松开再发",
      code: "MODIFIERS_STILL_DOWN", atMs: T - 30_000, attempts: 1, channel: { game: "lol", id: "all" },
    };
    const failed = {
      role: "notice" as const, text: "语音输入 推送：没发出去", atMs: T - 30_000,
      push: push("failed", FAILED_KEY, "没发出去", "ui://kotone/voice-input/failed", failedData),
    };
    const conversation = {
      version: 1, id: "c1", title: "这局", createdAtMs: T - 600_000, updatedAtMs: T - 30_000,
      messages: [
        { role: "user", text: "在吗", source: "chat", atMs: T - 600_000 },
        { role: "assistant", text: "在的。", atMs: T - 599_000 },
        sent, failed,
      ],
    };
    // i18n-ignore-end
    const state = {
      current: "c1", running: null, notReadyReason: null,
      conversations: [{ id: "c1", title: conversation.title, createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs, messageCount: conversation.messages.length }],
    };
    const bridge = {
      calls: [] as { tool: string; args: Record<string, unknown> }[],
      failed,
      async push(event: unknown) {
        await emit("kotone://push", event);
      },
    };
    Object.assign(window, { voicePushTest: bridge });
    await browser.updateSettings({ assistant: { connectionId: "test-ai" } });
    mockWindows("settings");
    mockIPC(async (command, args) => {
      const a = (args ?? {}) as Record<string, never>;
      switch (command) {
        case "agent_state": return structuredClone({ ...state, permissions: { mode: "default", rules: {} } });
        case "agent_conversation": return structuredClone(conversation);
        case "agent_held_cards": return [];
        case "automation_runs": return { runs: [] };
        case "usage_conversation": return null;
        case "list_connections": return [{ id: "test-ai", displayName: "测试连接", provider: "test", kind: "remote", baseUrl: "https://example.invalid", model: "test", hasApiKey: true }];
        case "get_startup_options": return { onboarding: "never" };
        case "update_settings": return browser.updateSettings((a as { patch: never }).patch);
        case "plugins_list": return plugins.listPlugins();
        case "plugin_call": {
          const { plugin, tool, arguments: toolArgs } = a as { plugin: string; tool: string; arguments: Record<string, unknown> };
          if (tool === "voice-input.resend") {
            bridge.calls.push({ tool, args: toolArgs });
            return { content: { status: "sent", game: "lol", gameName: "英雄联盟", channel: "all", channelName: "所有人" } }; // i18n-ignore
          }
          return plugins.pluginCall(plugin, tool, toolArgs);
        }
      }
      if (command.startsWith("plugin:")) return null;
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as () => unknown)() : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  }, { T, SENT_KEY, FAILED_KEY });
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
}

const pushAt = (page: Page, key: string) => page.locator(`[data-testid="push-item"][data-key="${key}"]`);

test("the chat shows 发出去的话 with merged lines and 没发出去 with the reason and 重发, which resends through the plugin tool", async ({ page }, testInfo) => {
  await openChat(page);
  const sent = pushAt(page, SENT_KEY);
  await expect(sent.getByTestId("push-from")).toContainText("语音输入 推送");
  await expect(sent.getByTestId("push-title")).toHaveText("发出去的话");
  const sentCard = sent.getByTestId("voice-sent-card");
  await expect(sentCard.getByTestId("voice-sent-to")).toHaveText("你向「队伍」发了：");
  await expect(sentCard.getByTestId("voice-sent-game")).toHaveText("英雄联盟");
  const lines = sentCard.getByTestId("voice-sent-line");
  await expect(lines).toHaveCount(2);
  await expect(lines.nth(0)).toContainText("打野来中路");
  await expect(lines.nth(0).getByTestId("voice-sent-source")).toHaveText("原话：打野来中路吧");
  await expect(lines.nth(1)).toContainText("小龙刷了");
  await expect(lines.nth(1).getByTestId("voice-sent-source")).toHaveCount(0);

  const failed = pushAt(page, FAILED_KEY);
  await expect(failed.getByTestId("push-title")).toHaveText("没发出去");
  const failedCard = failed.getByTestId("voice-failed-card");
  await expect(failedCard.getByTestId("voice-failed-text")).toHaveText("我去下路");
  await expect(failedCard.getByTestId("voice-failed-reason")).toContainText("一直按着");
  const resend = failedCard.getByTestId("voice-failed-resend");
  await expect(resend).toHaveText("重发");
  await failed.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "voice-input-push-cards") });

  // 点“重发”：调插件给页面的工具（这一句的 id、当时的频道），卡片不打开右侧栏
  await resend.click();
  await expect.poll(() => page.evaluate(() => window.voicePushTest.calls)).toEqual([
    { tool: "voice-input.resend", args: { id: "s3", channel: { game: "lol", id: "all" } } },
  ]);
  await expect(failedCard.getByTestId("voice-failed-resent")).toHaveText("已重发到「所有人」");
  await expect(page.getByTestId("right-sidebar").getByTestId("sidebar-plugin-page")).toHaveCount(0);

  // 推送原地更新（后端标成已重发）：还是这一张，写着已重发
  await page.evaluate(async (FAILED_KEY) => {
    const t = window.voicePushTest;
    const message = structuredClone(t.failed);
    const card = (message.push as { card: { structuredContent: Record<string, unknown> } }).card;
    card.structuredContent.resent = { game: { id: "lol", name: "英雄联盟" }, channel: { id: "all", name: "所有人" }, atMs: 1 }; // i18n-ignore
    await t.push({ kind: "updated", conversationId: "c1", plugin: "voice-input", trigger: "failed", key: FAILED_KEY, held: false, alert: false, message });
  }, FAILED_KEY);
  await expect(page.locator(`[data-testid="push-item"][data-key="${FAILED_KEY}"]`)).toHaveCount(1);
  await expect(failed.getByTestId("voice-failed-card")).toHaveAttribute("data-state", "resent");
  await expect(failed.getByTestId("voice-failed-resend")).toHaveCount(0);
  await failed.scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "voice-input-push-resent") });

  // 卡片本身点了照旧在右侧栏打开（同一个组件）
  await sent.getByTestId("plugin-card-open").click();
  const sidebar = page.getByTestId("right-sidebar");
  await expect(sidebar.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", "ui://kotone/voice-input/sent");
  await expect(sidebar.getByTestId("voice-sent-line")).toHaveCount(2);
});

test("the plugin page lists the two voice input pushes, on by default", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-voice-input-open").click();
  await page.getByTestId("plugin-settings-button").click();
  for (const [id, title] of [["sent", "发出去的话"], ["failed", "没发出去"]]) {
    const row = page.getByTestId(`plugin-voice-input-push-${id}-row`);
    await expect(row).toContainText(title);
    await expect(row.getByTestId("plugin-push-form")).toHaveText("卡片");
    await expect(page.getByTestId(`plugin-voice-input-push-${id}`)).toBeChecked();
  }
});

test("the history shows which game and channel a line went to, and old records show nothing", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await page.getByTestId("plugins-card-voice-input-open").click();
  await page.getByTestId("voice-tab-history").click();
  const records = page.getByTestId("history-record");
  await expect(records).toHaveCount(2);
  await expect(records.nth(0).getByTestId("history-record-sent-to")).toHaveText("· 英雄联盟 · 队伍");
  await expect(records.nth(1).getByTestId("history-record-sent-to")).toHaveCount(0);
  await records.nth(0).scrollIntoViewIfNeeded();
  await screenshot(page, { path: shot(testInfo, "voice-input-history-sent-to") });
  await records.nth(0).getByRole("button", { name: "详情" }).click();
  await expect(page.getByTestId("history-detail-sent-to")).toHaveText("英雄联盟 · 队伍");
});
