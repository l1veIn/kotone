// 英雄联盟插件页面的数据（照 MCP Apps 的 structuredContent）：和后端 plugins/lol/src/pages.rs 一一对应。
export const PAGE_MATCH = "ui://kotone/lol/match";
export const PAGE_PLAYER = "ui://kotone/lol/player";
export const PAGE_GAME = "ui://kotone/lol/game";

export interface Champion { id: number; label: string; short: string; alias: string }

export interface Summary {
  games: number; wins: number; losses: number;
  /** 正数 = 连胜几局，负数 = 连败几局。 */
  streak: number;
  kda: number; deaths: number;
  modeGames: number; modeWins: number; modeDpm: number;
  champGames: number; champWins: number;
}

export interface RecentGame { gameId: number; win: boolean; championId: number; kills: number; deaths: number; assists: number }

export interface MatchPlayer {
  puuid: string;
  champion: Champion;
  name: string;
  riotId: string;
  me: boolean;
  ally: boolean;
  /** 开黑分组（同一边同一个数 = 一起排的）。 */
  party: number | null;
  /** 还在查。 */
  pending: boolean;
  hidden: boolean;
  rank: string | null;
  error: string | null;
  summary: Summary | null;
  tags: string[];
  recent: RecentGame[];
  /** 是突破口时的原因。 */
  weak: string[] | null;
}

export interface MatchData {
  gameId: number; queueId: number; mode: string; random: boolean; waiting: number;
  allies: MatchPlayer[]; enemies: MatchPlayer[];
}

export interface Named { id: number; name: string }

export interface PlayerGame {
  gameId: number; queueId: number; queue: string; createdMs: number; durationS: number;
  champion: Champion; win: boolean; kills: number; deaths: number; assists: number;
  damage: number; dpm: number; gold: number; cs: number; augments: Named[];
}

export interface PlayerData {
  puuid: string; riotId: string; hidden: boolean; rank: string | null; error: string | null;
  summary: Summary | null;
  champions: { champion: Champion; games: number; wins: number }[];
  games: PlayerGame[];
}

export interface GamePlayer {
  puuid: string; riotId: string; champion: Champion; level: number;
  kills: number; deaths: number; assists: number; damage: number; dpm: number; taken: number; gold: number; cs: number;
  items: Named[]; augments: Named[];
}

export interface GameData {
  gameId: number; queueId: number; queue: string; createdMs: number; durationS: number;
  teams: { teamId: number; win: boolean; players: GamePlayer[] }[];
}

/** 工具没给页面时（不在对局里、客户端没开）给她的那份：`{status, text}`。 */
export function statusText(content: unknown): string {
  if (content && typeof content === "object" && "text" in content) return String((content as { text: unknown }).text ?? "");
  return typeof content === "string" ? content : "";
}

// ---------- 第 3 步：网上的统计（和后端 plugins/lol/src/stats_tools.rs 一一对应） ----------
export const PAGE_CHAMPION = "ui://kotone/lol/champion";
export const PAGE_TIER = "ui://kotone/lol/tier";
export const OVERLAY_CHAMP_SELECT = "ui://kotone/lol/champ-select";
export const OVERLAY_AUGMENTS = "ui://kotone/lol/augment-scores";

/** 统计里的英雄（没有英文 id）。 */
export interface StatChampion { id: number; label: string; short: string }

/** 数据从哪来（页面上署名）。 */
export interface StatSource { name: string; date?: string; license?: string }

/** 胜率、选用率都是 0..1。 */
export interface Matchup { champion: StatChampion; winRate: number | null }

export interface BuildOption { items: Named[]; winRate: number | null; pickRate: number | null }

export interface RunePage { primary: Named[]; secondary: Named[]; shards: Named[]; winRate: number | null; pickRate: number | null; games: number }

export interface TierRowRaw {
  rank: number | null; championId: number; tier: string; lane: string;
  winRate: number | null; pickRate: number | null; banRate: number | null; counters: number[]; rankChange: number | null;
}

export interface OpggBlock { text?: string; error?: string; attribution: string; url: string; stale?: boolean }

export interface ChampionRift {
  mode: "rift";
  champion: StatChampion;
  lane: string; laneName: string; patch: string; date: string;
  tier: TierRowRaw | null;
  goodAgainst: Matchup[]; badAgainst: Matchup[];
  build: { starting: BuildOption[]; boots: BuildOption[]; core: BuildOption[]; fourth: BuildOption[]; fifth: BuildOption[]; sixth: BuildOption[] };
  runes: RunePage[];
  skillPriority: string[]; skillOrder: string[];
  spells: { spells: Named[]; winRate: number | null; pickRate: number | null }[];
  sources: StatSource[];
  stale: boolean;
  opgg: OpggBlock | null;
}

export interface MayhemAugment { id: number; name: string; rarity: string; games: number; winRate: number; delta: number; lowSample: boolean }

export interface ChampionMayhem {
  mode: "mayhem";
  champion: StatChampion;
  rank: number | null; winRate: number | null; pickRate: number | null;
  partners: StatChampion[];
  augments: MayhemAugment[];
  /** aramhex-data 的署名（CC BY-NC 4.0）；没读到 aramhex 时 null。 */
  attribution: string | null;
  sources: StatSource[];
}

export type ChampionData = ChampionRift | ChampionMayhem;

export interface TierRow {
  champion: StatChampion; tier: string; rank: number | null;
  winRate: number | null; pickRate: number | null; banRate: number | null; counters: StatChampion[];
}

export interface TierRift {
  mode: "rift"; date: string;
  lanes: Record<string, TierRow[]>;
  laneNames: { id: string; name: string }[];
  sources: StatSource[];
}

export interface TierMayhem {
  mode: "mayhem"; date: string;
  rows: { champion: StatChampion; rank: number | null; rankChange: number | null; winRate: number | null; pickRate: number | null }[];
  sources: StatSource[];
}

export type TierData = TierRift | TierMayhem;

/** 覆盖层：选英雄（客户端窗口右上角）。 */
export interface ChampSelectOverlayData {
  mode: "mayhem" | "rift";
  team: { champion: string; me: boolean; winRate: number | null; rank?: number | null; lane?: string | null; tier?: string | null }[];
  bench: { champion: string; winRate: number | null; rank?: number | null }[];
  source: string;
}

/** 覆盖层：海斗三选一，一张卡片一个。 */
export interface AugmentOverlayData {
  augment: string; champion: string;
  score: { games: number; winRate: number; smoothed: number; delta: number; lowSample: boolean } | null;
  best: boolean;
  attribution: string;
}

/** aramhex-data 的仓库（页面上署名链接）。 */
export const ARAMHEX_URL = "https://github.com/czgreat/aramhex-data";
export const OPGG_URL = "https://op.gg";
