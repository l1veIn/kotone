// 和宿主说话（MCP Apps 规范 2026-01-26）：用 ext-apps 官方的 App（握手顺序照规范：ui/initialize →
// initialized 之后宿主才推东西，ext-apps #671 的教训）。同一份代码在 Kotone 和 Claude 这类宿主里都能用：
// - 调工具用 `kotone.exe mcp` 对外的名字（`kotone_lol_tier`）；Kotone 显示时宿主的桥按同一张对照表还原成 `lol.tier`；
// - 主题、宿主的颜色和字体、语言、显示方式（inline 聊天卡片 / fullscreen 插件页）从宿主上下文（hostContext）来，
//   写到 <html> 上（docs/design/lol-app.md 第一节 2）：`data-theme`、`--color-*`、`data-display`。
import {
  App,
  applyDocumentTheme,
  applyHostFonts,
  applyHostStyleVariables,
  type McpUiDisplayMode,
  type McpUiHostContext,
} from "@modelcontextprotocol/ext-apps";
import { setLocale } from "./i18n.svelte";
import type { ToolState } from "./status";

export type ToolResult = Awaited<ReturnType<App["callServerTool"]>>;
export type DisplayMode = McpUiDisplayMode;

/** 对外（MCP）的工具名：Kotone 工具名的点换成下划线，前面加 `kotone_`（`crates/kotone-cli/src/mcp.rs`）。 */
export const TOOL_TIER = "kotone_lol_tier";
export const TOOL_SCOUT = "kotone_lol_scout";
export const TOOL_PLAYER = "kotone_lol_player";
export const TOOL_GAME = "kotone_lol_game";
export const TOOL_CHAMPION = "kotone_lol_champion";

/** 插件的设置页（宿主的页面：数据开关、风险提示、操作客户端的偏好）。 */
export const PAGE_SETTINGS = "ui://kotone/lol/settings";

/** 插件给资源时插的那一行（`plugins/lol/src/app.rs`）：这个 HTML 显示哪一页。 */
export function currentPage(): string {
  return document.querySelector('meta[name="kotone:page"]')?.getAttribute("content") ?? "tier";
}

export interface Session {
  context: McpUiHostContext | undefined;
  /** 宿主推来的工具结果（聊天卡片、Claude 里调了 `lol.scout`）；宿主没推（插件页直接打开）时是 null。 */
  initial: Promise<ToolResult | null>;
  /** 之后宿主又推来的结果（例如她又调了一次）。 */
  onResult(listener: (result: ToolResult) => void): void;
  /** 宿主推来的工具参数（`ui/notifications/tool-input`）；结果没来时页面照它自己取（例如单局详情的 gameId）。 */
  input(): Record<string, unknown> | null;
  /** 宿主上下文变了（主题、显示方式……）；已经写到 <html> 上了，页面按需要更新自己。 */
  onContext(listener: (context: McpUiHostContext) => void): void;
  call(tool: string, args?: Record<string, unknown>): Promise<ToolResult>;
  /** 请宿主换显示方式（卡片上的“打开”）；宿主不支持时什么也不做。 */
  requestDisplayMode(mode: DisplayMode): Promise<void>;
  /** 用宿主打开一个网址（出处链接）。 */
  openLink(url: string): void;
  /**
   * 请宿主打开本插件的另一个页面（`ui://kotone/lol/<页面>?参数`；Kotone 的插件页认，在里面点进去、能返回）。
   * App 里还没有的页面（英雄）先这样开；宿主不认时什么也不发生。
   */
  openPage(uri: string, args: Record<string, string>): void;
}

/** 宿主上下文 → <html>。没给的项不动（规范：host-context-changed 只带变了的项）。 */
function applyContext(context: McpUiHostContext | undefined) {
  if (!context) return;
  if (context.theme) applyDocumentTheme(context.theme);
  if (context.styles?.variables) applyHostStyleVariables(context.styles.variables);
  if (context.styles?.css?.fonts) applyHostFonts(context.styles.css.fonts);
  if (context.locale) setLocale(context.locale);
  if (context.displayMode) document.documentElement.dataset.display = context.displayMode;
}

/** 显示方式：宿主没说就当聊天卡片（Claude 默认就是 inline）。 */
export function displayOf(context: McpUiHostContext | undefined): DisplayMode {
  return context?.displayMode ?? "inline";
}

/** 宿主说了是因为调工具才打开的，最多等这么久它把结果推来，不来就自己取。 */
const RESULT_WAIT_MS = 4000;

export async function connect(): Promise<Session> {
  document.documentElement.dataset.display ??= "inline";
  const app = new App({ name: "kotone-lol", version: "0.2.0" });
  const listeners: ((result: ToolResult) => void)[] = [];
  const contextListeners: ((context: McpUiHostContext) => void)[] = [];
  let settle!: (result: ToolResult | null) => void;
  let settled = false;
  let input: Record<string, unknown> | null = null;
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
  app.ontoolinput = (params) => (input = params.arguments ?? null);
  app.ontoolcancelled = () => settle(null);
  app.onhostcontextchanged = () => {
    const context = app.getHostContext() ?? {};
    applyContext(context);
    for (const listener of contextListeners) listener(context);
  };
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
    input: () => input,
    onContext: (listener) => contextListeners.push(listener),
    call: (tool, args = {}) => app.callServerTool({ name: tool, arguments: args }),
    requestDisplayMode: async (mode) => {
      if (!app.getHostContext()?.availableDisplayModes?.includes(mode)) return;
      try {
        await app.requestDisplayMode({ mode });
      } catch (error) {
        console.error("[lol app] requestDisplayMode", error);
      }
    },
    openLink: (url) => {
      app.openLink({ url }).catch(() => window.open(url, "_blank", "noopener"));
    },
    openPage: (uri, args) => {
      const url = `${uri}?${new URLSearchParams(args)}`;
      app.openLink({ url }).catch((error) => console.error("[lol app] openPage", error));
    },
  };
}

/** 结果里给模型的那几行字（`{"status","text"}` 的 JSON，或者就是一句话）。 */
export function resultText(result: ToolResult): string {
  return resultStatus(result).text;
}

/** 结果里的状态（`ok` / `not_in_game` / `error`……）、细分的原因和那句话；不是 JSON 时状态是空（出错时 `error`）。 */
export function resultStatus(result: ToolResult): ToolState {
  const block = result.content?.find((c) => c.type === "text");
  const text = block && "text" in block ? String(block.text) : "";
  try {
    const parsed = JSON.parse(text) as { status?: unknown; reason?: unknown; text?: unknown; error?: unknown };
    const status = typeof parsed.status === "string" ? parsed.status : result.isError ? "error" : "";
    const reason = typeof parsed.reason === "string" ? parsed.reason : undefined;
    if (typeof parsed.text === "string") return { status, reason, text: parsed.text };
    if (typeof parsed.error === "string") return { status: "error", text: parsed.error };
    return { status, reason, text: "" };
  } catch {
    // 就是一句话
  }
  return { status: result.isError ? "error" : "", text };
}
