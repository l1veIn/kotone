import { expect, test, type Page } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

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
  await screenshot(page, { path: testInfo.outputPath("mcp-cards.png"), animations: "disabled" });
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
  await screenshot(page, { path: testInfo.outputPath("mcp-trust-prompt.png"), animations: "disabled" });
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
  // 风险不在设置里摆（ADR 043：一个插件一个弹窗）：顶栏的“风险和许可”
  await expect(settings.getByTestId("remote-plugin-risk")).toHaveCount(0);
  await page.getByTestId("plugin-risk-link").click();
  const risk = page.getByTestId("plugin-risk-dialog-weather");
  await expect(risk.getByTestId("plugin-risk-weather-third-party")).toContainText("别人写的程序");
  await expect(risk.getByTestId("plugin-risk-weather-declared")).toHaveCount(0);
  await risk.getByTestId("plugin-risk-ok").click();
  await expect(risk).toHaveCount(0);
  // 没有 Kotone 清单：没有“Kotone 清单”这一节
  await expect(settings.getByTestId("remote-plugin-manifest")).toHaveCount(0);
  const key = settings.locator('[data-key="token"]');
  await expect(key).toHaveAttribute("data-filled", "false");
  await key.getByTestId("remote-plugin-key-input").fill("k-123");
  await key.getByTestId("remote-plugin-key-save").click();
  await expect(key).toHaveAttribute("data-filled", "true");
  await expect(settings.getByTestId("remote-plugin-trust")).toHaveAttribute("data-trusted", "false");
  await settings.getByTestId("remote-plugin-trust-button").click();
  await expect(settings.getByTestId("remote-plugin-trust")).toHaveAttribute("data-trusted", "true");
  await settings.getByTestId("remote-plugin-settings-reconnect").click();
  await screenshot(page, { path: testInfo.outputPath("mcp-settings.png"), animations: "disabled" });

  // 回到它的页面：连上了，一页工具列表（只读的标出来）
  await page.getByTestId("plugin-app-name").click();
  await expect(app).toHaveAttribute("data-status", "connected");
  await expect(app.getByTestId("remote-plugin-untrusted")).toHaveCount(0);
  const echo = app.locator('[data-tool="weather.echo"]');
  await expect(echo.getByTestId("remote-plugin-tool-read-only")).toBeVisible();
  await expect(app.locator('[data-tool="weather.add"]').getByTestId("remote-plugin-tool-read-only")).toHaveCount(0);
  await screenshot(page, { path: testInfo.outputPath("mcp-tools.png"), animations: "disabled" });

  // 删掉：先问，删完回到插件页
  await page.getByTestId("plugin-settings-button").click();
  await page.getByTestId("remote-plugin-remove").click();
  await page.getByTestId("remote-plugin-remove-confirm").click();
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  await expect(page.getByTestId("plugins-card-weather")).toHaveCount(0);
});

// 插件标准 0.1（ADR 047 第 1 期）：第三方插件的 Kotone 清单。它自己写的风险说明进“风险和许可”弹窗（第一次点开时弹一次）；
// 设置里“Kotone 清单”一节说清从哪来，给作者看的小字：不认识的字段、清单的问题、标准比这一版新、这台电脑上没有的游戏。
test("a third-party manifest's risk goes into the risk dialog and its unknown fields are listed for the author", async ({ page }, testInfo) => {
  await openPlugins(page, [{
    id: "zzz",
    entry: { type: "http", url: "http://127.0.0.1:23001/mcp" },
    status: "connected",
    trusted: true,
    kotoneManifest: {
      source: "server",
      local: true,
      newerStandard: true,
      unknownGames: ["zzz"],
      unknown: [{ from: "server", text: "colour" }, { from: "server", text: "watches[0].retry" }, { from: "local", text: "oops" }],
      problems: [{ from: "server", text: "events[1]：name 要是 snake_case（小写字母开头，小写字母、数字、下划线），丢掉" }],
      manifest: {
        standard: "0.2",
        games: ["zzz"],
        events: [{ name: "run_finished", label: "一条龙跑完了", data: {} }],
        risk: "一条龙会替你操作游戏窗口，跑的时候别动鼠标。 https://example.com/zzz-od",
      },
    },
  }]);
  await page.getByTestId("plugins-card-zzz-open").click();
  // 写了风险说明：第一次点开时弹一次，第三方插件那一段在前，它写的那段原样显示（里面的网址不变成链接）
  const risk = page.getByTestId("plugin-risk-dialog-zzz");
  await expect(risk).toBeVisible();
  await expect(risk.getByTestId("plugin-risk-zzz-third-party")).toContainText("别人写的程序");
  await expect(risk.getByTestId("plugin-risk-zzz-declared")).toContainText("跑的时候别动鼠标");
  await expect(risk.getByTestId("plugin-risk-zzz-declared").getByRole("button")).toHaveCount(0);
  await screenshot(page, { path: testInfo.outputPath("mcp-manifest-risk.png"), animations: "disabled" });
  await risk.getByTestId("plugin-risk-ok").click();
  await expect(risk).toHaveCount(0);

  await page.getByTestId("plugin-settings-button").click();
  const section = page.getByTestId("remote-plugin-manifest");
  await expect(section).toHaveAttribute("data-source", "server");
  await expect(section.getByTestId("remote-plugin-manifest-source")).toContainText("它自己给了一份 Kotone 清单（照插件标准 0.2 写的）");
  await expect(section.getByTestId("remote-plugin-manifest-source")).toContainText("本地 manifest.json");
  await expect(section.getByTestId("remote-plugin-manifest-newer")).toContainText("这一版 Kotone 照 0.1 读");
  await expect(section.getByTestId("remote-plugin-manifest-unknown-games")).toContainText("zzz");
  await expect(section.getByTestId("remote-plugin-manifest-unknown")).toHaveText(
    "这份清单里 Kotone 不认识的字段：colour、watches[0].retry、oops（你的 manifest.json）（插件按更新的标准写的，或者拼错了），先不用。");
  await expect(section.getByTestId("remote-plugin-manifest-problem")).toHaveCount(1);
  await expect(section.getByTestId("remote-plugin-manifest-problem")).toContainText("服务器给的：events[1]");
  await expect(section.getByTestId("remote-plugin-manifest-path")).toContainText("plugins\\zzz\\manifest.json");
  await screenshot(page, { path: testInfo.outputPath("mcp-manifest-notes.png"), animations: "disabled" });

  // 看过一次：再点开不再自己弹
  await page.getByTestId("plugin-page-back").click();
  await page.getByTestId("plugins-card-zzz-open").click();
  await expect(page.getByTestId("remote-plugin-page")).toBeVisible();
  await expect(page.getByTestId("plugin-risk-dialog-zzz")).toHaveCount(0);
});

