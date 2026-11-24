/**
 * dev:web 的插件后端（ADR 038）：照桌面端 `plugins.rs` 列出内置插件、按设置现算开关、
 * 插件页面调工具。清单的文字从插件自己的文案读（plugins/<id>/messages/zh-CN.json），和 plugin.toml 一致
 * （插件的 Rust 测试对过）。英雄联盟的工具返回示例数据（lolMock.ts），页面和聊天卡片照真的显示；
 * 游戏语音（默认关）的驱动、默认麦克风是模拟的（gameVoiceMock.ts）；放歌（默认开）是模拟的网易云和播放器（musicMock.ts），
 * 状态照真的作为插件事件发出（[`subscribe`]）；同传（默认开）的“开始 / 停止”只模拟她的耳朵在不在听（interpreterMock.ts）。
 */
import lolZh from "../../../../../plugins/lol/messages/zh-CN.json";
import gameVoiceZh from "../../../../../plugins/game-voice/messages/zh-CN.json";
import musicZh from "../../../../../plugins/music/messages/zh-CN.json";
import interpreterZh from "../../../../../plugins/interpreter/messages/zh-CN.json";
import voiceInputZh from "../../../../../plugins/voice-input/messages/zh-CN.json";
import type { PluginInfo, PluginManifest, PluginToolResult, PluginVisibility } from "../plugins";
import { getSettings } from "./browserMock";
import { remotePlugins } from "./mcpServersMock";
import { gameVoiceCall } from "./gameVoiceMock";
import { interpreterCall } from "./interpreterMock";
import { musicCall, pluginTurnedOff as musicTurnedOff, subscribe as subscribeMusic } from "./musicMock";
import { findSamplePlayer, sampleGame, sampleMatch, samplePlayer } from "./lolMock";
import {
  findSampleChampion, sampleChampionList, sampleChampionMayhem, sampleChampionRift, sampleTierMayhem, sampleTierRift,
} from "./lolStatsMock";
import {
  OVERLAY_AUGMENTS, OVERLAY_CHAMP_SELECT, PAGE_CHAMPION, PAGE_GAME, PAGE_MATCH, PAGE_PLAYER, PAGE_TIER,
} from "$plugins/lol/ui/types";

const lol: PluginManifest = {
  id: "lol",
  name: lolZh.plugin_lol_name,
  description: lolZh.plugin_lol_description,
  games: ["lol"],
  adapter: "lol",
  enabledByDefault: true,
  sources: [{
    id: "client",
    name: lolZh.plugin_lol_source_client_name,
    description: lolZh.plugin_lol_source_client_desc,
    enabledByDefault: true,
    risk: lolZh.plugin_lol_source_client_risk,
  }, {
    id: "stats",
    name: lolZh.plugin_lol_source_stats_name,
    description: lolZh.plugin_lol_source_stats_desc,
    enabledByDefault: true,
    risk: lolZh.plugin_lol_source_stats_risk,
  }, {
    // OP.GG：等 OP.GG 回复同意前默认关（opgginc/opgg-mcp#26）
    id: "opgg",
    name: lolZh.plugin_lol_source_opgg_name,
    description: lolZh.plugin_lol_source_opgg_desc,
    enabledByDefault: false,
    risk: lolZh.plugin_lol_source_opgg_risk,
  }, {
    // 操作客户端（照 LeagueAkari，第 3.5 步）：默认关，风险提示写着 LeagueAkari #523
    id: "control",
    name: lolZh.plugin_lol_source_control_name,
    description: lolZh.plugin_lol_source_control_desc,
    enabledByDefault: false,
    risk: lolZh.plugin_lol_source_control_risk,
  }],
  pages: [
    { uri: PAGE_MATCH, title: lolZh.plugin_lol_page_match, needs: ["client"] },
    { uri: PAGE_PLAYER, title: lolZh.plugin_lol_page_player, needs: ["client"] },
    { uri: PAGE_GAME, title: lolZh.plugin_lol_page_game, needs: ["client"] },
    { uri: PAGE_CHAMPION, title: lolZh.plugin_lol_page_champion, needs: ["stats"] },
    { uri: PAGE_TIER, title: lolZh.plugin_lol_page_tier, needs: ["stats"] },
    { uri: "ui://kotone/lol/settings", title: lolZh.plugin_lol_tab_settings, needs: [] },
  ],
  overlays: [
    { uri: OVERLAY_CHAMP_SELECT, title: lolZh.plugin_lol_overlay_team_mayhem, needs: ["client", "stats"] },
    { uri: OVERLAY_AUGMENTS, title: lolZh.plugin_lol_best_augments, needs: ["stats"] },
  ],
  agent: { overview: "", rule: "" },
  // 游戏中心的事（plugin.toml 的 [[events]]）：名字用插件的文案；都要“读客户端数据”；带 team 的能选哪一边
  events: ([
    ["first_blood", true], ["dragon_kill", true], ["herald_kill", true], ["baron_kill", true], ["turret_killed", true],
    ["inhib_killed", true], ["ace", true], ["multikill", true], ["my_death", false], ["my_respawn", false],
  ] as [string, boolean][]).map(([name, team]) => ({
    type: `lol.${name}`,
    label: (lolZh as Record<string, string>)[`plugin_lol_event_${name}`],
    needs: ["client"],
    data: (team ? { team: "ally / enemy" } : {}) as Record<string, string>,
  })),
};

