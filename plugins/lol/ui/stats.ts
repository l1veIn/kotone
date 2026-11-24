// 英雄联盟“网上的统计”页面共用的小工具（不含文字）。
import type { StatSource } from "./types";

/** 海克斯稀有度的颜色（棱彩 / 金 / 银）。 */
export function rarityClass(rarity: string): string {
  if (rarity === "prismatic") return "bg-fuchsia-400";
  if (rarity === "gold") return "bg-amber-400";
  return "bg-slate-400";
}

/** 数据来源写成一行：名字（许可）日期。日期只留到天。 */
export function sourcesLine(sources: StatSource[]): string {
  return sources
    .map((s) => [s.name, s.license ? `${s.license}` : "", s.date ? s.date.slice(0, 10) : ""].filter(Boolean).join(" · "))
    .join(" | ");
}
