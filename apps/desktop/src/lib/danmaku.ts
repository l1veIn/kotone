import hello from "../assets/brand/stickers/hello.webp";
import cheering from "../assets/brand/stickers/cheering.webp";
import pointing from "../assets/brand/stickers/pointing.webp";
import amazed from "../assets/brand/stickers/amazed.webp";

export type DanmakuEmote = "none" | "hello" | "cheering" | "pointing" | "amazed";
export interface DanmakuPresentation {
  id: number;
  text: string;
  startedAtMs: number;
  expiresAtMs: number;
  speedPxPerSec: number;
  fontSize: number;
  topOffsetPx: number;
  color: string;
  emote: DanmakuEmote;
}

export const DANMAKU_EMOTES: { id: DanmakuEmote; label: string; src: string | null }[] = [
  { id: "none", label: "纯文字", src: null },
  { id: "hello", label: "招手", src: hello },
  { id: "cheering", label: "加油", src: cheering },
  { id: "pointing", label: "提醒", src: pointing },
  { id: "amazed", label: "惊讶", src: amazed },
];
export const DANMAKU_PREVIEW_EVENT = "kotone:preview-danmaku";
export const DANMAKU_PREVIEW_FINISHED_EVENT = "kotone:preview-danmaku-finished";
export const MAX_DANMAKU_DURATION_MS = 45_000;
export const DEFAULT_DANMAKU_TOP_OFFSET_PX = 120;

const clamp = (value: number, fallback: number, min: number, max: number): number =>
  Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : fallback;

export function danmakuText(text: string): string {
  return text.replace(/\s+/gu, " ").trim();
}

export function danmakuStyle(presentation: Pick<DanmakuPresentation, "text" | "fontSize" | "color" | "emote">) {
  const requestedSize = clamp(presentation.fontSize, 32, 16, 72);
  return {
    fontSize: Array.from(danmakuText(presentation.text)).length > 100
      ? Math.min(requestedSize, 24) : requestedSize,
    color: /^#[\da-f]{6}$/i.test(presentation.color) ? presentation.color : "#FFFFFF",
    emote: DANMAKU_EMOTES.find((item) => item.id === presentation.emote) ?? DANMAKU_EMOTES[0],
  };
}

/** Actual rendered width (including the emote) determines travel time, in CSS pixels. */
export function danmakuMotion(viewportWidth: number, contentWidth: number, speedPxPerSec: number) {
  const startX = Math.max(0, viewportWidth) + 24;
  const endX = -Math.max(0, contentWidth) - 24;
  const speed = clamp(speedPxPerSec, 160, 40, 600);
  const durationMs = Math.min(MAX_DANMAKU_DURATION_MS, Math.max(1_200, (startX - endX) / speed * 1000));
  return { startX, endX, durationMs };
}

/** Position in logical/CSS pixels; missing values from older events keep the default. */
export function danmakuTop(viewportHeight: number, contentHeight: number, topOffsetPx?: number): number {
  const requested = Math.round(clamp(topOffsetPx ?? DEFAULT_DANMAKU_TOP_OFFSET_PX, DEFAULT_DANMAKU_TOP_OFFSET_PX, 0, 2160));
  return Math.min(requested, Math.max(0, viewportHeight - contentHeight));
}

/** Explicit browser-only fixture event; it does not schedule an OS reminder. */
export function showDanmakuPreview(presentation: DanmakuPresentation | null): void {
  window.dispatchEvent(new CustomEvent(DANMAKU_PREVIEW_EVENT, { detail: presentation }));
}
