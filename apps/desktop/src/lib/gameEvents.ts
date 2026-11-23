// 游戏状态和事件（ADR 036）：界面里的事件名、阶段名，自动化“游戏事件触发”的说明，现在在玩什么。
// 事件目录和后端 kotone_core::games 的一致（保存时后端还会再校验一次）。
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";
import type { EventTrigger } from "./automation";
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
  /** generic = 任何有进程名的游戏都有；lol = 英雄联盟 */
  adapter: "generic" | "lol";
  /** 数据里有 team（我方 / 对面）可以选 */
  team: boolean;
}

export const GAME_EVENTS: GameEventInfo[] = [
  { type: "game.started", adapter: "generic", team: false },
  { type: "game.stopped", adapter: "generic", team: false },
  { type: "game.phase_changed", adapter: "generic", team: false },
  { type: "game.match_started", adapter: "generic", team: false },
  { type: "game.match_ended", adapter: "generic", team: false },
  { type: "game.focused", adapter: "generic", team: false },
  { type: "game.unfocused", adapter: "generic", team: false },
  { type: "lol.first_blood", adapter: "lol", team: true },
  { type: "lol.dragon_kill", adapter: "lol", team: true },
  { type: "lol.herald_kill", adapter: "lol", team: true },
  { type: "lol.baron_kill", adapter: "lol", team: true },
  { type: "lol.turret_killed", adapter: "lol", team: true },
  { type: "lol.inhib_killed", adapter: "lol", team: true },
  { type: "lol.ace", adapter: "lol", team: true },
  { type: "lol.multikill", adapter: "lol", team: true },
  { type: "lol.my_death", adapter: "lol", team: false },
  { type: "lol.my_respawn", adapter: "lol", team: false },
];

/** 这个游戏配置能给哪些事（不选游戏 = 只有通用的）。 */
export function eventsFor(game: string | null | undefined): GameEventInfo[] {
  return GAME_EVENTS.filter((e) => e.adapter === "generic" || e.adapter === game);
}

export function eventLabel(type: string): string {
  switch (type) {
    case "game.started": return m.game_event_started();
    case "game.stopped": return m.game_event_stopped();
    case "game.phase_changed": return m.game_event_phase_changed();
    case "game.match_started": return m.game_event_match_started();
    case "game.match_ended": return m.game_event_match_ended();
    case "game.focused": return m.game_event_focused();
    case "game.unfocused": return m.game_event_unfocused();
    case "lol.first_blood": return m.game_event_lol_first_blood();
    case "lol.dragon_kill": return m.game_event_lol_dragon_kill();
    case "lol.herald_kill": return m.game_event_lol_herald_kill();
    case "lol.baron_kill": return m.game_event_lol_baron_kill();
    case "lol.turret_killed": return m.game_event_lol_turret_killed();
    case "lol.inhib_killed": return m.game_event_lol_inhib_killed();
    case "lol.ace": return m.game_event_lol_ace();
    case "lol.multikill": return m.game_event_lol_multikill();
    case "lol.my_death": return m.game_event_lol_my_death();
    case "lol.my_respawn": return m.game_event_lol_my_respawn();
    case "lol.*": return m.game_event_any_lol();
    default: return type;
  }
}

function values(value: unknown): string[] {
  if (value === undefined || value === null) return [];
  return (Array.isArray(value) ? value : [value]).map(String);
}

/** 给人看的触发说明：“英雄联盟 · 阶段变成加载 · 每局一次”。`gameName` 把游戏配置 id 换成名字。 */
export function describeTrigger(on: EventTrigger, gameName: (id: string) => string = (id) => id): string {
  const match = on.match ?? {};
  const events = on.event.map((type) => {
    if (type === "game.phase_changed" && values(match.to).length) {
      const phases = values(match.to).map(phaseLabel).join(" / ");
      return on.for ? m.game_event_phase_to_hold({ phase: phases, seconds: on.for }) : m.game_event_phase_to({ phase: phases });
    }
    const team = values(match.team);
    const side = team.length === 1 ? (team[0] === "ally" ? m.game_event_team_ally() : m.game_event_team_enemy()) : "";
    return side ? m.game_event_with_team({ event: eventLabel(type), team: side }) : eventLabel(type);
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
