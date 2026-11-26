/**
 * dev:web 和 e2e 用的英雄联盟“网上的统计”示例数据（ADR 038 第 3 步）：英雄页（峡谷 / 海斗）、强度榜、选英雄卡片。
 * 形状照后端 plugins/lol/src/stats_tools.rs 和 pushes/champ_select.rs。数字是编的（看上去合理就行），不是真实统计。
 */
import type {
  ChampionMayhem, ChampionRift, Named, SourceChoice, StatChampion, TierMayhem, TierRift, TierRow,
} from "$plugins/lol/ui/types";

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
/** 英雄技能（被动、Q、W、E、R）：客户端的图标路径（照 CommunityDragon 上的 champions/{id}.json）。只写了艾瑞莉娅和亚索，别的英雄空着（页面显示字母格子）。 */
type Ability = { key: string; name: string; icon: string };
const ability = (key: string, name: string, file: string): Ability =>
  ({ key, name, icon: `/lol-game-data/assets/ASSETS/Characters/${file.split("_")[0]}/HUD/Icons2D/${file}.png` });
const ABILITIES: Record<number, Ability[]> = {
  39: [ability("P", "艾欧尼亚热诚", "Irelia_Passive"), ability("Q", "利刃冲击", "Irelia_Q"), ability("W", "距破之舞", "Irelia_W"),
    ability("E", "比翼双刃", "Irelia_E"), ability("R", "先锋之刃", "Irelia_R")],
  157: [ability("P", "浪客之道", "Yasuo_Passive"), ability("Q", "斩钢闪", "Yasuo_Q1"), ability("W", "风之障壁", "Yasuo_W"),
    ability("E", "踏前斩", "Yasuo_E"), ability("R", "狂风绝息斩", "Yasuo_R")],
};
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

/** 来源标签（ADR 043 D8）：这一块用的、能换成的（OP.GG 的开关关着时 OP.GG 用不了）。 */
function sourceChoice(kind: "rift" | "mayhemAugments", id: "qq101" | "opgg" | "aramhex", opggOn: boolean): SourceChoice {
  const first = kind === "rift" ? "qq101" : "aramhex";
  return { kind, id, options: [{ id: first, available: true }, { id: "opgg", available: opggOn }] };
}

/** OP.GG 的回复（原样的一段文字；示例是编的）。 */
function opggBlock(champion: StatChampion, what: string) {
  // i18n-ignore-start：示例数据（真实的是 OP.GG 原样的回复）
  return {
    text: `class ${champion.label}${what} {
  win_rate: 0.512, pick_rate: 0.041
  core_items: [破败王者之刃, 海妖杀手, 死亡之舞]
}`,
    attribution: "OP.GG（全球数据，op.gg）", url: "https://op.gg", stale: false,
  };
  // i18n-ignore-end
}

export function sampleChampionRift(champion: StatChampion, lane = "TOP", source: "qq101" | "opgg" = "qq101", opggOn = true): ChampionRift & { abilities: Ability[] } {
  const laneName = LANES.find((l) => l.id === lane)?.name ?? lane;
  if (source === "opgg") {
    return {
      mode: "rift", champion, lane, laneName, patch: "", date: "", tier: null, goodAgainst: [], badAgainst: [],
      build: { starting: [], boots: [], core: [], fourth: [], fifth: [], sixth: [] },
      runes: [], skillPriority: [], skillOrder: [], abilities: ABILITIES[champion.id] ?? [], spells: [],
      sources: [{ name: "OP.GG（全球数据，op.gg）" }], stale: false, // i18n-ignore：示例数据
      opgg: opggBlock(champion, ""), source: sourceChoice("rift", "opgg", opggOn),
    };
  }
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
    abilities: ABILITIES[champion.id] ?? [],
    spells: [{ spells: [SPELLS[0], SPELLS[1]], winRate: 0.512, pickRate: 0.7 }, { spells: [SPELLS[0], SPELLS[2]], winRate: 0.505, pickRate: 0.22 }],
    sources: [{ name: TENCENT, date: DATE }],
    stale: false,
    opgg: null,
    source: sourceChoice("rift", "qq101", opggOn),
  };
}

