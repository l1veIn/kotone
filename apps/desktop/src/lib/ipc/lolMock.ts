/**
 * dev:web 和 e2e 用的英雄联盟示例数据（ADR 038 第 2 步）：这局、某人的战绩、一局的详情，形状照后端
 * plugins/lol/src/pages.rs。玩家名都是编的（玩家甲#1001……），不是真人。
 */
import type { Champion, GameData, MatchData, MatchPlayer, PlayerData, PlayerGame, Summary } from "$plugins/lol/ui/types";

// i18n-ignore-start：整个文件是浏览器预览的示例数据（真实应用里来自英雄联盟客户端），不是界面文字
const CHAMPIONS: Champion[] = [
  { id: 39, label: "刀锋舞者 艾瑞莉娅", short: "艾瑞莉娅", alias: "Irelia" },
  { id: 99, label: "光辉女郎 拉克丝", short: "拉克丝", alias: "Lux" },
  { id: 86, label: "德玛西亚之力 盖伦", short: "盖伦", alias: "Garen" },
  { id: 222, label: "暴走萝莉 金克丝", short: "金克丝", alias: "Jinx" },
  { id: 412, label: "魂锁典狱长 锤石", short: "锤石", alias: "Thresh" },
  { id: 157, label: "疾风剑豪 亚索", short: "亚索", alias: "Yasuo" },
  { id: 103, label: "九尾妖狐 阿狸", short: "阿狸", alias: "Ahri" },
  { id: 22, label: "寒冰射手 艾希", short: "艾希", alias: "Ashe" },
  { id: 238, label: "影流之主 劫", short: "劫", alias: "Zed" },
  { id: 30, label: "死亡颂唱者 卡尔萨斯", short: "卡尔萨斯", alias: "Karthus" },
];
const NAMES = ["玩家甲", "玩家乙", "玩家丙", "玩家丁", "玩家戊", "对手子", "对手丑", "对手寅", "对手卯", "对手辰"];
const RANKS = ["黄金 2", "铂金 4", null, "白银 1", "翡翠 3", "钻石 4", "黄金 1", "白银 3", null, "铂金 2"];
const QUEUE = "海克斯大乱斗";
const AUGMENTS = [{ id: 1001, name: "泰坦的坚决" }, { id: 1004, name: "回归基本功" }, { id: 1006, name: "利刃华尔兹" }];
const ITEMS = [{ id: 3031, name: "无尽之刃" }, { id: 6672, name: "海妖杀手" }, { id: 3036, name: "多米尼克领主的致意" }, { id: 3006, name: "狂战士胫甲" }];

export const LOL_SAMPLE_T0 = 1_795_000_000_000;

function puuid(i: number): string {
  return `sample-puuid-${i}`;
}

/** 第 i 个人最近的输赢（对面 5 号连败、7 号隐藏战绩）。 */
function wins(i: number): boolean[] {
  // 对面 5 号：先 6 连败，20 局只赢 6 局
  if (i === 5) return Array.from({ length: 20 }, (_, g) => g >= 6 && g % 2 === 0 && g < 18);
  return Array.from({ length: 20 }, (_, g) => (g + i) % 3 !== 0);
}

function summary(i: number): Summary {
  const w = wins(i);
  const first = w[0];
  const streakLength = w.findIndex((x) => x !== first);
  const n = streakLength < 0 ? w.length : streakLength;
  const won = w.filter(Boolean).length;
  return {
    games: w.length, wins: won, losses: w.length - won, streak: first ? n : -n,
    kda: Math.round((2 + ((i * 7) % 30) / 10) * 10) / 10, deaths: Math.round((5 + ((i * 13) % 80) / 10) * 10) / 10,
    modeGames: 12, modeWins: Math.round(won * 0.6), modeDpm: 1200 + i * 90, champGames: 0, champWins: 0,
  };
}

