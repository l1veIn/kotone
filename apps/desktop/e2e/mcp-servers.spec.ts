import { expect, test, type Page } from "@playwright/test";

// 第三方插件（MCP 方案第 2 期）的界面：插件页上的卡片和开关、“加 MCP 服务器”（粘贴配置、从 Claude Desktop 导入）
// 和信任提示、点开以后的状态和工具列表、设置里填 Key / 信任 / 删掉。浏览器预览用 ipc/mcpServersMock（不连任何服务器）；
// 页面加载前放 `window.__kotoneMcpSeed` 当作手改 mcp.json 加的服务器（没信任、缺 Key）。

const WEATHER = {
  id: "weather",
  entry: { type: "http", url: "https://weather.example.com/mcp", headers: { Authorization: "Bearer ${input:token}" } },
  status: "failed",
  error: "还没填 Key：token",
  trusted: false,
};

async function openPlugins(page: Page, seed: unknown[] = [WEATHER]) {
  await page.addInitScript((servers) => {
    (window as unknown as { __kotoneMcpSeed: unknown }).__kotoneMcpSeed = servers;
  }, seed);
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-plugins").click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
}

test("third-party cards sit in their own group with a badge, status, fallback logo and the same switch", async ({ page }, testInfo) => {
  await openPlugins(page);
  const group = page.getByTestId("plugins-grid-third-party");
  const card = group.getByTestId("plugins-card-weather");
  await expect(card).toBeVisible();
  await expect(page.getByTestId("plugins-grid").getByTestId("plugins-card-weather")).toHaveCount(0);
  await expect(card.getByTestId("plugins-card-weather-third-party")).toHaveText("第三方");
  await expect(card.getByTestId("plugins-card-weather-status")).toHaveText("还没填 Key");
  await expect(card.getByTestId("plugins-card-weather-logo")).toHaveAttribute("data-logo", "fallback");
  await expect(card).toContainText("https://weather.example.com/mcp");
  // 开关就是设置 plugins.<id>.enabled
  await card.getByTestId("plugins-card-weather-enabled").click();
  await expect(card).toHaveAttribute("data-enabled", "false");
  await card.getByTestId("plugins-card-weather-enabled").click();
  await expect(card).toHaveAttribute("data-enabled", "true");
  await page.screenshot({ path: testInfo.outputPath("mcp-cards.png"), animations: "disabled" });
});

test("pasting a config shows the trust prompt (program, package, keys moving out) before anything is added", async ({ page }, testInfo) => {
  await openPlugins(page, []);
  await expect(page.getByTestId("plugins-grid-third-party")).toHaveCount(0);
  await page.getByTestId("plugins-add-mcp").click();
  const dialog = page.getByTestId("mcp-add-dialog");
  await dialog.getByTestId("mcp-add-paste").fill(JSON.stringify({
    mcpServers: {
      github: { command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: { GITHUB_TOKEN: "ghp_secret" } },
      old: { type: "sse", url: "https://x.example.com/sse" },
    },
  }));
  await dialog.getByTestId("mcp-add-read").click();
  const candidates = dialog.getByTestId("mcp-add-candidate");
  await expect(candidates).toHaveCount(2);
  const old = dialog.locator('[data-name="old"]');
  await expect(old.getByTestId("mcp-add-candidate-error")).toBeVisible();
  await expect(old.getByTestId("mcp-add-candidate-check")).toBeDisabled();
  const github = dialog.locator('[data-name="github"]');
  await expect(github.getByTestId("mcp-add-candidate-id")).toHaveValue("github");
  await github.getByTestId("mcp-add-candidate-id").fill("gh");
  await dialog.getByTestId("mcp-add-next").click();

  // 信任提示：风险、运行什么、哪个包、Key 搬进安全存储（值不出现）
  await expect(dialog.getByTestId("mcp-trust-risk")).toContainText("别人写的程序");
  const item = dialog.getByTestId("mcp-trust-item");
  await expect(item).toHaveCount(1);
  await expect(item).toHaveAttribute("data-id", "gh");
  await expect(item.getByTestId("mcp-trust-facts-command")).toHaveText("npx -y @modelcontextprotocol/server-github");
  await expect(item.getByTestId("mcp-trust-facts-package")).toContainText("@modelcontextprotocol/server-github");
  await expect(item.getByTestId("mcp-trust-facts-moved")).toContainText("GITHUB_TOKEN");
  await expect(dialog).not.toContainText("ghp_secret");
  await page.screenshot({ path: testInfo.outputPath("mcp-trust-prompt.png"), animations: "disabled" });
  // 还没点“信任并添加”：什么都没加
  await expect(page.getByTestId("plugins-card-gh")).toHaveCount(0);
  await dialog.getByTestId("mcp-trust-confirm").click();
  await expect(dialog).toHaveCount(0);
  const card = page.getByTestId("plugins-grid-third-party").getByTestId("plugins-card-gh");
  await expect(card).toBeVisible();
  await expect(card.getByTestId("plugins-card-gh-status")).toHaveText("连上了");
});

