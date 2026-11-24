/**
 * 「设置」的分页（ADR 028）：陪玩组（怎么叫她 · 她怎么回答 · 权限 · 用量，原「琴音 → 设置」）+
 * 通用组（模型 · API 连接 · 声音 · 调参 · 外观 · 系统 · 浏览器 · 插件）。原「其他热键」分页拆走了：
 * 语音输入的热键在「语音输入 → 输入」，停止热键在「系统 → 停下」，启动快捷键在识别卡片里。
 */
import { m } from "../../../../lib/paraglide/messages.js";

export type CompanionSection = "call" | "reply" | "permissions" | "usage";
export type GeneralSection = "models" | "connections" | "sound" | "tuning" | "appearance" | "system" | "browser" | "plugins";
export type AdvancedSection = CompanionSection | GeneralSection;

export const COMPANION_SECTIONS: { id: CompanionSection; label: string }[] = [
  { id: "call", label: m.system_section_call() }, { id: "reply", label: m.system_section_reply() }, { id: "permissions", label: m.system_section_permissions() }, { id: "usage", label: m.system_section_usage() },
];
export const GENERAL_SECTIONS: { id: GeneralSection; label: string }[] = [
  { id: "models", label: m.system_section_models() }, { id: "connections", label: m.system_section_connections() }, { id: "sound", label: m.system_section_sound() },
  { id: "tuning", label: m.system_section_tuning() }, { id: "appearance", label: m.system_section_appearance() }, { id: "system", label: m.system_section_system() },
  // 内置浏览器留在本体（ADR 038“怎么定的”）：开关、扩展、清除登录
  { id: "browser", label: m.system_section_browser() },
  // 插件的列表和开关（ADR 038）；每个插件自己的设置在它的页面里
  { id: "plugins", label: m.system_section_plugins() },
];

export function isAdvancedSection(value: string): value is AdvancedSection {
  return [...COMPANION_SECTIONS, ...GENERAL_SECTIONS].some((item) => item.id === value);
}
