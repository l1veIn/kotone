// 第三方插件（MCP 方案第 2 期）：玩家自己加的别人的 MCP 服务器，一个服务器就是一个插件（后端 `remote_plugins.rs`、
// `mcp_servers_cmd.rs`，crate `kotone-remote-plugin`）。这里是界面那一半：
//
// - 插件清单（`plugins_list`）里第三方插件多一个 `remote`：怎么连、要哪些 Key、信任没有、连接状态、服务器说的自己
//   （名字、说明、图标，后端已剥掉看不见的字符）、它的 MCP App 页面、一页工具列表；
// - 加服务器：粘贴配置或从 Claude Desktop / VS Code 导入 → 候选 → 信任提示 → `mcp_server_add`；
// - 卡片和插件页：开关照旧是设置 `plugins.<id>.enabled`；点卡片进它的 MCP App（有 `ui://` 页面的）或一页工具列表，
//   顶栏的“设置”里是连接、Key、信任、删除（[`REMOTE_UI`] 登记进 `PLUGIN_UI`，和内置插件同一个容器）；
// - 确认卡片：第三方工具多一个“总是允许这个工具”（权限里这个工具一条“允许”）或“信任这个插件”（它说只读的工具不再问）；
// - `mcp.json` 的情况（`mcp_servers_status`）：读不出来的原因、没加进来的项（名字和 Kotone 自带的撞了、不能当 id），
//   插件页的第三方一组里说一句人话。插件清单本身还是一个数组，这些另外读。
// 浏览器预览（dev:web、Playwright）用 ipc/mcpServersMock。
import { invoke } from "@tauri-apps/api/core";
import type { Component } from "svelte";
import { writable, type Readable } from "svelte/store";
import { Blocks } from "@lucide/svelte";
import { isTauri, updateSettings } from "./ipc";
import * as mock from "./ipc/mcpServersMock";
import { listPlugins, pluginsStore, type PluginInfo, type PluginVisibility } from "./plugins";
import { PLUGIN_UI, type PluginPageProps } from "./pluginUi";
import { settingsStore } from "./stores/ui";
import RemotePluginPage from "./components/plugins/RemotePluginPage.svelte";
import RemotePluginSettings from "./components/plugins/RemotePluginSettings.svelte";

/** 信任提示要给玩家看的事实（后端 `ServerSpec::trust_facts`；环境变量、请求头只有名字）。 */
export interface TrustFacts {
  kind: "stdio" | "http";
  summary: string;
  command?: string;
  args?: string[];
  cwd?: string | null;
  /** npx / uvx / docker 后面的包名或镜像（猜不出是 null）。 */
  package?: string | null;
  envNames?: string[];
  url?: string;
  headerNames?: string[];
  /** 要玩家填的 Key（`${input:名字}`）。 */
  keys: string[];
}

/** 服务器说的自己（握手时的 serverInfo）。 */
export interface RemoteServerAbout {
  name: string;
  title?: string | null;
  version?: string;
  description?: string | null;
  icons: { src: string; mimeType?: string; sizes?: string[]; theme?: string }[];
  websiteUrl?: string | null;
}

export interface RemoteTool {
  name: string;
  title?: string | null;
  description?: string | null;
  readOnly: boolean;
  visibility: PluginVisibility[];
  page?: string | null;
}

/** `plugins_list` 每项的 `remote`（后端 `remote_plugins::describe`）。 */
export interface RemoteInfo {
  status: "notConnected" | "connecting" | "connected" | "failed";
  error?: string | null;
  trusted: boolean;
  server?: RemoteServerAbout | null;
  pages: { uri: string; title: string; slug: string }[];
  toolCount: number;
  skippedTools?: string[];
  config?: TrustFacts;
  configError?: string;
  keys: { name: string; filled: boolean }[];
  fingerprint: string;
  toolList?: RemoteTool[];
}

export type RemotePluginInfo = PluginInfo & { remote: RemoteInfo };

