/**
 * dev:web 的插件后端（ADR 038）：照桌面端 `plugins.rs` 列出内置插件、按设置现算开关、
 * 插件页面调工具。清单的文字从插件自己的文案读（plugins/<id>/messages/zh-CN.json），和 plugin.toml 一致
 * （插件的 Rust 测试对过）。英雄联盟的工具返回示例数据（lolMock.ts），页面和聊天卡片照真的显示。
 */
import lolZh from "../../../../../plugins/lol/messages/zh-CN.json";
import type { PluginInfo, PluginManifest, PluginToolResult, PluginVisibility } from "../plugins";
import { getSettings } from "./browserMock";
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
};

const MANIFESTS: PluginManifest[] = [lol];

export async function listPlugins(): Promise<PluginInfo[]> {
  const settings = await getSettings();
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
  });
}

/** 和真的一样：工具要的数据开关关着时只回一句“已关闭”。 */
async function sourceOff(source: string): Promise<boolean> {
  const settings = await getSettings();
  const config = settings.plugins?.lol ?? {};
  const spec = lol.sources.find((s) => s.id === source);
  return (config.enabled ?? true) === false || (config.sources?.[source] ?? spec?.enabledByDefault ?? true) === false;
}

export async function pluginCall(plugin: string, tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  const spec = (TOOLS[plugin] ?? []).find((t) => t.name === tool);
  if (!spec) throw new Error(`${plugin}: ${tool}`);
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
    case "lol.scout":
      return { content: { status: "ok", text: "scout" }, structuredContent: sampleMatch(), ui: { uri: PAGE_MATCH } };
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