export function sampleChampionMayhem(champion: StatChampion, source: "aramhex" | "opgg" = "aramhex", opggOn = true): ChampionMayhem {
  if (source === "opgg") {
    return {
      mode: "mayhem", champion, rank: 31, winRate: 0.5212, pickRate: 0.0123, partners: [pick(10), pick(11), pick(12)],
      augments: [], attribution: null, sources: [{ name: TENCENT, date: DATE }, { name: "OP.GG（全球数据，op.gg）" }], // i18n-ignore：示例数据
      augmentsOpgg: opggBlock(champion, " · ARAM augments"), source: sourceChoice("mayhemAugments", "opgg", opggOn),
    };
  }
  return {
    mode: "mayhem", champion, rank: 31, winRate: 0.5212, pickRate: 0.0123,
    partners: [pick(10), pick(11), pick(12)],
    augments: AUGMENTS.map((a, k) => ({
      ...a, games: [10_891, 6_280, 32_573, 15_156, 19_177, 260][k], winRate: [0.578, 0.579, 0.561, 0.555, 0.553, 0.62][k],
      delta: [7.9, 7.9, 6.5, 5.7, 5.6, 2.1][k], lowSample: k === 5,
    })),
    attribution: "海斗数据：github.com/czgreat/aramhex-data（CC BY-NC 4.0，版本 16.23，截至 2026-11-20T08:00:00+08:00，4991976 局）", // i18n-ignore：示例数据（真实的由后端写）
    sources: [{ name: TENCENT, date: DATE }, { name: "github.com/czgreat/aramhex-data", license: "CC BY-NC 4.0", date: "2026-11-20T08:00:00+08:00" }],
    source: sourceChoice("mayhemAugments", "aramhex", opggOn),
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

/** 选英雄卡片的示例（推送 `champ-select` 的数据，页面 `ui://kotone/lol/champ-select`）。
 * 评级和插件（plugins/lol/src/pushes/champ_select.rs、pushes/rules.rs）算出来的一样：海斗榜 172 个英雄，前 10% S、30% A、60% B、85% C。
 * `me`：我现在是谁（重随以后换了：测试看卡片原地更新）。 */
// i18n-ignore-start：示例数据
export function sampleChampSelect(
  mode: "mayhem" | "rift" = "mayhem", me = "艾瑞莉娅", ended = false,
  /** B 轮（ADR 043）：峡谷选人到哪一步（卡片长出对应的几块）、“操作客户端”开没开；不给时和 A 轮一样只有我方那几行。 */
  options: { stage?: "planning" | "banning" | "waiting" | "picking" | "locked"; control?: boolean; mine?: boolean } = {},
) {
  const champ = (id: number, short: string) => ({ id, short, icon: `/lol-game-data/assets/v1/champion-icons/${id}.png` });
  const blocks = sampleChampSelectBlocks(mode, options, champ);
  if (mode === "rift") {
    return {
      mode: "rift", source: "qq101", ended, bench: [], ...blocks,
      team: [
        { champion: champ(157, "亚索"), me: false, lane: "中路", tier: "T1", winRate: 0.5023 },
        { champion: champ(39, me), me: true, lane: "上路", tier: "T0", winRate: 0.5231 },
        { champion: champ(64, "李青"), me: false, lane: "打野", tier: "T2", winRate: 0.4911 },
        { champion: champ(222, "金克丝"), me: false, lane: "下路", tier: null, winRate: null },
        { champion: champ(412, "锤石"), me: false, lane: "辅助", tier: "T1", winRate: 0.5088 },
      ],
    };
  }
  const mine = me === "艾瑞莉娅"
    ? { champion: champ(39, "艾瑞莉娅"), me: true, winRate: 0.5212, rank: 31, grade: "A" }
    : { champion: champ(901, me), me: true, winRate: 0.5302, rank: 6, grade: "S" };
  return {
    mode: "mayhem", source: "qq101", total: 172, ended, ...blocks,
    team: [
      mine,
      { champion: champ(223, "塔姆"), me: false, winRate: 0.5384, rank: 1, grade: "S" },
      { champion: champ(99, "拉克丝"), me: false, winRate: 0.5101, rank: 48, grade: "A" },
      { champion: champ(412, "锤石"), me: false, winRate: 0.4930, rank: 120, grade: "C" },
      { champion: champ(103, "阿狸"), me: false, winRate: 0.5055, rank: 60, grade: "B" },
    ],
    // 按胜率从高到低；比艾瑞莉娅（52.1%）高的两个标出来，最高的圈出来（重随成斯莫德以后谁都不比它高）
    bench: me === "艾瑞莉娅"
      ? [
        { champion: champ(901, "斯莫德"), winRate: 0.5302, rank: 6, grade: "S", better: true, best: true },
        { champion: champ(50, "斯维因"), winRate: 0.5266, rank: 12, grade: "S", better: true, best: false },
        { champion: champ(145, "卡莎"), winRate: 0.5129, rank: 41, grade: "A", better: false, best: false },
        { champion: champ(17, "提莫"), winRate: 0.4821, rank: 158, grade: "D", better: false, best: false },
      ]
      : [
        { champion: champ(50, "斯维因"), winRate: 0.5266, rank: 12, grade: "S", better: false, best: false },
        { champion: champ(39, "艾瑞莉娅"), winRate: 0.5212, rank: 31, grade: "A", better: false, best: false },
        { champion: champ(145, "卡莎"), winRate: 0.5129, rank: 41, grade: "A", better: false, best: false },
        { champion: champ(17, "提莫"), winRate: 0.4821, rank: 158, grade: "D", better: false, best: false },
      ],
  };
  // i18n-ignore-end
}

/** 选英雄卡片 B 轮的几块（后端 plugins/lol/src/pushes/champ_select.rs）：峡谷照选人的进度长（预选 / 禁用 → 可以禁的；
 * 轮到你 → 英雄池；锁定 → 对线），我方 3 个人选了就有阵容，有英雄就有“我的英雄”；海斗只有“我的英雄”（aramhex-data）。 */
// i18n-ignore-start：示例数据
function sampleChampSelectBlocks(
  mode: "mayhem" | "rift",
  options: { stage?: "planning" | "banning" | "waiting" | "picking" | "locked"; control?: boolean; mine?: boolean },
  champ: (id: number, short: string) => { id: number; short: string; icon: string },
) {
  const stage = options.stage;
  if (!stage && !options.mine) return {};
  const item = (id: number, name: string) => ({ id, name, icon: `/lol-game-data/assets/v1/items/${id}.png` });
  const spell = (id: number, name: string) => ({ id, name, icon: "" });
  const rune = (id: number, name: string) => ({ id, name, icon: "" });
  const control = options.control === true;
  if (mode === "mayhem") {
    return {
      control,
      mine: {
        champion: champ(39, "艾瑞莉娅"), lane: null, locked: true, source: "aramhex", patch: "16.23",
        runes: null, skills: null,
        augments: [
          { id: 1001, name: "泰坦的坚决", rarity: "kPrismatic", icon: "", delta: 4.2, games: 3000 },
          { id: 1002, name: "尖端发明家", rarity: "kGold", icon: "", delta: 2.8, games: 1800 },
          { id: 1003, name: "恶魔之舞", rarity: "kSilver", icon: "", delta: 1.1, games: 900 },
        ],
        items: [
          { kind: "core", items: [item(3153, "破败王者之刃"), item(6333, "死亡之舞"), item(2517, "破舰者")], winRate: 0.5727, pickRate: 0.0296 },
          { kind: "core", items: [item(3153, "破败王者之刃"), item(3078, "三相之力"), item(6333, "死亡之舞")], winRate: 0.5214, pickRate: 0.0339 },
        ],
        spells: [spell(4, "闪现"), spell(32, "标记")], spellsWinRate: 0.5101,
        importable: { runes: false, items: true, spells: true },
      },
    };
  }
  const locked = stage === "locked";
  const out: Record<string, unknown> = {
    stage: stage ?? "waiting", control, bans: null, pool: null, matchup: null,
    mine: {
      champion: champ(39, "艾瑞莉娅"), lane: "上路", locked, source: "qq101", patch: "16.23",
      runes: {
        styles: [rune(8000, "精密"), rune(8400, "坚决")],
        perks: [rune(8010, "征服者"), rune(9111, "凯旋"), rune(9104, "传说：欢欣"), rune(8299, "最后的抵抗"),
          rune(8444, "复苏"), rune(8473, "骸骨镀层"), rune(5005, "攻击速度"), rune(5008, "适应之力"), rune(5001, "生命值")],
        winRate: 0.4758, pickRate: 0.4527,
      },
      items: [
        { kind: "starting", items: [item(1055, "多兰之刃"), item(2003, "生命药水")], winRate: 0.5127, pickRate: 0.8044 },
        { kind: "boots", items: [item(3047, "铁板靴"), item(3111, "水银之靴")], winRate: null, pickRate: null },
        { kind: "core", items: [item(3153, "破败王者之刃"), item(6672, "海妖杀手"), item(6333, "死亡之舞")], winRate: 0.5806, pickRate: 0.181 },
        { kind: "late", items: [item(3026, "守护天使"), item(3143, "兰顿之兆"), item(3053, "斯特拉克的挑战护手")], winRate: null, pickRate: null },
      ],
      skills: { priority: ["Q", "E", "W"], order: ["Q", "E", "W", "Q", "Q", "R"] },
      spells: [spell(4, "闪现"), spell(12, "传送")], spellsWinRate: 0.5053,
      augments: [],
      importable: { runes: true, items: true, spells: true },
    },
    comp: {
      source: "client",
      ally: { count: 4, physical: 4, magic: 0, mixed: 0, frontline: 0, cc: 1, melee: 3, ranged: 1, flags: ["all_physical", "no_frontline"] },
      enemy: stage === "picking" || locked
        ? { count: 3, physical: 1, magic: 2, mixed: 0, frontline: 1, cc: 2, melee: 1, ranged: 2, flags: [] }
        : null,
    },
  };
  if (stage === "planning" || stage === "banning") {
    out.bans = {
      lane: "上路", against: champ(39, "艾瑞莉娅"), source: "qq101",
      list: [
        { champion: champ(266, "亚托克斯"), reason: "counter", winRate: 0.4461 },
        { champion: champ(58, "雷克顿"), reason: "counter", winRate: 0.4552 },
        { champion: champ(62, "孙悟空"), reason: "counter", winRate: 0.4612 },
        { champion: champ(875, "瑟提"), reason: "strong", tier: "T0", winRate: 0.5281, banRate: 0.21 },
        { champion: champ(86, "盖伦"), reason: "strong", tier: "T1", winRate: 0.5162, banRate: 0.08 },
      ],
    };
  }
  if (stage === "picking") {
    out.pool = {
      lane: "上路", source: "qq101", against: [champ(266, "亚托克斯"), champ(103, "阿狸"), champ(222, "金克丝")],
      list: [
        { champion: champ(86, "盖伦"), games: 6, wins: 4, good: [{ champion: champ(266, "亚托克斯"), winRate: 0.5381 }], bad: [], tier: "T1", winRate: 0.5162 },
        { champion: champ(39, "艾瑞莉娅"), games: 9, wins: 5, good: [], bad: [{ champion: champ(266, "亚托克斯"), winRate: 0.4455 }], tier: "T1", winRate: 0.5023 },
        { champion: champ(122, "德莱厄斯"), games: 3, wins: 1, good: [], bad: [], tier: "T2", winRate: 0.4987 },
      ],
    };
  }
  if (locked) {
    out.matchup = {
      me: champ(39, "艾瑞莉娅"), opponent: champ(266, "亚托克斯"), lane: "上路", guessed: true,
      laneAdvantage: "opponent", soloKillAdvantage: "me", playStyle: "even",
      tip: "Dodge Darkin Blade (Q) sweet spots and fight when his Q is on cooldown.", source: "opgg", url: "https://op.gg",
    };
  }
  return out;
}
// i18n-ignore-end