// 审查后改的（2026-11-25）：mcp.json 里和内置插件撞名（手抄来的 `lol`）、不能当 id 的项不进插件表，在第三方一组里说原因，
// 能从 mcp.json 删掉那一项（内置的英雄联盟照常，还是一张内置卡片）；mcp.json 读不出来时说一句，不再一声不吭。
test("mcp.json entries that clash with built-in plugins are listed with the reason, not as cards", async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    (window as unknown as { __kotoneMcpFileSeed: unknown }).__kotoneMcpFileSeed = {
      problems: [
        { id: "lol", kind: "taken", error: "名字“lol”和 Kotone 里已有的插件或工具撞了，换一个" },
        { id: "Bad_Name", kind: "invalid", error: "名字“Bad_Name”不能当插件 id" },
      ],
    };
  });
  await openPlugins(page);
  const status = page.getByTestId("plugins-mcp-status");
  await expect(status).toBeVisible();
  await expect(status.locator('[data-id="lol"]')).toHaveAttribute("data-kind", "taken");
  await expect(status.locator('[data-id="lol"]')).toContainText("撞了");
  await expect(status.locator('[data-id="Bad_Name"]')).toContainText("小写字母");
  await expect(status).toContainText("mcp.json");
  await expect(page.getByTestId("plugins-mcp-file-error")).toHaveCount(0);
  // 内置的英雄联盟只有一张卡片，在内置那一组、没有“第三方”标
  await expect(page.getByTestId("plugins-card-lol")).toHaveCount(1);
  await expect(page.getByTestId("plugins-grid").getByTestId("plugins-card-lol")).toBeVisible();
  await expect(page.getByTestId("plugins-card-lol-third-party")).toHaveCount(0);
  await status.scrollIntoViewIfNeeded();
  await screenshot(page, { path: testInfo.outputPath("mcp-problems.png"), animations: "disabled" });
  // 从 mcp.json 删掉那一项：只删它，内置的照旧
  await status.locator('[data-id="lol"]').getByTestId("plugins-mcp-problem-remove").click();
  await expect(status.locator('[data-id="lol"]')).toHaveCount(0);
  await expect(page.getByTestId("plugins-grid").getByTestId("plugins-card-lol")).toBeVisible();
  await expect(page.getByTestId("plugins-card-weather")).toBeVisible();
});

test("a broken mcp.json is reported on the plugins page instead of silently dropping third-party plugins", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __kotoneMcpFileSeed: unknown }).__kotoneMcpFileSeed = {
      fileError: "mcp.json 不是合法的 JSON：expected `,` or `}` at line 3 column 5",
    };
  });
  await openPlugins(page, []);
  const error = page.getByTestId("plugins-mcp-file-error");
  await expect(error).toBeVisible();
  await expect(error).toContainText("mcp.json 读不出来");
  await expect(error).toContainText("line 3 column 5");
  await page.getByTestId("plugins-mcp-reload").click();
  await expect(error).toBeVisible();
});
