// 音乐页（ADR 034 第五节）：浏览器预览里的模拟网易云（lib/ipc/musicMock.ts）。
// 登录（网易云官方登录页的窗口，预览里没有窗口）→ 发现（每日推荐、私人 FM、我的歌单）→ 放歌和控制；
// 搜索结果里要 VIP 的灰掉；歌单；她要放歌而你没登录时聊天页的登录卡片。
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { openChat } from "./nav";

const SHOTS = process.env.KOTONE_MUSIC_SHOTS;
const shot = (testInfo: TestInfo, name: string) => (SHOTS ? `${SHOTS}/${name}.png` : testInfo.outputPath(`${name}.png`));
const preview = (page: Page, step: string) =>
  page.evaluate((s) => (window as unknown as { __kotoneMusicPreview: (s: string) => unknown }).__kotoneMusicPreview(s), step);

async function openMusic(page: Page, query = "") {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto(`/#/settings?onboarding=never${query}`);
  await page.getByTestId("settings-nav-music").click();
  await expect(page.getByTestId("music-page")).toBeVisible();
}

test("logged out: log in through NetEase's login window, then play daily picks and control playback", async ({ page }, testInfo) => {
  await openMusic(page, "&mockMusicLogin=manual");
  // 音乐在“游戏陪玩”一组里、自动化下面
  const order = await page.getByTestId("app-sidebar").locator('nav [data-testid^="settings-nav-"]').evaluateAll(
    (els) => els.map((el) => el.getAttribute("data-testid")));
  expect(order.slice(0, 4)).toEqual(["settings-nav-assistant", "settings-nav-game", "settings-nav-automation", "settings-nav-music"]);

  const card = page.getByTestId("music-login-card");
  await expect(card).toHaveAttribute("data-stage", "idle");
  await page.screenshot({ path: shot(testInfo, "music-logged-out") });
  await expect(card.getByTestId("music-login-start")).toHaveText("登录网易云音乐");
  await card.getByTestId("music-login-start").click();
  // 登录在弹出的网易云官方登录页里做：卡片上没有二维码，只有提示、重新打开窗口和取消
  await expect(card).toHaveAttribute("data-stage", "waiting");
  await expect(card.getByTestId("music-login-stage")).toHaveText("在弹出的网易云音乐窗口里扫码登录（用网易云音乐 App 扫）");
  await expect(card.locator("svg rect")).toHaveCount(0);
  await expect(card.getByTestId("music-login-cancel")).toBeVisible();
  await card.getByTestId("music-login-reopen").click();
  await expect(card).toHaveAttribute("data-stage", "waiting");
  await page.screenshot({ path: shot(testInfo, "music-login-waiting") });
  await preview(page, "confirmed");
  await expect(page.getByTestId("music-account")).toContainText("开黑的猫");

  // 发现：每日推荐、私人 FM、我的歌单（“我喜欢的音乐”排第一）
  await expect(page.getByTestId("music-daily-card")).toBeVisible();
  await expect(page.getByTestId("music-fm-card")).toBeVisible();
  const playlists = page.getByTestId("music-playlist-card");
  await expect(playlists).toHaveCount(5);
  await expect(playlists.first()).toHaveAttribute("data-liked", "true");
  await expect(playlists.first()).toContainText("我喜欢的音乐");
  await page.screenshot({ path: shot(testInfo, "music-home") });

  // 放每日推荐：播放条和左侧栏的小播放条
  await page.getByTestId("music-daily-play").click();
  const bar = page.getByTestId("music-player-bar");
  await expect(bar).toHaveAttribute("data-status", "playing");
  const title = bar.getByTestId("music-player-title");
  await expect(title).toHaveText("晴天（翻唱）");
  const mini = page.getByTestId("sidebar-mini-player");
  await expect(mini.getByTestId("sidebar-mini-title")).toHaveText("晴天（翻唱）");

  await bar.getByTestId("music-player-toggle").click();
  await expect(bar).toHaveAttribute("data-status", "paused");
  await bar.getByTestId("music-player-toggle").click();
  await expect(bar).toHaveAttribute("data-status", "playing");
  await bar.getByTestId("music-player-next").click();
  await expect(title).toHaveText("起风了");
  // 下一首是要 VIP 的《孤勇者》：跳过，放能完整放的（D6）
  await bar.getByTestId("music-player-next").click();
  await expect(title).toHaveText("夜空中最亮的星");
  await expect(bar).toHaveAttribute("data-status", "playing");
  await bar.getByTestId("music-player-prev").click();
  await expect(title).toHaveText("起风了");
  await expect(bar).toHaveAttribute("data-status", "playing");

  const like = bar.getByTestId("music-player-like");
  const before = await like.getAttribute("aria-pressed");
  await like.click();
  await expect(like).toHaveAttribute("aria-pressed", before === "true" ? "false" : "true");

  await bar.getByTestId("music-player-queue").click();
  const queue = page.getByTestId("music-queue");
  await expect(queue.getByTestId("music-queue-item")).toHaveCount(10);
  await expect(queue.locator('[data-testid="music-queue-item"][data-current="true"]')).toContainText("起风了");
  await page.screenshot({ path: shot(testInfo, "music-queue") });
  await page.keyboard.press("Escape");
  await expect(queue).toHaveCount(0);

  // 左侧栏的小播放条：暂停、去音乐页
  await page.getByTestId("settings-nav-automation").click();
  await expect(page.getByTestId("music-page")).toHaveCount(0);
  await mini.getByTestId("sidebar-mini-toggle").click();
  await mini.getByTestId("sidebar-mini-open").click();
  await expect(page.getByTestId("music-player-bar")).toHaveAttribute("data-status", "paused");
  await page.getByTestId("music-player-toggle").click();
  await expect(page.getByTestId("music-player-bar")).toHaveAttribute("data-status", "playing");
  await page.screenshot({ path: shot(testInfo, "music-playing") });
});