// 游戏语音（ADR 020、038 第 4 步）：工具名沿用 voice_chat.*，默认关
const gameVoice: PluginManifest = {
  id: "game-voice",
  name: gameVoiceZh.plugin_game_voice_name,
  description: gameVoiceZh.plugin_game_voice_description,
  games: [],
  adapter: null,
  enabledByDefault: false,
  sources: [],
  pages: [{ uri: "ui://kotone/game-voice/home", title: gameVoiceZh.plugin_game_voice_name, needs: [] }],
  overlays: [],
  agent: { overview: "", rule: "" },
};

// 放歌（ADR 034、038 第 4 步）：她的 music + 音乐页用的 music.*，默认开
const music: PluginManifest = {
  id: "music",
  name: musicZh.plugin_music_name,
  description: musicZh.plugin_music_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [{ uri: "ui://kotone/music/home", title: musicZh.plugin_music_name, needs: [] }],
  overlays: [],
  agent: { overview: "", rule: "" },
};

// 同传（ADR 021、038）：工具只给页面用，默认开
const interpreter: PluginManifest = {
  id: "interpreter",
  name: interpreterZh.plugin_interpreter_name,
  description: interpreterZh.plugin_interpreter_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [{ uri: "ui://kotone/interpreter/home", title: interpreterZh.plugin_interpreter_name, needs: [] }],
  overlays: [],
  agent: { overview: "", rule: "" },
};

// 语音输入（ADR 038 第 5 步）：默认开；没有给她的工具（game_chat.send 是本体的，ADR 039 D2）
const voiceInput: PluginManifest = {
  id: "voice-input",
  name: voiceInputZh.plugin_voice_input_name,
  description: voiceInputZh.plugin_voice_input_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [{ uri: "ui://kotone/voice-input/home", title: voiceInputZh.plugin_voice_input_name, needs: [] }],
  overlays: [],
  agent: { overview: "", rule: "" },
};

interface MockTool { name: string; needs: string[]; visibility: PluginVisibility[]; page: string | null }
const BOTH: PluginVisibility[] = ["model", "app"];
const TOOLS: Record<string, MockTool[]> = {
  lol: [
    { name: "lol.match", needs: ["client"], visibility: BOTH, page: null },
    { name: "lol.spell", needs: ["client"], visibility: BOTH, page: null },
    { name: "lol.scout", needs: ["client"], visibility: BOTH, page: PAGE_MATCH },
    { name: "lol.player", needs: ["client"], visibility: BOTH, page: PAGE_PLAYER },
    { name: "lol.game", needs: ["client"], visibility: ["app"], page: PAGE_GAME },
    { name: "lol.champion", needs: ["stats"], visibility: BOTH, page: PAGE_CHAMPION },
    { name: "lol.tier", needs: ["stats"], visibility: BOTH, page: PAGE_TIER },
    { name: "lol.client", needs: ["client", "control"], visibility: BOTH, page: null },
    { name: "lol.champions", needs: [], visibility: ["app"], page: "ui://kotone/lol/settings" },
  ],
  "game-voice": [
    { name: "voice_chat.status", needs: [], visibility: BOTH, page: "ui://kotone/game-voice/home" },
    { name: "voice_chat.install", needs: [], visibility: BOTH, page: null },
    { name: "voice_chat.default_mic", needs: [], visibility: BOTH, page: null },
    { name: "voice_chat.say", needs: [], visibility: BOTH, page: null },
    { name: "voice_chat.test", needs: [], visibility: BOTH, page: null },
  ],
  music: [
    { name: "music", needs: [], visibility: BOTH, page: null },
    ...["state", "login_start", "login_cancel", "logout", "search", "home", "playlist", "play", "control", "queue", "set_quality"]
      .map((t): MockTool => ({ name: `music.${t}`, needs: [], visibility: ["app"], page: null })),
  ],
  "voice-input": [],
  interpreter: [
    { name: "interpreter.status", needs: [], visibility: ["app"], page: null },
    { name: "interpreter.start", needs: [], visibility: ["app"], page: null },
    { name: "interpreter.stop", needs: [], visibility: ["app"], page: null },
  ],
};

