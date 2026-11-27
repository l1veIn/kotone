import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";

let previewSession: string | null = null;

export async function beginVoiceInputTrial(): Promise<string> {
  if (isTauri) return invoke<string>("voice_input_trial_begin");
  if (previewSession) throw new Error("A voice trial is already running");
  previewSession = crypto.randomUUID();
  return previewSession;
}

export async function finishVoiceInputTrial(sessionId: string): Promise<string> {
  if (isTauri) return invoke<string>("voice_input_trial_finish", { sessionId });
  if (sessionId !== previewSession) throw new Error("Voice trial has ended");
  previewSession = null;
  return "集合以后我们一起走。"; // i18n-ignore: sample transcript for the browser demo.
}

export async function cancelVoiceInputTrial(sessionId: string): Promise<void> {
  if (isTauri) return invoke<void>("voice_input_trial_cancel", { sessionId });
  if (sessionId === previewSession) previewSession = null;
}
