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

/** 交互模式预设（ADR-006；缺省 null = 由 hotkey.mode + autoSend 旧字段推导） */
export type InteractionMode = "push-to-talk" | "dictation" | "one-shot";

/** 桌面壳 UI 状态（core settings `ui` 段） */
export interface UiConfig {
  /** 首启向导已完成（或已跳过） */
  firstRunCompleted: boolean;
  /** app 启动后自动进入 Running（warmup 引擎 + 注册热键 + 显示悬浮窗） */
  autoStart: boolean;
}

/** 识别历史配置（core history 模块） */
export interface HistoryConfig {
  /** capped 只留最近 maxRecords 条 / keep-all 全留 / off 不记录 */
  mode: "capped" | "keep-all" | "off";
  maxRecords: number;
  /** 是否随记录保存音频（从 eval 录档复制 wav） */
  includeAudio: boolean;
}

export interface Settings {
  hotkey: HotkeyConfig;
  /** 热键后端：auto（Windows 优先 LL 钩子）/ llhook / register */
  hotkeyBackend: "auto" | "llhook" | "register";
  audioDeviceId: string;
  sttEngine: string;
  engineOptions: Record<string, unknown>;
  autoSend: boolean;
  activeProfileId: string | null;
  language: string;
  evalRecording: boolean;
  /** 启动时自动以管理员重启自身（UIPI 方案，默认关） */
  runAsAdminOnStart: boolean;
  /** 交互模式预设（null = 旧字段推导） */
  interactionMode: InteractionMode | null;
  /** VAD 静音判停阈值 ms（one-shot 生效，200-5000） */
  vadSilenceMs: number;
  history: HistoryConfig;
  ui: UiConfig;
}

// ---------- 运行时「启动」开关 ----------
export type RuntimePhase = "stopped" | "starting" | "running" | "stopping";

