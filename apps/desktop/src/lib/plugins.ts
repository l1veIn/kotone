// 插件（ADR 038）：插件清单、开关、插件页面调本插件的工具。字段与后端（`plugins_cmd.rs`、
// `kotone-plugin-api` 的 serde，camelCase）一致；浏览器预览（dev:web）用 ipc/pluginsMock。
//
// - 清单从后端读一次（`plugins_list`），开关的状态按设置 `plugins.<id>` 现算（设置一变界面就跟着变）；
// - 开关就是设置：`plugins.<id>.enabled`、`plugins.<id>.sources.<开关>`（没写按清单的默认）；
// - 界面上插件的文字用插件自己的文案（`plugins/<id>/messages/<语言>.json`，键 `plugin_<id>_…`），
//   找不到时用清单里的中文。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { writable, type Readable } from "svelte/store";
import { isTauri, updateSettings } from "./ipc";
import type { Settings } from "./ipc/contracts";
import * as pluginsMock from "./ipc/pluginsMock";
import * as messages from "./paraglide/messages.js";
import { settingsStore } from "./stores/ui";

export type PluginVisibility = "model" | "app";

export interface PluginSourceSpec {
  id: string;
  name: string;
  description: string;
  enabledByDefault: boolean;
  /** 风险提示；空 = 没有。 */
  risk: string;
}
export interface PluginPageSpec {
  /** `ui://kotone/<插件>/<页面>` */
  uri: string;
  title: string;
  needs: string[];
}
export interface PluginManifest {
  id: string;
  name: string;
  description: string;
  /** 挂在哪些游戏配置上（游戏配置 id）。 */
  games: string[];
  adapter: string | null;
  enabledByDefault: boolean;
  sources: PluginSourceSpec[];
  pages: PluginPageSpec[];
  overlays: PluginPageSpec[];
  agent: { overview: string; rule: string };
}
export interface PluginToolInfo {
  name: string;
  visibility: PluginVisibility[];
  page: string | null;
  /** 插件和它要的数据开关都开着。 */
  available: boolean;
}
export interface PluginInfo {
  manifest: PluginManifest;
  /** 后端列出来那一刻的开关（界面用 [`pluginEnabled`] / [`sourceEnabled`] 按设置现算）。 */
  enabled: boolean;
  sources: Record<string, boolean>;
  tools: PluginToolInfo[];
}
/** 工具结果（照 MCP）：content 给她，structuredContent 给页面，ui 用哪个页面显示。 */
export interface PluginToolResult {
  content: unknown;
  structuredContent?: unknown;
  ui?: { uri: string; params?: unknown };
}

/** 插件的设置（config.json `plugins.<id>`）。 */
export interface PluginConfig {
  enabled?: boolean;
  sources?: Record<string, boolean>;
  settings?: Record<string, unknown>;
}

const store = writable<PluginInfo[]>([]);
/** 全部插件（后端的静态表，按侧栏顺序）。 */
export const pluginsStore: Readable<PluginInfo[]> = { subscribe: store.subscribe };

/** 读一次插件清单（启动设置窗口时；之后开关按设置现算，不用再读）。 */
export async function listPlugins(): Promise<PluginInfo[]> {
  const raw = isTauri ? await invoke<PluginInfo[] | null>("plugins_list") : await pluginsMock.listPlugins();
  // 只模拟了部分命令的测试会回 null：当成没有插件，别让用到插件清单的地方出错
  const list = Array.isArray(raw) ? raw : [];
  store.set(list);
  return list;
}

/** 插件页面调本插件的工具（只给页面用的也能调）。 */
export async function pluginCall(plugin: string, tool: string, args: Record<string, unknown> = {}): Promise<PluginToolResult> {
  if (!isTauri) return pluginsMock.pluginCall(plugin, tool, args);
  return invoke<PluginToolResult>("plugin_call", { plugin, tool, arguments: args });
}

/** 取消订阅（插件的页面在 apps/desktop 外面，用这个名字，不直接引 Tauri 的包）。 */
export type Unlisten = UnlistenFn;

