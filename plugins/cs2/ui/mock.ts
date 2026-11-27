// dev:web（浏览器预览、Playwright）里的 CS2 插件：清单照 plugin.toml（文字从插件的文案读，Rust 测试对过两边一致），
// 工具照后端回：`cs2.status` / `cs2.cfg` 用一个假的 cfg 目录（不碰任何真东西），`cs2.state` / `cs2.rounds` 用一局合成的数据。
// 测试用 `window.__kotoneCs2Match = "live" | "none"` 切有没有在打。apps/desktop/src/lib/ipc/pluginsMock.ts 引它。
import zh from "../messages/zh-CN.json";
import type { PluginManifest, PluginToolResult, PluginVisibility } from "$lib/plugins";
import type { Cs2Status, MatchView } from "./cs2";

const text = zh as Record<string, string>;
const BOTH: PluginVisibility[] = ["model", "app"];

export const cs2Manifest: PluginManifest = {
  id: "cs2",
  name: text.plugin_cs2_name,
  description: text.plugin_cs2_description,
  games: ["cs2"],
  adapter: "cs2",
  enabledByDefault: true,
  sources: [],
  pages: [
    { uri: "ui://kotone/cs2/home", title: text.plugin_cs2_name, needs: [] },
    { uri: "ui://kotone/cs2/match", title: text.plugin_cs2_section_match, needs: [] },
    { uri: "ui://kotone/cs2/bomb", title: text.plugin_cs2_push_bomb_title, needs: [] },
  ],
  agent: { overview: "", rule: "" },
  pushes: ([
    ["bomb", ["card"], "normal"],
    ["bomb-ten", ["line", "voice"], "high"],
    ["match-point", ["line", "voice"], "normal"],
    ["halftime", ["line", "voice"], "low"],
    ["match-end", ["card"], "normal"],
  ] as [string, ("card" | "line" | "voice")[], "high" | "normal" | "low"][]).map(([id, forms, priority]) => {
    const key = id.replace(/-/g, "_");
    return {
      id, forms, priority, enabledByDefault: true, needs: [],
      title: text[`plugin_cs2_push_${key}_title`], when: text[`plugin_cs2_push_${key}_when`], what: text[`plugin_cs2_push_${key}_what`],
    };
  }),
  events: ["round_started", "round_ended", "bomb_planted", "bomb_defused", "bomb_exploded", "halftime", "match_point", "match_ended", "my_death"]
    .map((name) => ({ type: `cs2.${name}`, label: text[`plugin_cs2_event_${name}`], needs: [], data: {} as Record<string, string> })),
};

export const cs2Tools: { name: string; needs: string[]; visibility: PluginVisibility[]; page: string | null }[] = [
  { name: "cs2.state", needs: [], visibility: BOTH, page: "ui://kotone/cs2/match" },
  { name: "cs2.rounds", needs: [], visibility: BOTH, page: null },
  { name: "cs2.status", needs: [], visibility: ["app"], page: null },
  { name: "cs2.cfg", needs: [], visibility: ["app"], page: null },
];

// i18n-ignore-start：示例数据（一直到 sampleMatch 结束）
const PATH = "D:\\SteamLibrary\\steamapps\\common\\Counter-Strike Global Offensive\\game\\csgo\\cfg\\gamestate_integration_kotone.cfg";
let writeCfg = true;

function sampleMatch(): MatchView {
  const now = Date.now();
  const wins: [string, boolean, string][] = [
    ["CT", false, "elimination"], ["T", true, "bomb"], ["T", true, "elimination"], ["CT", false, "defuse"],
    ["T", true, "elimination"], ["T", true, "bomb"], ["CT", false, "time"],
  ];
  let ct = 0;
  let t = 0;
  const rounds = wins.map(([winner, won, reason], i) => {
    if (winner === "CT") ct++; else t++;
    return { round: i + 1, winner: winner as "CT" | "T", won, reason, ctScore: ct, tScore: t };
  });
  return {
    playing: true, over: false, map: "de_mirage", mapName: "荒漠迷城", mode: "competitive", modeName: "竞技",
    phase: "live", round: 8, ctScore: ct, tScore: t, myTeam: "T", myScore: t, enemyScore: ct, result: "",
    stats: { kills: 9, deaths: 5, assists: 2, mvps: 2, score: 21 },
    me: { health: 100, armor: 100, helmet: true, money: 3200, roundKills: 1, equipValue: 4700 },
    bomb: { round: 8, state: "planted", plantedAtMs: now - 12_000, explodesAtMs: now + 28_000, map: "de_mirage", mapName: "荒漠迷城" },
    rounds, updatedAtMs: now,
  };
}
// i18n-ignore-end

function playing(): boolean {
  return ((globalThis as { __kotoneCs2Match?: string }).__kotoneCs2Match ?? "none") === "live";
}

function status(): Cs2Status {
  const live = playing();
  return {
    enabled: true, listening: 23730, bindError: "", connected: live, lastPostMs: live ? Date.now() : 0, rejected: 0, gameRunning: live,
    cfg: { writeCfg, found: "dir", path: PATH, state: writeCfg ? "written" : "absent", error: "", port: 23730 },
    match: live ? sampleMatch() : { playing: false, connected: false, lastMatch: null },
  };
}

export async function cs2Call(tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  switch (tool) {
    case "cs2.status":
      return { content: status() };
    case "cs2.cfg":
      writeCfg = args.write === true;
      return { content: { ...status(), status: "ok" } };
    case "cs2.state":
      return playing()
        ? { content: { status: "ok" }, structuredContent: sampleMatch(), ui: { uri: "ui://kotone/cs2/match" } }
        : { content: { status: "no_data", text: "dev:web" } };
    case "cs2.rounds":
      return { content: playing() ? { status: "ok", rounds: sampleMatch().rounds } : { status: "no_data", text: "dev:web" } };
    default:
      return { content: { status: "error", text: tool } };
  }
}
