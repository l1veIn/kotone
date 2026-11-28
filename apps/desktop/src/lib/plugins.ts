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
import { m } from "./paraglide/messages.js";
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
/** 插件登记的游戏事件（`plugin.toml` 的 `[[events]]`，ADR 036 D4）：类型、中文说明、`data` 字段和说明、要的数据开关。 */
export interface PluginEventSpec {
  /** `<适配器>.<名字>` */
  type: string;
  /** 中文说明（界面上用插件的文案 `plugin_<id>_event_<名字>`，没有时用它）。 */
  label: string;
  data: Record<string, string>;
  needs: string[];
}
/** 推送的样子（ADR 043 D3）：卡片 / 一行 / 语音。 */
export type PushForm = "card" | "line" | "voice";
/**
 * 插件清单里的一条内置推送（`plugin.toml` 的 `[[pushes]]`，ADR 043 D4）：插件页「设置 → 推送」照它画一行（名字、时机、做什么、
 * 样子、开关）；开关在 `plugins.<id>.pushes.<推送 id>`，由宿主执行。
 */
export interface PluginPushSpec {
  id: string;
  title: string;
  when: string;
  what: string;
  forms: PushForm[];
  priority: "high" | "normal" | "low";
  enabledByDefault: boolean;
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
  agent: { overview: string; rule: string };
  /** 带适配器的插件登记的游戏事件（没有 = 不登记）。 */
  events?: PluginEventSpec[];
  /** 内置推送（没有 = 不推送）。 */
  pushes?: PluginPushSpec[];
  /** 许可和出处（`plugin.toml` 的 `[[notices]]`，ADR 043 D8；没有 = 没有）：和数据开关的风险一起放进“风险和许可”弹窗。 */
  notices?: PluginNoticeSpec[];
  /** 参考（`plugin.toml` 的 `[[references]]`，ADR 049 D7；没写 = 没有）：照着写的开源项目，「插件信息」里列出来。 */
  references?: PluginReferenceSpec[];
}
/** 一条参考：项目的名字和地址（`https://…`）。 */
export interface PluginReferenceSpec {
  name: string;
  url: string;
}
/** 一条许可或出处：标题、一段话、链接（`https://…`）。 */
export interface PluginNoticeSpec {
  id: string;
  title: string;
  body: string;
  links: string[];
}
export interface PluginToolInfo {
  name: string;
  visibility: PluginVisibility[];
  page: string | null;
  /** 插件和它要的数据开关都开着。 */
  available: boolean;
}
/**
 * 内置插件的「插件信息」里宿主统一填的（后端 `plugins::builtin_about`，ADR 049 D7）：作者、代码仓库、许可证、
 * 版本（= Kotone 的版本）、“参考”旁边链接的 THIRD_PARTY_NOTICES.md。第三方插件没有（它的在 `remote.server`）。
 */
export interface PluginAbout {
  author: string;
  repository: string;
  license: string;
  version: string;
  notices: string;
}
export interface PluginInfo {
  manifest: PluginManifest;
  /** 内置插件才有。 */
  about?: PluginAbout;
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
  /** 内置推送的开关（玩家动过的才有；ADR 043 D4）。 */
  pushes?: Record<string, boolean>;
  /** “风险和许可”弹窗看过第几版（ADR 043 D8；没有 = 没看过）。 */
  riskSeen?: number;
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

/**
 * 一条推送的开关在哪个位置（只看这一条；插件关着时开关灰着、位置不变）。和后端 `Manifest::push_switch_in` 同一条规则：
 * 玩家动过按他的；没动过按清单的默认——但旧配置里“自动卡片”总开关（`agent.autoCards.enabled`）是关的，带卡片的算关（读旧写新）。
 */
export function pushSwitch(settings: Settings | null | undefined, manifest: PluginManifest, push: PluginPushSpec): boolean {
  const own = configOf(settings, manifest.id).pushes?.[push.id];
  if (own !== undefined) return own;
  // 旧的总开关只管只有卡片的推送（rc.9 的自动卡片）；还有一行、语音的（海斗三选一）不受它管
  if (settings?.agent?.autoCards?.enabled === false && push.forms.every((form) => form === "card")) return false;
  return push.enabledByDefault;
}

/** 这条推送现在会不会推：插件开着、要的数据开关开着、这一条开着（宿主照同一条规则执行）。 */
export function pushEnabled(settings: Settings | null | undefined, manifest: PluginManifest, push: PluginPushSpec): boolean {
  return pluginEnabled(settings, manifest)
    && push.needs.every((source) => sourceEnabled(settings, manifest, source))
    && pushSwitch(settings, manifest, push);
}

/** 玩家在插件页上动了某条推送的开关：写 `plugins.<id>.pushes.<推送>`（从此不再看旧的总开关）。 */
export async function setPluginPush(id: string, push: string, enabled: boolean): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [id]: { pushes: { [push]: enabled } } } }));
}

