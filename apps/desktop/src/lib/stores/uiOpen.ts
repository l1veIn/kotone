// `ui.open`（她和外部 Agent 在 Kotone 窗口里打开一个页面，ADR 038 D5）落到界面上：不打断玩家正在看的东西
// （共享资源评估第二节第 6 条，2026-11-25；ADR 039 实施记录“ui.open 不打断玩家”）。
//
// - 玩家在聊天页，右侧栏空着（收着、停在“卡片”、“网页”里没开网页）或者显示的就是这个插件的页面：直接显示
//   （同一个插件的下一页往上叠，“返回”回到刚才在看的）；本体的设置页照旧跳过去（玩家正和她聊，她带你过去）。
// - 别的时候（玩家在别的页面；右侧栏显示着另一个插件的页面、开着的网页、展开看的后台的事）：不切页、不换掉，
//   放进右侧栏“卡片”最上面等玩家点（`waitingPages`），窗口里出一个提示（svelte-sonner，不拿焦点），点“打开”才打开。
// - 同一个页面（地址 + 参数）反复打开只算一次：显示着的不重新挂、不再叠；等着的不重复放、不再提示
//   （照自动卡片按 key 去重：同一个插件同一个 key 只发一次）。等着的最多留 5 张，只在内存里（和右侧栏的页面一样）。
//
// 谁调的（她还是外部 Agent）这里不分：规矩只有一条。真窗口里处理完把结果（shown / waiting）交回后端
// （`ui_open_reply`），工具结果里的 `status` 就是它。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast as sonner } from "svelte-sonner";
import { get, writable } from "svelte/store";
import { isTauri } from "../ipc";
import { m } from "../paraglide/messages.js";
import { pluginText } from "../plugins";
import { pluginOfUri, viewOf } from "../pluginUi";
import { companionName } from "./companion";
import { openTarget } from "./navigation";
import { openPluginPage, pluginPageStack, pushPluginPage } from "./pluginPages";
import { openRightSidebar, setRightSidebarTab } from "./rightSidebar";

/** `ui.open` 的事件（后端 `plugins::open_page` 发的 `kotone://ui-open`）。`byPlayer`：玩家自己按了“打开页面”的热键
 * （ADR 040 第 3 期，`hotkey_run.rs`）——是他自己要看，直接切过去（窗口后端已经拿到前面了）；`thirdParty`：第三方插件的
 * 页面，打开左侧栏「插件」里它的页面（右侧栏还显示不了第三方的 App）。 */
export interface UiOpen {
  uri: string; params?: Record<string, unknown> | null; title?: string; plugin?: string | null;
  byPlayer?: boolean; thirdParty?: boolean; standalone?: boolean;
}

/** 处理完的结果：直接显示了 / 放在右侧栏等玩家点。 */
export type UiOpenOutcome = "shown" | "waiting";

/** 右侧栏现在显示着什么（RightSidebar 报上来）：空着（收着、“卡片”、没开网页的“网页”）、某个插件的页面、别的。 */
export type SidebarShowing = { kind: "free" } | { kind: "app"; plugin: string } | { kind: "busy" };
export const SIDEBAR_FREE: SidebarShowing = { kind: "free" };
export const sidebarShowing = writable<SidebarShowing>(SIDEBAR_FREE);

/** 玩家正看着聊天页（设置窗口 Settings.svelte 报上来）。 */
export const chatShowing = writable(false);

/** 等玩家点的一个页面。`plugin` 为 null 的是本体的设置页（`section`）。 */
export interface WaitingPage {
  key: string;
  uri: string;
  params: Record<string, unknown> | null;
  plugin: string | null;
  section: string | null;
  title: string;
}
export const waitingPages = writable<WaitingPage[]>([]);
const MAX_WAITING = 5;
/** 提示只有一个：新的替换旧的（照消息提示）。 */
const TOAST_ID = "kotone-ui-open";
const TOAST_MS = 6_000;

/** 现在能不能直接显示（不打断玩家）。`plugin` 为 null 的是本体的设置页。 */
export function canShowNow(plugin: string | null, onChat: boolean, sidebar: SidebarShowing): boolean {
  if (!onChat) return false;
  if (plugin === null) return true;
  return sidebar.kind === "free" || (sidebar.kind === "app" && sidebar.plugin === plugin);
}

/** 同一个页面：地址 + 参数（键排好序）。 */
export function pageKey(uri: string, params: Record<string, unknown> | null | undefined): string {
  const sorted = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(sorted);
    if (value && typeof value === "object") {
      return Object.fromEntries(Object.keys(value as Record<string, unknown>).sort().map((k) => [k, sorted((value as Record<string, unknown>)[k])]));
    }
    return value;
  };
  return `${uri} ${JSON.stringify(sorted(params ?? {}))}`;
}

const SETTINGS_LABELS: Record<string, () => string> = {
  models: m.system_section_models, connections: m.system_section_connections, sound: m.system_section_sound,
  tuning: m.system_section_tuning, appearance: m.system_section_appearance, system: m.system_section_system,
  browser: m.system_section_browser,
};
/** 原来在「设置」、现在在角色页的分页（地址没变，navigation.ts 的 normalizeTarget 落过去）。 */
const CHARACTER_LABELS: Record<string, () => string> = {
  call: m.companion_page_tab_call, reply: m.companion_page_tab_reply,
  permissions: m.companion_page_tab_permissions, usage: m.companion_page_tab_usage,
};

