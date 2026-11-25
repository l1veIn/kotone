// 浏览器预览（dev:web、Playwright）里的热键表：后端 `kotone_core::hotkey_table` / `hotkey_table_cmd.rs` 的内存版，照它的规矩写。
// 真正的冲突检查只在后端一处（ADR 040 D6）；这里只是预览用的假后端，界面不自己判断冲突。
// 预览的设置是“老用户”的样子：热键的旧字段（`hotkey.key`……）当作存放处，改键时旧字段和 `hotkeys.bindings` 一起改。
import type {
  BindablePage, BindableTool, GameKeyKind, GameProfile, HotkeyBinding, HotkeyBindingOptions, HotkeyCheck, HotkeyTable, KeySource,
  KeyUse, NamedHotkeyStatus, Settings,
} from "./contracts";
import { companionBindings, hasLegacyReminders, usesLegacyVoiceInput } from "./contracts";

const MAIN_KEYS: Record<string, string> = {
  mouse4: "Mouse4", xbutton1: "Mouse4", mouse5: "Mouse5", xbutton2: "Mouse5", space: "Space", tab: "Tab",
  enter: "Enter", return: "Enter", esc: "Escape", escape: "Escape", backspace: "Backspace", back: "Backspace",
  delete: "Delete", del: "Delete", insert: "Insert", ins: "Insert", home: "Home", end: "End",
  pageup: "PageUp", pgup: "PageUp", pagedown: "PageDown", pgdn: "PageDown", up: "Up", down: "Down",
  left: "Left", right: "Right", printscreen: "PrintScreen", print: "PrintScreen", pause: "Pause",
  capslock: "CapsLock", caps: "CapsLock", backquote: "Backquote", "`": "Backquote", "~": "Backquote",
  grave: "Backquote", tilde: "Backquote",
};
const MODIFIERS: Record<string, "Ctrl" | "Alt" | "Shift"> = { ctrl: "Ctrl", control: "Ctrl", alt: "Alt", menu: "Alt", shift: "Shift" };

/** 同后端 `parse_hotkey` + `combo_name`：规范写法；认不出为空。 */
export function canonicalCombo(key: string): string {
  const parts = key.split("+").map((part) => part.trim()).filter(Boolean);
  const main = parts.pop();
  if (!main || MODIFIERS[main.toLowerCase()]) return "";
  const lower = main.toLowerCase();
  let name = MAIN_KEYS[lower];
  if (!name && /^[a-z0-9]$/i.test(main)) name = main.toUpperCase();
  if (!name && /^f([1-9]|1[0-9]|2[0-4])$/.test(lower)) name = lower.toUpperCase();
  if (!name) return "";
  const mods = new Set<string>();
  for (const part of parts) {
    const mod = MODIFIERS[part.toLowerCase()];
    if (!mod) return "";
    mods.add(mod);
  }
  return [...(["Ctrl", "Alt", "Shift"].filter((mod) => mods.has(mod))), name].join("+");
}

function collides(use: KeyUse, key: string): boolean {
  const combo = canonicalCombo(key);
  if (!use.combo || !combo) return false;
  if (use.id === "escape") return combo.split("+").pop() === "Escape";
  return use.combo === combo;
}

function entry(id: string, key: string, source: KeySource, label: string, page: string, field: string, extra: Partial<KeyUse> = {}): KeyUse {
  return { id, key: key.trim(), combo: canonicalCombo(key), source, label, page, field, enabled: true, ...extra };
}

export interface HotkeySources {
  settings: Settings;
  profiles: GameProfile[];
  voiceInput: { id: string; name: string; enabled: boolean };
  /** 英雄联盟插件（一键喊话的四条预设）：开没开、“读客户端数据”开没开。 */
  lol?: { id: string; name: string; enabled: boolean; client: boolean };
}

type MirrorKey = "automationStopHotkey" | "runtimeToggleHotkey" | "channelCycleHotkey" | "resendLastHotkey"
  | "togglePostProcessingHotkey" | "cyclePostProcessingHotkey";

