// 游戏状态和事件（ADR 036）：界面里的事件名、阶段名，自动化“游戏事件触发”的说明，现在在玩什么。
// 事件目录只有一种登记方式（插件重构第 7 步）：本体的 game.* 在这里；游戏专用的事由带适配器的插件在 plugin.toml 的
// [[events]] 里登记，界面从插件清单读（`manifest.events`），名字用插件自己的文案（plugin_<id>_event_<名字>）。
// 插件关着（或它要的数据开关关着）时它的事不在选择里，但还认得（已经保存的照样显示、能保存）。
// 规则和后端 games::event_catalog 一致（保存时后端还会再校验一次）。
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";
import type { Settings } from "./ipc/contracts";
import type { EventTrigger } from "./automation";
import { pluginEnabled, pluginName, pluginText, sourceEnabled, sourceName, type PluginInfo, type PluginManifest, type PluginEventSpec } from "./plugins";
import { m } from "./paraglide/messages.js";

export type GamePhase = "stopped" | "running" | "menu" | "queue" | "pre_match" | "loading" | "in_match" | "post_match";

/** 触发器里能选的阶段（stopped / running 用“游戏启动 / 退出”更直接，不列）。 */
export const TRIGGER_PHASES: GamePhase[] = ["menu", "queue", "pre_match", "loading", "in_match", "post_match"];

export function phaseLabel(phase: string): string {
  switch (phase) {
    case "stopped": return m.game_phase_stopped();
    case "running": return m.game_phase_running();
    case "menu": return m.game_phase_menu();
    case "queue": return m.game_phase_queue();
    case "pre_match": return m.game_phase_pre_match();
    case "loading": return m.game_phase_loading();
    case "in_match": return m.game_phase_in_match();
    case "post_match": return m.game_phase_post_match();
    default: return phase;
  }
}

export interface GameEventInfo {
  type: string;
  /** 中文说明（插件登记的；本体的 game.* 用本体的文案） */
  label: string;
  /** 登记它的插件；本体的 game.* 为 null（任何有进程名的游戏都有） */
  plugin: PluginManifest | null;
  /** 数据里有 team（我方 / 对面）可以选 */
  team: boolean;
  /** 现在为什么不会发生（“英雄联盟插件关着”）；null = 会发生 */
  off: string | null;
}

/** 本体登记的（和 kotone_core::games::generic_events 一致）。 */
const GENERIC_EVENTS = [
  "game.started", "game.stopped", "game.phase_changed", "game.match_started", "game.match_ended", "game.focused", "game.unfocused",
];

function eventOff(settings: Settings | null | undefined, plugin: PluginManifest, spec: PluginEventSpec): string | null {
  if (!pluginEnabled(settings, plugin)) return m.game_event_off_plugin({ plugin: pluginName(plugin) });
  const missing = (spec.needs ?? []).find((s) => !sourceEnabled(settings, plugin, s));
  if (missing === undefined) return null;
  const source = plugin.sources.find((s) => s.id === missing);
  return m.game_event_off_source({ plugin: pluginName(plugin), source: source ? sourceName(plugin, source) : missing });
}

/** 合并后的事件目录：本体的 game.*，加上每个插件登记的（关着的也在，标着为什么不会发生）。 */
export function eventCatalog(plugins: PluginInfo[] | null | undefined, settings: Settings | null | undefined): GameEventInfo[] {
  const out: GameEventInfo[] = GENERIC_EVENTS.map((type) => ({ type, label: type, plugin: null, team: false, off: null }));
  for (const { manifest } of plugins ?? []) {
    if (!manifest.adapter) continue;
    for (const spec of manifest.events ?? []) {
      if (out.some((e) => e.type === spec.type)) continue;
      out.push({ type: spec.type, label: spec.label, plugin: manifest, team: "team" in (spec.data ?? {}), off: eventOff(settings, manifest, spec) });
    }
  }
  return out;
}

