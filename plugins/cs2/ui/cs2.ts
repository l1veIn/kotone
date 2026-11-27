// CS2 插件（ADR 036 D9、ADR 047 第 3 期）的界面数据：后端 plugins/cs2/src/lib.rs、engine.rs。
// 页面调 `cs2.status`（收数据和配置文件的情况 + 这一局）、`cs2.cfg`（写不写游戏的配置文件），都只给页面用；
// 她用 `cs2.state`（结果的 structuredContent 就是这里的 MatchView，用 `ui://kotone/cs2/match` 显示）。
import { pluginCall } from "$lib/plugins";
import { getLocale } from "$lib/paraglide/runtime.js";

export const PLUGIN_ID = "cs2";
export const PAGE_CS2_HOME = "ui://kotone/cs2/home";
export const PAGE_CS2_MATCH = "ui://kotone/cs2/match";
export const PAGE_CS2_BOMB = "ui://kotone/cs2/bomb";

export type Side = "CT" | "T";

export interface RoundView {
  round: number;
  winner: Side;
  won?: boolean;
  /** elimination / bomb / defuse / time；不知道时为空 */
  reason: string;
  ctScore: number;
  tScore: number;
}

export interface BombView {
  round: number;
  /** planted / defused / exploded / over（回合结束了还没炸） */
  state: string;
  plantedAtMs: number;
  explodesAtMs: number;
  endedAtMs?: number;
  map: string;
  mapName: string;
}

/** 一张图（`cs2.state` 的 structuredContent、比赛结束的卡片）。没在打时 `playing: false`，可能带上一张打完的。 */
export interface MatchView {
  playing: boolean;
  over?: boolean;
  connected?: boolean;
  map?: string;
  mapName?: string;
  mode?: string;
  modeName?: string;
  /** 游戏的 map.phase：warmup / live / intermission / gameover；菜单里是 menu */
  phase?: string;
  round?: number;
  ctScore?: number;
  tScore?: number;
  myTeam?: Side;
  myScore?: number;
  enemyScore?: number;
  /** win / loss / tie；没结束或不知道你在哪边时为空 */
  result?: string;
  stats?: { kills: number; deaths: number; assists: number; mvps: number; score: number };
  me?: { health: number; armor: number; helmet: boolean; money: number; roundKills: number; equipValue?: number };
  bomb?: BombView;
  rounds?: RoundView[];
  updatedAtMs?: number;
  lastMatch?: MatchView | null;
}

/** `cs2.status`。 */
export interface Cs2Status {
  enabled: boolean;
  listening: number | null;
  bindError: string;
  connected: boolean;
  lastPostMs: number;
  rejected: number;
  gameRunning: boolean;
  cfg: {
    writeCfg: boolean;
    /** dir / not_installed / isolated */
    found: string;
    path: string;
    /** written / absent / wait_write / wait_remove / foreign；找不到目录时 null */
    state: string | null;
    error: string;
    port: number;
  };
  match: MatchView;
}

export async function cs2Status(): Promise<Cs2Status | null> {
  const result = await pluginCall(PLUGIN_ID, "cs2.status");
  const content = result.content as (Cs2Status & { status?: string; text?: string }) | null;
  if (content?.status === "disabled") return null;
  if (!content?.cfg) throw new Error(content?.text ?? "cs2.status");
  return content;
}

export async function setWriteCfg(write: boolean): Promise<Cs2Status> {
  const result = await pluginCall(PLUGIN_ID, "cs2.cfg", { write });
  const content = result.content as (Cs2Status & { status?: string; text?: string }) | null;
  if (content?.status !== "ok") throw new Error(content?.text ?? "cs2.cfg");
  return content;
}

/** 地图名：中文界面用国服的叫法（后端给的 mapName），别的语言用游戏里的名字去掉前缀（de_mirage → Mirage）。 */
export function mapLabel(map?: string, mapName?: string): string {
  if (!map) return "";
  if (getLocale() === "zh-CN" && mapName) return mapName;
  const bare = map.replace(/^(de|cs|ar|dm)_/, "").replace(/^workshop\/\d+\//, "");
  return bare ? bare[0].toUpperCase() + bare.slice(1) : map;
}

/** 模式名：中文界面用后端给的中文名，别的语言用游戏里的名字。 */
export function modeLabel(mode?: string, modeName?: string): string {
  if (!mode) return "";
  return getLocale() === "zh-CN" && modeName ? modeName : mode;
}

/** 比分两边：知道你在哪边时“我们 / 对面”，不知道时“CT / T”。 */
export function scoreSides(v: MatchView): { left: number; right: number; mine: boolean } {
  if (v.myTeam && v.myScore != null && v.enemyScore != null) return { left: v.myScore, right: v.enemyScore, mine: true };
  return { left: v.ctScore ?? 0, right: v.tScore ?? 0, mine: false };
}

/** 炸弹还剩几秒（下了还没结束时；不到 0 就是 0）。 */
export function bombLeft(bomb: BombView, now: number): number {
  return Math.max(0, Math.ceil((bomb.explodesAtMs - now) / 1000));
}