/** get_runtime_status 返回值；kotone://runtime 事件同构（全量推送） */
export interface RuntimeStatus {
  phase: RuntimePhase;
  restartNeeded: boolean;
  engineId: string | null;
  engineName: string | null;
  modelId: string | null;
  interactionMode: InteractionMode | null;
  /** 过渡阶段提示（warmup / hotkey / overlay / unload），稳态为 null */
  stage: string | null;
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

// ---------- 识别历史（core history 模块，camelCase 对齐 serde） ----------
export interface HistoryRecord {
  /** 与 eval 录档互查的会话 id */
  sessionId: string;
  /** 终态落账时间（ISO 8601 UTC） */
  ts: string;
  engineId: string;
  profileId: string | null;
  finalText: string;
  audioMs: number;
  firstPartialMs: number | null;
  finalizeLatencyMs: number | null;
  outcome: "sent" | "cancelled" | "error";
  error: string | null;
  /** 相对 history/audio/ 的文件名；无音频为 null */
  audioFile: string | null;
}

// ---------- 提权（UIPI 方案，docs/development.md §10 R-1） ----------

/** detect_foreground_game 返回值：profile 字段平铺 + 目标进程提权状态 */
export interface ForegroundGameInfo extends GameProfile {
  /** null = 无法判断（进程未运行 / 句柄失败） */
  targetElevated: boolean | null;
}

/** get_elevation_status 返回值 */
export interface ElevationStatus {
  /** Kotone 自身是否已提权 */
  elevated: boolean;
  /** 当前激活 profile 的游戏进程是否提权；null = 无法判断 */
  activeGameElevated: boolean | null;
}

/** get_hotkey_status 返回值：热键注册状态（多实例/占用冲突诊断） */
export interface HotkeyStatus {
  /** 当前是否处于已注册状态 */
  registered: boolean;
  /** 当前注册的热键；未注册为 null */
  key: string | null;
  /** 最近一次注册失败信息（成功后清空） */
  error: string | null;
  /** 当前生效后端：llhook（低级键盘钩子）/ register（RegisterHotKey）/ none */
  backend: string;
}

// ================================================================
// 浏览器 mock：内存态，模拟后端行为（仅 dev:web 使用）
// ================================================================

interface MockStore {
  settings: Settings;
  devices: AudioDevice[];
  engines: EngineInfo[];
  profiles: GameProfile[];
  models: ModelInfo[];
  history: HistoryRecord[];
}

const mock: MockStore = {
  settings: {
    hotkey: { key: "F8", mode: "toggle" },
    hotkeyBackend: "auto",
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
    runAsAdminOnStart: false,
    interactionMode: null,
    vadSilenceMs: 700,
    history: { mode: "capped", maxRecords: 1000, includeAudio: false },
    ui: { firstRunCompleted: true, autoStart: false },
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
  models: [
    { id: "ggml-small", engineId: "whisper-cpp-sidecar", sizeBytes: 466_000_000, downloaded: true },
    {
      id: "zipformer-bilingual-zh-en-2023-02-20",
      engineId: "sherpa-onnx-zipformer-zh",
      sizeBytes: 158_000_000,
      downloaded: false,
    },
  ],
  history: [
    {
      sessionId: "20260725-101443-736",
      ts: "2026-07-25T10:14:43Z",
      engineId: "mock-stream",
      profileId: "lol",
      finalText: "对面打野在下路",
      audioMs: 2465,
      firstPartialMs: 312,
      finalizeLatencyMs: 60,
      outcome: "sent",
      error: null,
      audioFile: null,
    },
    {
      sessionId: "20260725-100524-711",
      ts: "2026-07-25T10:05:24Z",
      engineId: "mock-stream",
      profileId: null,
      finalText: "",
      audioMs: 1200,
      firstPartialMs: null,
      finalizeLatencyMs: null,
      outcome: "cancelled",
      error: null,
      audioFile: null,
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

/** Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本） */
export async function confirmSend(): Promise<void> {
  if (!isTauri) return;
  return invoke<void>("confirm_send");
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

// ---------- 提权（UIPI 方案） ----------

/** 检测当前前台游戏并匹配 profile（附带目标进程提权状态） */
export async function detectForegroundGame(): Promise<ForegroundGameInfo | null> {
  if (!isTauri) return { ...clone(mock.profiles[1]), targetElevated: null };
  return invoke<ForegroundGameInfo | null>("detect_foreground_game");
}

/** 提权状态：自身是否提权 + 激活 profile 的游戏进程是否提权 */
export async function getElevationStatus(): Promise<ElevationStatus> {
  if (!isTauri) return { elevated: false, activeGameElevated: null };
  return invoke<ElevationStatus>("get_elevation_status");
}

/** 以管理员身份重启（弹 UAC；成功后当前进程退出，调用方无需后续操作） */
export async function restartAsAdmin(): Promise<void> {
  if (!isTauri) {
    console.info("[mock] restart_as_admin");
    return;
  }
  return invoke<void>("restart_as_admin");
}

/** 热键注册状态（registered/key/error/backend），设置页热键分区展示占用冲突 */
export async function getHotkeyStatus(): Promise<HotkeyStatus> {
  if (!isTauri)
    return { registered: true, key: mock.settings.hotkey.key, error: null, backend: "llhook" };
  return invoke<HotkeyStatus>("get_hotkey_status");
}

// ---------- 热键录入捕获（ADR-006） ----------

/** `kotone://hotkey-capture` 事件 payload：三选一 */
export interface HotkeyCaptureEvent {
  /** 捕获到的组合键（如 "Ctrl+Alt+V"） */
  combo?: string;
  /** 用户按 Esc 或调用方取消 */
  cancelled?: boolean;
  /** 超时未按键 */
  timeout?: boolean;
}

/** 开始热键捕获（设置页「点击录入」）；结果经 kotone://hotkey-capture 事件推送 */
export async function startHotkeyCapture(): Promise<void> {
  if (!isTauri) throw new Error("浏览器调试环境不支持热键录入");
  return invoke<void>("start_hotkey_capture");
}

/** 取消进行中的热键捕获（组件销毁/重新点击的兜底） */
export async function cancelHotkeyCapture(): Promise<void> {
  if (!isTauri) return;
  return invoke<void>("cancel_hotkey_capture");
}

// ---------- 运行时「启动」开关 ----------

/** mock 运行时：启动快照（推导 restartNeeded 用；对齐壳侧语义） */
let mockStarted: { engineId: string; modelId: string } | null = null;
let mockPhase: RuntimePhase = "stopped";

/** mock 的 restartNeeded 推导：Running 且快照 ≠ 当前配置 */
function mockRuntimeStatus(stage: string | null = null): RuntimeStatus {
  const s = mock.settings;
  const engineId = s.sttEngine;
  const modelId =
    (s.engineOptions[engineId] as Record<string, unknown> | undefined)?.model as string ??
    (engineId === "sherpa-onnx-zipformer-zh" ? "zipformer-bilingual-zh-en-2023-02-20" : "ggml-small");
  const restartNeeded =
    mockPhase === "running" &&
    mockStarted !== null &&
    (mockStarted.engineId !== engineId || mockStarted.modelId !== modelId);
  return {
    phase: mockPhase,
    restartNeeded,
    engineId,
    engineName: mock.engines.find((e) => e.id === engineId)?.displayName ?? null,
    modelId,
    interactionMode: s.interactionMode,
    stage,
  };
}

export async function getRuntimeStatus(): Promise<RuntimeStatus> {
  if (!isTauri) return mockRuntimeStatus();
  return invoke<RuntimeStatus>("get_runtime_status");
}

/** 启动：warmup 引擎 → 注册热键 → 显示悬浮窗；Running+restartNeeded 时等价重启 */
export async function startRuntime(): Promise<RuntimeStatus> {
  if (!isTauri) {
    mockPhase = "running";
    const s = mockRuntimeStatus();
    mockStarted = { engineId: s.engineId ?? "", modelId: s.modelId ?? "" };
    return mockRuntimeStatus();
  }
  return invoke<RuntimeStatus>("start_runtime");
}

/** 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎；Stopped 幂等 */
export async function stopRuntime(): Promise<RuntimeStatus> {
  if (!isTauri) {
    mockPhase = "stopped";
    mockStarted = null;
    return mockRuntimeStatus();
  }
  return invoke<RuntimeStatus>("stop_runtime");
}

// ---------- 模型下载（引擎与模型页） ----------

/** `kotone://download` 事件 payload：模型/运行时下载进度 */
export interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number;
}

/** 全部引擎的模型清单（downloaded 标记） */
export async function listModels(): Promise<ModelInfo[]> {
  if (!isTauri) return clone(mock.models);
  return invoke<ModelInfo[]>("list_models");
}

/** 下载模型/运行时（进度经 kotone://download 事件推送：{ id, downloaded, total }） */
export async function downloadModel(id: string): Promise<void> {
  if (!isTauri) {
    const m = mock.models.find((x) => x.id === id);
    if (m) m.downloaded = true;
    return;
  }
  return invoke<void>("download_model", { id });
}

// ---------- 识别历史（历史记录页） ----------

/** 识别历史列表（新→旧） */
export async function getHistory(): Promise<HistoryRecord[]> {
  if (!isTauri) return clone(mock.history);
  return invoke<HistoryRecord[]>("get_history");
}

/** 清空全部识别历史（含音频文件；调用方负责二次确认） */
export async function clearHistory(): Promise<void> {
  if (!isTauri) {
    mock.history = [];
    return;
  }
  return invoke<void>("clear_history");
}