/** 命令 ↔ 第 1 期的 id ↔ 旧字段（同后端 `hotkey_bindings::LEGACY`）。 */
const COMMANDS: { command: string; statusId: string; field: MirrorKey | "call" | "talk"; defaultKey?: string }[] = [
  { command: "kotone.call", statusId: "assistant", field: "call", defaultKey: "Ctrl+Backquote" },
  { command: "kotone.stop", statusId: "automation-stop", field: "automationStopHotkey", defaultKey: "Ctrl+Shift+F12" },
  { command: "kotone.speech-toggle", statusId: "runtime-toggle", field: "runtimeToggleHotkey" },
  { command: "voice-input.talk", statusId: "record", field: "talk", defaultKey: "CapsLock" },
  { command: "voice-input.channel-cycle", statusId: "channel-cycle", field: "channelCycleHotkey" },
  { command: "voice-input.resend-last", statusId: "resend-last", field: "resendLastHotkey" },
  { command: "voice-input.post-process-toggle", statusId: "post-process-toggle", field: "togglePostProcessingHotkey" },
  { command: "voice-input.post-process-cycle", statusId: "post-process-cycle", field: "cyclePostProcessingHotkey" },
];

function commandOf(id: string) {
  return COMMANDS.find((c) => c.command === id || c.statusId === id);
}

function mirror(settings: Settings, field: (typeof COMMANDS)[number]["field"]): string {
  if (field === "call") return companionBindings(settings.assistant)?.hotkey ?? "";
  if (field === "talk") return settings.hotkey.key;
  return settings[field] ?? "";
}

function setMirror(settings: Settings, field: (typeof COMMANDS)[number]["field"], key: string) {
  if (field === "call") {
    if (settings.assistant.companionBindings) settings.assistant.companionBindings.hotkey = key;
    else settings.assistant.hotkey = key;
  } else if (field === "talk") settings.hotkey.key = key;
  else settings[field] = key;
}

// i18n-ignore-start：后端给 Agent 的中文名和设置位置（同 hotkey_table.rs / hotkey_commands.rs / plugin.toml）；界面用 messages 里的名字
const NAMES: Record<string, string> = {
  "kotone.call": "呼叫键", "kotone.stop": "停下热键", "kotone.speech-toggle": "启动 / 停止本地语音识别热键",
  "voice-input.talk": "语音输入热键", "voice-input.channel-cycle": "频道切换热键", "voice-input.resend-last": "重发最近一条热键",
  "voice-input.post-process-toggle": "文字处理开关热键", "voice-input.post-process-cycle": "切换文字处理流程热键",
};
const PAGE = "设置 → 热键";

function commandRow(settings: Settings, c: (typeof COMMANDS)[number], plugin?: { owner: string; ownerName: string; enabled: boolean }): KeyUse {
  const field = c.field === "call" ? (settings.ui.legacyUpgrade === "optedIn" ? "assistant.companionBindings.hotkey" : "assistant.hotkey")
    : c.field === "talk" ? "hotkey.key" : c.field;
  return entry(c.command, mirror(settings, c.field), plugin ? "plugin" : "core", NAMES[c.command], PAGE, field, {
    command: c.command, statusId: c.statusId, ...(c.defaultKey ? { defaultKey: c.defaultKey } : {}), ...(plugin ?? {}),
  });
}

/** 英雄联盟登记的一键喊话（同 plugins/lol/plugin.toml 的 [[commands]]，ADR 041 第 3 期）：调 lol.shout，不带默认键，键存在热键表里。 */
const LOL_PRESETS: { command: string; name: string; description: string }[] = [
  { command: "lol.shout-enemy-flash", name: "喊对面闪现", description: "把对面谁的闪现没好、几点好（你报过的）和谁没带闪发到队伍聊天" },
  { command: "lol.shout-enemy-records", name: "喊对面战绩", description: "把对面每个人的段位、近 20 局胜负、KDA、连胜连败发到队伍聊天（一人一行）" },
  { command: "lol.shout-ally-records", name: "喊我方战绩", description: "把我方每个人的段位、近 20 局胜负、KDA、连胜连败发到队伍聊天（一人一行）" },
  { command: "lol.shout-premade", name: "喊开黑情况", description: "把两边谁和谁开黑发到队伍聊天" },
];

