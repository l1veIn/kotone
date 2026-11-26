// 英雄联盟插件页面的数据（照 MCP Apps 的 structuredContent）：和后端 plugins/lol/src/pages.rs 一一对应。
export const PAGE_MATCH = "ui://kotone/lol/match";
export const PAGE_PLAYER = "ui://kotone/lol/player";
export const PAGE_GAME = "ui://kotone/lol/game";

/**
 * 图（ADR 039 D6，调研 G0.1）：数据里只有 id 和客户端的资源路径（`icon`、`profileIcon`、`crest`、`championIcon`，
 * 例如 `/lol-game-data/assets/v1/champion-icons/103.png`），拿不到时是空字符串。页面自己决定从哪取：
 * Kotone 里经回环代理的素材层，别的宿主换成 CommunityDragon 的地址。下面带 `?` 的字段是 2026-11-25 加的。
 */
export interface Champion { id: number; label: string; short: string; alias: string; icon?: string }

/** 召唤师技能、符文、符文系。 */
export interface Iconed { id: number; name: string; icon: string }
/** 装备：`slot` 0–6（6 是饰品），空格不列。 */
export interface Item extends Iconed { slot: number }
/** 海克斯：`rarity` 是 `kSilver` / `kGold` / `kPrismatic`，不知道时空。 */
export interface Augment extends Iconed { rarity: string }
/** 符文：海斗、斗魂竞技场没有符文时整个是 null。 */
export interface Perks { keystone: Iconed | null; primaryStyle: Iconed | null; subStyle: Iconed | null }
/** 一个排位队列；没定级时 `tier` 空、`label` 是“未定级”。 */
export interface Ranked {
  queue: string; tier: string; division: string; lp: number; wins: number; losses: number;
  highestTier: string; label: string; crest: string;
}
/** 单双排、灵活组排；没查到的是 null。 */
export interface Ranks { solo: Ranked | null; flex: Ranked | null }

export interface Summary {
  games: number; wins: number; losses: number;
  /** 正数 = 连胜几局，负数 = 连败几局。 */
  streak: number;
  kda: number; deaths: number;
  modeGames: number; modeWins: number; modeDpm: number;
  champGames: number; champWins: number;
}

export interface RecentGame { gameId: number; win: boolean; championId: number; championIcon?: string; kills: number; deaths: number; assists: number }

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
  /** 标签：`{kind, text}`（2026-11-25 起带种类，App 按种类上色）；旧数据是一句话。 */
  tags: (string | { kind: string; text: string })[];
  recent: RecentGame[];
  /** 是突破口时的原因。 */
  weak: string[] | null;
  profileIconId?: number | null;
  profileIcon?: string;
  /** 还在查、查不到时 null。 */
  summonerLevel?: number | null;
  spells?: Iconed[];
  /** `TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`；大乱斗这类没有分路时空。 */
  position?: string;
  ranked?: Ranks;
}

export interface MatchData {
  gameId: number; queueId: number; mode: string; random: boolean; waiting: number;
  allies: MatchPlayer[]; enemies: MatchPlayer[];
}

export interface Named { id: number; name: string }

export interface PlayerGame {
  gameId: number; queueId: number; queue: string; createdMs: number; durationS: number;
  champion: Champion; win: boolean; kills: number; deaths: number; assists: number;
  damage: number; dpm: number; gold: number; cs: number; augments: (Named & Partial<Augment>)[];
  level?: number; spells?: Iconed[]; items?: Item[]; perks?: Perks | null;
}

export interface PlayerData {
  puuid: string; riotId: string; hidden: boolean; rank: string | null; error: string | null;
  profileIconId?: number | null; profileIcon?: string; summonerLevel?: number | null; ranked?: Ranks;
  summary: Summary | null;
  champions: { champion: Champion; games: number; wins: number }[];
  games: PlayerGame[];
}

export interface GamePlayer {
  puuid: string; riotId: string; champion: Champion; level: number;
  kills: number; deaths: number; assists: number; damage: number; dpm: number; taken: number; gold: number; cs: number;
  items: (Named & Partial<Item>)[]; augments: (Named & Partial<Augment>)[];
  profileIconId?: number | null; profileIcon?: string; spells?: Iconed[]; perks?: Perks | null;
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
/** 只在推送里出现的两张卡片（ADR 043 D7，和后端 plugins/lol/src/pushes.rs 一致）：选英雄（原来的选英雄覆盖层）、
 * 问要不要下载海克斯三选一要的本地文字识别模型。 */
export const PAGE_CHAMP_SELECT = "ui://kotone/lol/champ-select";
export const PAGE_AUGMENT_OCR = "ui://kotone/lol/augment-ocr";

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

/** aramhex-data 的仓库（页面上署名链接）。 */
export const ARAMHEX_URL = "https://github.com/czgreat/aramhex-data";
export const OPGG_URL = "https://op.gg";
