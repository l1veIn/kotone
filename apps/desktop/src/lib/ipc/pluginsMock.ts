/**
 * dev:web 的插件后端（ADR 038）：照桌面端 `plugins.rs` 列出内置插件、按设置现算开关、
 * 插件页面调工具。清单的文字从插件自己的文案读（plugins/<id>/messages/zh-CN.json），和 plugin.toml 一致
 * （插件的 Rust 测试对过）。
 */
import lolZh from "../../../../../plugins/lol/messages/zh-CN.json";
import type { PluginInfo, PluginManifest, PluginToolResult } from "../plugins";
import { getSettings } from "./browserMock";

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
  pages: [{ uri: "ui://kotone/lol/settings", title: lolZh.plugin_lol_tab_settings, needs: [] }],
  overlays: [],
  agent: { overview: "", rule: "" },
};

const TOOLS: Record<string, { name: string; needs: string[] }[]> = {
  lol: [
    { name: "lol.match", needs: ["client"] },
    { name: "lol.spell", needs: ["client"] },
    { name: "lol.scout", needs: ["client"] },
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
        visibility: ["model", "app"],
        page: null,
        available: enabled && t.needs.every((n) => sources[n]),
      })),
    };
  });
}

export async function pluginCall(plugin: string, tool: string, _args: Record<string, unknown>): Promise<PluginToolResult> {
  if (!(TOOLS[plugin] ?? []).some((t) => t.name === tool)) throw new Error(`${plugin}: ${tool}`);
  return { content: { status: "not_in_game", text: "dev:web" } };
}
