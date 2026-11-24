// 英雄联盟 App 的截图（开发用，不是测试）：用打好的 dist/index.html 加一个最小的假宿主（MCP Apps 的 postMessage
// JSON-RPC：ui/initialize、tools/call、size-changed），把页面数据样例喂进去，截各种状态、亮 / 暗、inline / fullscreen。
//
//   pnpm -C plugins/lol/app build
//   node plugins/lol/app/shots.mjs [输出目录]
//
// 样例数据：target/plugin-shots/round7/{match,player,game}-sample.json（`render_probe` 从录下来的一局生成，玩家名已换掉）；
// 这里在它上面改出选人 / 加载 / 对局中 / 结算几种状态，再补上段位、标签这些录的时候没有的字段。图从 CommunityDragon 取
// （假宿主不是 Kotone，没有回环代理）。Playwright 用 apps/desktop 装的那份，浏览器是本机的 Chrome（和 playwright.config 一样）。
import { readFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..", "..");
const require = createRequire(join(repo, "apps", "desktop", "package.json"));
const { chromium } = require("@playwright/test");

const out = resolve(process.argv[2] ?? join(repo, "target", "plugin-shots", "round7"));
mkdirSync(out, { recursive: true });
const samples = join(repo, "target", "plugin-shots", "round7");
const read = (name) => JSON.parse(readFileSync(join(samples, name), "utf8"));
const bundle = readFileSync(join(here, "dist", "index.html"), "utf8");
const clone = (v) => JSON.parse(JSON.stringify(v));

// ---------- 样例 → 各种状态 ----------
const RANKS = [
  { tier: "DIAMOND", division: "II", lp: 45 },
  { tier: "PLATINUM", division: "I", lp: 12 },
  { tier: "EMERALD", division: "IV", lp: 0 },
  { tier: "GOLD", division: "III", lp: 60 },
  null,
  { tier: "MASTER", division: "I", lp: 132 },
  { tier: "DIAMOND", division: "I", lp: 88 },
  { tier: "SILVER", division: "II", lp: 33 },
  { tier: "", division: "", lp: 0 },
  { tier: "GOLD", division: "I", lp: 71 },
];
const ranked = (r) =>
  r && { queue: "RANKED_SOLO_5x5", tier: r.tier, division: r.division, lp: r.lp, wins: 52, losses: 48, highestTier: r.tier ? "MASTER" : "", label: "", crest: "" };

function loadingMatch() {
  const m = clone(read("match-sample.json"));
  m.phase = "loading";
  m.phaseSince = Date.now() - 12_000;
  const all = [...m.allies, ...m.enemies];
  all.forEach((p, i) => {
    p.ranked = { solo: ranked(RANKS[i]), flex: null };
  });
  // 一个还在查、一个隐藏战绩、两个带标签、一个图取不到（看占位）
  const pending = m.allies[4];
  Object.assign(pending, { pending: true, summary: null, recent: [], tags: [], ranked: { solo: null, flex: null } });
  m.waiting = 1;
  m.allies[3].hidden = true;
  m.allies[0].tags = [{ kind: "win_streak", text: "3 连胜" }];
  m.allies[0].summary.streak = 3;
  m.enemies[0].tags = [{ kind: "loss_streak", text: "4 连败" }];
  m.enemies[0].summary.streak = -4;
  m.enemies[4].champion = { ...m.enemies[4].champion, id: 99999, icon: "/lol-game-data/assets/v1/champion-icons/99999.png" };
  return m;
}

function champSelect({ ranked: isRanked }) {
  const m = loadingMatch();
  m.phase = "champ_select";
  m.phaseSince = Date.now() - 30_000;
  m.waiting = 0;
  const lanes = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];
  m.allies.forEach((p, i) => {
    p.position = isRanked ? lanes[i] : "";
    p.pick = i === 3 ? "intent" : i === 4 ? "none" : "locked";
    if (i === 4) p.champion = { id: 0, label: "", short: "", alias: "", icon: "" };
  });
  if (isRanked) {
    m.mode = "单双排";
    m.queueId = 420;
    m.random = false;
    m.namesHidden = true;
    // 排位：看不到名字的两个队友只有英雄
    for (const p of [m.allies[1], m.allies[4]]) {
      Object.assign(p, { puuid: "", name: "", riotId: "", summary: null, recent: [], tags: [], party: null, summonerLevel: null, ranked: { solo: null, flex: null } });
    }
    m.allies[0].tags = [{ kind: "main", text: "常玩这个英雄（8 局 6 胜）" }];
    Object.assign(m.allies[0].summary, { champGames: 8, champWins: 6 });
  } else {
    m.bench = [22, 99, 86, 103, 412, 222, 157, 25, 63, 81].map((id) => ({ id, label: "", short: String(id), alias: "", icon: `/lol-game-data/assets/v1/champion-icons/${id}.png` }));
  }
  // 对面：只有英雄（选人时客户端只给这些）
  m.enemies = m.enemies.map((p, i) => ({
    ...p,
    puuid: "",
    name: "",
    riotId: "",
    summary: null,
    recent: [],
    tags: [],
    weak: null,
    pending: false,
    party: null,
    ranked: { solo: null, flex: null },
    pick: i === 2 ? "intent" : i === 4 ? "none" : "locked",
    champion: i === 4 ? { id: 0, label: "", short: "", alias: "", icon: "" } : p.champion,
  }));
  return m;
}

