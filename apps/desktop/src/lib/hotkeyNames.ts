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
  // 自己加的（ADR 040 第 3 期）：玩家起的名字，没起就是要做的事（后端给的，带着跟她说的那句话）
  if (use.source === "custom") return use.label;
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

/** 一行底下的补充：快捷指令跟她说的话（起了名字时）、运行自动化、Esc 做什么、找不到的为什么。 */
export function keyUseDetail(use: KeyUse, settings: Settings | null): string {
  if (use.id === "escape") return m.hotkeys_escape_detail();
  if (use.source === "unknown") return m.hotkeys_unknown_detail();
  if (use.source !== "custom") return "";
  if (use.action === "automation") return m.hotkeys_quick_automation();
  // 没起名字时名字里已经有那句话了
  const id = use.id.slice("custom.".length);
  const binding = settings?.hotkeys?.bindings?.find((b) => b.id === id);
  const quick = settings?.assistant.companion?.quickMessages?.find((q) => q.id === id);
  const named = !!(binding?.label?.trim() || quick?.label.trim());
  return named ? (use.detail ?? "") : "";
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
