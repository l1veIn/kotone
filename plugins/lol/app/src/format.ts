// 数字、时间、段位的小格式（文字在 messages/，这里只拼）。阈值照 docs/design/lol-app.md 第一节 9。
import { lang, m } from "./i18n.svelte";
import type { Ranked } from "./types";

/** 胜率（整数百分比，没局数时 0）。 */
export function pctOf(wins: number, games: number): number {
  return games > 0 ? Math.round((wins / games) * 100) : 0;
}

/** 胜率的好坏：≥ 53% 好、≤ 47% 差；少于 5 局不着色。 */
export function rateTone(wins: number, games: number): "good" | "bad" | "" {
  if (games < 5) return "";
  const p = pctOf(wins, games);
  return p >= 53 ? "good" : p <= 47 ? "bad" : "";
}

/** KDA 的好坏：≥ 3.5 好、< 2 差（LeagueAkari 按全场四分位，我们先用固定阈值，要改只改这里）。 */
export function kdaTone(kda: number): "good" | "bad" | "" {
  return kda >= 3.5 ? "good" : kda < 2 ? "bad" : "";
}

/** (K + A) / D，一位小数（没死时按 1 算）。 */
export function kdaOf(k: number, d: number, a: number): number {
  return Math.round(((k + a) / Math.max(1, d)) * 10) / 10;
}

/** 0..1 的比例 → 百分比（一位小数）；没有时 “—”。 */
export function rate(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—";
}

/** 数字按语言分组（12,345）。 */
export function num(value: number): string {
  return Math.round(value).toLocaleString(lang());
}

/** 一位小数。 */
export function one(value: number): string {
  return (Math.round(value * 10) / 10).toFixed(1);
}

/** 时长 mm:ss。 */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** 开局时间（月-日 时:分）。 */
export function when(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 名字的第一个字（图挂了时的占位）。 */
export function firstChar(name: string | null | undefined): string {
  return Array.from(String(name ?? "").trim())[0] ?? "?";
}

/** `名字#编号` → [名字, 编号]。 */
export function splitRiotId(riotId: string): [string, string] {
  const at = riotId.lastIndexOf("#");
  return at < 0 ? [riotId, ""] : [riotId.slice(0, at), riotId.slice(at + 1)];
}

/** 分路的名字（`TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`，强度榜写 `SUPPORT`）；不认识的给空。 */
export function laneName(lane: string | null | undefined): string {
  switch (String(lane ?? "").toUpperCase()) {
    case "TOP":
      return m.plugin_lol_lane_top();
    case "JUNGLE":
      return m.plugin_lol_lane_jungle();
    case "MIDDLE":
      return m.plugin_lol_lane_middle();
    case "BOTTOM":
      return m.plugin_lol_lane_bottom();
    case "UTILITY":
    case "SUPPORT":
      return m.plugin_lol_lane_support();
    default:
      return "";
  }
}

const TIERS = ["IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "EMERALD", "DIAMOND", "MASTER", "GRANDMASTER", "CHALLENGER"];
const APEX = new Set(["MASTER", "GRANDMASTER", "CHALLENGER"]);

/** 段位的名字（`short`：卡片里的短写，“钻”）。不认识的段位原样给。 */
export function tierName(tier: string, short = false): string {
  const i = TIERS.indexOf(tier.toUpperCase());
  if (i < 0) return tier;
  const names = (short ? m.plugin_lol_tiers_short() : m.plugin_lol_tiers()).split(",");
  return names[i]?.trim() || tier;
}

/** “钻石 II · 45 点”（`short`：“钻 II”）；没定级是“未定级”。 */
export function rankText(r: Ranked, short = false): string {
  if (!r.tier) return m.plugin_lol_unranked();
  const name = tierName(r.tier, short);
  const div = APEX.has(r.tier.toUpperCase()) || !r.division ? "" : ` ${r.division}`;
  return short ? `${name}${div}` : m.plugin_lol_rank_lp({ rank: `${name}${div}`, lp: r.lp });
}
