// 琴音的对话气泡（屏幕角落）：状态由后端 bubble.rs 推送（kotone://bubble），这里只负责展示。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri, type BubbleCorner } from "./ipc";
import { m } from "./paraglide/messages.js";
import curious from "../assets/brand/stickers/curious.webp";
import thinking from "../assets/brand/stickers/thinking.webp";
import hello from "../assets/brand/stickers/hello.webp";
import pointing from "../assets/brand/stickers/pointing.webp";
import amazed from "../assets/brand/stickers/amazed.webp";
import sleepy from "../assets/brand/stickers/sleepy.webp";

export const BUBBLE_EVENT = "kotone://bubble";
/** 浏览器预览：设置页“试一下”在同一页面里派发。 */
export const BUBBLE_PREVIEW_EVENT = "kotone-bubble-preview";

export type BubblePhase =
  | "hidden" | "listening" | "recognizing" | "thinking" | "speaking" | "asking" | "note" | "error";

export interface BubbleSnapshot {
  revision: number;
  phase: BubblePhase;
  label: string;
  text: string;
  heard: string;
  askSeconds: number | null;
  askStartedMs: number | null;
  corner: BubbleCorner;
}

/** 每个状态用哪张表情、光环什么颜色；fx/fy 是贴纸里脸的中心（比例），头像按它对准。 */
export interface BubbleLook { face: string; fx: number; fy: number; ring: string }
const CURIOUS = { face: curious, fx: 0.42, fy: 0.43 };
export const bubbleLook: Record<Exclude<BubblePhase, "hidden">, BubbleLook> = {
  listening: { ...CURIOUS, ring: "listening" },
  recognizing: { ...CURIOUS, ring: "listening" },
  thinking: { face: thinking, fx: 0.5, fy: 0.38, ring: "thinking" },
  speaking: { face: pointing, fx: 0.47, fy: 0.4, ring: "speaking" },
  asking: { face: hello, fx: 0.47, fy: 0.38, ring: "asking" },
  note: { face: sleepy, fx: 0.4, fy: 0.58, ring: "idle" },
  error: { face: amazed, fx: 0.5, fy: 0.42, ring: "error" },
};

/** 正文：在听时显示听到的话；在想时显示玩家刚说的话（引号）；其他显示琴音的话。 */
export function bubbleBody(s: BubbleSnapshot): { text: string; quoted: boolean } {
  switch (s.phase) {
    case "listening":
    case "recognizing":
      return { text: s.heard, quoted: true };
    case "thinking":
      return { text: s.text, quoted: true };
    case "asking":
      return { text: s.text, quoted: false };
    default:
      return { text: s.text, quoted: false };
  }
}

/** 追问剩余比例（0–1）。 */
export function askRemaining(s: BubbleSnapshot, now: number): number | null {
  if (s.phase !== "asking" || !s.askSeconds || !s.askStartedMs) return null;
  const left = 1 - (now - s.askStartedMs) / (s.askSeconds * 1000);
  return Math.max(0, Math.min(1, left));
}

export async function bubbleSnapshot(): Promise<BubbleSnapshot | null> {
  if (!isTauri) return null;
  return invoke<BubbleSnapshot | null>("bubble_snapshot");
}

export async function bubblePreview(): Promise<void> {
  if (isTauri) await invoke("bubble_preview");
}

export async function subscribeBubble(cb: (s: BubbleSnapshot) => void): Promise<() => void> {
  if (!isTauri) {
    const onPreview = (e: Event) => cb((e as CustomEvent<BubbleSnapshot>).detail);
    window.addEventListener(BUBBLE_PREVIEW_EVENT, onPreview);
    return () => window.removeEventListener(BUBBLE_PREVIEW_EVENT, onPreview);
  }
  return listen<BubbleSnapshot>(BUBBLE_EVENT, (e) => cb(e.payload));
}

export const bubbleCorners: { value: BubbleCorner; label: string }[] = [
  { value: "bottom-right", label: m.misc_bubble_corner_bottom_right() },
  { value: "bottom-left", label: m.misc_bubble_corner_bottom_left() },
  { value: "top-right", label: m.misc_bubble_corner_top_right() },
  { value: "top-left", label: m.misc_bubble_corner_top_left() },
];
