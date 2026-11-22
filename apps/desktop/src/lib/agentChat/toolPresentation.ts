// Tool lifecycle presentation adapted from Codeg src/lib/tool-call-lifecycle.ts at commit
// 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, via RambleDesk
// apps/desktop/src/lib/agents/chat/activity-presentation.ts.
// Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0
// Kotone: Chinese labels; statuses come from Kotone's agent events and stored tool results.
import type { ToolStatus } from "../agent";
import { m } from "../paraglide/messages.js";

export function toolStatusUnsettled(status: ToolStatus): boolean {
  return status !== "completed" && status !== "failed";
}

/** A tool without a final result is "running" while its turn runs, otherwise "incomplete". */
export function toolPresentation(status: ToolStatus, runActive: boolean) {
  const unsettled = toolStatusUnsettled(status);
  return {
    spinning: unsettled && runActive,
    incomplete: unsettled && !runActive,
    failed: status === "failed",
    label: status === "completed" ? m.chat_tool_status_completed() : status === "failed" ? m.chat_tool_status_failed()
      : !runActive ? m.chat_tool_status_incomplete() : status === "pending" ? m.chat_tool_status_pending() : m.chat_tool_status_running(),
  };
}

/** Arguments and results may be truncated JSON, so malformed input stays readable as-is. */
export function formatToolJson(value: string | null | undefined): string {
  if (!value) return "";
  try { return JSON.stringify(JSON.parse(value), null, 2); } catch { return value; }
}
