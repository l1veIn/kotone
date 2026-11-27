// MCP App 的 `ui/open-link`（共享资源评估第二节第 5 条，2026-11-25）：App 请宿主打开一个外部网址时，宿主先在 Kotone 窗口里问玩家，
// 玩家点了「打开」才交给系统浏览器；点「取消」回 isError（规范的“Link opening denied by user”）。照 ext-apps AppBridge
// `onopenlink` 文档的例子（先弹“Open external link?”再打开）和 Claude 的做法。
// 原来直接打开：App 在对局中（聊天里的卡片在后台加载着）调一下，浏览器就跳出来盖住游戏。
// App 用英雄联盟的（浏览器预览里由 vite.config.ts 的 mcpAppsDevServer 给，test:e2e 先打包）。
// `ui://kotone/<本插件>/<页面>` 是 Kotone 里面的页面，照旧直接打开（plugins/lol/e2e/lol-app.spec.ts 管）。
import { expect, test, type Frame, type Page } from "@playwright/test";
import { openPlugin } from "./nav";

const HOME = "ui://kotone/lol/home";
const LINK = "https://op.gg/zh-cn/lol/champions/yasuo/build";

/** 浏览器预览的 openExternal 是 window.open：记下来，不真的开。 */
async function open(page: Page) {
  await page.addInitScript(() => {
    const opened: string[] = [];
    (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened = opened;
    window.open = ((url?: string | URL) => {
      opened.push(String(url));
      return null;
    }) as typeof window.open;
  });
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  const pluginPage = await openPlugin(page, "lol");
  await expect(pluginPage.locator(`[data-testid="mcp-app-frame"][data-uri="${HOME}"]`)).toHaveAttribute("data-phase", "ready", { timeout: 15_000 });
  return appFrame(page);
}

const opened = (page: Page) => page.evaluate(() => (window as unknown as { __kotoneOpened: string[] }).__kotoneOpened);

/** App 所在的那一层 iframe（沙盒代理页里面那一层）。 */
async function appFrame(page: Page): Promise<Frame> {
  await expect.poll(() => page.frames().filter((f) => /\/app\/lol\/home$/.test(f.url())).length, { timeout: 15_000 }).toBeGreaterThan(0);
  return page.frames().find((f) => /\/app\/lol\/home$/.test(f.url()))!;
}

/** 在 App 那一层发一条 JSON-RPC 请求给宿主（经沙盒代理页转），等回来的那一条。 */
function rpc(frame: Frame, id: number, method: string, params: unknown): Promise<{ result?: Record<string, unknown>; error?: { message: string } }> {
  return frame.evaluate(([id, method, params]) => new Promise((resolve) => {
    const listener = (event: MessageEvent) => {
      if (event.data?.id === id) {
        window.removeEventListener("message", listener);
        resolve(event.data);
      }
    };
    window.addEventListener("message", listener);
    window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
  }), [id, method, params] as const);
}

test("an App's external link asks the player in Kotone first: cancel opens nothing (isError), open hands it to the browser", async ({ page }) => {
  const app = await open(page);

  // App 请求打开：先问，什么都还没打开，请求也还没回
  let settled = false;
  const cancelled = rpc(app, 91_001, "ui/open-link", { url: LINK }).then((r) => { settled = true; return r; });
  const dialog = page.getByTestId("mcp-app-link-dialog");
  // 宿主处理完这个请求（弹了框，或者直接打开了）再看
  await expect.poll(async () => settled || (await opened(page)).length > 0 || (await dialog.isVisible())).toBe(true);
  expect(await opened(page), "nothing opens before the player says so").toEqual([]);
  expect(settled, "the request waits for the player").toBe(false);
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("mcp-app-link-url")).toHaveText(LINK);
  await expect(dialog).toContainText("英雄联盟");
  await dialog.getByTestId("mcp-app-link-cancel").click();
  expect((await cancelled).result?.isError).toBe(true);
  await expect(dialog).toBeHidden();
  expect(await opened(page)).toEqual([]);

  // 再来一次，这次点「打开」
  const accepted = rpc(app, 91_002, "ui/open-link", { url: LINK });
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("mcp-app-link-open").click();
  expect((await accepted).result?.isError).toBeFalsy();
  await expect(dialog).toBeHidden();
  expect(await opened(page)).toEqual([LINK]);

  // 已经在问的时候又来一个：不叠第二个框，后来的直接回 isError；Esc 关掉 = 取消
  const first = rpc(app, 91_003, "ui/open-link", { url: LINK });
  await expect(dialog).toBeVisible();
  expect((await rpc(app, 91_004, "ui/open-link", { url: "https://example.com/" })).result?.isError).toBe(true);
  await expect(dialog.getByTestId("mcp-app-link-url")).toHaveText(LINK);
  await page.keyboard.press("Escape");
  expect((await first).result?.isError).toBe(true);
  expect(await opened(page)).toEqual([LINK]);
});
