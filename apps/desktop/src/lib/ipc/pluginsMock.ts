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
import { getSettings, updateSettings } from "./browserMock";
import { remotePlugins } from "./mcpServersMock";
import { gameVoiceCall } from "./gameVoiceMock";
import { interpreterCall } from "./interpreterMock";
import { cs2Call, cs2Manifest, cs2Tools } from "$plugins/cs2/ui/mock";
import { musicCall, pluginTurnedOff as musicTurnedOff, subscribe as subscribeMusic } from "./musicMock";
import { findSamplePlayer, sampleGame, sampleHistory, sampleMatch, samplePlayer, type SamplePhase } from "./lolMock";
import {
  findSampleChampion, sampleChampionList, sampleChampionMayhem, sampleChampionRift, sampleTierMayhem, sampleTierRift,
} from "./lolStatsMock";
import {
  PAGE_AUGMENT_OCR, PAGE_AUGMENT_PICK, PAGE_CHAMP_SELECT, PAGE_CHAMPION, PAGE_ENEMY_BUILD, PAGE_GAME, PAGE_LOL_HOME, PAGE_MATCH,
  PAGE_PLAYER, PAGE_TIER, PAGE_TODAY,
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
    // OP.GG：默认开（ADR 043 D9，玩家决定“直接用”；用法照 opgginc/opgg-mcp#26 里答应的）
    id: "opgg",
    name: lolZh.plugin_lol_source_opgg_name,
    description: lolZh.plugin_lol_source_opgg_desc,
    enabledByDefault: true,
    risk: lolZh.plugin_lol_source_opgg_risk,
  }, {
    // 操作客户端（照 LeagueAkari，第 3.5 步）：默认关，风险提示写着 LeagueAkari #523
    id: "control",
    name: lolZh.plugin_lol_source_control_name,
    description: lolZh.plugin_lol_source_control_desc,
    enabledByDefault: false,
    risk: lolZh.plugin_lol_source_control_risk,
  }, {
    // 在客户端里发聊天（ADR 041 第 2 期）：选英雄、房间里经客户端的聊天接口发，不按键；默认开，风险提示带出处
    id: "chat",
    name: lolZh.plugin_lol_source_chat_name,
    description: lolZh.plugin_lol_source_chat_desc,
    enabledByDefault: true,
    risk: lolZh.plugin_lol_source_chat_risk,
  }],
  pages: [
    { uri: PAGE_LOL_HOME, title: lolZh.plugin_lol_page_home, needs: [] },
    { uri: PAGE_MATCH, title: lolZh.plugin_lol_page_match, needs: ["client"] },
    { uri: PAGE_PLAYER, title: lolZh.plugin_lol_page_player, needs: ["client"] },
    { uri: PAGE_GAME, title: lolZh.plugin_lol_page_game, needs: ["client"] },
    { uri: PAGE_CHAMPION, title: lolZh.plugin_lol_page_champion, needs: ["stats"] },
    { uri: PAGE_TIER, title: lolZh.plugin_lol_page_tier, needs: ["stats"] },
    // 只在推送里出现的两张卡片（ADR 043 D7）
    { uri: PAGE_CHAMP_SELECT, title: lolZh.plugin_lol_page_champ_select, needs: ["stats"] },
    { uri: PAGE_AUGMENT_OCR, title: lolZh.plugin_lol_page_augment_ocr, needs: ["stats"] },
    // 进房间或排队时的“今天的状态”（ADR 043 B 轮）
    { uri: PAGE_TODAY, title: lolZh.plugin_lol_page_today, needs: ["client"] },
    // 海斗三选一的卡片（ADR 043 D7 的 2026-11-26 晚修订）、你阵亡时对面的装备（B 轮）
    { uri: PAGE_AUGMENT_PICK, title: lolZh.plugin_lol_page_augment_pick, needs: ["stats"] },
    { uri: PAGE_ENEMY_BUILD, title: lolZh.plugin_lol_page_enemy_build, needs: ["client"] },
    { uri: "ui://kotone/lol/settings", title: lolZh.plugin_lol_tab_settings, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  // 许可和出处（plugin.toml 的 [[notices]]，ADR 043 D8）：标题、正文用插件的文案，链接和 plugin.toml 一样
  notices: ([
    ["aramhex-data", ["https://github.com/czgreat/aramhex-data", "https://creativecommons.org/licenses/by-nc/4.0/"]],
    ["opgg", ["https://op.gg/", "https://github.com/opgginc/opgg-mcp/issues/26"]],
    ["images", ["https://www.communitydragon.org/"]],
    ["riot", ["https://www.riotgames.com/en/legal"]],
  ] as [string, string[]][]).map(([id, links]) => ({
    id,
    title: (lolZh as Record<string, string>)[`plugin_lol_notice_${id.replace(/-/g, "_")}_title`],
    body: (lolZh as Record<string, string>)[`plugin_lol_notice_${id.replace(/-/g, "_")}_body`],
    links,
  })),
  // 内置推送（plugin.toml 的 [[pushes]]，ADR 043 D4）：名字、时机、做什么用插件的文案
  pushes: ([
    ["opening", ["card"], "normal", ["client"]],
    ["summary", ["card"], "normal", ["client"]],
    ["today", ["card"], "normal", ["client"]],
    ["champ-select", ["card"], "normal", ["client", "stats"]],
    ["augment-pick", ["card", "voice", "line"], "high", ["stats"]],
    // 对局中的提醒（ADR 043 B 轮第 2 条）
    ["enemy-ult", ["voice", "line"], "normal", ["client"]],
    ["enemy-item", ["voice", "line"], "low", ["client"]],
    ["spell-ready", ["voice", "line"], "normal", ["client"]],
    ["objective", ["voice", "line"], "normal", ["client"]],
    ["enemies-down", ["voice", "line"], "high", ["client"]],
    ["my-death", ["card"], "normal", ["client"]],
  ] as [string, ("card" | "line" | "voice")[], "high" | "normal" | "low", string[]][]).map(([id, forms, priority, needs]) => ({
    id,
    title: (lolZh as Record<string, string>)[`plugin_lol_push_${id.replace(/-/g, "_")}_title`],
    when: (lolZh as Record<string, string>)[`plugin_lol_push_${id.replace(/-/g, "_")}_when`],
    what: (lolZh as Record<string, string>)[`plugin_lol_push_${id.replace(/-/g, "_")}_what`],
    forms,
    priority,
    enabledByDefault: true,
    needs,
  })),
  // 游戏中心的事（plugin.toml 的 [[events]]）：名字用插件的文案；都要“读客户端数据”；带 team 的能选哪一边
  events: ([
    ["first_blood", true], ["dragon_kill", true], ["herald_kill", true], ["baron_kill", true], ["turret_killed", true],
    ["inhib_killed", true], ["ace", true], ["multikill", true], ["my_death", false], ["my_respawn", false],
    ["level", true], ["item", true],
  ] as [string, boolean][]).map(([name, team]) => ({
    type: `lol.${name}`,
    label: (lolZh as Record<string, string>)[`plugin_lol_event_${name}`],
    needs: ["client"],
    data: (team ? { team: "ally / enemy" } : {}) as Record<string, string>,
  })),
};

// 游戏语音（ADR 020、038 第 4 步）：工具名沿用 voice_chat.*，默认关。推送一条（她对队友说了）、事件一件（ADR 043 D11）
const gameVoice: PluginManifest = {
  id: "game-voice",
  name: gameVoiceZh.plugin_game_voice_name,
  description: gameVoiceZh.plugin_game_voice_description,
  games: [],
  adapter: null,
  enabledByDefault: false,
  sources: [],
  pages: [
    { uri: "ui://kotone/game-voice/home", title: gameVoiceZh.plugin_game_voice_name, needs: [] },
    { uri: "ui://kotone/game-voice/said", title: gameVoiceZh.plugin_game_voice_push_said_title, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  pushes: [{
    id: "said",
    title: gameVoiceZh.plugin_game_voice_push_said_title,
    when: gameVoiceZh.plugin_game_voice_push_said_when,
    what: gameVoiceZh.plugin_game_voice_push_said_what,
    forms: ["card"],
    priority: "normal",
    enabledByDefault: true,
    needs: [],
  }],
  events: [{ type: "game-voice.said", label: gameVoiceZh.plugin_game_voice_event_said, needs: [], data: { text: "text" } }],
};

// 放歌（ADR 034、038 第 4 步）：她的 music + 音乐页用的 music.*，默认开。推送一条（登录过期，ADR 043 D11）
const music: PluginManifest = {
  id: "music",
  name: musicZh.plugin_music_name,
  description: musicZh.plugin_music_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [
    { uri: "ui://kotone/music/home", title: musicZh.plugin_music_name, needs: [] },
    { uri: "ui://kotone/music/login", title: musicZh.plugin_music_push_login_title, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  pushes: [{
    id: "login",
    title: musicZh.plugin_music_push_login_title,
    when: musicZh.plugin_music_push_login_when,
    what: musicZh.plugin_music_push_login_what,
    forms: ["card"],
    priority: "normal",
    enabledByDefault: true,
    needs: [],
  }],
};

// 同传（ADR 021、038）：页面的工具只给页面用，interpreter.show 给后台；默认开。推送一条（这局的同传：卡片 + 一行）、
// 事件一件（原话和译文只给自动化，ADR 043 D11）
const interpreter: PluginManifest = {
  id: "interpreter",
  name: interpreterZh.plugin_interpreter_name,
  description: interpreterZh.plugin_interpreter_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [
    { uri: "ui://kotone/interpreter/home", title: interpreterZh.plugin_interpreter_name, needs: [] },
    { uri: "ui://kotone/interpreter/lines", title: interpreterZh.plugin_interpreter_push_lines_title, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  pushes: [{
    id: "lines",
    title: interpreterZh.plugin_interpreter_push_lines_title,
    when: interpreterZh.plugin_interpreter_push_lines_when,
    what: interpreterZh.plugin_interpreter_push_lines_what,
    forms: ["card", "line"],
    priority: "normal",
    enabledByDefault: true,
    needs: [],
  }],
  events: [{ type: "interpreter.line", label: interpreterZh.plugin_interpreter_event_line, needs: [], data: { text: "text", heard: "heard", app: "app" } }],
};

// 语音输入（ADR 038 第 5 步）：默认开；没有给她的工具（game_chat.send 是本体的，ADR 039 D2）。推送两条（发出去的话、
// 没发出去）、事件两件（没有适配器，以插件 id 开头），卡片上的“重发”只给页面（ADR 043 D10）
const voiceInputText = voiceInputZh as Record<string, string>;
const voiceInput: PluginManifest = {
  id: "voice-input",
  name: voiceInputZh.plugin_voice_input_name,
  description: voiceInputZh.plugin_voice_input_description,
  games: [],
  adapter: null,
  enabledByDefault: true,
  sources: [],
  pages: [
    { uri: "ui://kotone/voice-input/home", title: voiceInputZh.plugin_voice_input_name, needs: [] },
    { uri: "ui://kotone/voice-input/sent", title: voiceInputZh.plugin_voice_input_push_sent_title, needs: [] },
    { uri: "ui://kotone/voice-input/failed", title: voiceInputZh.plugin_voice_input_push_failed_title, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  pushes: ["sent", "failed"].map((id) => ({
    id,
    title: voiceInputText[`plugin_voice_input_push_${id}_title`],
    when: voiceInputText[`plugin_voice_input_push_${id}_when`],
    what: voiceInputText[`plugin_voice_input_push_${id}_what`],
    forms: ["card" as const],
    priority: "normal" as const,
    enabledByDefault: true,
    needs: [],
  })),
  events: [
    { type: "voice-input.sent", label: voiceInputZh.plugin_voice_input_event_sent, needs: [], data: { text: "text", channel: "team / all", channelName: "channel" } },
    { type: "voice-input.failed", label: voiceInputZh.plugin_voice_input_event_failed, needs: [], data: { reason: "reason", code: "code" } },
  ],
};

interface MockTool { name: string; needs: string[]; visibility: PluginVisibility[]; page: string | null }
const BOTH: PluginVisibility[] = ["model", "app"];
const TOOLS: Record<string, MockTool[]> = {
  lol: [
    { name: "lol.match", needs: ["client"], visibility: BOTH, page: null },
    { name: "lol.spell", needs: ["client"], visibility: BOTH, page: null },
    { name: "lol.scout", needs: ["client"], visibility: BOTH, page: PAGE_MATCH },
    { name: "lol.player", needs: ["client"], visibility: BOTH, page: PAGE_PLAYER },
    { name: "lol.history", needs: ["client"], visibility: ["app"], page: null },
    { name: "lol.game", needs: ["client"], visibility: ["app"], page: PAGE_GAME },
    { name: "lol.champion", needs: ["stats"], visibility: BOTH, page: PAGE_CHAMPION },
    { name: "lol.tier", needs: ["stats"], visibility: BOTH, page: PAGE_TIER },
    { name: "lol.client", needs: ["client", "control"], visibility: BOTH, page: null },
    { name: "lol.champions", needs: [], visibility: ["app"], page: "ui://kotone/lol/settings" },
    // 来源标签换来源（ADR 043 D8）：只给页面，写 plugins.lol.settings.sourceFor
    { name: "lol.source", needs: [], visibility: ["app"], page: null },
    { name: "lol.augment_ocr", needs: ["stats"], visibility: ["app"], page: PAGE_AUGMENT_OCR },
    // 选人卡片上的一键导入（ADR 043 B 轮）：只给页面，要“操作客户端”（默认关）
    { name: "lol.import", needs: ["client", "control"], visibility: ["app"], page: null },
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
  "voice-input": [{ name: "voice-input.resend", needs: [], visibility: ["app"], page: null }],
  cs2: cs2Tools,
  interpreter: [
    { name: "interpreter.status", needs: [], visibility: ["app"], page: null },
    { name: "interpreter.start", needs: [], visibility: ["app"], page: null },
    { name: "interpreter.stop", needs: [], visibility: ["app"], page: null },
    { name: "interpreter.show", needs: [], visibility: BOTH, page: null },
  ],
};

const MANIFESTS: PluginManifest[] = [voiceInput, lol, cs2Manifest, gameVoice, music, interpreter];

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

/** 预览里插件工具被调过的记录（测试看 App 调工具时宿主绑定的是哪个插件）：`window.__kotonePluginCalls`。
 * 开发服务器热更新过这个文件时，页面里可能有两份这个模块（带不同的 `?t=`）：两份记进同一个数组，不互相盖掉。 */
const calls: { plugin: string; tool: string }[] = typeof window !== "undefined"
  ? ((window as unknown as { __kotonePluginCalls?: { plugin: string; tool: string }[] }).__kotonePluginCalls ??= [])
  : [];

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
    // 预览里不打字：重发照真的回“没重发”和原因（卡片照样显示）；别的没模拟的工具回工具名
    if (tool === "voice-input.resend") return { content: { status: "refused", reason: "dev:web" } };
    return { content: { status: "error", text: tool } };
  }
  if (plugin === "interpreter") {
    if (!(await enabled(interpreter))) return { content: { status: "disabled", text: interpreter.name } };
    return interpreterCall(tool, args);
  }
  if (plugin === "cs2") {
    if (!(await enabled(cs2Manifest))) return { content: { status: "disabled", text: cs2Manifest.name } };
    return cs2Call(tool, args);
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
    case "lol.augment_ocr": {
      // 预览里不真的下载：点“下载”马上算下好了，点“不用”记下（和真的一样回卡片的数据）
      const action = typeof args.action === "string" ? args.action : "status";
      const state = action === "download" ? "ready" : action === "decline" ? "declined" : "ask";
      const card = state === "declined" ? { state } : { state, model: "PP-OCRv5 mobile", sizeMb: state === "ask" ? 21 : 0 };
      return { content: { status: "ok", text: state }, structuredContent: card, ui: { uri: PAGE_AUGMENT_OCR } };
    }
    case "lol.import": {
      // 预览里没有客户端：照真的回答写了什么（符文页新建、出装方案写好、召唤师技能换好）
      // i18n-ignore-start：示例数据
      const name = "Kotone 亚索 中路";
      // i18n-ignore-end
      return { content: {
        status: "ok", text: "dev:web", champion: 157, name,
        runes: { result: "created", name },
        items: { result: "written", name, blocks: 5 },
        spells: { result: "written", spells: [4, 14] },
      } };
    }
    case "lol.client":
      // 预览里没有客户端：照真的回答“现在不在这个阶段”
      return { content: { status: "not_now", text: "dev:web" } };
    case "lol.champion": {
      const query = typeof args.champion === "string" ? args.champion : "";
      const champion = findSampleChampion(query);
      if (!champion) return { content: { status: "not_found", text: query } };
      // 来源（ADR 043 D8）：参数里写了用参数的，不然用玩家选的（plugins.lol.settings.sourceFor），OP.GG 关着退回默认
      const mayhem = args.mode === "mayhem";
      const kind = mayhem ? "mayhemAugments" : "rift";
      const opggOn = !(await sourceOff("opgg"));
      const picked = ((await getSettings()).plugins?.lol?.settings as { sourceFor?: Record<string, string> } | undefined)?.sourceFor?.[kind];
      const asked = typeof args.source === "string" ? args.source : picked;
      const opgg = asked === "opgg" && opggOn;
      const data = mayhem
        ? sampleChampionMayhem(champion, opgg ? "opgg" : "aramhex", opggOn)
        : sampleChampionRift(champion, typeof args.lane === "string" && args.lane ? args.lane : "TOP", opgg ? "opgg" : "qq101", opggOn);
      return { content: { status: "ok", text: champion.label }, structuredContent: data, ui: { uri: PAGE_CHAMPION } };
    }
    case "lol.source": {
      const kind = String(args.kind ?? "");
      const source = String(args.source ?? "");
      const options: Record<string, string[]> = { rift: ["qq101", "opgg"], mayhemAugments: ["aramhex", "opgg"] };
      if (!options[kind]?.includes(source)) return { content: { status: "error", text: `${kind}: ${source}` } };
      const lolSettings = ((await getSettings()).plugins?.lol?.settings ?? {}) as { sourceFor?: Record<string, string> };
      const sourceFor = { ...(lolSettings.sourceFor ?? {}), [kind]: source };
      await updateSettings({ plugins: { lol: { settings: { ...lolSettings, sourceFor } } } } as never);
      return { content: { status: "ok", kind, source } };
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
      if (args.includeProfileBackground === true && !id && !query) {
        data.profileBackground = "/lol-game-data/assets/ASSETS/Characters/Irelia/Skins/Base/Images/irelia_splash_uncentered_0.jpg";
      }
      return { content: { status: "ok", text: data.riotId }, structuredContent: data, ui: { uri: PAGE_PLAYER } };
    }
    case "lol.history": {
      const { puuid, offset, pageSize } = args;
      if (typeof puuid !== "string" || typeof offset !== "number" || !Number.isSafeInteger(offset) || offset < 0 ||
        (pageSize !== 10 && pageSize !== 20)) return { content: { status: "error", text: "Invalid history page" } };
      return { content: { status: "ok" }, structuredContent: sampleHistory(puuid, offset, pageSize) };
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

/** 做成 App 的页面（和 plugins/lol/src/app.rs 的 APP_PAGES 一致）和它们要的数据开关（plugin.toml 的 pages.needs）。
 * 三处（这里、plugins/lol/ui/index.ts 的 `app: true`、Rust）对不上时桌面端的 Rust 测试
 * `plugin_apps::tests::app_pages_agree_with_the_frontend` 报错。 */
const APP_PAGES: Record<string, string | null> = {
  [PAGE_LOL_HOME]: null,
  [PAGE_MATCH]: "client", [PAGE_PLAYER]: "client", [PAGE_GAME]: "client", [PAGE_CHAMPION]: "stats", [PAGE_TIER]: "stats",
  [PAGE_CHAMP_SELECT]: "stats", [PAGE_AUGMENT_OCR]: "stats", [PAGE_TODAY]: "client",
  [PAGE_AUGMENT_PICK]: "stats", [PAGE_ENEMY_BUILD]: "client",
};
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
  if (!(uri in APP_PAGES)) throw new Error(`${uri} is not an app`);
  const source = APP_PAGES[uri];
  if (!(await enabled(lol)) || (source && await sourceOff(source))) throw new Error(`${uri} is off`);
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
  if (plugin !== "lol" || !(uri in APP_PAGES)) throw new Error(`${plugin} cannot read ${uri}`);
  const page = uri.slice(uri.lastIndexOf("/") + 1);
  const text = await (await fetch(`/__kotone/app-html/lol/${page}`)).text();
  return { uri, mimeType: "text/html;profile=mcp-app", text, _meta: { ui: { csp: { resourceDomains: ["https://raw.communitydragon.org"] } } } };
}
