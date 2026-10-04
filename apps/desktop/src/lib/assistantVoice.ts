import { writable } from "svelte/store";
import {
  beginAssistantVoice, cancelAssistantVoice, confirmAssistantVoice, endAssistantVoice,
  type AssistantVoiceSession,
} from "./assistant";
import type { AssistantInteractionMode } from "./ipc";
import type { AppState } from "./stores/state";

type Phase = "idle" | "starting" | "listening" | "preview" | "working" | "cancelling";
interface VoiceView { phase: Phase; mode: AssistantInteractionMode }
interface OwnedSession {
  start: Promise<AssistantVoiceSession>;
  snapshot?: AssistantVoiceSession;
  released: boolean;
  cancelled: boolean;
  cancelling: boolean;
  operation: boolean;
  observedActive: boolean;
}

/** Owns exactly one button-created session. Cancellation never queues behind AI. */
export function createAssistantVoiceControl(onError: (error: unknown) => void) {
  const view = writable<VoiceView>({ phase: "idle", mode: "push-to-talk" });
  let current: OwnedSession | undefined;
  let observed: Pick<AppState, "state" | "purpose"> = { state: "idle", purpose: "dictation" };
  let disposed = false;

  function publish(phase: Phase, mode = current?.snapshot?.interactionMode ?? "push-to-talk") {
    if (!disposed) view.set({ phase, mode });
  }
  function clear(session: OwnedSession) {
    if (current !== session) return;
    current = undefined;
    publish("idle");
  }
  function sync() {
    const session = current;
    if (!session || session.cancelled) return;
    const active = !["idle", "success", "error"].includes(observed.state);
    if (observed.purpose === "assistant" && active) session.observedActive = true;
    if (!session.snapshot) return;
    if (session.operation) { publish("working"); return; }
    if (session.observedActive && !active) { clear(session); return; }
    publish(observed.purpose === "assistant" && observed.state === "preview" ? "preview"
      : observed.purpose === "assistant" && observed.state !== "listening" && active ? "working" : "listening");
  }
  async function cancelOwned(session: OwnedSession) {
    if (session.cancelling) return;
    session.cancelling = true;
    session.cancelled = true;
    publish("cancelling");
    try {
      const snapshot = await session.start;
      await cancelAssistantVoice(snapshot.sessionId);
    } catch (error) { if (!disposed) onError(error); }
    finally { clear(session); }
  }
  async function act(session: OwnedSession, confirm: boolean) {
    if (current !== session || session.operation || session.cancelled) return;
    session.operation = true;
    publish("working");
    try {
      const snapshot = await session.start;
      if (session.cancelled || current !== session) return;
      if (confirm) await confirmAssistantVoice(snapshot.sessionId);
      else await endAssistantVoice(snapshot.sessionId);
    } catch (error) {
      if (!session.cancelled && !disposed) onError(error);
    } finally {
      session.operation = false;
      if (current === session && !session.cancelled) sync();
    }
  }
  return {
    subscribe: view.subscribe,
    begin(mode: AssistantInteractionMode, held: boolean) {
      if (current || disposed) return;
      const session: OwnedSession = {
        start: beginAssistantVoice(), released: !held, cancelled: false,
        cancelling: false, operation: false, observedActive: false,
      };
      current = session;
      publish("starting", mode);
      void session.start.then((snapshot) => {
        session.snapshot = snapshot;
        if (current !== session || session.cancelled) return;
        sync();
        if (snapshot.interactionMode === "push-to-talk" && session.released) void act(session, false);
      }).catch((error) => {
        if (!session.cancelled && !disposed) onError(error);
        if (!session.cancelled) clear(session);
      });
    },
    release() {
      if (!current) return;
      current.released = true;
      if (current.snapshot?.interactionMode === "push-to-talk") void act(current, false);
    },
    finish() { if (current) void act(current, false); },
    confirm() { if (current) void act(current, true); },
    cancel() { if (current) void cancelOwned(current); },
    observe(state: Pick<AppState, "state" | "purpose">) { observed = state; sync(); },
    dispose() { disposed = true; if (current) void cancelOwned(current); },
  };
}