function presetOf(id: string) {
  return LOL_PRESETS.find((p) => p.command === id);
}

/** 同后端 `hotkey_table::tool_command`：照 lol.shout 能不能按（插件关着不登记；“读客户端数据”关着写原因）。 */
function presetRow(settings: Settings, p: (typeof LOL_PRESETS)[number], lol: NonNullable<HotkeySources["lol"]>): KeyUse {
  const key = (settings.hotkeys?.bindings ?? []).find((b) => b.command === p.command)?.key ?? "";
  return entry(p.command, key, "plugin", p.name, PAGE, "hotkeys.bindings", {
    command: p.command, statusId: p.command, action: "tool", detail: p.description,
    owner: lol.id, ownerName: lol.name, enabled: lol.enabled,
    ...(lol.enabled && !lol.client ? { blocked: `要先在左侧栏「插件 → ${lol.name}」打开“读客户端数据”` } : {}),
  });
}

function bindings({ settings, voiceInput, lol }: HotkeySources): KeyUse[] {
  const legacyOnly = usesLegacyVoiceInput(settings.ui);
  const out: KeyUse[] = [];
  for (const c of COMMANDS.filter((c) => c.command.startsWith("kotone."))) {
    if (legacyOnly && c.command !== "kotone.speech-toggle") continue;
    out.push(commandRow(settings, c));
  }
  if (hasLegacyReminders(settings.ui)) {
    out.push(entry("legacy-reminder", settings.assistant.hotkey, "core", "提醒助手热键", "提醒助手 → 提醒设置", "assistant.hotkey", { statusId: "legacy-reminder" }));
  }
  out.push(entry("escape", "Escape", "fixed", "Esc", "", "", { detail: "取消这次录音；她操作游戏时让她停下" }));
  const plugin = { owner: voiceInput.id, ownerName: voiceInput.name, enabled: voiceInput.enabled };
  for (const c of COMMANDS.filter((c) => c.command.startsWith("voice-input."))) out.push(commandRow(settings, c, plugin));
  if (lol) for (const p of LOL_PRESETS) out.push(presetRow(settings, p, lol));
  if (!legacyOnly) {
    // 自己加的（第 3 期，后端 `hotkey_custom`）：在热键表里，每条一行，行 id 是 `custom.<id>`
    for (const b of customEntries(settings)) out.push(customRow(settings, b));
  }
  return out;
}

// ---------- 自己加的（ADR 040 第 3 期：说一句、运行自动化、调工具、打开页面；绑的时候问权限） ----------

type CustomKind = "say" | "automation" | "tool" | "page";

/** 同后端 `Binding::action`：种类只能有一个，带 command 的不算；`automation.run` 只带 id / args 的是运行自动化。 */
export function customKind(b: HotkeyBinding): CustomKind | null {
  if (!b || typeof b !== "object" || "command" in b) return null;
  const kinds = (["say", "tool", "page"] as const).filter((k) => k in b);
  if (kinds.length !== 1) return null;
  if (kinds[0] !== "tool") return kinds[0];
  const args = b.args as Record<string, unknown> | undefined;
  const saved = b.tool === "automation.run" && !!args && typeof args.id === "string" && !!args.id.trim()
    && Object.keys(args).every((k) => k === "id" || k === "args");
  return saved ? "automation" : "tool";
}

function customEntries(settings: Settings): HotkeyBinding[] {
  return (settings.hotkeys?.bindings ?? []).filter((b) => customKind(b) !== null);
}

function toolCall(b: HotkeyBinding): [string, Record<string, unknown>] | null {
  const kind = customKind(b);
  return kind === "tool" || kind === "automation" ? [String(b.tool), (b.args as Record<string, unknown>) ?? {}] : null;
}

