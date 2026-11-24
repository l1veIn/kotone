// 英雄联盟插件“操作客户端”的偏好（ADR 038 第 3.5 步，照 LeagueAkari）：存在设置 `plugins.lol.settings`
// （camelCase，和后端 plugins/lol/src/control.rs 的 ControlSettings 一致）。数据开关 `control` 默认关。
//
// 设置补丁是 JSON merge：对象按字段合并，数组整个替换。所以改英雄顺序时整份 `champions` 一起发，
// 其余的偏好不动。
import type { Settings } from "$lib/ipc/contracts";
import { updateSettings } from "$lib/ipc";
import { settingsStore } from "$lib/stores/ui";

export interface SelectPrefs {
  enabled: boolean;
  /** 按顺序，第一个能用的。 */
  champions: number[];
  /** 选了就锁定（禁英雄总是直接禁）。 */
  lockIn: boolean;
  /** 轮到自己以后等几秒。 */
  delaySeconds: number;
}

export interface ControlPrefs {
  autoAccept: boolean;
  acceptDelaySeconds: number;
  pick: SelectPrefs;
  ban: SelectPrefs;
}

/** 和后端夹的范围一致：确认对局一共约 10 秒，最多等 8 秒；选 / 禁最多等 20 秒。 */
export const ACCEPT_DELAY_MAX = 8;
export const SELECT_DELAY_MAX = 20;

const num = (v: unknown, max: number) => {
  const n = typeof v === "number" && Number.isFinite(v) ? Math.round(v) : 0;
  return Math.min(Math.max(n, 0), max);
};

function select(v: unknown): SelectPrefs {
  const o = (v && typeof v === "object" ? v : {}) as Record<string, unknown>;
  return {
    enabled: o.enabled === true,
    champions: Array.isArray(o.champions) ? o.champions.filter((c): c is number => typeof c === "number" && c > 0) : [],
    lockIn: o.lockIn === true,
    delaySeconds: num(o.delaySeconds, SELECT_DELAY_MAX),
  };
}

/** 设置里现在的偏好（没写的按默认：全关、不等）。 */
export function controlPrefs(settings: Settings | null | undefined): ControlPrefs {
  const raw = (settings?.plugins?.lol?.settings ?? {}) as Record<string, unknown>;
  return {
    autoAccept: raw.autoAccept === true,
    acceptDelaySeconds: num(raw.acceptDelaySeconds, ACCEPT_DELAY_MAX),
    pick: select(raw.pick),
    ban: select(raw.ban),
  };
}

/** 存一部分偏好（对象按字段合并，数组整个替换）。 */
export async function saveControl(patch: Record<string, unknown>): Promise<void> {
  settingsStore.set(await updateSettings({ plugins: { lol: { settings: patch } } }));
}

/** 列表里上移 / 下移一项（到头了不动）。 */
export function move(list: number[], index: number, delta: -1 | 1): number[] {
  const to = index + delta;
  if (index < 0 || to < 0 || to >= list.length) return list;
  const next = [...list];
  [next[index], next[to]] = [next[to], next[index]];
  return next;
}
