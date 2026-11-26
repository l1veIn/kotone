// 语音输入的推送卡片（ADR 043 D10）：页面地址、卡片里的数据（和 plugins/voice-input/src/pushes.rs 一致）、重发的工具。
export const PAGE_SENT = "ui://kotone/voice-input/sent";
export const PAGE_FAILED = "ui://kotone/voice-input/failed";
/** 卡片上的“重发”（只给页面用）。 */
export const TOOL_RESEND = "voice-input.resend";

export interface Place {
  id: string;
  name: string;
}

/** “发出去的话”：同一个游戏、同一个频道，连着发的几句。 */
export interface SentCardData {
  game: Place;
  channel: Place;
  lines: { text: string; source?: string; atMs: number }[];
}

/** “没发出去”：一句话、原因、重发要的 id；重发成功了有 `resent`。 */
export interface FailedCardData {
  id: string;
  text: string;
  source?: string;
  reason: string;
  code: string;
  atMs: number;
  attempts: number;
  /** 当时选的频道（Kotone 重启过时重发用） */
  channel?: { game: string; id: string };
  resent?: { game: Place; channel: Place; atMs: number };
}

/** 重发的结果（工具的 content）。 */
export interface ResendResult {
  status: "sent" | "failed" | "refused" | "disabled";
  reason?: string;
  channelName?: string;
  text?: string;
}

export function clockTime(atMs: number): string {
  return new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}