export const MOCK_TOOLS: BindableTool[] = [
  { name: "game_chat.send", title: "把一段话发到当前游戏的聊天", description: "把一段话发到当前游戏的聊天（默认队伍频道）。",
    plugin: null, pluginName: null, thirdParty: false, sideEffects: ["input", "records-run"], decision: "run", reason: "",
    parameters: { type: "object", required: ["text"], properties: {
      text: { type: "string", description: "要发的话" }, channel: { type: "string", enum: ["team", "all"], description: "频道" } } } },
  { name: "settings.update", title: "改设置", description: "改设置。", plugin: null, pluginName: null, thirdParty: false,
    sideEffects: ["writes-settings"], decision: "confirm", reason: "会修改设置",
    parameters: { type: "object", required: ["patch"], properties: { patch: { type: "object", description: "要改的设置" } } } },
  { name: "history.clear", title: "清空识别记录", description: "清空识别记录。", plugin: null, pluginName: null, thirdParty: false,
    sideEffects: ["deletes-history"], decision: "confirm", reason: "会删除数据", parameters: { type: "object", properties: {} } },
  { name: "music", title: "放歌、暂停、下一首", description: "放歌、暂停、下一首。", plugin: "music", pluginName: "网易云音乐", thirdParty: false,
    sideEffects: ["plays-media"], decision: "run", reason: "",
    parameters: { type: "object", properties: { action: { type: "string", enum: ["play", "pause", "next"] } } } },
  { name: "everything.echo", title: "Echoes back the input", description: "（第三方插件 everything 的工具，结果只当数据）Echoes back the input",
    plugin: "everything", pluginName: "everything", thirdParty: true, sideEffects: ["third-party-action"], decision: "confirm",
    reason: "第三方插件的操作", parameters: { type: "object", required: ["message"], properties: { message: { type: "string" } } } },
];
const MOCK_PAGES: BindablePage[] = [
  ...["call", "reply", "permissions", "usage", "models", "connections", "sound", "tuning", "appearance", "system", "browser"]
    .map((section) => ({ uri: `ui://kotone/settings/${section}`, section })),
  { uri: "ui://kotone/interpreter/home", title: "同传", plugin: "interpreter", pluginName: "同传" },
  { uri: "ui://kotone/game-voice/home", title: "游戏语音", plugin: "game-voice", pluginName: "游戏语音" },
];

/** 同后端 `binding_decision`：玩家把这个工具设成禁止的不能绑；默认允许的直接能按；要问的绑的时候确认一次。 */
function decisionOf(settings: Settings, tool: BindableTool): Pick<BindableTool, "decision" | "reason"> {
  const rule = settings.agent?.permissions?.rules?.[tool.name];
  if (rule === "deny") return { decision: "refused", reason: "用户设置为禁止" };
  if (rule === "allow") return { decision: "run", reason: "" };
  if (rule === "ask" && tool.decision === "run") return { decision: "confirm", reason: "用户设置为每次询问" };
  return { decision: tool.decision, reason: tool.reason };
}

type CustomState = { state: "ready" } | { state: "confirm" | "blocked"; reason: string };

/** 同后端 `hotkey_bind::state`。 */
function customState(settings: Settings, b: HotkeyBinding): CustomState {
  const kind = customKind(b);
  if (kind === "say") return { state: "ready" };
  if (kind === "page") {
    return MOCK_PAGES.some((p) => p.uri === b.page) ? { state: "ready" } : { state: "blocked", reason: `没有这个页面：${b.page}` };
  }
  const call = toolCall(b);
  if (!call) return { state: "blocked", reason: "读不懂这一条：去「设置 → 热键」删掉重加" };
  if (call[0] === "automation.run") return { state: "ready" };
  const tool = MOCK_TOOLS.find((t) => t.name === call[0]);
  if (!tool) return { state: "blocked", reason: `找不到工具 ${call[0]}（插件没装或被删了）：去「设置 → 热键」换一个或删掉` };
  const { decision, reason } = decisionOf(settings, tool);
  if (decision === "refused") return { state: "blocked", reason: `${reason}：去她的「权限」里改（${tool.name}）` };
  const confirmed = Array.isArray(b.confirmed) ? (b.confirmed as string[]) : null;
  if (decision === "confirm" && !(confirmed && tool.sideEffects.every((e) => confirmed.includes(e)))) {
    return { state: "confirm", reason: `${reason}（${tool.name}）` };
  }
  return { state: "ready" };
}