function inGame() {
  const m = loadingMatch();
  m.phase = "in_game";
  m.allies[4] = clone(read("match-sample.json")).allies[4];
  m.allies[4].ranked = { solo: ranked(RANKS[4]), flex: null };
  m.waiting = 0;
  return m;
}

function postMatch() {
  const m = inGame();
  m.phase = "post_match";
  m.phaseSince = Date.now() - 20_000;
  m.result = { win: true, durationS: 1112, gameId: 301078269638 };
  m.resultPending = false;
  return m;
}

const TIER_NAMES = { 22: "寒冰", 99: "拉克丝", 86: "盖伦", 103: "阿狸", 412: "锤石", 222: "金克丝", 157: "亚索", 25: "莫甘娜", 63: "布兰德", 81: "伊泽瑞尔", 64: "李青", 238: "劫", 122: "诺手", 55: "卡特琳娜", 54: "石头人" };
const champ = (id) => ({ id, label: TIER_NAMES[id] ?? String(id), short: TIER_NAMES[id] ?? String(id) });
function tierRift() {
  const ids = [122, 86, 157, 238, 55, 54, 64, 103, 99, 22, 81, 222];
  const rows = ids.map((id, i) => ({
    champion: champ(id),
    tier: ["T0", "T1", "T1", "T2", "T2", "T2", "T3", "T3", "T3", "T4", "T4", "T5"][i],
    rank: i + 1,
    winRate: 0.535 - i * 0.004,
    pickRate: 0.12 - i * 0.008,
    banRate: 0.08 - i * 0.005,
    counters: [ids[(i + 3) % ids.length], ids[(i + 5) % ids.length], ids[(i + 7) % ids.length]].map(champ),
  }));
  return {
    mode: "rift",
    date: "20261124",
    lanes: { TOP: rows, JUNGLE: rows.slice(2), MIDDLE: rows.slice(1), BOTTOM: rows.slice(3), SUPPORT: rows.slice(4) },
    laneNames: ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "SUPPORT"].map((id) => ({ id, name: id })),
    sources: [{ name: "腾讯 101", date: "20261124" }],
  };
}

const ok = (structuredContent, text = "ok") => ({ content: [{ type: "text", text: JSON.stringify({ status: "ok", text }) }], structuredContent });
const notInGame = { content: [{ type: "text", text: JSON.stringify({ status: "not_in_game", text: "现在不在英雄联盟的对局里" }) }] };

