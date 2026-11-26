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
/**
 * 看得到是谁没有（ADR 043 D9）：known 查战绩；hidden 隐藏了身份（客户端不给名字和 puuid）；name_hidden 选人时看不到名字；
 * bot 电脑（接口标了的）。后三种只有英雄，不查。旧数据没有这个字段：没有 puuid、名字的按 name_hidden。
 */
export type Identity = "known" | "hidden" | "name_hidden" | "bot";

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
  identity?: Identity;
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
  /** 这一行是我（看别人的对局时都是 false）。 */
  me?: boolean;
}

export interface GameTeam {
  teamId: number;
  win: boolean;
  /** 我在这一队。 */
  ally?: boolean;
  players: GamePlayer[];
}

export interface GameData {
  gameId: number;
  queueId: number;
  queue: string;
  createdMs: number;
  durationS: number;
  teams: GameTeam[];
  /** 这一局刚结束（结算、“这局总结”自动卡片打开时）。 */
  justEnded?: boolean;
}

export function isGameData(value: unknown): value is GameData {
  const v = value as Partial<GameData> | null;
  return !!v && Array.isArray(v.teams);
}

// ---------- 英雄（`lol.champion`，plugins/lol/src/stats_tools.rs 的 champion_tool） ----------

/** 出装、符文、召唤师技能的一项：图标路径是后加的（客户端资料读过才有，没有时空）。 */
export interface Named {
  id: number;
  name: string;
  icon?: string;
}
/** 英雄的一个技能：`P`（被动）、`Q`、`W`、`E`、`R`；图标是客户端资源路径，拿不到时空（页面显示字母）。 */
export interface Ability {
  key: string;
  name: string;
  icon: string;
}
export interface Matchup {
  champion: Champion;
  winRate: number | null;
}
export interface BuildOption {
  items: Named[];
  winRate: number | null;
  pickRate: number | null;
}
export interface RunePage {
  primary: Named[];
  secondary: Named[];
  shards: Named[];
  winRate: number | null;
  pickRate: number | null;
  games: number;
}
export interface TierRowRaw {
  rank: number | null;
  championId: number;
  tier: string;
  lane: string;
  winRate: number | null;
  pickRate: number | null;
  banRate: number | null;
}
export interface OpggBlock {
  text?: string;
  error?: string;
  attribution: string;
  url: string;
  stale?: boolean;
}
export interface ChampionRift {
  mode: "rift";
  champion: Champion;
  lane: string;
  laneName: string;
  patch: string;
  date: string;
  tier: TierRowRaw | null;
  goodAgainst: Matchup[];
  badAgainst: Matchup[];
  build: { starting: BuildOption[]; boots: BuildOption[]; core: BuildOption[]; fourth: BuildOption[]; fifth: BuildOption[]; sixth: BuildOption[] };
  runes: RunePage[];
  skillPriority: string[];
  skillOrder: string[];
  /** 被动和 Q W E R（按这个顺序；客户端没开、CommunityDragon 也没取到时空；旧的服务器没有这个字段）。 */
  abilities?: Ability[];
  spells: { spells: Named[]; winRate: number | null; pickRate: number | null }[];
  sources: Source[];
  stale: boolean;
  /** 来源换成 OP.GG 时它的结果（文字）；用腾讯 101 时 null。 */
  opgg: OpggBlock | null;
  /** 这一块的来源标签（ADR 043 D8；旧数据没有时按腾讯 101）。 */
  source?: SourceChoice;
}
export interface MayhemAugment {
  id: number;
  name: string;
  rarity: string;
  icon?: string;
  games: number;
  winRate: number;
  delta: number;
  lowSample: boolean;
}
export interface ChampionMayhem {
  mode: "mayhem";
  champion: Champion;
  rank: number | null;
  winRate: number | null;
  pickRate: number | null;
  partners: Champion[];
  augments: MayhemAugment[];
  /** aramhex-data 的署名（CC BY-NC 4.0）；没读到时 null。 */
  attribution: string | null;
  sources: Source[];
  /** 海克斯换成 OP.GG 时它的结果（文字）。 */
  augmentsOpgg?: OpggBlock | null;
  /** 海克斯那一块的来源标签（旧数据没有时按 aramhex-data）。 */
  source?: SourceChoice;
}
export type ChampionData = ChampionRift | ChampionMayhem;

export function isChampionData(value: unknown): value is ChampionData {
  const v = value as { mode?: unknown; champion?: unknown } | null;
  return !!v && (v.mode === "rift" || v.mode === "mayhem") && typeof v.champion === "object" && v.champion !== null;
}

/** aramhex-data 的仓库（署名链接）。 */
export const ARAMHEX_URL = "https://github.com/czgreat/aramhex-data";
/** OP.GG（我们答应过的标志和链接，opgg-mcp#26）。 */
export const OPGG_URL = "https://op.gg";

/** 数据源的 id（plugins/lol/src/stats_tools.rs 的 `FROM_*`；`client` 是英雄联盟客户端）。 */
export type SourceId = "client" | "qq101" | "opgg" | "aramhex";
/** 能换来源的一块（`KIND_*`）。 */
export type SourceKind = "rift" | "mayhemAugments";
/** 一块数据现在用哪个来源、能换成哪些（`available`：OP.GG 的开关关着时 false）。 */
export interface SourceChoice {
  kind: SourceKind;
  id: SourceId;
  options: { id: SourceId; available: boolean }[];
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
  const v = value as { mode?: unknown; lanes?: unknown; rows?: unknown } | null;
  return (v?.mode === "rift" && typeof v.lanes === "object" && v.lanes !== null) || (v?.mode === "mayhem" && Array.isArray(v.rows));
}
