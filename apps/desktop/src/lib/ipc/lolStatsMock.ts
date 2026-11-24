/**
 * dev:web 和 e2e 用的英雄联盟“网上的统计”示例数据（ADR 038 第 3 步）：英雄页（峡谷 / 海斗）、强度榜、覆盖层。
 * 形状照后端 plugins/lol/src/stats_tools.rs 和 overlays.rs。数字是编的（看上去合理就行），不是真实统计。
 */
import type {
  ChampionMayhem, ChampionRift, Named, StatChampion, TierMayhem, TierRift, TierRow,
} from "$plugins/lol/ui/types";
import { OVERLAY_AUGMENTS, OVERLAY_CHAMP_SELECT } from "$plugins/lol/ui/types";

// i18n-ignore-start：整个文件是浏览器预览的示例数据（真实应用里来自腾讯 101、aramhex-data），不是界面文字
const CHAMPS: StatChampion[] = [
  { id: 39, label: "刀锋舞者 艾瑞莉娅", short: "艾瑞莉娅" },
  { id: 157, label: "疾风剑豪 亚索", short: "亚索" },
  { id: 24, label: "武器大师 贾克斯", short: "贾克斯" },
  { id: 58, label: "荒漠屠夫 雷克顿", short: "雷克顿" },
  { id: 62, label: "齐天大圣 孙悟空", short: "孙悟空" },
  { id: 266, label: "暗裔剑魔 亚托克斯", short: "亚托克斯" },
  { id: 875, label: "腕豪 瑟提", short: "瑟提" },
  { id: 79, label: "酒桶 古拉加斯", short: "古拉加斯" },
  { id: 50, label: "策士统领 斯维因", short: "斯维因" },
  { id: 901, label: "炽炎雏龙 斯莫德", short: "斯莫德" },
  { id: 223, label: "河流之王 塔姆", short: "塔姆" },
  { id: 99, label: "光辉女郎 拉克丝", short: "拉克丝" },
  { id: 103, label: "九尾妖狐 阿狸", short: "阿狸" },
  { id: 222, label: "暴走萝莉 金克丝", short: "金克丝" },
  { id: 412, label: "魂锁典狱长 锤石", short: "锤石" },
  { id: 64, label: "盲僧 李青", short: "李青" },
];
const NICKNAMES: Record<string, number> = { 刀妹: 39, 女刀: 39, 快乐风男: 157, 武器: 24, 鳄鱼: 58, 猴子: 62, 剑魔: 266, 塔姆: 223, 瞎子: 64 };
const LANES = [
  { id: "TOP", name: "上路" }, { id: "JUNGLE", name: "打野" }, { id: "MIDDLE", name: "中路" },
  { id: "BOTTOM", name: "下路" }, { id: "SUPPORT", name: "辅助" },
];
const TENCENT = "腾讯 101（掌盟，国服）";
const DATE = "20261123";
const ITEMS: Named[] = [
  { id: 3153, name: "破败王者之刃" }, { id: 6672, name: "海妖杀手" }, { id: 6333, name: "死亡之舞" },
  { id: 3047, name: "铁板靴" }, { id: 3111, name: "水银之靴" }, { id: 1055, name: "多兰之刃" },
  { id: 2003, name: "生命药水" }, { id: 3071, name: "黑色切割者" }, { id: 3026, name: "守护天使" }, { id: 3074, name: "贪欲九头蛇" },
];
const RUNES: Named[] = [
  { id: 8010, name: "征服者" }, { id: 9111, name: "凯旋" }, { id: 9104, name: "传说：欢欣" }, { id: 8299, name: "坚毅不倒" },
  { id: 8444, name: "复苏之风" }, { id: 8453, name: "无畏战车" }, { id: 5005, name: "攻击速度" }, { id: 5008, name: "适应之力" }, { id: 5011, name: "生命值" },
];
const SPELLS: Named[] = [{ id: 4, name: "闪现" }, { id: 12, name: "传送" }, { id: 14, name: "引燃" }];
const AUGMENTS = [
  { id: 1162, name: "终极九头蛇", rarity: "prismatic" }, { id: 1288, name: "弹跳蜜蜂", rarity: "gold" },
  { id: 1124, name: "鼓声暴起 低音轰鸣", rarity: "prismatic" }, { id: 1067, name: "命运之武器", rarity: "gold" },
  { id: 1201, name: "将他们放倒倒倒", rarity: "silver" }, { id: 1001, name: "泰坦的坚决", rarity: "prismatic" },
];
// i18n-ignore-end

