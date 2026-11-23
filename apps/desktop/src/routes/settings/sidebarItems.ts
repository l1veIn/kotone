// 左侧栏的页面清单（AppSidebar 和“全部页面”共用）。分组和默认顺序在 lib/sidebarLayout.ts。
import type { Component } from "svelte";
import { Bell, Bot, Gamepad2, Info, Mic, Music, Settings2, Zap } from "@lucide/svelte";
import { m } from "$lib/paraglide/messages.js";
import { SIDEBAR_GROUPS, type SidebarGroupId, type SidebarPrefs } from "../../lib/sidebarLayout";
import { updateSettings } from "../../lib/ipc";
import { errText, settingsStore, toast } from "../../lib/stores/ui";

export type PageId =
  | "general" | "processing" | "assistant" | "automation" | "game" | "music"
  | "history" | "advanced" | "about" | "character" | "legacy-reminders";

export type NavItem = { id: PageId; label: string; icon: Component };

/** 可以排的两组里的页面（名字跟着当前角色名、旧版语音输入用户看到的“游戏适配”）。 */
export function navItems(companionName: string, legacyVoiceOnly: boolean): Record<string, NavItem> {
  return {
    assistant: { id: "assistant", label: companionName, icon: Bot },
    game: { id: "game", label: legacyVoiceOnly ? m.shell_sidebar_game_adapt() : m.shell_sidebar_game_list(), icon: Gamepad2 },
    automation: { id: "automation", label: m.shell_sidebar_automation(), icon: Zap },
    music: { id: "music", label: m.shell_sidebar_music(), icon: Music },
    general: { id: "general", label: m.shell_sidebar_voice_input(), icon: Mic },
    "legacy-reminders": { id: "legacy-reminders", label: m.shell_sidebar_legacy_reminders(), icon: Bell },
  };
}

/** 底部固定的两项（不能排、不能藏）。 */
export function footItems(): NavItem[] {
  return [
    { id: "advanced", label: m.shell_sidebar_settings(), icon: Settings2 },
    { id: "about", label: m.shell_sidebar_about(), icon: Info },
  ];
}

export function groupLabel(id: SidebarGroupId): string {
  return id === "companion" ? m.shell_sidebar_group_companion() : m.shell_sidebar_group_voice_input();
}

/** 每组现在有的页面（默认顺序）：旧提醒只有升级用户有。 */
export function presentGroups(legacyReminders: boolean): { id: SidebarGroupId; pages: string[] }[] {
  return SIDEBAR_GROUPS.map((group) => ({
    id: group.id,
    pages: group.pages.filter((id) => id !== "legacy-reminders" || legacyReminders),
  }));
}

/** 存左侧栏的顺序 / 隐藏（整份写回 `ui.sidebar`；数组在设置补丁里是整个替换）。不弹“已保存”：点一下就看得到。 */
export async function saveSidebarPrefs(next: SidebarPrefs): Promise<void> {
  try {
    settingsStore.set(await updateSettings({ ui: { sidebar: { order: next.order, hidden: next.hidden } } }));
  } catch (error) {
    toast(false, m.shell_sidebar_save_failed({ error: errText(error) }));
  }
}