/** 这个游戏配置现在能选的事（不选游戏 = 只有通用的）：插件关着的不在里面。 */
export function eventsFor(catalog: GameEventInfo[], game: string | null | undefined): GameEventInfo[] {
  return catalog.filter((e) => !e.off && (!e.plugin || (!!game && (e.plugin.games.length === 0 || e.plugin.games.includes(game)))));
}

/** 这个写法（一种事，或 `<范围>.*`）对上的事。 */
function matching(catalog: GameEventInfo[], pattern: string): GameEventInfo[] {
  if (!pattern.endsWith(".*")) return catalog.filter((e) => e.type === pattern);
  const prefix = pattern.slice(0, -1);
  return catalog.filter((e) => e.type.startsWith(prefix));
}

export function eventLabel(type: string, catalog: GameEventInfo[]): string {
  switch (type) {
    case "game.started": return m.game_event_started();
    case "game.stopped": return m.game_event_stopped();
    case "game.phase_changed": return m.game_event_phase_changed();
    case "game.match_started": return m.game_event_match_started();
    case "game.match_ended": return m.game_event_match_ended();
    case "game.focused": return m.game_event_focused();
    case "game.unfocused": return m.game_event_unfocused();
  }
  if (type.endsWith(".*")) {
    const plugin = matching(catalog, type).find((e) => e.plugin)?.plugin;
    return plugin ? m.game_event_any_plugin({ plugin: pluginName(plugin) }) : type;
  }
  const info = catalog.find((e) => e.type === type);
  if (!info?.plugin) return type;
  return pluginText(info.plugin.id, `event_${type.slice(type.indexOf(".") + 1)}`, info.label);
}

/** 触发里有整种不会发生的事时，第一个原因（“英雄联盟插件关着”）；都会发生（或触发关着）时 null。 */
export function triggerOff(on: EventTrigger, catalog: GameEventInfo[]): string | null {
  if (!on.enabled) return null;
  for (const pattern of on.event) {
    const hits = matching(catalog, pattern);
    if (hits.length && hits.every((e) => e.off)) return hits[0].off;
  }
  return null;
}

function values(value: unknown): string[] {
  if (value === undefined || value === null) return [];
  return (Array.isArray(value) ? value : [value]).map(String);
}

/** 给人看的触发说明：“英雄联盟 · 阶段变成加载 · 每局一次”。`gameName` 把游戏配置 id 换成名字；事件名从目录来。 */
export function describeTrigger(on: EventTrigger, catalog: GameEventInfo[], gameName: (id: string) => string = (id) => id): string {
  const match = on.match ?? {};
  const events = on.event.map((type) => {
    if (type === "game.phase_changed" && values(match.to).length) {
      const phases = values(match.to).map(phaseLabel).join(" / ");
      return on.for ? m.game_event_phase_to_hold({ phase: phases, seconds: on.for }) : m.game_event_phase_to({ phase: phases });
    }
    const team = values(match.team);
    const side = team.length === 1 ? (team[0] === "ally" ? m.game_event_team_ally() : m.game_event_team_enemy()) : "";
    return side ? m.game_event_with_team({ event: eventLabel(type, catalog), team: side }) : eventLabel(type, catalog);
  });
  const parts = [on.game ? gameName(on.game) : m.game_event_any_game(), events.join(" / ")];
  if (on.oncePerMatch) parts.push(m.game_event_once_per_match());
  return parts.join(" · ");
}

/** 一个在跑的游戏（`game_state`）。 */
export interface PlayingGame {
  game: string;
  name: string;
  phase: GamePhase;
  nativePhase: string;
  focused: boolean;
  runningSeconds: number;
  matchSeconds: number | null;
}

/** 现在在玩什么（浏览器预览里没有游戏在跑）。 */
export async function playingGames(): Promise<PlayingGame[]> {
  if (!isTauri) return [];
  const state = await invoke<{ playing: PlayingGame[] }>("game_state");
  return state.playing;
}
