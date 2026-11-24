// 页面的数据：工具结果的 structuredContent（plugins/lol/src/pages.rs、stats_tools.rs）。字段说明见
// docs/design/lol-app.md 第二节和 docs/plugin-refactor.md 第九节“英雄联盟 App：素材层和页面数据”。
// 图都只给客户端路径（拿不到时空字符串），页面经 assets.ts 决定从哪取。带 `?` 的是后加的字段：旧数据里没有时照样能显示。

export interface Champion {
  id: number;
  label: string;
  short: string;
  alias?: string;
  icon?: string;
}

/** 召唤师技能、符文、符文系、装备、海克斯：名字 + 图标路径。 */
export interface Iconed {
  id: number;
  name: string;
  icon: string;
}
/** 装备：`slot` 0–6（6 是饰品），空格不列。 */
export interface Item extends Iconed {
  slot: number;
}
/** 海克斯：`rarity` 是 `kSilver` / `kGold` / `kPrismatic`（也可能是 `kEventChoice`），不知道时空。 */
export interface Augment extends Iconed {
  rarity: string;
}
export interface Perks {
  keystone: Iconed | null;
  primaryStyle: Iconed | null;
  subStyle: Iconed | null;
}
/** 一个排位队列；没定级时 `tier` 空。 */
export interface Ranked {
  queue: string;
  tier: string;
  division: string;
  lp: number;
  wins: number;
  losses: number;
  highestTier: string;
  label: string;
  crest: string;
}
export interface Ranks {
  solo: Ranked | null;
  flex: Ranked | null;
}

export interface Summary {
  games: number;
  wins: number;
  losses: number;
  streak: number;
  kda: number;
  deaths: number;
  modeGames: number;
  modeWins: number;
  modeDpm: number;
  champGames: number;
  champWins: number;
}

export interface RecentGame {
  gameId: number;
  win: boolean;
  championId: number;
  championIcon?: string;
  kills: number;
  deaths: number;
  assists: number;
}

export type TagKind = "win_streak" | "loss_streak" | "first_time" | "main";
/** 标签：带种类的（新）或者只有一句话（旧数据）。 */
export type Tag = { kind: TagKind | string; text: string } | string;

/** 这局的阶段（游戏中心的阶段，ADR 036）。旧数据没有时按“在对局里”显示。 */
export type MatchPhase = "champ_select" | "loading" | "in_game" | "post_match";
/** 选人时：锁了 / 预选 / 还没选。 */
export type PickState = "locked" | "intent" | "none";

export interface MatchPlayer {
  puuid: string;
  champion: Champion;
  name: string;
  riotId: string;
  me: boolean;
  ally: boolean;
  party: number | null;
  pending: boolean;
  hidden: boolean;
  rank: string | null;
  error: string | null;
  summary: Summary | null;
  tags: Tag[];
  recent: RecentGame[];
  weak: string[] | null;
  profileIconId?: number | null;
  profileIcon?: string;
  summonerLevel?: number | null;
  spells?: Iconed[];
  position?: string;
  ranked?: Ranks;
  pick?: PickState;
}

export interface MatchResult {
  win: boolean;
  durationS: number;
  gameId: number;
}

export interface MatchData {
  gameId: number;
  queueId: number;
  mode: string;
  random: boolean;
  waiting: number;
  allies: MatchPlayer[];
  enemies: MatchPlayer[];
  phase?: MatchPhase;
  /** 进这个阶段的时间（毫秒）。 */
  phaseSince?: number | null;
  /** 排位选人时名字被隐藏。 */
  namesHidden?: boolean;
  /** 备选席（大乱斗、海斗选人）。 */
  bench?: Champion[];
  result?: MatchResult | null;
  resultPending?: boolean;
}

export function isMatchData(value: unknown): value is MatchData {
  const v = value as Partial<MatchData> | null;
  return !!v && Array.isArray(v.allies) && Array.isArray(v.enemies);
}

export interface PlayerGame {
  gameId: number;
  queueId: number;
  queue: string;
  createdMs: number;
  durationS: number;
  champion: Champion;
  win: boolean;
  kills: number;
  deaths: number;
  assists: number;
  damage: number;
  dpm: number;
  gold: number;
  cs: number;
  augments: Augment[];
  level?: number;
  spells?: Iconed[];
  items?: Item[];
  perks?: Perks | null;
}

export interface PlayerData {
  puuid: string;
  riotId: string;
  hidden: boolean;
  rank: string | null;
  error: string | null;
  profileIconId?: number | null;
  profileIcon?: string;
  summonerLevel?: number | null;
  ranked?: Ranks;
  summary: Summary | null;
  champions: { champion: Champion; games: number; wins: number }[];
  games: PlayerGame[];
}

export function isPlayerData(value: unknown): value is PlayerData {
  const v = value as Partial<PlayerData> | null;
  return !!v && typeof v.puuid === "string" && Array.isArray(v.games);
}

export interface GamePlayer {
  puuid: string;
  riotId: string;
  champion: Champion;
  level: number;
  kills: number;
  deaths: number;
  assists: number;
  damage: number;
  dpm: number;
  taken: number;
  gold: number;
  cs: number;
  items: Item[];
  augments: Augment[];
  profileIconId?: number | null;
  profileIcon?: string;
  spells?: Iconed[];
  perks?: Perks | null;
}

export interface GameData {
  gameId: number;
  queueId: number;
  queue: string;
  createdMs: number;
  durationS: number;
  teams: { teamId: number; win: boolean; players: GamePlayer[] }[];
}

export function isGameData(value: unknown): value is GameData {
  const v = value as Partial<GameData> | null;
  return !!v && Array.isArray(v.teams);
}

// ---------- 强度榜（`lol.tier`，plugins/lol/src/stats_tools.rs 的 tier_tool） ----------

export interface Source {
  name: string;
  date?: string;
  license?: string;
  url?: string;
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
  | { mode: "rift"; date: string; lanes: Record<string, RiftRow[]>; laneNames: { id: string; name: string }[]; sources: Source[]; stale?: boolean }
  | { mode: "mayhem"; date: string; rows: MayhemRow[]; sources: Source[]; stale?: boolean };

export function isTierData(value: unknown): value is TierData {
  const mode = (value as { mode?: unknown } | null)?.mode;
  return mode === "rift" || mode === "mayhem";
}