const MANIFESTS: PluginManifest[] = [voiceInput, lol, gameVoice, music, interpreter];

/** 插件开着没有（设置里写了按设置，没写按清单的默认）。 */
async function enabled(manifest: PluginManifest): Promise<boolean> {
  return (await getSettings()).plugins?.[manifest.id]?.enabled ?? manifest.enabledByDefault;
}

/** 插件发给界面的事件（真的是 `kotone://plugin`）：放歌的 `state` 由模拟播放器发。 */
export function subscribe(plugin: string, event: string, callback: (payload: unknown) => void): () => void {
  if (plugin === "music" && event === "state") return subscribeMusic(callback);
  return () => {};
}

/** 设置变了（插件开关）：和桌面端一样，关掉放歌插件就停下。 */
export async function settingsChanged(): Promise<void> {
  if (!(await enabled(music))) musicTurnedOff();
}

export async function listPlugins(): Promise<PluginInfo[]> {
  const settings = await getSettings();
  // 第三方插件（MCP 方案第 2 期）接在内置的后面，和桌面端 plugins::list 一样
  const remote = await remotePlugins();
  return MANIFESTS.map((manifest) => {
    const config = settings.plugins?.[manifest.id] ?? {};
    const enabled = config.enabled ?? manifest.enabledByDefault;
    const sources = Object.fromEntries(manifest.sources.map((s) => [s.id, enabled && (config.sources?.[s.id] ?? s.enabledByDefault)]));
    return {
      manifest: structuredClone(manifest),
      enabled,
      sources,
      tools: (TOOLS[manifest.id] ?? []).map((t) => ({
        name: t.name,
        visibility: [...t.visibility],
        page: t.page,
        available: enabled && t.needs.every((n) => sources[n]),
      })),
    };
  }).concat(remote);
}

/** 和真的一样：工具要的数据开关关着时只回一句“已关闭”。 */
async function sourceOff(source: string): Promise<boolean> {
  const settings = await getSettings();
  const config = settings.plugins?.lol ?? {};
  const spec = lol.sources.find((s) => s.id === source);
  return (config.enabled ?? true) === false || (config.sources?.[source] ?? spec?.enabledByDefault ?? true) === false;
}

/** 预览里插件工具被调过的记录（测试看 App 调工具时宿主绑定的是哪个插件）：`window.__kotonePluginCalls`。 */
const calls: { plugin: string; tool: string }[] = [];
if (typeof window !== "undefined") (window as unknown as { __kotonePluginCalls: typeof calls }).__kotonePluginCalls = calls;

