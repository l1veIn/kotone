import { expect, test, type Page } from "@playwright/test";
import { openChat, openSettingsTab } from "./nav";

// `ui.open`（她和外部 Agent 在 Kotone 窗口里打开一个页面，ADR 038 D5）不打断玩家正在看的东西
// （共享资源评估第二节第 6 条，2026-11-25；ADR 039 实施记录“ui.open 不打断玩家”）：
// - 玩家在聊天页，右侧栏空着（收着、停在“卡片”、“网页”里没开网页）或者显示的就是这个插件的页面：照旧直接显示
//   （同一个插件的下一页往上叠，能返回）；设置页照旧跳过去（她带你过去）。
// - 玩家在别的页面，或者右侧栏正显示别的（另一个插件的页面、网页、展开看的后台的事）：不切页、不换掉，
//   放进右侧栏“卡片”最上面等玩家点，窗口里出一个不抢焦点的提示，点“打开”才打开。
// - 同一个页面反复打开只算一次（按地址和参数去重，和自动卡片按 key 去重一样）：不重叠、不再提示。
// 浏览器预览里没有后端事件：用 window.kotoneUiOpen（真窗口里 kotone://ui-open 走同一个处理函数，返回值交回后端，
// 工具结果里的 status 就是它）。

type Outcome = "shown" | "waiting" | null;
const uiOpen = (page: Page, event: Record<string, unknown>) =>
  page.evaluate((e) => (window as unknown as { kotoneUiOpen: (e: unknown) => Outcome }).kotoneUiOpen(e), event);

const INTERPRETER = "ui://kotone/interpreter/home";
const GAME_VOICE = "ui://kotone/game-voice/home";

async function open(page: Page) {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/settings?onboarding=never");
  await expect(page.getByTestId("assistant-chat")).toBeVisible();
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "cards");
}

const toasts = (page: Page, text: string) => page.locator("[data-sonner-toast]").filter({ hasText: text });

test("on another page ui.open does not switch pages: the page waits in the right sidebar and a notice offers to open it", async ({ page }) => {
  await open(page);
  await openSettingsTab(page, "sound");
  const focused = await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 80));
  expect(await uiOpen(page, { uri: INTERPRETER, plugin: "interpreter" })).toBe("waiting");
  // 还在「设置 → 声音」，没被切到聊天页
  await expect(page.getByTestId("advanced-nav-sound")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-chat")).toBeHidden();
  // 窗口里一个提示（不抢焦点：焦点还在原处）
  const notice = toasts(page, "同传");
  await expect(notice).toHaveCount(1);
  await expect(notice).toContainText("右侧栏");
  expect(await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 80))).toBe(focused);
  // 同一个页面再开：只算一次（还是一个提示，卡片里也只有一张）
  expect(await uiOpen(page, { uri: INTERPRETER, plugin: "interpreter" })).toBe("waiting");
  await expect(toasts(page, "同传")).toHaveCount(1);
  // 设置页也不跳：玩家在「插件」时打开「设置 → 模型」
  await page.getByTestId("settings-nav-plugins").click();
  expect(await uiOpen(page, { uri: "ui://kotone/settings/models" })).toBe("waiting");
  await expect(page.getByTestId("plugins-page")).toBeVisible();
  // 提示只有一个：新的替换旧的（照消息提示）
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(1);
  // 玩家点提示上的“打开”：才跳过去
  await toasts(page, "设置 · 模型").getByRole("button", { name: "打开" }).click();
  await expect(page.getByTestId("advanced-nav-models")).toHaveAttribute("aria-current", "page");
  // 同传还在聊天页右侧栏的“卡片”最上面等着：玩家回到聊天页，点它的“打开”才在右侧栏打开
  await openChat(page);
  const waiting = page.getByTestId("ui-open-waiting");
  await expect(waiting).toHaveCount(1);
  await expect(waiting).toHaveAttribute("data-uri", INTERPRETER);
  await expect(waiting).toContainText("同传");
  await waiting.getByTestId("ui-open-waiting-open").click();
  await expect(page.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", INTERPRETER);
  await expect(waiting).toHaveCount(0);
});