// ---------- 假宿主 ----------
const HOST = `<!doctype html><html><head><meta charset="utf-8"><style>
  html, body { margin: 0; background: #e9e9e4; }
  body.dark { background: #0d0e11; }
  #frame { display: inline-block; vertical-align: top; }
  #frame.inline { margin: 16px; overflow: hidden; border: 1px solid rgb(0 0 0 / .09); border-radius: 12px; background: #fff; }
  body.dark #frame.inline { border-color: rgb(255 255 255 / .09); background: #212140; }
  #frame .head { display: flex; align-items: center; gap: 6px; padding: 8px 12px; border-bottom: 1px solid rgb(0 0 0 / .09);
    font: 12px/16px "Segoe UI", "Microsoft YaHei UI", sans-serif; color: #5f6380; }
  body.dark #frame .head { border-color: rgb(255 255 255 / .09); color: #a3a6c8; }
  #frame .head i { width: 16px; height: 16px; border-radius: 4px; background: #4c58dd; }
  iframe { display: block; border: 0; width: 100%; height: 120px; background: transparent; }
</style></head><body><div id="frame"></div><script>
  const q = new URLSearchParams(location.search);
  const push = q.get("push") || "";
  const theme = q.get("theme") || "light", display = q.get("display") || "fullscreen", width = +q.get("width") || 1120;
  document.body.classList.toggle("dark", theme === "dark");
  const frame = document.getElementById("frame");
  frame.className = display;
  frame.style.width = width + "px";
  if (display === "inline") frame.innerHTML = '<div class="head"><i></i>英雄联盟发来一张卡片 · 21:04</div>';
  const iframe = document.createElement("iframe");
  iframe.src = "/app/" + q.get("page") + ".html";
  frame.appendChild(iframe);
  const post = (msg) => iframe.contentWindow.postMessage({ jsonrpc: "2.0", ...msg }, "*");
  window.addEventListener("message", (e) => {
    if (e.source !== iframe.contentWindow) return;
    const msg = e.data || {};
    const reply = (result) => post({ id: msg.id, result });
    switch (msg.method) {
      case "ui/initialize":
        return reply({ protocolVersion: msg.params?.protocolVersion || "2026-01-26", hostInfo: { name: "shots", version: "0" },
          hostCapabilities: { serverTools: {}, openLinks: {} },
          hostContext: { theme, displayMode: display, availableDisplayModes: ["inline", "fullscreen"], locale: q.get("locale") || "zh-CN", platform: "desktop",
            ...(push ? { toolInfo: { tool: { name: push, inputSchema: { type: "object" } } } } : {}) } });
      case "ui/notifications/initialized":
        // 像她调了工具以后打开的卡片：宿主把参数和结果推过去
        if (push) {
          post({ method: "ui/notifications/tool-input", params: { arguments: window.FIXTURE_ARGS || {} } });
          post({ method: "ui/notifications/tool-result", params: window.FIXTURES[push] });
        }
        return;
      case "tools/call":
        return reply(window.FIXTURES[msg.params.name] ?? { content: [{ type: "text", text: "no fixture" }], isError: true });
      case "ui/notifications/size-changed":
        if (msg.params?.height) iframe.style.height = Math.ceil(msg.params.height) + "px";
        return;
      default:
        if (msg.id !== undefined) reply({});
    }
  });
</script></body></html>`;

function appHtml(page) {
  return bundle.replace(/<head>/, `<head><meta name="kotone:page" content="${page}">`);
}

const shots = [];
const add = (name, page, fixtures, opts = {}) => shots.push({ name, page, fixtures, ...opts });

