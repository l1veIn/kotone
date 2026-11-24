// 侧边栏可以自己排（ADR 034 第六节）：两组（游戏陪玩、语音输入）里的页面在组内上移 / 下移、隐藏。
// 插件不在左侧栏（ADR 038，2026-11-24 玩家定的）：都在「设置 → 插件」里点开。旧配置里记的插件页
// （`plugin:<id>`、`music`）不在任何一组里，排序时自然跳过。
// 存在设置 `ui.sidebar`（order、hidden），她也能改。“设置”和“关于”在底部固定，不在这里。
// 规则：order 只管它认得的页面；没在 order 里的（新加的页面、旧版本没有的）按默认位置插回去，
// 所以排过序以后新加的页面也看得见。纯函数，不碰界面。

export type SidebarGroupId = "companion" | "voice";

/** 两组的默认顺序（页面 ID 和 AppSidebar 的 PageId 一致）。 */
export const SIDEBAR_GROUPS: { id: SidebarGroupId; pages: string[] }[] = [
  { id: "companion", pages: ["assistant", "game", "automation"] },
  { id: "voice", pages: ["general", "legacy-reminders"] },
];

/** 永远不能隐藏的页面（“设置”：藏起来就改不回来了）。 */
export const PINNED_PAGES = new Set(["advanced"]);

export interface SidebarPrefs { order: string[]; hidden: string[] }

export const DEFAULT_SIDEBAR: SidebarPrefs = { order: [], hidden: [] };

export function normalizePrefs(value: unknown): SidebarPrefs {
  const v = (value ?? {}) as Partial<SidebarPrefs>;
  const ids = (list: unknown) => (Array.isArray(list)
    ? [...new Set(list.filter((x): x is string => typeof x === "string" && !!x))]
    : []);
  return { order: ids(v.order), hidden: ids(v.hidden).filter((id) => !PINNED_PAGES.has(id)) };
}

/**
 * 一组页面按玩家的顺序排（含隐藏的）。`present` = 这一组现在有的页面（按默认顺序；例如没有旧提醒就没有那一项）。
 * order 里有的按 order 排；没有的插回默认位置：跟在默认顺序里它前面最近的那一项后面（前面都没有就放最前）。
 */
export function arrange(present: string[], prefs: SidebarPrefs): string[] {
  const known = prefs.order.filter((id) => present.includes(id));
  const out = [...known];
  present.forEach((id, index) => {
    if (out.includes(id)) return;
    let at = 0;
    for (let i = index - 1; i >= 0; i--) {
      const before = out.indexOf(present[i]);
      if (before >= 0) { at = before + 1; break; }
    }
    out.splice(at, 0, id);
  });
  return out;
}

/** 整个侧边栏现在的顺序（每组按 arrange 排好再接起来），写回 order 用。 */
function fullOrder(groups: string[][], prefs: SidebarPrefs): string[] {
  const out: string[] = [];
  for (const present of groups) out.push(...arrange(present, prefs));
  // 这次没出现的页面（例如条件页面现在不显示）保留它在 order 里的记录，免得下次出现时位置丢了
  for (const id of prefs.order) if (!out.includes(id)) out.push(id);
  return out;
}

/** 在组内上移（delta = -1）/ 下移（+1）；到头了不动。 */
export function movePage(groups: string[][], prefs: SidebarPrefs, id: string, delta: -1 | 1): SidebarPrefs {
  const group = groups.find((g) => g.includes(id));
  if (!group) return prefs;
  const ordered = arrange(group, prefs);
  const from = ordered.indexOf(id);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= ordered.length) return prefs;
  const swapped = [...ordered];
  [swapped[from], swapped[to]] = [swapped[to], swapped[from]];
  const order = fullOrder(groups.map((g) => (g === group ? swapped : g)), { ...prefs, order: [...swapped, ...prefs.order] });
  return { order, hidden: [...prefs.hidden] };
}

export function setPageHidden(prefs: SidebarPrefs, id: string, hidden: boolean): SidebarPrefs {
  if (PINNED_PAGES.has(id)) return prefs;
  const rest = prefs.hidden.filter((x) => x !== id);
  return { order: [...prefs.order], hidden: hidden ? [...rest, id] : rest };
}

/** 能不能再上移 / 下移（按钮灰不灰）。 */
export function canMove(groups: string[][], prefs: SidebarPrefs, id: string, delta: -1 | 1): boolean {
  const group = groups.find((g) => g.includes(id));
  if (!group) return false;
  const ordered = arrange(group, prefs);
  const to = ordered.indexOf(id) + delta;
  return to >= 0 && to < ordered.length;
}

export function isDefault(prefs: SidebarPrefs): boolean {
  return !prefs.order.length && !prefs.hidden.length;
}
