/**
 * 「设置」的分页（ADR 028）：陪玩组（怎么叫她 · 她怎么回答 · 权限 · 用量，原「琴音 → 设置」）+
 * 通用组（模型 · API 连接 · 声音 · 调参 · 外观 · 系统）。原「其他热键」分页拆走了：
 * 语音输入的热键在「语音输入 → 输入」，停止热键在「系统 → 停下」，启动快捷键在识别卡片里。
 */
export type CompanionSection = "call" | "reply" | "permissions" | "usage";
export type GeneralSection = "models" | "connections" | "sound" | "tuning" | "appearance" | "system";
export type AdvancedSection = CompanionSection | GeneralSection;

export const COMPANION_SECTIONS: { id: CompanionSection; label: string }[] = [
  { id: "call", label: "怎么叫她" }, { id: "reply", label: "她怎么回答" }, { id: "permissions", label: "权限" }, { id: "usage", label: "用量" },
];
export const GENERAL_SECTIONS: { id: GeneralSection; label: string }[] = [
  { id: "models", label: "模型" }, { id: "connections", label: "API 连接" }, { id: "sound", label: "声音" },
  { id: "tuning", label: "调参" }, { id: "appearance", label: "外观" }, { id: "system", label: "系统" },
];

export function isAdvancedSection(value: string): value is AdvancedSection {
  return [...COMPANION_SECTIONS, ...GENERAL_SECTIONS].some((item) => item.id === value);
}