test("importing from Claude Desktop lists its servers with the file they came from", async ({ page }) => {
  await openPlugins(page, []);
  await page.getByTestId("plugins-add-mcp").click();
  const dialog = page.getByTestId("mcp-add-dialog");
  await dialog.getByTestId("mcp-add-tab-claude-desktop").click();
  await expect(dialog).toContainText("claude_desktop_config.json");
  const time = dialog.locator('[data-name="time"]');
  await expect(time).toBeVisible();
  await expect(dialog.locator('[data-name="github"]')).toBeVisible();
  await expect(dialog.getByTestId("mcp-add-next")).toBeDisabled();
  await time.getByTestId("mcp-add-candidate-check").check();
  await dialog.getByTestId("mcp-add-next").click();
  await expect(dialog.getByTestId("mcp-trust-facts-package")).toContainText("mcp-server-time");
  await dialog.getByTestId("mcp-trust-confirm").click();
  await expect(page.getByTestId("plugins-card-time")).toBeVisible();
  // VS Code 那一份
  await page.getByTestId("plugins-add-mcp").click();
  await dialog.getByTestId("mcp-add-tab-vscode").click();
  await expect(dialog.locator('[data-name="weather"]')).toBeVisible();
});

test("a third-party plugin opens to its status and tool list; settings fill the key, trust it and remove it", async ({ page }, testInfo) => {
  await openPlugins(page);
  await page.getByTestId("plugins-card-weather-open").click();
  const app = page.getByTestId("remote-plugin-page");
  await expect(app).toBeVisible();
  await expect(app.getByTestId("remote-plugin-needs-key")).toContainText("token");
  await expect(app.getByTestId("remote-plugin-untrusted")).toBeVisible();
  await expect(app.getByTestId("remote-plugin-error")).toContainText("还没填 Key");
  await expect(app.getByTestId("remote-plugin-no-tools")).toBeVisible();

  // 设置：顶栏的“设置”（同一个容器）
  await app.getByTestId("remote-plugin-open-settings").click();
  const settings = page.getByTestId("remote-plugin-settings");
  await expect(settings.getByTestId("remote-plugin-facts-url")).toHaveText("https://weather.example.com/mcp");
  await expect(settings.getByTestId("remote-plugin-risk")).toContainText("别人写的程序");
  const key = settings.locator('[data-key="token"]');
  await expect(key).toHaveAttribute("data-filled", "false");
  await key.getByTestId("remote-plugin-key-input").fill("k-123");
  await key.getByTestId("remote-plugin-key-save").click();
  await expect(key).toHaveAttribute("data-filled", "true");
  await expect(settings.getByTestId("remote-plugin-trust")).toHaveAttribute("data-trusted", "false");
  await settings.getByTestId("remote-plugin-trust-button").click();
  await expect(settings.getByTestId("remote-plugin-trust")).toHaveAttribute("data-trusted", "true");
  await settings.getByTestId("remote-plugin-settings-reconnect").click();
  await page.screenshot({ path: testInfo.outputPath("mcp-settings.png"), animations: "disabled" });

  // 回到它的页面：连上了，一页工具列表（只读的标出来）
  await page.getByTestId("plugin-app-name").click();
  await expect(app).toHaveAttribute("data-status", "connected");
  await expect(app.getByTestId("remote-plugin-untrusted")).toHaveCount(0);
  const echo = app.locator('[data-tool="weather.echo"]');
  await expect(echo.getByTestId("remote-plugin-tool-read-only")).toBeVisible();
  await expect(app.locator('[data-tool="weather.add"]').getByTestId("remote-plugin-tool-read-only")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("mcp-tools.png"), animations: "disabled" });

  // 删掉：先问，删完回到插件页
  await page.getByTestId("plugin-settings-button").click();
  await page.getByTestId("remote-plugin-remove").click();
  await page.getByTestId("remote-plugin-remove-confirm").click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  await expect(page.getByTestId("plugins-card-weather")).toHaveCount(0);
});