test("on the chat page with the sidebar free ui.open shows the page; the same plugin stacks; another plugin's page waits instead of replacing it", async ({ page }) => {
  await open(page);
  // 右侧栏停在“卡片”：直接显示
  expect(await uiOpen(page, { uri: "ui://kotone/lol/tier", params: { mode: "rift", lane: "中" }, plugin: "lol" })).toBe("shown");
  const sidebar = page.getByTestId("right-sidebar");
  const stack = page.getByTestId("sidebar-plugin-page");
  await expect(sidebar).toHaveAttribute("data-tab", "page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/tier");
  // 同一个页面再开：只算一次（不重新叠、不重新挂）
  expect(await uiOpen(page, { uri: "ui://kotone/lol/tier", params: { mode: "rift", lane: "中" }, plugin: "lol" })).toBe("shown");
  await expect(stack).toHaveAttribute("data-depth", "1");
  // 显示的就是英雄联盟：它的另一页往上叠，“返回”回到刚才在看的强度榜
  expect(await uiOpen(page, { uri: "ui://kotone/lol/champion", params: { champion: "亚索", mode: "rift" }, plugin: "lol" })).toBe("shown");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/champion");
  await expect(stack).toHaveAttribute("data-depth", "2");
  // 另一个插件的页面：不换掉正在看的英雄联盟
  expect(await uiOpen(page, { uri: INTERPRETER, plugin: "interpreter" })).toBe("waiting");
  await expect(sidebar).toHaveAttribute("data-tab", "page");
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/champion");
  await expect(toasts(page, "同传")).toHaveCount(1);
  await stack.getByTestId("sidebar-plugin-page-back").click();
  await expect(stack).toHaveAttribute("data-uri", "ui://kotone/lol/tier");
  // 等着的那张在“卡片”最上面；“不看了”就拿掉
  await page.getByTestId("sidebar-tab-cards").click();
  const waiting = page.getByTestId("ui-open-waiting");
  await expect(waiting).toHaveCount(1);
  await expect(waiting).toContainText("同传");
  await waiting.getByTestId("ui-open-waiting-dismiss").click();
  await expect(waiting).toHaveCount(0);
  // 在聊天页、右侧栏空着时设置页照旧跳过去（她带你过去）
  expect(await uiOpen(page, { uri: "ui://kotone/settings/sound" })).toBe("shown");
  await expect(page.getByTestId("advanced-nav-sound")).toHaveAttribute("aria-current", "page");
});

test("a web page or an opened background task in the sidebar is not replaced; a closed sidebar counts as free", async ({ page }) => {
  await open(page);
  // “网页”里开着网页
  await page.getByTestId("sidebar-tab-web").click();
  const address = page.getByTestId("browser-address");
  await address.click();
  await address.fill("music.163.com");
  await address.press("Enter");
  await expect(page.getByTestId("browser-tab")).toHaveCount(1);
  expect(await uiOpen(page, { uri: GAME_VOICE, plugin: "game-voice" })).toBe("waiting");
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "web");
  // 回到“卡片”、展开看一件后台的事：也不换掉
  await page.getByTestId("sidebar-tab-cards").click();
  await expect(page.getByTestId("ui-open-waiting")).toHaveCount(1);
  const toggle = page.getByTestId("sidebar-task-toggle").first();
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(await uiOpen(page, { uri: INTERPRETER, plugin: "interpreter" })).toBe("waiting");
  await expect(page.getByTestId("right-sidebar")).toHaveAttribute("data-tab", "cards");
  // 新等着的排在最上面
  await expect(page.getByTestId("ui-open-waiting").first()).toHaveAttribute("data-uri", INTERPRETER);
  await toggle.click();
  // 右侧栏收着：算空着，打开它显示；等着的同一页一起拿掉
  await page.getByTestId("right-sidebar-close").click();
  await expect(page.getByTestId("right-sidebar")).toHaveCount(0);
  expect(await uiOpen(page, { uri: INTERPRETER, plugin: "interpreter" })).toBe("shown");
  await expect(page.getByTestId("sidebar-plugin-page")).toHaveAttribute("data-uri", INTERPRETER);
  await page.getByTestId("sidebar-tab-cards").click();
  await expect(page.getByTestId("ui-open-waiting")).toHaveCount(1);
  await expect(page.getByTestId("ui-open-waiting")).toHaveAttribute("data-uri", GAME_VOICE);
});
