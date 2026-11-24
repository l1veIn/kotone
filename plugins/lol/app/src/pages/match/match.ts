// “这局”页面算出来的东西：开黑字母、队伍表头的平均数、阶段。只拼数，不写字（字在 messages/）。
import { pctOf, rateTone } from "../../format";
import type { MatchData, MatchPhase, MatchPlayer } from "../../types";

export interface PartyInfo {
  letter: string;
  color: string;
  size: number;
}

const COLORS = ["a", "b", "c", "d"];

/**
 * 开黑：同一队里 `party` 一样、2 人以上的算一组；整局按出现顺序（我方先）给 A、B、C、D。
 * `party` 是客户端的 `teamParticipantId`，只在一队里有意义，所以按“哪一队 + 编号”分组。
 */
export function partyOf(match: MatchData): (p: MatchPlayer) => PartyInfo | null {
  const all = [...match.allies, ...match.enemies];
  const key = (p: MatchPlayer) => (p.party === null || p.party === undefined ? null : `${p.ally ? "a" : "e"}:${p.party}`);
  const sizes = new Map<string, number>();
  for (const p of all) {
    const k = key(p);
    if (k) sizes.set(k, (sizes.get(k) ?? 0) + 1);
  }
  const groups = [...new Set(all.map(key).filter((k): k is string => !!k && (sizes.get(k) ?? 0) > 1))];
  return (p) => {
    const k = key(p);
    const i = k ? groups.indexOf(k) : -1;
    if (i < 0 || !k) return null;
    return {
      letter: String.fromCharCode(65 + i),
      color: `var(--lol-party-${COLORS[i % COLORS.length]})`,
      size: sizes.get(k) ?? 0,
    };
  };
}

/** 一队的开黑组（字母和人数），按字母排。 */
export function partiesIn(team: MatchPlayer[], party: (p: MatchPlayer) => PartyInfo | null): PartyInfo[] {
  const seen = new Map<string, PartyInfo>();
  for (const p of team) {
    const info = party(p);
    if (info) seen.set(info.letter, info);
  }
  return [...seen.values()].sort((a, b) => a.letter.localeCompare(b.letter));
}

export interface TeamStats {
  /** 查到战绩的人数；0 时表头不写平均数。 */
  known: number;
  rate: number;
  rateTone: "good" | "bad" | "";
  kda: number;
}

/** 队伍表头：有战绩的人合起来的胜率、KDA 的平均。 */
export function teamStats(team: MatchPlayer[]): TeamStats {
  const s = team.map((p) => p.summary).filter((x): x is NonNullable<typeof x> => !!x && x.games > 0);
  const games = s.reduce((a, x) => a + x.games, 0);
  const wins = s.reduce((a, x) => a + x.wins, 0);
  const kda = s.length ? s.reduce((a, x) => a + x.kda, 0) / s.length : 0;
  return { known: s.length, rate: pctOf(wins, games), rateTone: rateTone(wins, games), kda };
}

/** 这局的阶段；旧数据没有 `phase` 时是 null（显示成“在对局里”）。 */
export function phaseOf(match: MatchData): MatchPhase | null {
  const p = match.phase;
  return p === "champ_select" || p === "loading" || p === "in_game" || p === "post_match" ? p : null;
}

/** 选人时这个人锁了没有（别的阶段都算锁了）。 */
export function pickOf(p: MatchPlayer): "locked" | "intent" | "none" {
  if (p.pick === "intent" || p.pick === "none") return p.pick;
  return p.champion.id > 0 ? "locked" : "none";
}
