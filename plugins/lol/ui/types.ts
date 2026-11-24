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
