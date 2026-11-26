// 悬浮窗头像旁边放气泡还是聊天小窗（ADR 043 D6，2026-11-26 晚修订）——这条规则只写在这里（floatingShape）。
// 玩家：“右下角肯定有角色头像，但是有的时候是聊天气泡，因为是一条简单消息，有的时候是chat小窗，因为是复杂的消息”。
//
// - 气泡：一条简单的——她在听、在想、说了一句短话，或者推送的一行。
// - 聊天小窗：复杂的——弹出了一张卡片（新的卡片推送、带提醒的更新）、她这一轮用了工具（她说话、问一句、发弹幕不算）、
//   工具结果带卡片、只写不念的细节、长回答（超过 LONG_TEXT 个字或者分了行），或者这一次出现以后来了不止一条
//   （她在说话时来了一条推送、一下来了两条推送、她答了不止一轮）。
// - 这一次出现里换成了聊天小窗就一直是小窗，收起以后再从头算（Floating.svelte 记着，免得来回跳）。
//
// 还有进度条的算法（ringFraction）：后端给 `at_ms` 那一刻还剩多少、走不走，这里算现在还剩几成。
import type { BubbleSnapshot } from "../../lib/bubble";
import type { FloatingRing } from "../../lib/floating";
import type { Turn } from "../../lib/agentChat/timeline";

export type FloatingShape = "bubble" | "mini";

/** 多长算长回答（字）：气泡里大约三行。 */
export const LONG_TEXT = 60;
/** 推送记进对话的时间比悬浮窗出现早一点点：算“这一次来的”时往前放宽这么多（毫秒）。 */
export const FRESH_SLACK_MS = 2_000;
/** 她怎么说话的几个工具：不算“用了工具”。 */
const NARRATION = new Set(["speak", "reply.ask", "danmaku"]);

/** 长回答：超过 LONG_TEXT 个字，或者分了行。 */
export function isLong(text: string): boolean {
  const trimmed = text.trim();
  return [...trimmed].length > LONG_TEXT || trimmed.includes("\n");
}

/** `speak` 的参数：text 念出来，written 只写不念。参数还没写完时为空。 */
function speakArgs(args: string): { text: string; written: string } {
  try {
    const value = JSON.parse(args);
    return { text: String(value?.text ?? ""), written: String(value?.written ?? "") };
  } catch { return { text: "", written: "" }; }
}

/** 推送那一轮的标识（`插件:key`，和后端、聊天页一样）。 */
export function turnPushId(turn: Turn): string | null {
  return turn.push ? `${turn.push.plugin}:${turn.push.key.trim()}` : null;
}

/**
 * 这一次悬浮窗出现以后来的那几轮：她正在回答的、`since` 以后开始的、推送在 `since` 以后更新过的、弹出小窗定位的那条。
 * `since` 是后端记的这一次出现的时间（0 = 没在显示）。
 */
export function freshTurns(turns: readonly Turn[], since: number, focus: string | null): Turn[] {
  if (!since) return [];
  const from = since - FRESH_SLACK_MS;
  return turns.filter((turn) => turn.active
    || (turn.startedAtMs ?? 0) >= from
    || (turn.push?.updatedAtMs ?? 0) >= from
    || (focus !== null && turnPushId(turn) === focus));
}

/** 她这一轮里有没有复杂的：用了工具（说话、问一句、弹幕不算）、带卡片、只写不念的细节、长回答、复盘。 */
export function complexTurn(turn: Turn): boolean {
  return turn.blocks.some((block) => {
    switch (block.kind) {
      case "tool": {
        if (block.tool.display?.ui?.uri) return true;
        if (!NARRATION.has(block.tool.call.name)) return true;
        if (block.tool.call.name !== "speak") return false;
        const said = speakArgs(block.tool.call.arguments);
        return said.written.trim() !== "" || isLong(said.text);
      }
      case "text":
        return isLong(block.text);
      case "recap":
        return true;
      default:
        return false;
    }
  });
}

/** 她这一行自己的话（在说、在问、收尾、出错；在听 / 在想时显示的是玩家的话，不算）。 */
function herWords(her: BubbleSnapshot): string {
  return her.phase === "speaking" || her.phase === "asking" || her.phase === "note" || her.phase === "error" ? her.text : "";
}

export interface ShapeInput {
  /** 后端弹出了聊天小窗（新的卡片推送、带提醒的更新）。 */
  popped: boolean;
  /** 她这一行（显示着时）。 */
  her: BubbleSnapshot | null;
  /** 气泡里推送的一行有几条。 */
  lines: number;
  /** 这一次出现以后来的那几轮（freshTurns）。 */
  fresh: readonly Turn[];
}

/** 头像旁边放气泡还是聊天小窗。 */
export function floatingShape({ popped, her, lines, fresh }: ShapeInput): FloatingShape {
  if (popped) return "mini";
  // 推送那几轮由 popped / lines 代表；剩下的是她的（玩家说的话和她的回答）
  const talks = fresh.filter((turn) => !turn.push);
  if (lines + Math.max(her ? 1 : 0, talks.length) > 1) return "mini";
  if (talks.some(complexTurn)) return "mini";
  if (her && isLong(herWords(her))) return "mini";
  return "bubble";
}

/** 进度条现在还剩几成（0–1）。没有进度条时是满的。 */
export function ringFraction(ring: FloatingRing | null | undefined, now: number): number {
  if (!ring || ring.totalMs <= 0) return 1;
  const left = ring.running ? ring.leftMs - Math.max(0, now - ring.atMs) : ring.leftMs;
  return Math.max(0, Math.min(1, left / ring.totalMs));
}