function games(i: number): PlayerGame[] {
  return wins(i).map((win, g) => {
    const champion = CHAMPIONS[(i + g) % CHAMPIONS.length];
    const durationS = 900 + ((i + g) % 7) * 60;
    const damage = 22_000 + ((i * 3 + g * 5) % 20) * 1500;
    return {
      gameId: 9_000_000 + i * 100 + g, queueId: 2400, queue: QUEUE, createdMs: LOL_SAMPLE_T0 - (g + 1) * 3_600_000, durationS,
      champion, win, kills: 3 + ((i + g) % 9), deaths: 2 + ((i * 2 + g) % 8), assists: 8 + ((i + g * 3) % 15),
      damage, dpm: Math.round(damage / (durationS / 60)), gold: 11_000 + g * 120, cs: 30 + g, augments: AUGMENTS.slice(0, 1 + (g % 3)),
    };
  });
}

function player(i: number): MatchPlayer {
  const s = summary(i);
  const hidden = i === 7;
  const weak = i === 5 ? ["最近 20 局只赢了 6 局", "6 连败", "场均死 13.2 次"] : null;
  const tags = s.streak >= 3 ? [`${s.streak} 连胜`] : s.streak <= -3 ? [`${-s.streak} 连败`] : [];
  return {
    puuid: puuid(i), champion: CHAMPIONS[i], name: NAMES[i], riotId: `${NAMES[i]}#${1001 + i}`,
    me: i === 0, ally: i < 5, party: i === 1 || i === 2 ? 2 : i === 6 || i === 8 ? 7 : null,
    pending: false, hidden, rank: RANKS[i], error: null, summary: s, tags,
    recent: games(i).slice(0, 10).map((g) => ({ gameId: g.gameId, win: g.win, championId: g.champion.id, kills: g.kills, deaths: g.deaths, assists: g.assists })),
    weak,
  };
}

/** 这局（`lol.scout` 的页面数据）。 */
export function sampleMatch(): MatchData {
  return {
    gameId: 9_100_000, queueId: 2400, mode: QUEUE, random: true, waiting: 0,
    allies: [0, 1, 2, 3, 4].map(player), enemies: [5, 6, 7, 8, 9].map(player),
  };
}

/** 某人的战绩（`lol.player`）：按 puuid，找不到就是自己。 */
export function samplePlayer(id?: string): PlayerData {
  const index = Math.max(0, Array.from({ length: 10 }, (_, i) => puuid(i)).indexOf(id ?? ""));
  const list = games(index);
  const top = new Map<number, { champion: Champion; games: number; wins: number }>();
  for (const g of list) {
    const row = top.get(g.champion.id) ?? { champion: g.champion, games: 0, wins: 0 };
    row.games += 1;
    row.wins += g.win ? 1 : 0;
    top.set(g.champion.id, row);
  }
  return {
    puuid: puuid(index), riotId: `${NAMES[index]}#${1001 + index}`, hidden: index === 7, rank: RANKS[index], error: null,
    summary: summary(index), champions: [...top.values()].sort((a, b) => b.games - a.games).slice(0, 5), games: list,
  };
}

/** 按 名字#编号 或这局里的名字 / 英雄找人；找不到为 null。 */
export function findSamplePlayer(query: string): string | null {
  const q = query.trim();
  for (let i = 0; i < 10; i++) {
    if (`${NAMES[i]}#${1001 + i}` === q || NAMES[i] === q || CHAMPIONS[i].short === q) return puuid(i);
  }
  return null;
}

/** 一局的详情（`lol.game`）。 */
export function sampleGame(gameId: number): GameData {
  const durationS = 1080;
  const row = (i: number) => {
    const damage = 18_000 + i * 2_300;
    return {
      puuid: puuid(i), riotId: `${NAMES[i]}#${1001 + i}`, champion: CHAMPIONS[i], level: 14 + (i % 4),
      kills: 2 + i, deaths: 3 + (i % 5), assists: 10 + (i % 7), damage, dpm: Math.round(damage / 18), taken: 20_000 + i * 900,
      gold: 10_500 + i * 300, cs: 40 + i * 3, items: ITEMS.slice(0, 2 + (i % 3)), augments: AUGMENTS.slice(0, 1 + (i % 3)),
    };
  };
  return {
    gameId, queueId: 2400, queue: QUEUE, createdMs: LOL_SAMPLE_T0 - 3_600_000, durationS,
    teams: [
      { teamId: 100, win: true, players: [0, 1, 2, 3, 4].map(row) },
      { teamId: 200, win: false, players: [5, 6, 7, 8, 9].map(row) },
    ],
  };
}
// i18n-ignore-end
