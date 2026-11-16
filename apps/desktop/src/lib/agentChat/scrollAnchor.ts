// Reverse-scroll anchor and history threshold adapted from Codeg
// src/components/message/virtualized-message-thread.tsx at commit
// 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1, via RambleDesk
// apps/desktop/src/lib/agents/chat/{scroll-anchor.ts,timeline-window.ts}.
// Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0
// Kotone: anchors are chat turns (data-turn-anchor) of a locally loaded conversation.

export type ScrollAnchor = Readonly<{ id: string; offset: number; scrollTop: number; scrollHeight: number }>;

const SELECTOR = "[data-turn-anchor]";

/** Remember the first visible turn and its offset before older turns are prepended. */
export function captureScrollAnchor(viewport: HTMLElement): ScrollAnchor | null {
  const top = viewport.getBoundingClientRect().top;
  const firstVisible = Array.from(viewport.querySelectorAll<HTMLElement>(SELECTOR))
    .find((element) => element.getBoundingClientRect().bottom > top);
  if (!firstVisible?.dataset.turnAnchor) return null;
  return {
    id: firstVisible.dataset.turnAnchor,
    offset: firstVisible.getBoundingClientRect().top - top,
    scrollTop: viewport.scrollTop,
    scrollHeight: viewport.scrollHeight,
  };
}

/** Put the remembered turn back at the same place on screen. */
export function restoreScrollAnchor(viewport: HTMLElement, anchor: ScrollAnchor): void {
  const element = Array.from(viewport.querySelectorAll<HTMLElement>(SELECTOR))
    .find((candidate) => candidate.dataset.turnAnchor === anchor.id);
  viewport.scrollTop = element
    ? viewport.scrollTop + element.getBoundingClientRect().top - viewport.getBoundingClientRect().top - anchor.offset
    : anchor.scrollTop + viewport.scrollHeight - anchor.scrollHeight;
}

/** A near-top crossing only: initial layout and short lists never drain history. */
export function crossedHistoryThreshold(previous: number | null, next: number): boolean {
  return previous !== null && previous >= 240 && next < 240;
}