/** 玩家说的名字（短名、称号、外号）→ 英雄；找不到时 null。 */
/** `lol.champions`（只给页面）的示例：全部英雄的名字（这里二十个）。 */
export function sampleChampionList(): StatChampion[] {
  // i18n-ignore-start：示例数据
  const more: StatChampion[] = [
    { id: 238, label: "影流之主 劫", short: "劫" },
    { id: 30, label: "死亡颂唱者 卡尔萨斯", short: "卡尔萨斯" },
    { id: 63, label: "复仇焰魂 布兰德", short: "布兰德" },
    { id: 86, label: "德玛西亚之力 盖伦", short: "盖伦" },
  ];
  // i18n-ignore-end
  return [...CHAMPS, ...more].sort((a, b) => a.short.localeCompare(b.short, "zh-CN"));
}

export function findSampleChampion(query: string): StatChampion | null {
  const q = query.trim();
  if (!q) return null;
  const byNick = NICKNAMES[q];
  return CHAMPS.find((c) => c.id === byNick || c.short === q || c.label.includes(q)) ?? null;
}

const pick = (i: number) => CHAMPS[i % CHAMPS.length];

export function sampleChampionRift(champion: StatChampion, lane = "TOP"): ChampionRift {
  const laneName = LANES.find((l) => l.id === lane)?.name ?? lane;
  const option = (ids: number[], win: number, pickRate: number) => ({
    items: ids.map((i) => ITEMS[i]), winRate: win, pickRate,
  });
  return {
    mode: "rift", champion, lane, laneName, patch: "16.23", date: DATE,
    tier: { rank: 25, championId: champion.id, tier: "T1", lane, winRate: 0.5043, pickRate: 0.0322, banRate: 0.0425, counters: [24, 58, 62], rankChange: 0 },
    goodAgainst: [7, 8, 9].map((i, k) => ({ champion: pick(i), winRate: 0.66 - k * 0.025 })),
    badAgainst: [2, 3, 4].map((i, k) => ({ champion: pick(i), winRate: 0.4394 + k * 0.0065 })),
    build: {
      starting: [option([5, 6], 0.512, 0.81)],
      boots: [option([3], 0.515, 0.62), option([4], 0.498, 0.31)],
      core: [option([0, 1, 2], 0.551, 0.125), option([0, 7, 2], 0.538, 0.083)],
      fourth: [option([8], 0.57, 0.21)],
      fifth: [option([9], 0.58, 0.12)],
      sixth: [],
    },
    runes: [{ primary: RUNES.slice(0, 4), secondary: RUNES.slice(4, 6), shards: RUNES.slice(6, 9), winRate: 0.521, pickRate: 0.305, games: 12_840 }],
    skillPriority: ["Q", "E", "W"],
    skillOrder: ["Q", "E", "W", "Q", "Q", "R", "Q", "E", "Q", "E", "R"],
    spells: [{ spells: [SPELLS[0], SPELLS[1]], winRate: 0.512, pickRate: 0.7 }, { spells: [SPELLS[0], SPELLS[2]], winRate: 0.505, pickRate: 0.22 }],
    sources: [{ name: TENCENT, date: DATE }],
    stale: false,
    opgg: null,
  };
}

export function sampleChampionMayhem(champion: StatChampion): ChampionMayhem {
  return {
    mode: "mayhem", champion, rank: 31, winRate: 0.5212, pickRate: 0.0123,
    partners: [pick(10), pick(11), pick(12)],
    augments: AUGMENTS.map((a, k) => ({
      ...a, games: [10_891, 6_280, 32_573, 15_156, 19_177, 260][k], winRate: [0.578, 0.579, 0.561, 0.555, 0.553, 0.62][k],
      delta: [7.9, 7.9, 6.5, 5.7, 5.6, 2.1][k], lowSample: k === 5,
    })),
    attribution: "海斗数据：github.com/czgreat/aramhex-data（CC BY-NC 4.0，版本 16.23，截至 2026-11-20T08:00:00+08:00，4991976 局）", // i18n-ignore：示例数据（真实的由后端写）
    sources: [{ name: TENCENT, date: DATE }, { name: "github.com/czgreat/aramhex-data", license: "CC BY-NC 4.0", date: "2026-11-20T08:00:00+08:00" }],
  };
}

