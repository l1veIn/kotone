// 原神插件的几组名字：跑的是什么、现在怎样、计划的运行状态（和后端 runs.rs / bgi.rs 的取值一一对应）。
import { m } from "$lib/paraglide/messages.js";
import type { RunKind, RunStatus } from "./genshin";

export function kindLabel(kind: RunKind | string): string {
  switch (kind) {
    case "dragon": return m.plugin_genshin_kind_dragon();
    case "group": return m.plugin_genshin_kind_group();
    case "plan": return m.plugin_genshin_kind_plan();
    default: return m.plugin_genshin_kind_task();
  }
}

export function statusLabel(status: RunStatus | string): string {
  switch (status) {
    case "starting": return m.plugin_genshin_status_starting();
    case "running": return m.plugin_genshin_status_running();
    case "succeeded": return m.plugin_genshin_status_succeeded();
    case "partial": return m.plugin_genshin_status_partial();
    case "cancelled": return m.plugin_genshin_status_cancelled();
    case "failed": return m.plugin_genshin_status_failed();
    default: return m.plugin_genshin_status_unknown();
  }
}

/** Pulonia 的运行状态（`bgi::run_status`）。 */
export function planStatusLabel(status: string): string {
  switch (status) {
    case "queued": return m.plugin_genshin_plan_status_queued();
    case "cancelling": return m.plugin_genshin_plan_status_cancelling();
    case "timed_out": return m.plugin_genshin_plan_status_timed_out();
    case "interrupted": return m.plugin_genshin_plan_status_interrupted();
    case "needs_attention": return m.plugin_genshin_plan_status_needs_attention();
    case "expired": return m.plugin_genshin_plan_status_expired();
    default: return statusLabel(status);
  }
}

/** 这次运行的样子：进行中 / 好的 / 不好的 / 不知道。 */
export function tone(status: RunStatus | string): "live" | "good" | "bad" | "plain" {
  if (status === "starting" || status === "running") return "live";
  if (status === "succeeded") return "good";
  if (status === "failed" || status === "partial") return "bad";
  return "plain";
}
