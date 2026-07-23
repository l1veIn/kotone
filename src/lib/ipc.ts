/*
 * 所有 invoke 调用的类型化封装（docs/development.md §5.3 IPC 契约）。
 * 与 src-tauri/src/lib.rs 注册的命令一一对应。
 *
 * 浏览器（vite dev:web 纯前端调试）环境下自动降级为内存 mock，
 * 便于脱离 Tauri 调试 UI；Tauri 环境直接走 invoke。
 */

import { invoke } from "@tauri-apps/api/core";

/** 是否运行在 Tauri WebView 中（否则为纯浏览器调试） */
export const isTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// ---------- 设置与配置 ----------
export interface HotkeyConfig {
  key: string;
  mode: "toggle" | "hold";
}

export interface Settings {
  hotkey: HotkeyConfig;
  audioDeviceId: string;
  sttEngine: string;
  engineOptions: Record<string, unknown>;
  autoSend: boolean;
  activeProfileId: string | null;
  language: string;
  evalRecording: boolean;
}

/** update_settings 接受任意局部 patch（后端做深合并） */
export type SettingsPatch = Record<string, unknown>;

export interface AudioDevice {
  id: string;
  name: string;
}

// ---------- STT 引擎 ----------
export interface EngineCapabilities {
  streaming: boolean;
  hotwords: boolean;
  gpu: boolean;
  offline: boolean;
  languages: string[];
}

export interface EngineInfo {
  id: string;
  displayName: string;
  capabilities: EngineCapabilities;
  isReady: boolean;
}

// ---------- 游戏 profile ----------
export interface GameProfile {
  id: string;
  displayName: string;
  processNames: string[];
  windowTitlePatterns: string[];
  openChatKey: string;
  sendKey: string;
  preOpenDelayMs: number;
  prePasteDelayMs: number;
  preSendDelayMs: number;
  preferClipboardPaste: boolean;
  hotwords: string[];
}

// ---------- 模型 ----------
export interface ModelInfo {
  id: string;
  engineId: string;
  sizeBytes: number;
  downloaded: boolean;
}

// ================================================================
// 浏览器 mock：内存态，模拟后端行为（仅 dev:web 使用）
// ================================================================

interface MockStore {
  settings: Settings;
  devices: AudioDevice[];
  engines: EngineInfo[];
  profiles: GameProfile[];
}

const mock: MockStore = {
  settings: {
    hotkey: { key: "F8", mode: "toggle" },
    audioDeviceId: "default",
    sttEngine: "mock-stream",
    engineOptions: {
      "whisper-cpp-sidecar": { model: "ggml-small", threads: 4 },
      "sherpa-onnx-zipformer-zh": { model: "zipformer-zh-small", provider: "cpu" },
    },
    autoSend: false,
    activeProfileId: "lol",
    language: "zh",
    evalRecording: true,
  },
  devices: [
    { id: "default", name: "系统默认（Mock 麦克风）" },
    { id: "mock-mic-1", name: "Mock USB 耳麦" },
  ],
  engines: [
    {
      id: "mock-stream",
      displayName: "Mock 流式引擎（联调用）",
      capabilities: { streaming: true, hotwords: false, gpu: false, offline: true, languages: ["zh"] },
      isReady: true,
    },
    {
      id: "whisper-cpp-sidecar",
      displayName: "whisper.cpp（sidecar）",
      capabilities: { streaming: false, hotwords: true, gpu: false, offline: true, languages: ["zh", "en"] },
      isReady: false,
    },
    {
      id: "sherpa-onnx-zipformer-zh",
      displayName: "sherpa-onnx 流式 Zipformer-zh",
      capabilities: { streaming: true, hotwords: true, gpu: false, offline: true, languages: ["zh"] },
      isReady: false,
    },
  ],
  profiles: [
    {
      id: "generic",
      displayName: "通用（任意前台窗口）",
      processNames: [],
      windowTitlePatterns: [],
      openChatKey: "Enter",
      sendKey: "Enter",
      preOpenDelayMs: 20,
      prePasteDelayMs: 20,
      preSendDelayMs: 20,
      preferClipboardPaste: false,
      hotwords: [],
    },
    {
      id: "lol",
      displayName: "League of Legends",
      processNames: ["League of Legends.exe"],
      windowTitlePatterns: [".*League of Legends.*"],
      openChatKey: "Enter",
      sendKey: "Enter",
      preOpenDelayMs: 20,
      prePasteDelayMs: 20,
      preSendDelayMs: 20,
      preferClipboardPaste: false,
      hotwords: ["闪现", "大龙", "gank", "打野", "推塔", "回城"],
    },
  ],
};