test("search: VIP originals are greyed and labelled; a full version plays", async ({ page }, testInfo) => {
  await openMusic(page, "&mockMusic=loggedIn");
  await page.getByTestId("music-search-input").fill("孤勇者");
  await page.getByTestId("music-search-input").press("Enter");
  await expect(page.getByTestId("music-search-title")).toContainText("孤勇者");
  const results = page.getByTestId("music-search-results");
  const vip = results.locator('[data-testid="music-track"][data-playable="vip"]').first();
  await expect(vip).toContainText("陈奕迅");
  await expect(vip.getByTestId("music-track-badge")).toHaveText("VIP");
  await expect(vip.getByTestId("music-track-play")).toHaveCount(0);
  const piano = results.getByTestId("music-track").filter({ hasText: "钢琴版" });
  await expect(piano).toHaveAttribute("data-playable", "full");
  await piano.hover();
  await piano.getByTestId("music-track-play").click();
  await expect(page.getByTestId("music-player-title")).toHaveText("孤勇者（钢琴版）");
  await expect(piano).toHaveAttribute("data-current", "true");
  await page.screenshot({ path: shot(testInfo, "music-search") });
  await page.getByTestId("music-back").click();
  await expect(page.getByTestId("music-daily-card")).toBeVisible();
});

test("playlist: open it, play all, then go back", async ({ page }, testInfo) => {
  await openMusic(page, "&mockMusic=loggedIn");
  await page.getByTestId("music-playlist-card").filter({ hasText: "开黑歌单" }).getByRole("button").first().click();
  await expect(page.getByTestId("music-list-title")).toHaveText("开黑歌单");
  const tracks = page.getByTestId("music-list-tracks").getByTestId("music-track");
  await expect(tracks).toHaveCount(8);
  await expect(tracks.filter({ hasText: "Enemy" })).toHaveAttribute("data-playable", "vip");
  await page.getByTestId("music-list-play").click();
  await expect(page.getByTestId("music-player-title")).toHaveText("Legends Never Die");
  await expect(page.getByTestId("music-player-title").locator("..")).toContainText("开黑歌单");
  await page.screenshot({ path: shot(testInfo, "music-playlist") });
  await page.getByTestId("music-back").click();
  await expect(page.getByTestId("music-playlists")).toBeVisible();
});

test("quality and logout from the header; when she wants to play and you're logged out, the chat shows a login card", async ({ page }, testInfo) => {
  await openMusic(page, "&mockMusic=loggedIn&mockMusicLogin=manual");
  await page.getByTestId("music-quality").click();
  await page.getByTestId("music-quality-lossless").click();
  await expect(page.getByTestId("music-quality")).toContainText("无损");
  await page.getByTestId("music-account").click();
  await page.getByTestId("music-logout").click();
  await expect(page.getByTestId("music-login-card")).toHaveAttribute("data-stage", "idle");

  // 聊天页：她的 music 工具发现没登录 → needed（她不自己弹窗），输入框上面出登录卡片
  await openChat(page);
  const compact = page.getByTestId("assistant-chat").getByTestId("music-login-card");
  await expect(compact).toHaveCount(0);
  await preview(page, "needed");
  await expect(compact).toHaveAttribute("data-stage", "needed");
  await expect(compact.getByTestId("music-login-stage")).toHaveText("她要放歌：先登录一下网易云（只要一次）");
  await page.screenshot({ path: shot(testInfo, "music-chat-needed") });
  await compact.getByTestId("music-login-start").click();
  await expect(compact).toHaveAttribute("data-stage", "waiting");
  await expect(compact.getByTestId("music-login-stage")).toHaveText("在弹出的网易云音乐窗口里扫码登录（用网易云音乐 App 扫）");
  await expect(compact.getByTestId("music-login-reopen")).toBeVisible();
  await compact.getByTestId("music-login-cancel").click();
  await expect(compact).toHaveCount(0);

  // 5 分钟没登录（窗口关了）/ 出错：再点一下重新打开
  await preview(page, "needed");
  await compact.getByTestId("music-login-start").click();
  await preview(page, "expired");
  await expect(compact.getByTestId("music-login-stage")).toHaveText("太久没登录，登录窗口关掉了；再点一下重新打开");
  await expect(compact.getByTestId("music-login-start")).toBeVisible();
  await preview(page, "failed");
  await expect(compact.getByTestId("music-login-stage")).toContainText("网易云没有回应");
  await compact.getByTestId("music-login-start").click();
  await expect(compact).toHaveAttribute("data-stage", "waiting");
  // 你自己关了登录窗口：卡片收起
  await preview(page, "closed");
  await expect(compact).toHaveCount(0);
  // 先不登录
  await preview(page, "needed");
  await compact.getByTestId("music-login-dismiss").click();
  await expect(compact).toHaveCount(0);
});
