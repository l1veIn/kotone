// 浏览器预览（dev:web、Playwright）里的热键表：后端 `kotone_core::hotkey_table` 的内存版，照它的规矩写。
// 真正的冲突检查只在后端一处（ADR 040 D6）；这里只是预览用的假后端，界面不自己判断冲突。
import type { GameKeyKind, GameProfile, HotkeyCheck, HotkeyTable, KeySource, KeyUse, Settings } from "./contracts";
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

// i18n-ignore-start：后端给 Agent 的中文名和设置位置（同 hotkey_table.rs / settings_guide.rs）；界面按 id 用 messages 里的名字
function bindings({ settings, voiceInput }: HotkeySources): KeyUse[] {
  const legacyOnly = usesLegacyVoiceInput(settings.ui);
  const out: KeyUse[] = [];
  if (!legacyOnly) {
    const optedIn = settings.ui.legacyUpgrade === "optedIn";
    out.push(entry("assistant", companionBindings(settings.assistant)?.hotkey ?? "", "core", "呼叫键", "琴音 → 唤醒 → 呼叫键",
      optedIn ? "assistant.companionBindings.hotkey" : "assistant.hotkey"));
    out.push(entry("automation-stop", settings.automationStopHotkey ?? "", "core", "停下热键", "设置 → 系统 → 停下", "automationStopHotkey"));
  }
  out.push(entry("runtime-toggle", settings.runtimeToggleHotkey, "core", "启动 / 停止本地语音识别热键", "设置 → 模型 → 语音识别", "runtimeToggleHotkey"));
  if (hasLegacyReminders(settings.ui)) {
    out.push(entry("legacy-reminder", settings.assistant.hotkey, "core", "提醒助手热键", "提醒助手 → 提醒设置", "assistant.hotkey"));
  }
  out.push(entry("escape", "Escape", "fixed", "Esc", "", "", { detail: "取消这次录音；她操作游戏时让她停下" }));
  const plugin = { owner: voiceInput.id, ownerName: voiceInput.name, enabled: voiceInput.enabled };
  const others = "插件 → 语音输入 → 输入 → 其他热键";
  out.push(entry("record", settings.hotkey.key, "plugin", "语音输入热键", "插件 → 语音输入 → 输入", "hotkey.key", plugin));
  out.push(entry("channel-cycle", settings.channelCycleHotkey, "plugin", "切换频道热键", others, "channelCycleHotkey", plugin));
  out.push(entry("resend-last", settings.resendLastHotkey, "plugin", "重发上一句热键", others, "resendLastHotkey", plugin));
  out.push(entry("post-process-toggle", settings.togglePostProcessingHotkey, "plugin", "开关文字处理热键", others, "togglePostProcessingHotkey", plugin));
  out.push(entry("post-process-cycle", settings.cyclePostProcessingHotkey, "plugin", "切换文字处理方案热键", others, "cyclePostProcessingHotkey", plugin));
  if (!legacyOnly) {
    (settings.assistant.companion?.quickMessages ?? []).slice(0, 8).forEach((message, index) => {
      const detail = message.automationId?.trim() ? `运行自动化 ${message.automationId.trim()}` : (message.label.trim() || message.text.trim());
      out.push(entry(`quick-message-${index + 1}`, message.hotkey, "custom", `快捷指令 ${index + 1}`, "琴音 → 唤醒 → 快捷指令",
        `assistant.companion.quickMessages.${index}.hotkey`, { detail }));
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

export function checkHotkey(src: HotkeySources, key: string, except?: string): HotkeyCheck {
  const trimmed = key.trim();
  const conflicts = [...bindings(src), ...gameKeys(src.profiles, src.settings.activeProfileId)]
    .filter((use) => use.id !== except && collides(use, trimmed));
  const warnings = COMMON_GAME_KEYS.some((k) => k.toLowerCase() === trimmed.toLowerCase())
    ? [`「${trimmed}」是常见游戏/系统键位，可能与游戏内操作冲突`] // i18n-ignore：后端原样给的提示（预览用假数据）
    : [];
  return { key: trimmed, conflicts, warnings };
}
