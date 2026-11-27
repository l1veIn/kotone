/**
 * dev:web 和 e2e 用的英雄联盟示例数据（ADR 038 第 2 步）：这局、某人的战绩、一局的详情，形状照后端
 * plugins/lol/src/pages.rs。玩家名都是编的（玩家甲#1001……），不是真人。
 */
import type { Champion, GameData, HistoryPage, MatchData, MatchPlayer, PlayerData, PlayerGame, Summary } from "$plugins/lol/ui/types";

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
const HISTORY_LENGTH = 47;
const AUGMENTS = [{ id: 1001, name: "泰坦的坚决" }, { id: 1004, name: "回归基本功" }, { id: 1006, name: "利刃华尔兹" }];
const ITEMS = [{ id: 3031, name: "无尽之刃" }, { id: 6672, name: "海妖杀手" }, { id: 3036, name: "多米尼克领主的致意" }, { id: 3006, name: "狂战士胫甲" }];

export const LOL_SAMPLE_T0 = 1_795_000_000_000;

function puuid(i: number): string {
  return `sample-puuid-${i}`;
}

/** 第 i 个人最近的输赢（对面 5 号连败、7 号隐藏战绩；我方 3、4 号隐藏了身份）。 */
function wins(i: number, count = 20): boolean[] {
  // 对面 5 号：先 6 连败，20 局只赢 6 局
  if (i === 5) return Array.from({ length: count }, (_, g) => g >= 6 && g % 2 === 0 && g < 18);
  return Array.from({ length: count }, (_, g) => (g + i) % 3 !== 0);
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

function games(i: number, count = 20): PlayerGame[] {
  return wins(i, count).map((win, g) => {
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

/** 浏览器预览的账号有 47 局，可以翻过概况的 20 局并看到不足一页的末页。 */
export function sampleHistory(id: string, offset: number, pageSize: 10 | 20): HistoryPage {
  const index = NAMES.findIndex((_, i) => puuid(i) === id);
  const history = index < 0 ? [] : games(index, HISTORY_LENGTH);
  const rows = history.slice(offset, offset + pageSize);
  const hasMore = offset + rows.length < history.length;
  return { puuid: id, offset, pageSize, games: rows, hasMore, nextOffset: hasMore ? offset + rows.length : null };
}

/** 我方 3、4 号隐藏了身份（照 2026-11-26 那一把海斗：客户端不给名字和 puuid，只有英雄；ADR 043 D9）。 */
const HIDDEN_IDENTITY = [3, 4];

function player(i: number): MatchPlayer {
  if (HIDDEN_IDENTITY.includes(i)) {
    return {
      puuid: "", champion: CHAMPIONS[i], name: "", riotId: "", me: false, ally: i < 5, party: null,
      pending: false, hidden: false, rank: null, error: null, summary: null, tags: [], recent: [], identity: "hidden",
    };
  }
  const s = summary(i);
  const hidden = i === 7;
  const tags = s.streak >= 3 ? [`${s.streak} 连胜`] : s.streak <= -3 ? [`${-s.streak} 连败`] : [];
  return {
    puuid: puuid(i), champion: CHAMPIONS[i], name: NAMES[i], riotId: `${NAMES[i]}#${1001 + i}`,
    me: i === 0, ally: i < 5, party: i === 1 || i === 2 ? 2 : i === 6 || i === 8 ? 7 : null,
    pending: false, hidden, rank: RANKS[i], error: null, summary: s, tags,
    recent: games(i).slice(0, 10).map((g) => ({ gameId: g.gameId, win: g.win, championId: g.champion.id, kills: g.kills, deaths: g.deaths, assists: g.assists })),
    identity: "known",
  };
}

/** 这局的阶段（预览和测试里用 `window.__kotoneLolPhase` 切：champ_select / loading / in_game / post_match）。 */
export type SamplePhase = "champ_select" | "loading" | "in_game" | "post_match";

/** 这局（`lol.scout` 的页面数据）。 */
export function sampleMatch(phase: SamplePhase = "loading"): MatchData & Record<string, unknown> {
  return {
    gameId: 9_100_000, queueId: 2400, mode: QUEUE, random: true, waiting: 0,
    allies: [0, 1, 2, 3, 4].map(player), enemies: [5, 6, 7, 8, 9].map(player),
    phase, phaseSince: Date.now() - 12_000, namesHidden: false, bench: [],
    result: phase === "post_match" ? { win: true, durationS: 1112, gameId: 9_000_000 } : null, resultPending: false,
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
    ...(index === 0 ? {
      profileIconId: 5463, summonerLevel: 186,
      ranked: {
        solo: { queue: "RANKED_SOLO_5x5", tier: "GOLD", division: "II", lp: 64, wins: 43, losses: 35,
          highestTier: "GOLD", label: "黄金 II", crest: "" },
        flex: null,
      },
    } : {}),
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
  const historyIndex = gameId - 9_000_000;
  const owner = Math.floor(historyIndex / 100);
  const gameIndex = historyIndex % 100;
  const record = owner >= 0 && owner < NAMES.length ? games(owner, HISTORY_LENGTH)[gameIndex] : undefined;
  const durationS = record?.durationS ?? 1080;
  const blueWon = record ? (owner < 5 ? record.win : !record.win) : true;
  const row = (i: number) => {
    const played = i === owner ? record : undefined;
    const damage = played?.damage ?? 18_000 + i * 2_300;
    return {
      puuid: puuid(i), riotId: `${NAMES[i]}#${1001 + i}`, champion: CHAMPIONS[(i + (record ? gameIndex : 0)) % CHAMPIONS.length], level: 14 + (i % 4),
      kills: played?.kills ?? 2 + i, deaths: played?.deaths ?? 3 + (i % 5), assists: played?.assists ?? 10 + (i % 7),
      damage, dpm: Math.round(damage * 60 / durationS), taken: 20_000 + i * 900,
      gold: played?.gold ?? 10_500 + i * 300, cs: played?.cs ?? 40 + i * 3, items: ITEMS.slice(0, 2 + (i % 3)), augments: played?.augments ?? AUGMENTS.slice(0, 1 + (i % 3)),
    };
  };
  return {
    gameId, queueId: 2400, queue: QUEUE, createdMs: record?.createdMs ?? LOL_SAMPLE_T0 - 3_600_000, durationS,
    teams: [
      { teamId: 100, win: blueWon, players: [0, 1, 2, 3, 4].map(row) },
      { teamId: 200, win: !blueWon, players: [5, 6, 7, 8, 9].map(row) },
    ],
  };
}
/** 今天的状态（推送 `today` 的数据，页面 `ui://kotone/lol/today`，后端 plugins/lol/src/watcher/auto_cards/today.rs）：
 * 今天 5 局 2 胜 3 负，现在 2 连败；亚索今天 3 局 1 胜 2 负。`games` = 6 是又打了一局（赢了）以后（原地更新的测试）。 */
export function sampleToday(games = 5, date = "2026-11-26") {
  // 新的在前；5 局时没有最前面那一局（拉克丝，赢）
  const all: [number, boolean][] = [[99, true], [157, false], [103, false], [157, true], [22, true], [157, false]];
  const list = all.slice(all.length - Math.min(games, all.length));
  const champion = (id: number) => CHAMPIONS.find((c) => c.id === id)!;
  const wins = list.filter(([, w]) => w).length;
  const first = list[0]?.[1];
  const run = list.findIndex(([, w]) => w !== first);
  const n = run < 0 ? list.length : run;
  const yasuo = list.filter(([id]) => id === 157);
  const yasuoWins = yasuo.filter(([, w]) => w).length;
  return {
    date, games: list.length, wins, losses: list.length - wins, streak: list.length ? (first ? n : -n) : 0, capped: false, limit: 20,
    lostAtLeast: 2,
    lostOften: yasuo.length - yasuoWins >= 2 ? [{ champion: champion(157), games: yasuo.length, wins: yasuoWins, losses: yasuo.length - yasuoWins }] : [],
    modes: [{ queueId: 2400, queue: QUEUE, games: list.length - 1, wins: Math.max(0, wins - 1) }, { queueId: 420, queue: "单双排", games: 1, wins: 1 }],
    recent: list.map(([id, win], i) => ({ gameId: 9_200_000 - i, win, championId: id, kills: win ? 9 : 2, deaths: win ? 3 : 8, assists: 11 })),
  };
}

/** 海斗的“这局总结”（推送 `summary` 的数据）：我是我方第一个（`me`、`ally`），每个人 4 个海克斯（名字照客户端的写法，
 * 有长有短）。 */
export function sampleSummary(gameId = 9_100_000) {
  const game = sampleGame(gameId);
  const pool = [
    { id: 1018, name: "巨像的勇气", rarity: "kPrismatic", icon: "" }, { id: 1353, name: "坦克引擎", rarity: "kGold", icon: "" },
    { id: 1136, name: "扇巴掌", rarity: "kSilver", icon: "" }, { id: 3078, name: "鼓声暴起 低音轰鸣", rarity: "kGold", icon: "" },
    { id: 1048, name: "珠光护手", rarity: "kPrismatic", icon: "" }, { id: 2100, name: "急速之追求", rarity: "kGold", icon: "" },
    { id: 1112, name: "终极不可阻挡", rarity: "kSilver", icon: "" }, { id: 3086, name: "小蜜蜂！！！", rarity: "kGold", icon: "" },
  ];
  return {
    ...game, justEnded: true,
    teams: game.teams.map((t, ti) => ({
      ...t, ally: ti === 0,
      players: t.players.map((p, i) => ({ ...p, me: ti === 0 && i === 0, augments: [0, 1, 2, 3].map((k) => pool[(i + ti * 3 + k * 2) % pool.length]) })),
    })),
  };
}
// i18n-ignore-end
