// 选英雄卡片 B 轮加的几块（ADR 043 B 轮；后端 plugins/lol/src/pushes/champ_select.rs、draft.rs、comp.rs、import.rs）。
// 每块自己带来源（ADR 043 D8：一块只用一个来源）；没有的块是 null。
import type { Iconed, SourceId } from "../../types";

export interface SelectChampion {
  id: number;
  short: string;
  icon?: string;
}

/** 到哪一步了（draft.rs 的 `Stage`）。 */
export type Stage = "planning" | "banning" | "waiting" | "picking" | "locked";

export type BanCandidate =
  | { champion: SelectChampion; reason: "counter"; winRate: number | null }
  | { champion: SelectChampion; reason: "strong"; tier: string; winRate: number | null; banRate: number | null };

export interface BansBlock {
  lane: string | null;
  /** 我预选的英雄（“克制它的”说的是它）；还没预选时 null。 */
  against: SelectChampion | null;
  list: BanCandidate[];
  source: SourceId;
}

export type CompFlag = "all_physical" | "all_magic" | "no_frontline" | "no_cc";
export interface CompSide {
  count: number;
  physical: number;
  magic: number;
  mixed: number;
  frontline: number;
  cc: number;
  melee: number;
  ranged: number;
  flags: CompFlag[];
}
export interface CompBlock {
  ally: CompSide | null;
  enemy: CompSide | null;
  source: SourceId;
}

export interface PoolVs {
  champion: SelectChampion;
  winRate: number | null;
}
export interface PoolEntry {
  champion: SelectChampion;
  games: number;
  wins: number;
  good: PoolVs[];
  bad: PoolVs[];
  tier: string | null;
  winRate: number | null;
}
export interface PoolBlock {
  lane: string | null;
  against: SelectChampion[];
  list: PoolEntry[];
  source: SourceId;
}

export interface MatchupBlock {
  me: SelectChampion;
  opponent: SelectChampion;
  lane: string | null;
  /** 对手是猜的（对面在这一路选用率最高的）。 */
  guessed: boolean;
  laneAdvantage: "me" | "opponent" | null;
  soloKillAdvantage: "me" | "opponent" | null;
  /** OP.GG 的词（`even` 等）；认得的翻成中文，不认得的原样。 */
  playStyle: string | null;
  tip: string | null;
  source: SourceId;
  url?: string;
}

export interface ItemGroup {
  kind: "starting" | "boots" | "core" | "late";
  items: Iconed[];
  winRate: number | null;
  pickRate: number | null;
}
export interface MineBlock {
  champion: SelectChampion;
  lane: string | null;
  locked: boolean;
  source: SourceId;
  patch: string;
  runes: { styles: Iconed[]; perks: Iconed[]; winRate: number | null; pickRate: number | null } | null;
  items: ItemGroup[];
  skills: { priority: string[]; order: string[] } | null;
  spells: Iconed[];
  spellsWinRate: number | null;
  augments: { id: number; name: string; rarity: string; icon: string; delta: number; games: number }[];
  importable: { runes: boolean; items: boolean; spells: boolean };
}

/** 一键导入（`lol.import`）每一样写成了什么（plugins/lol/src/client/loadout.rs 的 `Written`）。 */
export type WriteResult = "created" | "updated" | "replaced" | "written" | "no_data" | "not_now" | "no_room" | "error";
export interface Written {
  result: WriteResult;
  name?: string;
  old?: string;
  error?: string;
  spells?: [number, number];
  blocks?: number;
}
export interface ImportResult {
  status: string;
  reason?: string;
  text?: string;
  runes?: Written;
  items?: Written;
  spells?: Written;
}