/** 同后端 `hotkey_table::custom_row`：名字是玩家起的，没起就用要做的事。 */
function customRow(settings: Settings, b: HotkeyBinding): KeyUse {
  const kind = customKind(b)!;
  const id = `custom.${b.id}`;
  const text = String(b.say ?? "").trim();
  const short = [...text].length > 16 ? `${[...text].slice(0, 16).join("")}…` : text;
  const automation = kind === "automation" ? String((b.args as { id: string }).id) : "";
  const name = { say: `快捷指令「${short}」`, automation: `运行自动化「${automation}」`, tool: `调用 ${b.tool}`, page: `打开 ${b.page}` }[kind];
  const detail = { say: text, automation: `运行自动化 ${automation}`, tool: String(b.tool), page: String(b.page) }[kind];
  const st = customState(settings, b);
  return entry(id, b.key ?? "", "custom", String(b.label ?? "").trim() || name, PAGE, "hotkeys.bindings", {
    detail, statusId: id, action: kind,
    ...(st.state === "confirm" ? { confirm: st.reason } : {}), ...(st.state === "blocked" ? { blocked: st.reason } : {}),
  });
}

/** 登记状态里自己加的那几条（同后端：能按的登记在键上；要确认、按不了的不登记，写原因）。 */
export function customStatus(settings: Settings): NamedHotkeyStatus[] {
  if (usesLegacyVoiceInput(settings.ui)) return [];
  return customEntries(settings)
    .filter((b) => String(b.key ?? "").trim())
    .map((b) => {
      const st = customState(settings, b);
      const error = st.state === "confirm" ? `要先在「设置 → 热键」确认一次：${st.reason}` : st.state === "blocked" ? st.reason : null;
      return { id: `custom.${b.id}`, key: error ? null : String(b.key), error };
    });
}

/** 绑了键的英雄联盟预设（同后端：登记 id 是命令名，随应用登记；插件关着不登记，按不了的写原因）。 */
export function presetStatus(src: HotkeySources): NamedHotkeyStatus[] {
  const lol = src.lol;
  if (!lol?.enabled) return [];
  return LOL_PRESETS.map((p) => presetRow(src.settings, p, lol))
    .filter((row) => row.key)
    .map((row) => ({ id: row.id, key: row.blocked ? null : row.key, error: row.blocked ?? null }));
}

/** 同后端 `hotkey_binding_options`。 */
export function bindingOptions(settings: Settings): HotkeyBindingOptions {
  return { tools: MOCK_TOOLS.map((t) => ({ ...t, ...decisionOf(settings, t) })), pages: MOCK_PAGES };
}

/** `custom.<id>`，或第 1、2 期的 `quick-message-n`（前 8 条“说一句 / 运行自动化”的第 n 条）→ 那一条。 */
function customOf(settings: Settings, id: string): HotkeyBinding | undefined {
  const slot = /^quick-message-(\d+)$/.exec(id);
  if (slot) {
    return customEntries(settings).filter((b) => customKind(b) === "say" || customKind(b) === "automation")[Number(slot[1]) - 1];
  }
  return id.startsWith("custom.") ? customEntries(settings).find((b) => `custom.${b.id}` === id) : undefined;
}

