// 浏览器预览（dev:web、Playwright）里的热键表：后端 `kotone_core::hotkey_table` / `hotkey_table_cmd.rs` 的内存版，照它的规矩写。
// 真正的冲突检查只在后端一处（ADR 040 D6）；这里只是预览用的假后端，界面不自己判断冲突。
// 预览的设置是“老用户”的样子：热键的旧字段（`hotkey.key`……）当作存放处，改键时旧字段和 `hotkeys.bindings` 一起改。
import type { GameKeyKind, GameProfile, HotkeyBinding, HotkeyCheck, HotkeyTable, KeySource, KeyUse, Settings } from "./contracts";
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

function bindings({ settings, voiceInput }: HotkeySources): KeyUse[] {
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
  if (!legacyOnly) {
    (settings.assistant.companion?.quickMessages ?? []).slice(0, 8).forEach((message, index) => {
      const detail = message.automationId?.trim() ? `运行自动化 ${message.automationId.trim()}` : (message.label.trim() || message.text.trim());
      out.push(entry(`quick-message-${index + 1}`, message.hotkey, "custom", `快捷指令 ${index + 1}`, PAGE,
        `assistant.companion.quickMessages.${index}.hotkey`, { detail, statusId: `quick-message-${index + 1}` }));
    });
  }
  return out;
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

/** `except`：表里的 id、命令名、第 1 期的 id 都认（同后端）。 */
function isSelf(use: KeyUse, except?: string): boolean {
  if (!except) return false;
  return use.id === except || use.command === except || (!!use.command && commandOf(except)?.command === use.command);
}

export function checkHotkey(src: HotkeySources, key: string, except?: string): HotkeyCheck {
  const trimmed = key.trim();
  const conflicts = [...bindings(src), ...gameKeys(src.profiles, src.settings.activeProfileId)]
    .filter((use) => !isSelf(use, except) && collides(use, trimmed));
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
  const quick = /^quick-message-(\d+)$/.exec(id);
  const command = commandOf(id);
  if (!quick && !command) throw new Error(id === "escape" ? "Esc 是固定的，不能改" : id === "legacy-reminder" ? "提醒助手热键在「提醒助手」页改" : `「${id}」在这里改不了`);
  // i18n-ignore-end
  if (trimmed) {
    const conflict = checkHotkey(src, trimmed, id).conflicts[0];
    if (conflict) throw new Error(conflictError(trimmed, conflict));
  }
  const { settings } = src;
  if (quick) {
    const messages = settings.assistant.companion?.quickMessages ?? [];
    const message = messages[Number(quick[1]) - 1];
    if (!message) throw new Error(`没有第 ${quick[1]} 条快捷指令了，刷新一下再改`); // i18n-ignore：后端原样给的错误
    message.hotkey = trimmed;
    return;
  }
  setMirror(settings, command!.field, trimmed);
  const list: HotkeyBinding[] = (settings.hotkeys?.bindings ?? []).filter((b) => b.command !== command!.command);
  if (trimmed) list.push({ key: trimmed, command: command!.command });
  settings.hotkeys = { bindings: list };
}
