/**
 * 「设置」的分页（ADR 028）：只剩原来的通用组（模型 · API 连接 · 声音 · 热键 · 调参 · 外观 · 系统 · 浏览器）。
 * 原来的陪玩组（怎么叫她 · 她怎么回答 · 权限 · 用量）搬进了角色页，叫 唤醒 · 交互 · 权限 · 用量（交接第十一节第 8 条，2026-11-25）；
 * 旧的分页 ID（call、reply、permissions、usage）由 navigation.ts 的 normalizeTarget 落到角色页。
 * 插件不在设置里：左侧栏有单独的「插件」（ADR 038）。原「其他热键」分页拆走了：
 * 语音输入的热键在「语音输入 → 输入」，停止热键在「系统 → 停下」，启动热键在「模型 → 语音识别」；
 * 「热键」一页列出全部（ADR 040 第 1 期只读，第 2 期起只在那里改）。
 */
import { m } from "../../../../lib/paraglide/messages.js";

export type GeneralSection = "models" | "connections" | "sound" | "hotkeys" | "tuning" | "appearance" | "system" | "browser";
export type AdvancedSection = GeneralSection;

export const GENERAL_SECTIONS: { id: GeneralSection; label: string }[] = [
  { id: "models", label: m.system_section_models() }, { id: "connections", label: m.system_section_connections() }, { id: "sound", label: m.system_section_sound() },
  // 全部热键一页（ADR 040 D2；这一期只读，“去改”跳到原来改它的地方）
  { id: "hotkeys", label: m.hotkeys_tab() },
  { id: "tuning", label: m.system_section_tuning() }, { id: "appearance", label: m.system_section_appearance() }, { id: "system", label: m.system_section_system() },
  // 内置浏览器留在本体（ADR 038“怎么定的”）：开关、扩展、清除登录
  { id: "browser", label: m.system_section_browser() },
];

export function isAdvancedSection(value: string): value is AdvancedSection {
  return GENERAL_SECTIONS.some((item) => item.id === value);
}