function tierRows(lane: string): TierRow[] {
  const offset = LANES.findIndex((l) => l.id === lane) * 3;
  return Array.from({ length: 12 }, (_, k) => ({
    champion: pick(offset + k), tier: k < 2 ? "T0" : k < 6 ? "T1" : "T2", rank: k + 1,
    winRate: 0.535 - k * 0.003, pickRate: 0.08 - k * 0.004, banRate: 0.06 - k * 0.003,
    counters: [pick(offset + k + 3), pick(offset + k + 4), pick(offset + k + 5)],
  }));
}

export function sampleTierRift(lane?: string): TierRift {
  const lanes = lane ? LANES.filter((l) => l.id === lane) : LANES;
  return {
    mode: "rift", date: DATE,
    lanes: Object.fromEntries(lanes.map((l) => [l.id, tierRows(l.id)])),
    laneNames: lanes, sources: [{ name: TENCENT, date: DATE }],
  };
}

export function sampleTierMayhem(): TierMayhem {
  return {
    mode: "mayhem", date: DATE,
    rows: Array.from({ length: 15 }, (_, k) => ({
      champion: pick(10 + k), rank: k + 1, rankChange: [0, 2, -1][k % 3], winRate: 0.5384 - k * 0.002, pickRate: 0.0156 - k * 0.0004,
    })),
    sources: [{ name: TENCENT, date: DATE }],
  };
}

/** 覆盖层的示例（`#/plugin-overlay?demo=augments|champ-select`）：rect 是覆盖层窗口里的比例（预览里窗口是整个屏幕）。 */
export function sampleOverlay(kind: string) {
  // i18n-ignore-start：示例数据
  if (kind === "champ-select") {
    return {
      items: [{
        plugin: "lol",
        rect: [0.79, 0.01, 0.2, 0.32],
        item: {
          id: "champ-select", uri: OVERLAY_CHAMP_SELECT, target: { kind: "process", exe: "LeagueClientUx.exe" },
          anchor: { kind: "corner", corner: "top-right", margin: 0.01, w: 0.2, h: 0.32 },
          data: {
            mode: "mayhem", source: TENCENT,
            team: [
              { champion: "艾瑞莉娅", me: true, winRate: 0.5212, rank: 31 },
              { champion: "塔姆", me: false, winRate: 0.5384, rank: 1 },
              { champion: "拉克丝", me: false, winRate: 0.5101, rank: 48 },
              { champion: "锤石", me: false, winRate: 0.4987, rank: 92 },
              { champion: "阿狸", me: false, winRate: 0.5055, rank: 60 },
            ],
            bench: [{ champion: "斯莫德", winRate: 0.5302, rank: 6 }, { champion: "斯维因", winRate: 0.5266, rank: 12 }],
          },
        },
      }],
    };
  }
  const card = (k: number, augment: string, score: { games: number; winRate: number; smoothed: number; delta: number; lowSample: boolean } | null, best: boolean) => ({
    plugin: "lol",
    rect: [0.18 + k * 0.25, 0.27, 0.14, 0.055],
    item: {
      id: `augment-${k}`, uri: OVERLAY_AUGMENTS, target: { kind: "process", exe: "League of Legends.exe" },
      anchor: { kind: "rect", x: 0.18 + k * 0.25, y: 0.27, w: 0.14, h: 0.055 },
      data: { augment, champion: "艾瑞莉娅", score, best, attribution: "aramhex-data" },
    },
  });
  return {
    items: [
      card(0, "鼓声暴起 低音轰鸣", { games: 32_573, winRate: 0.561, smoothed: 0.5609, delta: 6.5, lowSample: false }, false),
      card(1, "终极九头蛇", { games: 10_891, winRate: 0.578, smoothed: 0.5779, delta: 7.9, lowSample: false }, true),
      card(2, "泰坦的坚决", { games: 260, winRate: 0.62, smoothed: 0.552, delta: 2.1, lowSample: true }, false),
    ],
  };
  // i18n-ignore-end
}
