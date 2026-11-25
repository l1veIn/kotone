// 热键表里的东西给玩家看的名字（ADR 040）。冲突由后端查（`checkHotkey` / `getHotkeyTable`），这里只管怎么叫。
import type { HotkeyCheck, KeyUse, Settings } from "./ipc/contracts";
import { pluginText } from "./plugins";
import { m } from "./paraglide/messages.js";

/** 本体的命令和不是命令的几项（插件的命令用插件自己的文案：`plugin_<插件>_command_<命令>_name`）。 */
const NAMES: Record<string, () => string> = {
  "kotone.call": m.voice_hotkey_name_call,
  "kotone.stop": m.voice_hotkey_name_automation_stop,
  "kotone.speech-toggle": m.voice_hotkey_name_runtime_toggle,
  "legacy-reminder": m.voice_hotkey_name_legacy_reminder,
  escape: m.hotkeys_escape,
};

/** 快捷指令的格子号（`quick-message-3` → 3）；不是快捷指令为 null。 */
export function quickMessageNumber(id: string): number | null {
  const slot = /^quick-message-(\d+)$/.exec(id);
  return slot ? Number(slot[1]) : null;
}

/** 文案键里的一段：连字符换成下划线（`channel-cycle` → `channel_cycle`）。 */
function keyPart(id: string): string {
  return id.replace(/[^A-Za-z0-9_]/g, "_");
}

/** 一个占着键的东西叫什么：热键的名字，或者“《英雄联盟》的打开聊天框键”这样的游戏配置里的键。 */
export function keyUseName(use: KeyUse): string {
  if (use.source === "game") {
    const game = use.ownerName ?? use.owner ?? "";
    if (use.gameKey === "channel") return m.hotkeys_game_channel({ game, channel: use.detail ?? "" });
    if (use.gameKey === "send") return m.hotkeys_game_send({ game });
    if (use.gameKey === "console") return m.hotkeys_game_console({ game });
    return m.hotkeys_game_open_chat({ game });
  }
  const number = quickMessageNumber(use.id);
  if (number !== null) return m.voice_hotkey_name_quick_message({ number });
  if (use.source === "plugin" && use.owner && use.command) {
    const command = use.command.slice(use.owner.length + 1);
    return pluginText(use.owner, `command_${keyPart(command)}_name`, use.label);
  }
  return NAMES[use.command ?? use.id]?.() ?? use.label;
}

/** 这一项是谁的：Kotone、插件名、自己加的、固定、找不到。 */
export function keyUseSource(use: KeyUse): string {
  if (use.source === "plugin") return pluginText(use.owner ?? "", "name", use.ownerName ?? use.owner ?? "");
  if (use.source === "custom") return m.hotkeys_source_custom();
  if (use.source === "fixed") return m.hotkeys_source_fixed();
  if (use.source === "unknown") return m.hotkeys_source_unknown();
  return m.hotkeys_source_core();
}

/** 一行底下的补充：快捷指令说的话、Esc 做什么、找不到的为什么。 */
export function keyUseDetail(use: KeyUse, settings: Settings | null): string {
  if (use.id === "escape") return m.hotkeys_escape_detail();
  if (use.source === "unknown") return m.hotkeys_unknown_detail();
  const number = quickMessageNumber(use.id);
  if (number === null) return "";
  const message = settings?.assistant.companion?.quickMessages?.[number - 1];
  if (!message) return use.detail ?? "";
  if (message.automationId?.trim()) return message.label.trim() || m.hotkeys_quick_automation();
  return message.label.trim() || message.text.trim();
}

/** 好几个冲突连成一句（“语音输入热键、《英雄联盟》的打开聊天框键”）。 */
export function keyUseNames(uses: KeyUse[]): string {
  return uses.map(keyUseName).join(m.hotkeys_list_separator());
}

/** 后端查出来的第一个冲突的名字（改键时的提示只说一个）；没有冲突为 null。 */
export function firstConflictName(check: HotkeyCheck | null | undefined): string | null {
  const first = check?.conflicts[0];
  return first ? keyUseName(first) : null;
}
