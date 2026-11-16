// Adapted from xintaofei/codeg, commit 3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1
// (src/lib/ime-composition.ts and src/components/chat/composer/submit-key.ts),
// via RambleDesk apps/desktop/src/lib/imeComposition.ts and agents/composer/submit-key.ts.
// Copyright Codeg authors and contributors. SPDX-License-Identifier: Apache-2.0
// Kotone: plain <textarea> composer, so the ProseMirror code-block / list context and the
// configurable key bindings were removed; Enter sends, Shift+Enter inserts a newline.

/** Legacy `keyCode` an IME reports for the key it is processing. */
export const IME_PROCESS_KEY_CODE = 229;

export interface SubmitKeyEvent {
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  /** Standard DOM flag: a composition (IME) is in flight. */
  isComposing: boolean;
  /** Legacy 229 sentinel some IMEs report on the composition-confirming key. */
  keyCode: number;
}

/**
 * True when this keydown belongs to an IME composition (e.g. the Enter that picks a CJK
 * candidate). `isComposing` covers Chromium / WebView2; 229 and "Process" cover engines
 * that end the composition before the confirming keydown. Callers also track
 * compositionstart / compositionend as a fallback.
 */
export function isImeCompositionKey(event: Pick<SubmitKeyEvent, "key" | "isComposing" | "keyCode">): boolean {
  return event.isComposing === true || event.keyCode === IME_PROCESS_KEY_CODE || event.key === "Process";
}

/** Plain Enter (no modifiers) while not composing sends; everything else keeps the default. */
export function shouldSubmitOnEnter(event: SubmitKeyEvent, composing: boolean): boolean {
  if (event.key !== "Enter") return false;
  if (event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return false;
  if (isImeCompositionKey(event) || composing) return false;
  return true;
}
