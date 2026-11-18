/**
 * 外观偏好：配色、明暗、界面缩放、界面字体。
 *
 * Palette ids and swatches adapted from Codeg's src/lib/theme-presets.ts (via RambleDesk
 * apps/desktop/src/lib/appearance/appearanceSettings.ts).
 * Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0
 * Upstream: xintaofei/codeg, commit 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1.
 * Kotone: kotone palette as default, Chinese labels, light / dark / system mode in the same
 * preference, Windows CJK font stacks; the bounded-value and custom-font validation follows
 * RambleDesk's implementation.
 */
export const PALETTES = [
  { id: "kotone", label: "Kotone", swatch: "#00e5ff" },
  { id: "neutral", label: "中性", swatch: "oklch(0.205 0 0)" },
  { id: "zinc", label: "锌灰", swatch: "oklch(0.21 0.006 285.885)" },
  { id: "slate", label: "石板", swatch: "oklch(0.208 0.042 265.755)" },
  { id: "stone", label: "岩石", swatch: "oklch(0.216 0.006 56.043)" },
  { id: "gray", label: "灰", swatch: "oklch(0.21 0.034 264.665)" },
  { id: "red", label: "红", swatch: "oklch(0.637 0.237 25.331)" },
  { id: "rose", label: "玫瑰", swatch: "oklch(0.645 0.246 16.439)" },
  { id: "orange", label: "橙", swatch: "oklch(0.705 0.213 47.604)" },
  { id: "green", label: "绿", swatch: "oklch(0.723 0.219 149.579)" },
  { id: "blue", label: "蓝", swatch: "oklch(0.546 0.245 262.881)" },
  { id: "yellow", label: "黄", swatch: "oklch(0.795 0.184 86.047)" },
  { id: "violet", label: "紫", swatch: "oklch(0.606 0.25 292.717)" },
] as const;

export type PaletteId = (typeof PALETTES)[number]["id"];
export type ThemeMode = "dark" | "light" | "system";
export type FontId = "system" | "yahei" | "dengxian" | "noto-sans-sc" | "harmony" | "custom";

const SYSTEM_UI = '"Segoe UI", "Microsoft YaHei", system-ui, sans-serif';

export const THEME_MODES: readonly { id: ThemeMode; label: string }[] = [
  { id: "dark", label: "深色" },
  { id: "light", label: "浅色" },
  { id: "system", label: "跟随系统" },
];

export const UI_FONTS: readonly { id: FontId; label: string; stack: string }[] = [
  { id: "system", label: "默认（Segoe UI + 微软雅黑）", stack: SYSTEM_UI },
  { id: "yahei", label: "微软雅黑", stack: `"Microsoft YaHei UI", "Microsoft YaHei", ${SYSTEM_UI}` },
  { id: "dengxian", label: "等线", stack: `DengXian, ${SYSTEM_UI}` },
  { id: "noto-sans-sc", label: "思源黑体 / Noto Sans SC（需已安装）", stack: `"Noto Sans SC", "Source Han Sans SC", "Source Han Sans CN", ${SYSTEM_UI}` },
  { id: "harmony", label: "HarmonyOS Sans（需已安装）", stack: `"HarmonyOS Sans SC", "HarmonyOS Sans", ${SYSTEM_UI}` },
  { id: "custom", label: "自定义", stack: SYSTEM_UI },
];

/** 界面缩放（百分比）。字体与间距一起缩放，布局不变形。 */
export const ZOOM_LEVELS = [80, 90, 100, 110, 125, 150] as const;

export interface AppearanceSettings {
  palette: PaletteId;
  mode: ThemeMode;
  zoom: number;
  uiFont: FontId;
  uiCustomFont: string;
}

/** 默认就是现在的 Kotone 深色界面。 */
export const DEFAULT_APPEARANCE: Readonly<AppearanceSettings> = Object.freeze({
  palette: "kotone",
  mode: "dark",
  zoom: 100,
  uiFont: "system",
  uiCustomFont: "",
});

function choice<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && allowed.includes(value as T) ? (value as T) : fallback;
}

function nearest(value: unknown, allowed: readonly number[], fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return allowed.reduce((best, candidate) => (Math.abs(candidate - value) < Math.abs(best - value) ? candidate : best));
}

/** 只接受字体名：不接受 CSS 表达式、转义或声明语法。 */
function customFamilies(value: unknown): string[] | null {
  if (typeof value !== "string" || value.length > 160 || /[\u0000-\u001f\u007f]/u.test(value)) return null;
  const families = value.split(",").map((part) => {
    const name = part.trim();
    const quoted = (name.startsWith('"') && name.endsWith('"')) || (name.startsWith("'") && name.endsWith("'"));
    return (quoted ? name.slice(1, -1) : name).trim();
  });
  return families.length <= 8 && families.every((name) => name.length > 0 && /^[\p{L}\p{M}\p{N} ._'"-]+$/u.test(name))
    ? families
    : null;
}

export function normalizeAppearance(value: unknown): AppearanceSettings {
  const input: Record<string, unknown> =
    value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
  return {
    palette: choice(input.palette, PALETTES.map((palette) => palette.id), DEFAULT_APPEARANCE.palette),
    mode: choice(input.mode, THEME_MODES.map((mode) => mode.id), DEFAULT_APPEARANCE.mode),
    zoom: nearest(input.zoom, ZOOM_LEVELS, DEFAULT_APPEARANCE.zoom),
    uiFont: choice(input.uiFont, UI_FONTS.map((font) => font.id), DEFAULT_APPEARANCE.uiFont),
    uiCustomFont: customFamilies(input.uiCustomFont) ? (input.uiCustomFont as string).trim() : "",
  };
}

export function resolveFontFamily(settings: AppearanceSettings): string {
  if (settings.uiFont !== "custom") return UI_FONTS.find((font) => font.id === settings.uiFont)?.stack ?? SYSTEM_UI;
  const families = customFamilies(settings.uiCustomFont);
  if (!families) return SYSTEM_UI;
  return `${families.map((name) => `"${name.replace(/"/g, '\\"')}"`).join(", ")}, ${SYSTEM_UI}`;
}

