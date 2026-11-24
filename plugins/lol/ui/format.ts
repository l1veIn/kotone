// 英雄联盟页面里的小格式（不含文字，文字在 messages/）。
import { getLocale } from "$lib/paraglide/runtime.js";

export function kda(k: number, d: number, a: number): string {
  return `${k}/${d}/${a}`;
}

/** 胜率（整数百分比）；没局数时空。 */
export function pct(wins: number, games: number): string {
  return games > 0 ? `${Math.round((wins / games) * 100)}%` : "";
}

/** 时长 mm:ss。 */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** 数字按当前语言分组（12,345）。 */
export function num(value: number): string {
  return Math.round(value).toLocaleString(getLocale());
}

/** 开局时间（月-日 时:分，按当前语言）。 */
export function when(ms: number): string {
  return new Date(ms).toLocaleString(getLocale(), { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

/** 开黑分组的颜色（同一组同一个颜色）。 */
const PARTY_COLORS = ["bg-sky-500", "bg-amber-500", "bg-fuchsia-500", "bg-emerald-500"];
export function partyColor(party: number, all: (number | null)[]): string {
  const groups = [...new Set(all.filter((p): p is number => p !== null && all.filter((x) => x === p).length > 1))].sort((a, b) => a - b);
  const index = groups.indexOf(party);
  return index >= 0 ? PARTY_COLORS[index % PARTY_COLORS.length] : "";
}

/** 0..1 的比例 → 百分比（一位小数）；没有时 “—”。 */
export function rate(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—";
}

/** 比平均高 / 低几个百分点（带符号，一位小数）。 */
export function signed(value: number): string {
  return `${value > 0 ? "+" : value < 0 ? "−" : "±"}${Math.abs(value).toFixed(1)}`;
}

/** 局数：按当前语言的紧凑写法（1.5万 / 15K）。 */
export function compact(value: number): string {
  return new Intl.NumberFormat(getLocale(), { notation: "compact", maximumFractionDigits: 1 }).format(value);
}