/** 浅层递归合并（对齐后端 settings::merge_json 语义） */
function mergeJson(base: Record<string, unknown>, patch: Record<string, unknown>): void {
  for (const [k, v] of Object.entries(patch)) {
    const cur = base[k];
    if (
      cur !== null &&
      v !== null &&
      typeof cur === "object" &&
      typeof v === "object" &&
      !Array.isArray(cur) &&
      !Array.isArray(v)
    ) {
      mergeJson(cur as Record<string, unknown>, v as Record<string, unknown>);
    } else {
      base[k] = v;
    }
  }
}

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

// ================================================================
// 命令封装（与 lib.rs invoke_handler 注册名一致）
// ================================================================

/** 冒烟测试：验证 IPC 通路 */
export async function ping(): Promise<string> {
  if (!isTauri) return "pong(mock)";
  return invoke<string>("ping");
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri) return clone(mock.settings);
  return invoke<Settings>("get_settings");
}

export async function updateSettings(patch: SettingsPatch): Promise<Settings> {
  if (!isTauri) {
    const merged = clone(mock.settings) as unknown as Record<string, unknown>;
    mergeJson(merged, patch);
    mock.settings = merged as unknown as Settings;
    return clone(mock.settings);
  }
  return invoke<Settings>("update_settings", { patch });
}

export async function listAudioDevices(): Promise<AudioDevice[]> {
  if (!isTauri) return clone(mock.devices);
  return invoke<AudioDevice[]>("list_audio_devices");
}

export async function setAudioDevice(id: string): Promise<void> {
  if (!isTauri) {
    mock.settings.audioDeviceId = id;
    return;
  }
  return invoke<void>("set_audio_device", { id });
}

export async function listSttEngines(): Promise<EngineInfo[]> {
  if (!isTauri) return clone(mock.engines);
  return invoke<EngineInfo[]>("list_stt_engines");
}

export async function setSttEngine(id: string): Promise<void> {
  if (!isTauri) {
    if (!mock.engines.some((e) => e.id === id)) throw new Error(`未注册的 STT 引擎: ${id}`);
    mock.settings.sttEngine = id;
    return;
  }
  return invoke<void>("set_stt_engine", { id });
}

export async function getEngineOptions(id: string): Promise<unknown> {
  if (!isTauri) return mock.settings.engineOptions[id] ?? null;
  return invoke<unknown>("get_engine_options", { id });
}

export async function listProfiles(): Promise<GameProfile[]> {
  if (!isTauri) return clone(mock.profiles);
  return invoke<GameProfile[]>("list_profiles");
}

export async function saveProfile(profile: GameProfile): Promise<void> {
  if (!isTauri) {
    const i = mock.profiles.findIndex((p) => p.id === profile.id);
    if (i >= 0) mock.profiles[i] = clone(profile);
    else mock.profiles.push(clone(profile));
    return;
  }
  return invoke<void>("save_profile", { profile });
}

/** Preview 状态下确认（可带编辑后文本）发送 */
export async function confirmSend(text?: string): Promise<void> {
  if (!isTauri) return;
  return invoke<void>("confirm_send", { text: text ?? null });
}

/** 取消当前会话（任意状态回 Idle） */
export async function cancelSession(): Promise<void> {
  if (!isTauri) return;
  return invoke<void>("cancel_session");
}

/** 手动触发发送（调试/测试用） */
export async function simulateSend(text: string, profileId?: string): Promise<void> {
  if (!isTauri) {
    console.info(`[mock] simulate_send: "${text}" (profile: ${profileId ?? "generic"})`);
    return;
  }
  return invoke<void>("simulate_send", { text, profileId: profileId ?? null });
}
