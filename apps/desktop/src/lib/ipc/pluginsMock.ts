/**
 * dev:web 的插件后端（ADR 038）：照桌面端 `plugins.rs` 列出内置插件、按设置现算开关、
 * 插件页面调工具。清单的文字从插件自己的文案读（plugins/<id>/messages/zh-CN.json），和 plugin.toml 一致
 * （插件的 Rust 测试对过）。英雄联盟的工具返回示例数据（lolMock.ts），页面和聊天卡片照真的显示。
 */
import lolZh from "../../../../../plugins/lol/messages/zh-CN.json";
import type { PluginInfo, PluginManifest, PluginToolResult, PluginVisibility } from "../plugins";
import { getSettings } from "./browserMock";
import { findSamplePlayer, sampleGame, sampleMatch, samplePlayer } from "./lolMock";
import { PAGE_GAME, PAGE_MATCH, PAGE_PLAYER } from "$plugins/lol/ui/types";

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
  }],
  pages: [
    { uri: PAGE_MATCH, title: lolZh.plugin_lol_page_match, needs: ["client"] },
    { uri: PAGE_PLAYER, title: lolZh.plugin_lol_page_player, needs: ["client"] },
    { uri: PAGE_GAME, title: lolZh.plugin_lol_page_game, needs: ["client"] },
    { uri: "ui://kotone/lol/settings", title: lolZh.plugin_lol_tab_settings, needs: [] },
  ],
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

/** 和真的一样：数据开关关着时工具只回一句“已关闭”。 */
async function clientOff(): Promise<boolean> {
  const settings = await getSettings();
  const config = settings.plugins?.lol ?? {};
  return (config.enabled ?? true) === false || (config.sources?.client ?? true) === false;
}

export async function pluginCall(plugin: string, tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  const spec = (TOOLS[plugin] ?? []).find((t) => t.name === tool);
  if (!spec) throw new Error(`${plugin}: ${tool}`);
  if (plugin !== "lol") return { content: { status: "error", text: tool } };
  if (await clientOff()) return { content: { status: "disabled", text: lolZh.plugin_lol_source_client_name } };
  switch (tool) {
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
