// 插件的 MCP App（ADR 039 D5；MCP 方案 4.5、第 3 期）：宿主这一侧的数据和规则。显示在 McpAppFrame 里。
//
// - 打开：后端起这个插件的回环代理（每个插件一个源），给沙盒代理页的地址和资源的 `_meta.ui`（`plugin_app`）；
// - 工具名：App 的代码只有一份，在 Claude 这类宿主里调的是 `kotone.exe mcp` 对外的名字（`kotone_lol_tier`），
//   在 Kotone 里也一样；这里按和 `crates/kotone-cli/src/mcp.rs` 一样的规则还原成 Kotone 的工具名（点换成下划线、
//   前面加 `kotone_`；两个名字撞成一个时不猜），只在**宿主绑定的那个插件**的工具里找（页面说自己是谁不算数）；
// - 结果：插件工具的结果（`content` / `structuredContent` / `ui`）照 MCP 的 CallToolResult 交给 App，
//   和 `kotone.exe mcp` 给外部宿主的一样。
// 浏览器预览（dev:web）用 pluginsMock：沙盒代理页和 App 由 Vite 开发服务器在另一个源上给（vite.config.ts）。
import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { isTauri } from "./ipc";
import * as pluginsMock from "./ipc/pluginsMock";
import { listPlugins, pluginCall, pluginsStore, type PluginInfo, type PluginToolResult } from "./plugins";

/** 资源的 `_meta.ui`（MCP Apps `McpUiResourceMeta`）。 */
export interface AppResourceUi {
  csp?: { connectDomains?: string[]; resourceDomains?: string[]; frameDomains?: string[]; baseUriDomains?: string[] };
  prefersBorder?: boolean;
}

/** 打开一个 App 要的东西（后端 `plugin_app`）。 */
export interface AppHandle {
  plugin: string;
  uri: string;
  /** 回环代理上的沙盒代理页（`http://127.0.0.1:<端口>/<密钥>/sandbox/<插件>/<页面>`）。 */
  sandboxUrl: string;
  ui: AppResourceUi;
}

/** 打开一个插件页面的 App；插件或数据开关关着、页面不是 App 时报错。 */
export async function openApp(uri: string): Promise<AppHandle> {
  if (!isTauri) return pluginsMock.pluginApp(uri);
  return invoke<AppHandle>("plugin_app", { uri });
}

/** App 读自己插件的资源（`resources/read` 的 `contents` 一项）。 */
export async function readAppResource(plugin: string, uri: string): Promise<Record<string, unknown>> {
  if (!isTauri) return pluginsMock.pluginResource(plugin, uri);
  return invoke<Record<string, unknown>>("plugin_resource", { plugin, uri });
}

/** Kotone 工具名 → 对外（MCP）的名字（和 mcp.rs 的 `mcp_name` 一样，外部 Agent 已经在用，不能改）。 */
export function mcpToolName(tool: string): string {
  return `kotone_${tool.replaceAll(".", "_")}`;
}

/**
 * App 说的工具名 → 这个插件的 Kotone 工具名。认 MCP 名（`kotone_lol_tier`）和 Kotone 名（`lol.tier`）；
 * 不是这个插件的、只给她不给页面的、两个名字撞成一个的：null（不猜，照 mcp.rs）。
 */
export function resolveAppTool(name: string, plugin: PluginInfo): string | null {
  const tools = plugin.tools.filter((t) => t.visibility.includes("app")).map((t) => t.name);
  if (tools.includes(name)) return name;
  // 第三方插件（MCP 方案第 2 期）的 App 是它的服务器自己的页面，调的是服务器上原来的名字：在 Kotone 里是 `<插件 id>.<原名>`
  const prefixed = `${plugin.manifest.id}.${name}`;
  if ("remote" in plugin && tools.includes(prefixed)) return prefixed;
  const matches = tools.filter((t) => mcpToolName(t) === name);
  return matches.length === 1 ? matches[0] : null;
}

/** 插件信息（清单还没读过就读一次）。 */
export async function pluginInfo(id: string): Promise<PluginInfo | null> {
  let list = get(pluginsStore);
  if (!list.length) list = await listPlugins();
  return list.find((p) => p.manifest.id === id) ?? null;
}

/** MCP 的 CallToolResult。 */
export interface CallToolResult {
  content: { type: "text"; text: string }[];
  structuredContent?: Record<string, unknown>;
  isError?: boolean;
  _meta?: Record<string, unknown>;
  [key: string]: unknown;
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** 插件工具的结果 → CallToolResult（`content` 只有给模型的那份，`structuredContent` 是页面的数据）。 */
export function toCallToolResult(result: PluginToolResult): CallToolResult {
  const text = typeof result.content === "string" ? result.content : JSON.stringify(result.content ?? null);
  const out: CallToolResult = { content: [{ type: "text", text }] };
  if (isObject(result.structuredContent)) out.structuredContent = result.structuredContent;
  if (result.ui?.uri) out._meta = { ui: { resourceUri: result.ui.uri } };
  return out;
}

/**
 * App 调工具：只调宿主绑定的插件（`plugin`，按 iframe 记的）的、给页面的工具。后端还会再查一遍
 * （`plugins::Caller::Page`：别的插件的、不给页面的都拒）。不认识的名字抛错（App 收到 JSON-RPC 错误）。
 */
export async function callAppTool(plugin: string, name: string, args: Record<string, unknown>): Promise<CallToolResult> {
  const info = await pluginInfo(plugin);
  const tool = info ? resolveAppTool(name, info) : null;
  if (!tool) throw new Error(`the ${plugin} app cannot call ${name}`);
  return toCallToolResult(await pluginCall(plugin, tool, args));
}

/** App 让宿主打开一个链接：外链交给系统浏览器；本插件的 `ui://kotone/<插件>/<页面>?参数` 在 Kotone 里打开。 */
export function parseAppLink(url: string, plugin: string): { kind: "external"; url: string } | { kind: "page"; uri: string; params: Record<string, string> } | null {
  const page = new RegExp(`^(ui://kotone/${plugin}/[a-z0-9-]+)(?:\\?(.*))?$`).exec(url);
  if (page) return { kind: "page", uri: page[1], params: Object.fromEntries(new URLSearchParams(page[2] ?? "")) };
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "https:" || parsed.protocol === "http:") return { kind: "external", url: parsed.href };
  } catch {
    // 不是网址
  }
  return null;
}