/** 同后端 `save_custom_hotkey`：加 / 改一条；传进来的确认不算，做的事没变时原来的确认留着；新造成的冲突不让存。 */
export function saveCustom(src: HotkeySources, input: HotkeyBinding, confirm: boolean): void {
  const { settings } = src;
  const next: HotkeyBinding = { ...input, key: String(input.key ?? "").trim() };
  delete next.confirmed;
  delete next.command;
  if (!String(next.label ?? "").trim()) delete next.label;
  if (!customKind(next)) throw new Error("要做的事只能是一种：跟她说一句、调一个工具（运行自动化也是）、打开一个页面");
  if (next.key && !canonicalCombo(next.key)) throw new Error(`认不出「${next.key}」这个键：写成 F8、Ctrl+Alt+K、Mouse4 这样，或者点“录入”直接按`);
  if (customKind(next) === "say" && ![...String(next.say).trim()].length) throw new Error("快捷指令跟她说的话要 1 到 200 字");
  const list = settings.hotkeys?.bindings ?? [];
  const index = next.id ? list.findIndex((b) => customKind(b) && b.id === next.id) : -1;
  if (index >= 0) {
    const old = list[index];
    const same = JSON.stringify(toolCall(old)) === JSON.stringify(toolCall(next));
    if (same && old.confirmed) next.confirmed = old.confirmed;
  } else {
    const taken = new Set(customEntries(settings).map((b) => String(b.id)));
    let n = 1;
    while (taken.has(`qm-${n}`)) n += 1;
    next.id = `qm-${n}`;
  }
  if (next.key) {
    const conflict = checkHotkey(src, next.key, `custom.${next.id}`).conflicts[0];
    if (conflict) throw new Error(conflictError(next.key, conflict));
  }
  const bindings = [...list];
  if (index >= 0) bindings[index] = next;
  else bindings.push(next);
  settings.hotkeys = { ...(settings.hotkeys ?? {}), bindings };
  if (confirm) confirmCustom(src, `custom.${next.id}`);
}

export function removeCustom(src: HotkeySources, id: string): void {
  const target = customOf(src.settings, id);
  if (!target) throw new Error("这条已经没有了");
  src.settings.hotkeys = { ...src.settings.hotkeys, bindings: (src.settings.hotkeys?.bindings ?? []).filter((b) => b !== target) };
}

/** 同后端 `confirm_hotkey`：要问的记下这个工具会做的事；不用问的不改；禁止的不能确认。 */
export function confirmCustom(src: HotkeySources, id: string): void {
  const target = customOf(src.settings, id);
  if (!target) throw new Error("这条自己加的热键已经没有了，刷新一下再确认");
  const call = toolCall(target);
  if (!call) throw new Error("只有调工具、运行自动化的热键要确认");
  const tool = MOCK_TOOLS.find((t) => t.name === call[0]);
  if (!tool) return;
  const { decision, reason } = decisionOf(src.settings, tool);
  if (decision === "refused") throw new Error(`${reason}：先去她的「权限」里改，再回来确认`);
  if (decision === "confirm") target.confirmed = [...tool.sideEffects];
}

function gameKeys(profiles: GameProfile[], activeProfileId: string | null | undefined): KeyUse[] {
  const ordered = [...profiles].sort((a, b) => Number(b.id === activeProfileId) - Number(a.id === activeProfileId));
  const out: KeyUse[] = [];
  for (const profile of ordered) {
    const keys: [string, GameKeyKind, string, string][] = [[`game.${profile.id}.open-chat`, "open-chat", profile.openChatKey, ""]];
    for (const channel of profile.channels ?? []) {
      if (channel.openChatKey != null) keys.push([`game.${profile.id}.channel.${channel.id}`, "channel", channel.openChatKey, channel.displayName]);
    }
    keys.push([`game.${profile.id}.send`, "send", profile.sendKey, ""]);
    if (profile.consoleChat) keys.push([`game.${profile.id}.console`, "console", profile.consoleChat.triggerKey ?? "F23", ""]);
    const seen: string[] = [];
    for (const [id, gameKey, raw, detail] of keys) {
      const key = raw.trim();
      const combo = canonicalCombo(key);
      if (!key || seen.includes(combo || key)) continue;
      seen.push(combo || key);
      const label = { "open-chat": "打开聊天框键", channel: `「${detail}」频道的开聊天框键`, send: "发送键", console: "控制台触发键" }[gameKey];
      out.push(entry(id, key, "game", label, "游戏列表", "", { owner: profile.id, ownerName: profile.displayName, gameKey, ...(detail ? { detail } : {}) }));
    }
  }
  return out;
}

