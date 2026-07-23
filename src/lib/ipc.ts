/*
 * 所有 invoke 调用的类型化封装（docs/development.md §5.3 IPC 契约）。
 * 当前为占位：仅导出类型与空壳函数，业务命令由 Rust 侧后续补齐后逐一对应。
 */

// ---------- 设置与配置 ----------
export interface Settings {
  hotkey: { key: string; mode: "toggle" | "hold" };
  audioDeviceId: string;
  sttEngine: string;
  autoSend: boolean;
  activeProfileId: string | null;
  language: string;
  evalRecording: boolean;
}

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

// TODO: 使用 @tauri-apps/api 的 invoke 实现以下命令封装：
//   get_settings / update_settings / list_audio_devices / set_audio_device
//   list_stt_engines / set_stt_engine / get_engine_options
//   list_profiles / save_profile / detect_foreground_game
//   list_models / download_model / set_active_model
//   eval_list_sessions / eval_replay / eval_export
//   simulate_send