/** 插件发给界面的事件（后端 `Host::emit`：`{plugin, event, payload}`，和 `plugins.rs` 的 `EVENT` 一致）。 */
export const PLUGIN_EVENT = "kotone://plugin";

/** 听某个插件的某个事件（例如放歌插件的 `state`）。返回取消订阅。 */
export async function listenPluginEvent<T>(plugin: string, event: string, callback: (payload: T) => void): Promise<Unlisten> {
  if (!isTauri) return pluginsMock.subscribe(plugin, event, (payload) => callback(payload as T));
  return listen<{ plugin: string; event: string; payload: T }>(PLUGIN_EVENT, (e) => {
    if (e.payload?.plugin === plugin && e.payload.event === event) callback(e.payload.payload);
  });
}

function configOf(settings: Settings | null | undefined, id: string): PluginConfig {
  return settings?.plugins?.[id] ?? {};
}

/** 插件开着没有（设置里写了按设置，没写按清单的默认）。 */
export function pluginEnabled(settings: Settings | null | undefined, manifest: PluginManifest): boolean {
  return configOf(settings, manifest.id).enabled ?? manifest.enabledByDefault;
}

/** 插件里的某个数据开关开着没有（插件关着也算关）。 */
export function sourceEnabled(settings: Settings | null | undefined, manifest: PluginManifest, source: string): boolean {
  const spec = manifest.sources.find((s) => s.id === source);
  if (!spec || !pluginEnabled(settings, manifest)) return false;
  return configOf(settings, manifest.id).sources?.[source] ?? spec.enabledByDefault;
}

/** 数据开关自己的位置（不看插件开没开）：插件关着时开关显示成灰的、但保留原来的位置。 */
export function sourceSwitch(settings: Settings | null | undefined, manifest: PluginManifest, source: string): boolean {
  const spec = manifest.sources.find((s) => s.id === source);
  return configOf(settings, manifest.id).sources?.[source] ?? spec?.enabledByDefault ?? false;
}

export async function setPluginEnabled(id: string, enabled: boolean): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [id]: { enabled } } }));
}

export async function setPluginSource(id: string, source: string, enabled: boolean): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [id]: { sources: { [source]: enabled } } } }));
}

/** 文案键里的插件 id、开关 id：连字符换成下划线（`voice-input` → `voice_input`）。 */
function keyPart(id: string): string {
  return id.replace(/[^A-Za-z0-9_]/g, "_");
}

/**
 * 插件的文字：`plugin_<id>_<key>`（例如 `name`、`description`、`source_client_risk`），
 * 当前语言没有这条时用 fallback（清单里的中文）。
 */
export function pluginText(id: string, key: string, fallback: string): string {
  const fn = (messages as unknown as Record<string, ((inputs?: Record<string, unknown>) => string) | undefined>)[
    `plugin_${keyPart(id)}_${key}`
  ];
  return typeof fn === "function" ? fn({}) : fallback;
}

export const pluginName = (m: PluginManifest) => pluginText(m.id, "name", m.name);
export const pluginDescription = (m: PluginManifest) => pluginText(m.id, "description", m.description);
export const sourceName = (m: PluginManifest, s: PluginSourceSpec) => pluginText(m.id, `source_${keyPart(s.id)}_name`, s.name);
export const sourceDescription = (m: PluginManifest, s: PluginSourceSpec) => pluginText(m.id, `source_${keyPart(s.id)}_desc`, s.description);
export const sourceRisk = (m: PluginManifest, s: PluginSourceSpec) => (s.risk ? pluginText(m.id, `source_${keyPart(s.id)}_risk`, s.risk) : "");

/** 左侧栏里插件页面的 id。 */
export function pluginPageId(id: string): `plugin:${string}` {
  return `plugin:${id}`;
}

/** 页面 id 是哪个插件的（不是插件页面时 null）。 */
export function pluginOfPage(page: string): string | null {
  return page.startsWith("plugin:") ? page.slice("plugin:".length) : null;
}