/** 同后端 `new_conflict_error` 的话。 */
function conflictError(key: string, other: KeyUse): string {
  if (other.source === "fixed") return `「${key}」：Esc 留给停下她和取消收音，换一个键。`;
  if (other.source === "game") return `「${key}」是《${other.ownerName ?? ""}》的${other.label}，设成热键以后在游戏里按它会被 Kotone 拿走，换一个键。`;
  return `「${key}」已经是「${other.label}」的热键：先在「设置 → 热键」把它换掉，或者换一个键。`;
}
// i18n-ignore-end

export function hotkeyTable(src: HotkeySources): HotkeyTable {
  const game = gameKeys(src.profiles, src.settings.activeProfileId);
  const rows = bindings(src);
  for (const row of rows) {
    const conflicts = [...rows, ...game]
      .filter((other) => other.id !== row.id && (collides(other, row.key) || collides(row, other.key)))
      .map((other) => other.id);
    if (conflicts.length) row.conflicts = conflicts;
  }
  return { bindings: rows, gameKeys: game };
}

const COMMON_GAME_KEYS = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F9", "F10", "F11", "F12", "Tab", "Space", "Enter", "Shift", "Control", "Alt"];

/** `except`：表里的 id、命令名、第 1 期的 id、`quick-message-n` 都认（同后端）。 */
function isSelf(settings: Settings, use: KeyUse, except?: string): boolean {
  if (!except) return false;
  const custom = customOf(settings, except);
  return use.id === except || use.command === except || (!!use.command && commandOf(except)?.command === use.command)
    || (!!custom && use.id === `custom.${custom.id}`);
}

export function checkHotkey(src: HotkeySources, key: string, except?: string): HotkeyCheck {
  const trimmed = key.trim();
  const conflicts = [...bindings(src), ...gameKeys(src.profiles, src.settings.activeProfileId)]
    .filter((use) => !isSelf(src.settings, use, except) && collides(use, trimmed));
  const warnings = COMMON_GAME_KEYS.some((k) => k.toLowerCase() === trimmed.toLowerCase())
    ? [`「${trimmed}」是常见游戏/系统键位，可能与游戏内操作冲突`] // i18n-ignore：后端原样给的提示（预览用假数据）
    : [];
  return { key: trimmed, conflicts, warnings };
}

/** 同后端 `set_hotkey`：改一个键；新造成的冲突不让存（抛出说人话的错误）。改的是传进来的设置（预览的内存配置）。 */
export function setHotkey(src: HotkeySources, id: string, key: string): void {
  const trimmed = key.trim();
  // i18n-ignore-start：后端原样给的错误（预览用假数据）
  if (trimmed && !canonicalCombo(trimmed)) throw new Error(`认不出「${trimmed}」这个键：写成 F8、Ctrl+Alt+K、Mouse4 这样，或者点“录入”直接按`);
  const isQuick = id.startsWith("custom.") || /^quick-message-\d+$/.test(id);
  const command = commandOf(id);
  const preset = presetOf(id);
  if (!isQuick && !command && !preset) throw new Error(id === "escape" ? "Esc 是固定的，不能改" : id === "legacy-reminder" ? "提醒助手热键在「提醒助手」页改" : `「${id}」在这里改不了`);
  // i18n-ignore-end
  if (trimmed) {
    const conflict = checkHotkey(src, trimmed, id).conflicts[0];
    if (conflict) throw new Error(conflictError(trimmed, conflict));
  }
  const { settings } = src;
  if (isQuick) {
    const custom = customOf(settings, id);
    if (!custom) throw new Error("这条自己加的热键已经没有了，刷新一下再改"); // i18n-ignore：后端原样给的错误
    custom.key = trimmed;
    return;
  }
  // 插件的预设没有旧字段，键只在热键表里
  if (command) setMirror(settings, command.field, trimmed);
  const name = command?.command ?? preset!.command;
  const list: HotkeyBinding[] = (settings.hotkeys?.bindings ?? []).filter((b) => b.command !== name);
  if (trimmed) list.push({ key: trimmed, command: name });
  settings.hotkeys = { bindings: list };
}
