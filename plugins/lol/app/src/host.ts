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
export const TOOL_HISTORY = "kotone_lol_history";
export const TOOL_GAME = "kotone_lol_game";
export const TOOL_CHAMPION = "kotone_lol_champion";
/** 来源标签换来源（只给页面，ADR 043 D8）：记在插件设置里。 */
export const TOOL_SOURCE = "kotone_lol_source";

/** 插件的设置页（宿主的页面：数据开关、风险提示、操作客户端的偏好）。 */
export const PAGE_SETTINGS = "ui://kotone/lol/settings";

/** 插件给资源时插的那一行（`plugins/lol/src/app.rs`）：这个 HTML 显示哪一页。 */
export function currentPage(): string {
  return document.querySelector('meta[name="kotone:page"]')?.getAttribute("content") ?? "home";
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
  /**
   * 宿主没推结果（`initial` 是 null）、页面自己取第一份时用这个，不用 `call`：取的路上宿主推来了结果（推得晚的宿主），
   * 给推来的那份，不让自己取的（常常是缺省参数）盖掉宿主的。之后玩家自己点的（换分路、刷新）照常用 `call`。
   */
  firstCall(tool: string, args?: Record<string, unknown>): Promise<ToolResult>;
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

/** 宿主说了是因为调工具才打开的（toolInfo，或者推来了工具的输入），最多等这么久它把结果推来，不来就自己取。 */
const RESULT_WAIT_MS = 4000;
/**
 * 宿主上下文里没有 toolInfo（规范里是可选的，ext-apps 的 basic-host 就不带）时，先等这么久看宿主推不推工具的输入或结果：
 * 推了就是因工具打开的，照上面等结果；没推就当是直接打开的，自己取。
 */
const HINT_WAIT_MS = 1000;
/** Kotone 自己的宿主（McpAppFrame）：因工具打开时总带 toolInfo，没带就是插件页直接打开的，不用等。 */
const KOTONE_HOST = "Kotone";

export async function connect(): Promise<Session> {
  document.documentElement.dataset.display ??= "inline";
  const app = new App({ name: "kotone-lol", version: "0.2.0" });
  const listeners: ((result: ToolResult) => void)[] = [];
  const contextListeners: ((context: McpUiHostContext) => void)[] = [];
  let settle!: (result: ToolResult | null) => void;
  let settled = false;
  let input: Record<string, unknown> | null = null;
  /** 宿主推来过几次结果、最后一次是哪个（firstCall 用：取的路上推来了就给推来的）。 */
  let pushes = 0;
  let lastPushed: ToolResult | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const waitFor = (ms: number) => {
    clearTimeout(timer);
    timer = setTimeout(() => settle(null), ms);
  };
  const initial = new Promise<ToolResult | null>((resolve) => {
    settle = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(result);
    };
  });
  // 处理函数要在 connect 之前挂好：握手一完成宿主就会推工具的输入和结果
  app.ontoolresult = (result) => {
    pushes += 1;
    lastPushed = result;
    if (!settled) settle(result);
    else for (const listener of listeners) listener(result);
  };
  let hinting = false;
  app.ontoolinput = (params) => {
    input = params.arguments ?? null;
    // 没带 toolInfo 的宿主推来了工具的输入：是因工具打开的，结果会跟着来，照带了 toolInfo 的等
    if (hinting && !settled) {
      hinting = false;
      waitFor(RESULT_WAIT_MS);
    }
  };
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
  if (context?.toolInfo || input) waitFor(RESULT_WAIT_MS);
  else if (app.getHostVersion()?.name === KOTONE_HOST) settle(null);
  else {
    hinting = true;
    waitFor(HINT_WAIT_MS);
  }
  const call = (tool: string, args: Record<string, unknown> = {}) => app.callServerTool({ name: tool, arguments: args });
  return {
    context,
    initial,
    onResult: (listener) => listeners.push(listener),
    input: () => input,
    onContext: (listener) => contextListeners.push(listener),
    call,
    firstCall: async (tool, args = {}) => {
      const before = pushes;
      const pushed = () => (pushes !== before ? lastPushed : null);
      try {
        const own = await call(tool, args);
        return pushed() ?? own;
      } catch (error) {
        const result = pushed();
        if (result) return result;
        throw error;
      }
    },
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