/** 设置的一页叫什么（“设置 · 模型”；原来在设置、现在在角色页的是“{她的名字} · 唤醒”）：热键的“打开页面”也用。 */
export function settingsTitle(section: string): string {
  if (section === "plugins") return m.shell_sidebar_plugins();
  const character = CHARACTER_LABELS[section];
  if (character) return m.chat_ui_open_title_character({ name: get(companionName), section: character() });
  return m.chat_ui_open_title_settings({ section: SETTINGS_LABELS[section]?.() ?? section });
}

function pageTitle(uri: string, plugin: string, fallback: string | undefined): string {
  const name = pluginText(plugin, "name", plugin);
  const page = viewOf(uri)?.view.title() ?? fallback ?? "";
  return page && page !== name ? m.chat_ui_open_title_plugin({ plugin: name, page }) : name;
}

function settingsTargetOf(section: string) {
  return section === "plugins" ? { page: "plugins" as const } : { page: "advanced" as const, tab: section };
}

function removeWaiting(key: string): void {
  waitingPages.update((list) => list.filter((w) => w.key !== key));
}

function show(page: WaitingPage): void {
  removeWaiting(page.key);
  if (page.section !== null) { openTarget(settingsTargetOf(page.section)); return; }
  const stack = get(pluginPageStack);
  const top = stack[stack.length - 1];
  if (!top || pluginOfUri(top.uri) !== page.plugin) { openPluginPage(page.uri, page.params); return; }
  // 最上面就是这一页：只算一次，不重新挂；同一个插件的另一页：往上叠，“返回”回到刚才在看的
  if (pageKey(top.uri, top.params) !== page.key) pushPluginPage(page.uri, page.params);
  openRightSidebar();
  setRightSidebarTab("page");
}

function park(page: WaitingPage): void {
  // 已经在等着：只算一次（不重复放、不再提示）
  if (get(waitingPages).some((w) => w.key === page.key)) return;
  waitingPages.update((list) => [page, ...list].slice(0, MAX_WAITING));
  sonner.info(m.chat_ui_open_toast({ title: page.title }), {
    id: TOAST_ID, duration: TOAST_MS,
    action: { label: m.chat_ui_open_open(), onClick: () => openWaiting(page.key) },
  });
}

/** 玩家点了等着的页面（右侧栏的卡片、提示上的“打开”）：这是玩家自己要看，切过去。 */
export function openWaiting(key: string): void {
  const page = get(waitingPages).find((w) => w.key === key);
  if (!page) return;
  removeWaiting(key);
  sonner.dismiss(TOAST_ID);
  if (page.section !== null) { openTarget(settingsTargetOf(page.section)); return; }
  openTarget({ page: "assistant", section: "chat" });
  openPluginPage(page.uri, page.params);
}

/** 右侧栏卡片上的“不看了”。 */
export function dismissWaiting(key: string): void {
  removeWaiting(key);
}

/**
 * 处理 `ui.open`：插件的页面（`ui://kotone/<插件>/<页面>`）在聊天页的右侧栏；`ui://kotone/settings/<分页>` 是设置的那一页
 * （`plugins` 是旧地址：左侧栏的「插件」）。能直接显示就显示，不然放进右侧栏等玩家点（见文件开头）。
 * 从不把窗口拉到前面（玩家可能在游戏里）。认不出的地址返回 null、什么都不动。
 */
export function handleUiOpen(event: UiOpen): UiOpenOutcome | null {
  const uri = String(event?.uri ?? "");
  const params = event?.params && typeof event.params === "object" ? event.params : null;
  if (event?.byPlayer && event.thirdParty && event.plugin) {
    openTarget({ page: "plugin", id: event.plugin });
    return "shown";
  }
  const section = /^ui:\/\/kotone\/settings\/([a-z]+)$/.exec(uri)?.[1] ?? null;
  const plugin = section ? null : pluginOfUri(uri);
  if (!section && !plugin) return null;
  if (event.byPlayer && event.standalone && plugin && viewOf(uri)) {
    openTarget({ page: "plugin", id: plugin, params: { uri } });
    return "shown";
  }
  const page: WaitingPage = {
    key: pageKey(uri, params), uri, params, plugin, section,
    title: section ? settingsTitle(section) : pageTitle(uri, plugin!, event?.title),
  };
  if (event?.byPlayer) {
    // 玩家自己按热键要看：照他点“打开”时一样切过去
    if (section !== null) openTarget(settingsTargetOf(section));
    else { openTarget({ page: "assistant", section: "chat" }); show(page); }
    return "shown";
  }
  if (canShowNow(plugin, get(chatShowing), get(sidebarShowing))) {
    show(page);
    return "shown";
  }
  park(page);
  return "waiting";
}

/** 设置窗口挂上 `kotone://ui-open`，处理完把结果交回后端。浏览器预览里没有后端事件：给 e2e 和调试留一个 `window.kotoneUiOpen`。 */
export function listenUiOpen(): () => void {
  if (!isTauri) {
    (window as unknown as { kotoneUiOpen?: (e: UiOpen) => UiOpenOutcome | null }).kotoneUiOpen = handleUiOpen;
    return () => { delete (window as unknown as { kotoneUiOpen?: unknown }).kotoneUiOpen; };
  }
  const off = listen<UiOpen>("kotone://ui-open", (e) => {
    const outcome = handleUiOpen(e.payload);
    if (!e.payload?.byPlayer) {
      void invoke("ui_open_reply", { uri: String(e.payload?.uri ?? "").trim(), outcome }).catch(() => {});
    }
  });
  return () => { void off.then((unlisten) => unlisten()); };
}