const loading = loadingMatch();
add("lol-app-match-loading", "match", { kotone_lol_scout: ok(loading) }, { themes: ["light", "dark"] });
add("lol-app-match-loading-narrow", "match", { kotone_lol_scout: ok(loading) }, { width: 720 });
add("lol-app-match-select", "match", { kotone_lol_scout: ok(champSelect({ ranked: true })) }, { themes: ["light", "dark"] });
add("lol-app-match-select-bench", "match", { kotone_lol_scout: ok(champSelect({ ranked: false })) });
add("lol-app-match-ingame", "match", { kotone_lol_scout: ok(inGame()) }, { themes: ["light", "dark"] });
add("lol-app-match-post", "match", { kotone_lol_scout: ok(postMatch()) }, { themes: ["light", "dark"] });
add("lol-app-match-sample", "match", { kotone_lol_scout: ok(read("match-sample.json")) }, { themes: ["dark"] });
add("lol-app-match-none", "match", { kotone_lol_scout: notInGame });
add("lol-app-match-en", "match", { kotone_lol_scout: ok(loading) }, { locale: "en-US", themes: ["dark"] });
add("lol-app-card-loading-420", "match", { kotone_lol_scout: ok(loading) }, { display: "inline", width: 420, themes: ["light", "dark"] });
add("lol-app-card-loading-360", "match", { kotone_lol_scout: ok(loading) }, { display: "inline", width: 360, themes: ["light", "dark"] });
add("lol-app-card-select", "match", { kotone_lol_scout: ok(champSelect({ ranked: true })) }, { display: "inline", width: 420 });
add("lol-app-card-post", "match", { kotone_lol_scout: ok(postMatch()) }, { display: "inline", width: 420, themes: ["dark"] });
add("lol-app-tier", "tier", { kotone_lol_tier: ok(tierRift()) }, { themes: ["light", "dark"] });
add("lol-app-tier-card", "tier", { kotone_lol_tier: ok(tierRift()) }, { display: "inline", width: 420 });
add("lol-app-player", "player", { kotone_lol_player: ok(read("player-sample.json")) }, { themes: ["light"] });
add("lol-app-game", "game", { kotone_lol_game: ok(read("game-sample.json")) }, { themes: ["dark"], push: "kotone_lol_game" });

const only = process.env.SHOTS_ONLY ? new RegExp(process.env.SHOTS_ONLY) : null;
const browser = await chromium.launch({ channel: "chrome", headless: true });
try {
  for (const s of shots) {
    if (only && !only.test(s.name)) continue;
    for (const theme of s.themes ?? ["light"]) {
      const context = await browser.newContext({ viewport: { width: (s.width ?? 1120) + 64, height: 900 }, deviceScaleFactor: 1, colorScheme: theme });
      await context.route("http://kotone.test/**", (route) => {
        const url = new URL(route.request().url());
        if (url.pathname === "/host.html") return route.fulfill({ contentType: "text/html", body: HOST });
        const m = /^\/app\/([a-z-]+)\.html$/.exec(url.pathname);
        if (m) return route.fulfill({ contentType: "text/html", body: appHtml(m[1]) });
        return route.fulfill({ status: 404, body: "" });
      });
      const page = await context.newPage();
      page.on("pageerror", (e) => console.error(`[${s.name}] page error:`, e.message));
      page.on("console", (msg) => msg.type() === "error" && console.error(`[${s.name}] console:`, msg.text()));
      await page.addInitScript((fixtures) => (window.FIXTURES = fixtures), s.fixtures);
      const params = new URLSearchParams({ page: s.page, theme, display: s.display ?? "fullscreen", width: String(s.width ?? 1120), locale: s.locale ?? "zh-CN", push: s.push ?? "" });
      await page.goto(`http://kotone.test/host.html?${params}`);
      const frame = page.frameLocator("iframe");
      await frame.locator(`[data-testid="lol-app-${s.page}"]`).first().waitFor({ timeout: 15_000 });
      await page.waitForLoadState("networkidle").catch(() => {});
      await page.waitForTimeout(600);
      const file = join(out, `${s.name}-${theme}.png`);
      await page.locator("#frame").screenshot({ path: file });
      console.log("shot", file);
      await context.close();
    }
  }
} finally {
  await browser.close();
}