/** `mcp.json` 里没加进来的一项（后端 `remote_plugins::Problem`）。 */
export interface McpProblem {
  /** `mcp.json` 里的键。 */
  id: string;
  /** taken：和 Kotone 自带的插件或工具撞名；invalid：不能当插件 id。 */
  kind: "taken" | "invalid";
  /** 后端的原话。 */
  error: string;
}

/** `mcp.json` 的情况（后端 `mcp_servers_status`）。 */
export interface McpFileStatus {
  /** 文件在哪。 */
  path: string;
  /** 读不出来的原因（读不出来时第三方插件都没加载）。 */
  fileError: string | null;
  problems: McpProblem[];
}

const statusStore = writable<McpFileStatus | null>(null);
/** 上一次读到的 `mcp.json` 情况（没读过、读不到命令时是 null）。 */
export const mcpFileStatus: Readable<McpFileStatus | null> = { subscribe: statusStore.subscribe };

/** 读一次 `mcp.json` 的情况。只模拟了部分命令的测试会回 null：当成没有要说的。 */
export async function loadMcpStatus(): Promise<McpFileStatus | null> {
  try {
    const out = isTauri ? await invoke<McpFileStatus | null>("mcp_servers_status") : await mock.status();
    const status = out && typeof out === "object" && Array.isArray(out.problems) ? out : null;
    statusStore.set(status);
    return status;
  } catch {
    statusStore.set(null);
    return null;
  }
}

/** 重新读 `mcp.json`（玩家手改过、改好了写坏的）。读不出来时原因在 [`mcpFileStatus`] 里，这里不另报错。 */
export async function reloadServers(): Promise<void> {
  try {
    if (isTauri) await invoke("mcp_servers_reload");
    else await mock.reload();
  } catch {
    // 读不出来：原因由 mcp_servers_status 给
  }
  await refresh();
}

/** 一个候选的服务器（粘贴的、导入的；后端 `remote_plugins::preview`）。 */
export interface McpCandidate {
  /** 原来的名字（没有时后端猜的）。 */
  name: string;
  /** 建议的插件 id（和已有的不撞）。 */
  id: string;
  /** 原样的配置（加的时候原样交回去，明文 Key 由后端搬进凭据库）。 */
  entry: Record<string, unknown>;
  warnings: string[];
  /** 会搬进 Windows 凭据库的 Key 的名字。 */
  movedKeys: string[];
  facts?: TrustFacts;
  /** 配置写错了。 */
  error?: string;
  /** 已经加过同一份配置（那个插件的 id）。 */
  alreadyAdded?: string | null;
}

export interface McpImportSource {
  source: "claude-desktop" | "vscode";
  name: string;
  files: { path: string; error?: string; servers: McpCandidate[] }[];
}

export function remoteOf(plugin: PluginInfo | undefined | null): RemoteInfo | null {
  return ((plugin as Partial<RemotePluginInfo> | undefined)?.remote as RemoteInfo | undefined) ?? null;
}

export function isRemote(plugin: PluginInfo | undefined | null): plugin is RemotePluginInfo {
  return !!remoteOf(plugin);
}

/** 卡片上的图标：服务器声明的（只用 `data:image/`：主窗口不去连别人的网址拿图，也不让它知道你在看）；没有 = null（首字块）。 */
export function remoteIcon(plugin: PluginInfo | undefined | null): string | null {
  const icons = remoteOf(plugin)?.server?.icons ?? [];
  return icons.map((i) => i.src).find((src) => typeof src === "string" && src.startsWith("data:image/")) ?? null;
}

/** 卡片上的第二行名字：服务器自己说的名字（和插件 id 不一样时）。 */
export function remoteTitle(plugin: PluginInfo): string | null {
  const server = remoteOf(plugin)?.server;
  const title = server?.title?.trim() || server?.name?.trim() || "";
  return title && title !== plugin.manifest.id ? title : null;
}

