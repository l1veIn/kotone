// 和宿主说话（MCP Apps 规范 2026-01-26）：用 ext-apps 官方的 App（握手顺序照规范：ui/initialize →
// initialized 之后宿主才推东西，ext-apps #671 的教训）。同一份代码在 Kotone 和 Claude 这类宿主里都能用：
// - 调工具用 `kotone.exe mcp` 对外的名字（`kotone_lol_tier`）；Kotone 显示时宿主的桥按同一张对照表还原成 `lol.tier`；
// - 主题、语言从宿主上下文（hostContext）来。
import { App, applyDocumentTheme, type McpUiHostContext } from "@modelcontextprotocol/ext-apps";
import { setLocale } from "./i18n.svelte";

export type ToolResult = Awaited<ReturnType<App["callServerTool"]>>;

/** 对外（MCP）的工具名：Kotone 工具名的点换成下划线，前面加 `kotone_`（`crates/kotone-cli/src/mcp.rs`）。 */
export const TOOL_TIER = "kotone_lol_tier";

/** 插件给资源时插的那一行（`plugins/lol/src/app.rs`）：这个 HTML 显示哪一页。 */
export function currentPage(): string {
  return document.querySelector('meta[name="kotone:page"]')?.getAttribute("content") ?? "tier";
}

export interface Session {
  context: McpUiHostContext | undefined;
  /** 宿主推来的工具结果（聊天卡片、Claude 里调了 `lol.tier`）；宿主没推（插件页直接打开）时是 null。 */
  initial: Promise<ToolResult | null>;
  /** 之后宿主又推来的结果（例如她又调了一次）。 */
  onResult(listener: (result: ToolResult) => void): void;
  call(tool: string, args?: Record<string, unknown>): Promise<ToolResult>;
}

function applyContext(context: McpUiHostContext | undefined) {
  if (context?.theme) applyDocumentTheme(context.theme);
  if (context?.locale) setLocale(context.locale);
}

/** 宿主说了是因为调工具才打开的，最多等这么久它把结果推来，不来就自己取。 */
const RESULT_WAIT_MS = 4000;

export async function connect(): Promise<Session> {
  const app = new App({ name: "Kotone 英雄联盟", version: "0.1.0" });
  const listeners: ((result: ToolResult) => void)[] = [];
  let settle!: (result: ToolResult | null) => void;
  let settled = false;
  const initial = new Promise<ToolResult | null>((resolve) => {
    settle = (result) => {
      if (settled) return;
      settled = true;
      resolve(result);
    };
  });
  // 处理函数要在 connect 之前挂好：握手一完成宿主就会推工具的输入和结果
  app.ontoolresult = (result) => {
    if (!settled) settle(result);
    else for (const listener of listeners) listener(result);
  };
  app.ontoolcancelled = () => settle(null);
  app.onhostcontextchanged = (changed) => applyContext({ ...app.getHostContext(), ...changed });
  app.onerror = (error) => console.error("[lol app]", error);
  await app.connect();
  const context = app.getHostContext();
  applyContext(context);
  if (context?.toolInfo) setTimeout(() => settle(null), RESULT_WAIT_MS);
  else settle(null);
  return {
    context,
    initial,
    onResult: (listener) => listeners.push(listener),
    call: (tool, args = {}) => app.callServerTool({ name: tool, arguments: args }),
  };
}

/** 结果里给模型的那几行字（`{"status","text"}` 的 JSON，或者就是一句话）。 */
export function resultText(result: ToolResult): string {
  const block = result.content?.find((c) => c.type === "text");
  const text = block && "text" in block ? String(block.text) : "";
  try {
    const parsed = JSON.parse(text) as { text?: unknown; error?: unknown };
    if (typeof parsed.text === "string") return parsed.text;
    if (typeof parsed.error === "string") return parsed.error;
  } catch {
    // 就是一句话
  }
  return text;
}
