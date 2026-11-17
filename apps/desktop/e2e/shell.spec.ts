import { expect, test } from "@playwright/test";

// 设置窗口外壳：标题栏、侧边栏（展开 / 图标栏 / 窄窗口抽屉）、窗口按钮。

test("sidebar collapses to an icon rail and remembers it", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  const sidebar = page.getByTestId("app-sidebar");
  await expect(sidebar).toHaveAttribute("data-rail", "false");
  await expect(sidebar.getByTestId("settings-nav-general")).toHaveText("语音输入");
  await expect(sidebar.getByTestId("agent-conversations")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("shell-expanded.png"), animations: "disabled" });

  await page.getByTestId("sidebar-toggle").click();
  await expect(sidebar).toHaveAttribute("data-rail", "true");
  await expect(sidebar.getByTestId("agent-conversations")).toHaveCount(0);
  await expect(sidebar.getByRole("button", { name: "自动化", exact: true })).toBeVisible();
  await sidebar.getByRole("button", { name: "自动化", exact: true }).click();
  await expect(page.getByTestId("automation-page")).toBeVisible();

  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toHaveAttribute("data-rail", "true");
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.getByTestId("app-sidebar")).toHaveAttribute("data-rail", "false");
});

test("narrow windows use a drawer that closes after navigating", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 520, height: 700 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toHaveCount(0);
  await page.getByTestId("sidebar-toggle").click();
  const drawer = page.getByTestId("sidebar-drawer");
  await expect(drawer).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("shell-drawer.png"), animations: "disabled" });
  await drawer.getByTestId("settings-nav-assistant").click();
  await expect(drawer).toHaveCount(0);
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  // 窄窗口下输入框与发送按钮都在视口里
  await expect(page.getByTestId("agent-send")).toBeInViewport();
});

test("a conversation in the sidebar opens the chat tab from any page", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  await page.getByTestId("settings-nav-history").click();
  await page.getByTestId("agent-conversations").getByTestId("agent-conversation").first().click();
  await expect(page.getByTestId("assistant-nav-chat")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("agent-user-message").first()).toHaveText("帮我做一个每秒按 Q 的自动化");
  await page.getByTestId("sidebar-new-chat").click();
  await expect(page.getByTestId("agent-empty")).toBeVisible();
});

test("title bar maximizes and restores the window", async ({ page }) => {
  await page.route("**/__shell-test", (route) => route.fulfill({
    contentType: "text/html", body: '<!doctype html><html><body><div id="app"></div></body></html>',
  }));
  await page.goto("/__shell-test#/settings?onboarding=never");
  await page.evaluate(async () => {
    const { mockIPC, mockWindows } = await import("/node_modules/@tauri-apps/api/mocks.js");
    const browser = await import("/src/lib/ipc/browserMock.ts");
    let maximized = false;
    const calls: string[] = [];
    Object.assign(window, { shellCalls: calls });
    mockWindows("main");
    mockIPC(async (command, args) => {
      if (command.startsWith("plugin:window|")) {
        calls.push(command);
        if (command === "plugin:window|toggle_maximize") maximized = !maximized;
        if (command === "plugin:window|is_maximized") return maximized;
        return null;
      }
      if (command.startsWith("plugin:")) return null;
      if (command === "get_startup_options") return { onboarding: "never" };
      if (command === "agent_state") return { current: null, running: null, conversations: [], notReadyReason: null, permissions: { mode: "default", rules: {} } };
      const method = command.replace(/_([a-z])/g, (_all, letter: string) => letter.toUpperCase());
      const fn = (browser as Record<string, unknown>)[method];
      return typeof fn === "function" ? (fn as (a: unknown) => unknown)(args) : null;
    }, { shouldMockEvents: true });
    await import("/src/main.ts");
  });
  const button = page.getByTestId("window-maximize");
  await expect(button).toHaveAccessibleName("最大化");
  await button.click();
  await expect(button).toHaveAccessibleName("还原");
  expect(await page.evaluate(() => (window as unknown as { shellCalls: string[] }).shellCalls)).toContain("plugin:window|toggle_maximize");
  await button.click();
  await expect(button).toHaveAccessibleName("最大化");
});
