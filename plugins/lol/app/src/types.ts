// 页面的数据（`lol.tier` 的 structuredContent，plugins/lol/src/stats_tools.rs 的 tier_tool）。

export interface Champion {
  id: number;
  label: string;
  short: string;
}

export interface Source {
  name: string;
  date?: string;
  license?: string;
}

export interface RiftRow {
  champion: Champion;
  tier: string;
  rank: number | null;
  winRate: number | null;
  pickRate: number | null;
  banRate: number | null;
  counters: Champion[];
}

export interface MayhemRow {
  champion: Champion;
  rank: number | null;
  rankChange: number | null;
  winRate: number | null;
  pickRate: number | null;
}

export type TierData =
  | { mode: "rift"; date: string; lanes: Record<string, RiftRow[]>; laneNames: { id: string; name: string }[]; sources: Source[] }
  | { mode: "mayhem"; date: string; rows: MayhemRow[]; sources: Source[] };

export function isTierData(value: unknown): value is TierData {
  const mode = (value as { mode?: unknown } | null)?.mode;
  return mode === "rift" || mode === "mayhem";
}

/** 百分比（一位小数）；没有数据是 —。 */
export function rate(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—";
}

/** 数据来源一行。 */
export function sourcesLine(sources: Source[]): string {
  return sources
    .map((s) => [s.name, s.license ?? "", s.date ? s.date.slice(0, 10) : ""].filter(Boolean).join(" · "))
    .join(" | ");
}