/** 某个工具是不是第三方插件的：插件和这个工具（`<id>.<工具>`）。 */
export function thirdPartyTool(tool: string, plugins: PluginInfo[]): { plugin: RemotePluginInfo; tool: RemoteTool | null } | null {
  const plugin = plugins.find((p): p is RemotePluginInfo => isRemote(p) && tool.startsWith(`${p.manifest.id}.`));
  if (!plugin) return null;
  return { plugin, tool: plugin.remote.toolList?.find((t) => t.name === tool) ?? null };
}

/** 粘贴的配置里有哪些服务器（不改任何东西）。 */
export async function previewServers(text: string): Promise<McpCandidate[]> {
  const out = isTauri ? await invoke<{ servers: McpCandidate[] }>("mcp_servers_preview", { text }) : await mock.preview(text);
  return out.servers;
}

/** Claude Desktop、VS Code 的配置里有哪些服务器（不改任何东西）。 */
export async function importSources(): Promise<McpImportSource[]> {
  const out = isTauri ? await invoke<{ sources: McpImportSource[] }>("mcp_servers_import_sources") : await mock.importSources();
  return out.sources;
}

/** 加一个（玩家看过信任提示、同意了才调）。加完重新读插件清单。 */
export async function addServer(id: string, entry: Record<string, unknown>): Promise<void> {
  // 界面里的候选是响应式的代理：交出去前变回普通对象
  entry = JSON.parse(JSON.stringify(entry)) as Record<string, unknown>;
  if (isTauri) await invoke("mcp_server_add", { id, entry });
  else await mock.add(id, entry);
  await refresh();
}

export async function removeServer(id: string): Promise<void> {
  if (isTauri) await invoke("mcp_server_remove", { id });
  else await mock.remove(id);
  await refresh();
}

/** 信任 / 不再信任这个插件（它说只读的工具不再问 / 又要问）。 */
export async function trustServer(id: string, trusted = true): Promise<void> {
  if (isTauri) await invoke("mcp_server_trust", { id, trusted });
  else await mock.trust(id, trusted);
  await refresh();
}

/** 重新连接；连不上时返回原因。 */
export async function reconnectServer(id: string): Promise<string | null> {
  const out = isTauri
    ? await invoke<{ connected: boolean; error?: string }>("mcp_server_reconnect", { id })
    : await mock.reconnect(id);
  await refresh();
  return out.connected ? null : (out.error ?? "");
}

/** 填（value 是空的 = 清掉）一个 Key：存进 Windows 凭据库，下一次连接时用。 */
export async function setServerKey(id: string, name: string, value: string | null): Promise<void> {
  if (isTauri) await invoke("mcp_server_set_key", { id, name, value });
  else await mock.setKey(id, name, value);
  await refresh();
}

/** “总是允许这个工具”：权限里这个工具一条“允许”（角色页 → 权限里也看得到、改得回来）。 */
export async function alwaysAllow(tool: string): Promise<void> {
  settingsStore.set(await updateSettings({ agent: { permissions: { rules: { [tool]: "allow" } } } } as never));
}

async function refresh(): Promise<void> {
  await listPlugins();
  await loadMcpStatus();
}

/** 第三方插件在插件页（同一个容器，`PluginPage.svelte`）里的样子：App 或工具列表；顶栏的“设置”是连接、Key、信任、删除。 */
export const REMOTE_UI = {
  icon: Blocks as unknown as Component,
  page: RemotePluginPage as unknown as Component<PluginPageProps>,
  settings: RemotePluginSettings as unknown as Component<PluginPageProps>,
};

/** 第三方插件登记进 `PLUGIN_UI`（内置的编译时登记，第三方的读到清单时登记；删掉的拿掉）。 */
function register(list: PluginInfo[]): void {
  for (const [id, ui] of Object.entries(PLUGIN_UI)) {
    if (ui === REMOTE_UI && !list.some((p) => p.manifest.id === id && isRemote(p))) delete PLUGIN_UI[id];
  }
  for (const plugin of list) {
    if (isRemote(plugin) && !PLUGIN_UI[plugin.manifest.id]) PLUGIN_UI[plugin.manifest.id] = REMOTE_UI;
  }
}
pluginsStore.subscribe(register);