/**
 * “风险和许可”弹窗的版本（ADR 043 D8）：设置里 `plugins.<id>.riskSeen` 比它小 = 这一版还没看过。以后风险变大了
 * （多了有风险的开关、说法有实质的变化）就加一，玩家会再看到一次。
 */
export const RISK_REVISION = 1;

/** 插件有没有要放进“风险和许可”弹窗的东西：数据开关的风险提示，或者许可和出处。 */
export function hasRiskNotes(manifest: PluginManifest): boolean {
  return manifest.sources.some((s) => s.risk) || (manifest.notices?.length ?? 0) > 0;
}

/**
 * 这一版的“风险和许可”还没看过（插件有这些东西才算）。`extra`：清单以外还有要放进弹窗的（第三方插件：它是别人写的
 * 程序，还有它自己写的风险说明）。
 */
export function riskUnseen(settings: Settings | null | undefined, manifest: PluginManifest, extra = false): boolean {
  return (hasRiskNotes(manifest) || extra) && (configOf(settings, manifest.id).riskSeen ?? 0) < RISK_REVISION;
}

/**
 * 第三方插件（`plugins_list` 里带 `remote` 的，见 mcpServers.ts）在“风险和许可”里要放的：它是第三方插件（总有），
 * 它自己在 Kotone 清单里写的风险说明（`risk`，ADR 047；没写是 null）。放在这里而不是 mcpServers.ts：弹窗
 * （PluginRiskDialog）也被内置插件的页面用，从这里拿不会绕成循环引用。
 */
export function thirdPartyRisk(plugin: PluginInfo | null | undefined): { thirdParty: boolean; risk: string | null } {
  const remote = (plugin as { remote?: { kotoneManifest?: { manifest?: { risk?: string | null } } } } | null | undefined)?.remote;
  return { thirdParty: !!remote, risk: remote?.kotoneManifest?.manifest?.risk?.trim() || null };
}

/** 有风险的数据开关里现在有开着的（插件也开着）：第一次打开插件页、第一次打开这样的开关时弹“风险和许可”。 */
export function riskyOn(settings: Settings | null | undefined, manifest: PluginManifest): boolean {
  return manifest.sources.some((s) => s.risk && sourceEnabled(settings, manifest, s.id));
}

/** 记下看过这一版的“风险和许可”（弹出来就记；只是提示，不动任何开关）。 */
export async function markRiskSeen(id: string): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { [id]: { riskSeen: RISK_REVISION } } }));
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
/** 推送是谁发的：插件的名字（插件的文案）。自动化程序里的 push()（ADR 045 D2）也走推送，记成 plugin = "automation"，
 * 那是本体的“自动化”，不是插件，名字用本体的文案。 */
export const pushFromName = (id: string, fallback: string) => (id === "automation" ? m.auto_push_from() : pluginText(id, "name", fallback));
/** 推送的名字、时机、做什么：插件的文案 `plugin_<id>_push_<推送>_title / _when / _what`，没有时用清单里的中文。 */
export const pushTitle = (id: string, push: { id: string; title: string }) => pluginText(id, `push_${keyPart(push.id)}_title`, push.title);
export const pushWhen = (m: PluginManifest, p: PluginPushSpec) => pluginText(m.id, `push_${keyPart(p.id)}_when`, p.when);
export const pushWhat = (m: PluginManifest, p: PluginPushSpec) => pluginText(m.id, `push_${keyPart(p.id)}_what`, p.what);
/** 许可和出处的标题、正文：插件的文案 `plugin_<id>_notice_<条目>_title / _body`，没有时用清单里的中文。 */
export const noticeTitle = (m: PluginManifest, n: PluginNoticeSpec) => pluginText(m.id, `notice_${keyPart(n.id)}_title`, n.title);
export const noticeBody = (m: PluginManifest, n: PluginNoticeSpec) => pluginText(m.id, `notice_${keyPart(n.id)}_body`, n.body);

/** 左侧栏里插件页面的 id。 */
export function pluginPageId(id: string): `plugin:${string}` {
  return `plugin:${id}`;
}

/** 页面 id 是哪个插件的（不是插件页面时 null）。 */
export function pluginOfPage(page: string): string | null {
  return page.startsWith("plugin:") ? page.slice("plugin:".length) : null;
}