export async function pluginCall(plugin: string, tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  calls.push({ plugin, tool });
  const spec = (TOOLS[plugin] ?? []).find((t) => t.name === tool);
  if (!spec) throw new Error(`${plugin}: ${tool}`);
  if (plugin === "game-voice") {
    // 和桌面端 plugins::call 一样：插件关着时只回一句“关着”
    if (!(await enabled(gameVoice))) return { content: { status: "disabled", text: gameVoice.name } };
    return gameVoiceCall(tool, args);
  }
  if (plugin === "music") {
    if (!(await enabled(music))) return { content: { status: "disabled", text: music.name } };
    return musicCall(tool, args);
  }
  if (plugin === "voice-input") {
    if (!(await enabled(voiceInput))) return { content: { status: "disabled", text: voiceInput.name } };
    // 预览里不打字（和别的没模拟的工具一样回工具名）
    return { content: { status: "error", text: tool } };
  }
  if (plugin === "interpreter") {
    if (!(await enabled(interpreter))) return { content: { status: "disabled", text: interpreter.name } };
    return interpreterCall(tool);
  }
  if (plugin !== "lol") return { content: { status: "error", text: tool } };
  for (const needs of spec.needs) {
    if (await sourceOff(needs)) {
      const name = lol.sources.find((s) => s.id === needs)?.name ?? needs;
      return { content: { status: "disabled", text: name } };
    }
  }
  switch (tool) {
    case "lol.champions":
      return { content: { status: "ok" }, structuredContent: { champions: sampleChampionList() }, ui: { uri: "ui://kotone/lol/settings" } };
    case "lol.client":
      // 预览里没有客户端：照真的回答“现在不在这个阶段”
      return { content: { status: "not_now", text: "dev:web" } };
    case "lol.champion": {
      const query = typeof args.champion === "string" ? args.champion : "";
      const champion = findSampleChampion(query);
      if (!champion) return { content: { status: "not_found", text: query } };
      const data = args.mode === "mayhem"
        ? sampleChampionMayhem(champion)
        : sampleChampionRift(champion, typeof args.lane === "string" && args.lane ? args.lane : "TOP");
      return { content: { status: "ok", text: champion.label }, structuredContent: data, ui: { uri: PAGE_CHAMPION } };
    }
    case "lol.tier": {
      const data = args.mode === "mayhem" ? sampleTierMayhem() : sampleTierRift(typeof args.lane === "string" ? args.lane : undefined);
      return { content: { status: "ok", text: "tier" }, structuredContent: data, ui: { uri: PAGE_TIER } };
    }
    case "lol.scout": {
      // 预览和测试切阶段：window.__kotoneLolPhase（none = 不在对局里）
      const phase = (globalThis as { __kotoneLolPhase?: string }).__kotoneLolPhase ?? "loading";
      if (phase === "none") return { content: { status: "not_in_game", reason: "not_in_game", text: "dev:web" } };
      return { content: { status: "ok", text: "scout" }, structuredContent: sampleMatch(phase as SamplePhase), ui: { uri: PAGE_MATCH } };
    }
    case "lol.player": {
      const query = typeof args.player === "string" ? args.player : "";
      const id = typeof args.puuid === "string" ? args.puuid : query ? findSamplePlayer(query) : undefined;
      if (id === null) return { content: { status: "not_found", text: query } };
      const data = samplePlayer(id);
      return { content: { status: "ok", text: data.riotId }, structuredContent: data, ui: { uri: PAGE_PLAYER } };
    }
    case "lol.game": {
      const gameId = Number(args.gameId);
      if (!Number.isFinite(gameId) || gameId <= 0) return { content: { status: "error", text: "gameId" } };
      return { content: { status: "ok", gameId }, structuredContent: sampleGame(gameId), ui: { uri: PAGE_GAME } };
    }
    default:
      return { content: { status: "not_in_game", text: "dev:web" } };
  }
}

// ---------- 插件的 MCP App（ADR 039 D5） ----------

/** 做成 App 的页面（和 plugins/lol/src/app.rs 的 APP_PAGES 一致）。 */
const APP_PAGES: string[] = [PAGE_TIER];
/** 预览里代理路径的“密钥”（真的是每次启动随机的 64 位十六进制）。 */
const MOCK_KEY = "0000feed";

/** 预览的 App 地址：沙盒代理页和 App 由 Vite 开发服务器旁边的一个小服务照回环代理的路径给（vite.config.ts 的
 * mcpAppsDevServer），和真的一样在 127.0.0.1 的另一个端口上（另一个源，和 Kotone 的页面隔开）。 */
async function appBase(): Promise<string> {
  const { origin } = (await (await fetch("/__kotone/app-proxy")).json()) as { origin: string };
  return `${origin}/${MOCK_KEY}`;
}

/** `plugin_app`：和真的一样，插件或页面要的数据开关关着、页面不是 App 时报错。 */
export async function pluginApp(uri: string): Promise<{ plugin: string; uri: string; sandboxUrl: string; ui: Record<string, unknown> }> {
  if (!APP_PAGES.includes(uri)) throw new Error(`${uri} is not an app`);
  if (!(await enabled(lol)) || (await sourceOff("stats"))) throw new Error(`${uri} is off`);
  const page = uri.slice(uri.lastIndexOf("/") + 1);
  return {
    plugin: "lol",
    uri,
    sandboxUrl: `${await appBase()}/sandbox/lol/${page}`,
    ui: { csp: { resourceDomains: ["https://raw.communitydragon.org"] }, prefersBorder: false },
  };
}

/** `plugin_resource`：App 读自己插件的资源。 */
export async function pluginResource(plugin: string, uri: string): Promise<Record<string, unknown>> {
  if (plugin !== "lol" || !APP_PAGES.includes(uri)) throw new Error(`${plugin} cannot read ${uri}`);
  const page = uri.slice(uri.lastIndexOf("/") + 1);
  const text = await (await fetch(`/__kotone/app-html/lol/${page}`)).text();
  return { uri, mimeType: "text/html;profile=mcp-app", text, _meta: { ui: { csp: { resourceDomains: ["https://raw.communitydragon.org"] } } } };
}
