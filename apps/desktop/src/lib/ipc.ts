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

/** 模型存储配置（core settings `models` 段） */
export interface ModelsConfig {
  /** 自定义模型目录；空 = 默认 ~/.kotone/models */
  dir: string;
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
  models: ModelsConfig;
  /** 模型下载源配置（auto 默认：镜像优先失败回退官方） */
  download: DownloadConfig;
}

/** 模型下载配置（config.json `download` 段） */
export interface DownloadConfig {
  /** 下载源：auto（镜像优先+回退）/ official（仅官方）/ mirror（仅镜像） */
  source: "auto" | "official" | "mirror";
  /** GitHub 加速代理前缀（公益代理不稳定，做成可配置） */
  ghProxy: string;
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

// ---------- 模型目录管理 ----------
export interface ModelsDirInfo {
  dir: string;
  isDefault: boolean;
}

export interface ModelsDirMigration {
  dir: string;
  /** 成功移动的条目名 */
  moved: string[];
  /** 移动失败的条目名（需重新下载） */
  failed: string[];
}

export interface DeleteOutcome {
  /** 被删的是引擎当前活动模型（active 标记已清除，回退默认） */
  wasActive: boolean;
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
  /** 展示名（清单内置文案） */
  displayName: string;
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

/** import_hotwords 返回值（合并报告） */
export interface HotwordMergeReport {
  added: number;
  duplicates: number;
  total: number;
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
      "sherpa-onnx-x-asr-zh-en": { provider: "cpu" },
    },
    autoSend: false,
    activeProfileId: "lol",
    language: "zh",
    evalRecording: false,
    runAsAdminOnStart: false,
    interactionMode: null,
    vadSilenceMs: 700,
    history: { mode: "capped", maxRecords: 1000, includeAudio: false },
    ui: { firstRunCompleted: true, autoStart: false },
    models: { dir: "" },
    download: { source: "auto", ghProxy: "https://ghfast.top/" },
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
      id: "sherpa-onnx-x-asr-zh-en",
      displayName: "sherpa-onnx X-ASR 流式中英标点",
      capabilities: { streaming: true, hotwords: true, gpu: false, offline: true, languages: ["zh", "en"] },
      isReady: false,
    },
    {
      id: "sherpa-onnx-sensevoice",
      displayName: "sherpa-onnx SenseVoice 多语言",
      capabilities: { streaming: false, hotwords: false, gpu: false, offline: true, languages: ["zh", "en", "ja", "ko", "yue"] },
      isReady: false,
    },
    {
      id: "sherpa-onnx-funasr-nano",
      displayName: "sherpa-onnx FunASR-Nano 中英日",
      capabilities: { streaming: false, hotwords: true, gpu: false, offline: true, languages: ["zh", "en", "ja"] },
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
    {
      id: "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05",
      engineId: "sherpa-onnx-x-asr-zh-en",
      displayName: "X-ASR 流式中英标点（int8，480ms 低延迟）",
      sizeBytes: 169_000_000,
      downloaded: false,
    },
    {
      id: "sense-voice-zh-en-ja-ko-yue-2024-07-17",
      engineId: "sherpa-onnx-sensevoice",
      displayName: "sherpa SenseVoice 多语言（int8，非流式高准）",
      sizeBytes: 240_000_000,
      downloaded: false,
    },
    {
      id: "funasr-nano-int8-2025-12-30",
      engineId: "sherpa-onnx-funasr-nano",
      displayName: "FunASR-Nano 中英日（int8，非流式）",
      sizeBytes: 1_010_000_000,
      downloaded: false,
    },
    {
      id: "silero-vad",
      engineId: "vad-silero",
      displayName: "silero VAD 语音活动检测（one-shot 静音判停用）",
      sizeBytes: 644_000,
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

/** 导出 profile 热词到 UTF-8 文本（每行一词条），返回条数 */
export async function exportHotwords(profileId: string, path: string): Promise<number> {
  if (!isTauri) {
    const p = mock.profiles.find((x) => x.id === profileId);
    console.info(`[mock] export_hotwords: ${p?.hotwords.length ?? 0} 条 → ${path}`);
    return p?.hotwords.length ?? 0;
  }
  return invoke<number>("export_hotwords", { profileId, path });
}

/** 从 UTF-8 文本导入热词（合并去重），返回合并报告 */
export async function importHotwords(profileId: string, path: string): Promise<HotwordMergeReport> {
  if (!isTauri) return { added: 0, duplicates: 0, total: 0 };
  return invoke<HotwordMergeReport>("import_hotwords", { profileId, path });
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
    (engineId === "sherpa-onnx-x-asr-zh-en"
      ? "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
      : "default");
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

// ---------- 模型目录管理 ----------

export async function getModelsDir(): Promise<ModelsDirInfo> {
  if (!isTauri) {
    const dir = mock.settings.models.dir.trim();
    return { dir: dir || "~/.kotone/models (mock)", isDefault: dir === "" };
  }
  return invoke<ModelsDirInfo>("get_models_dir");
}

/** 切换模型目录（后端先迁移旧目录内容再写配置；空串 = 恢复默认） */
export async function setModelsDir(dir: string): Promise<ModelsDirMigration> {
  if (!isTauri) {
    mock.settings.models.dir = dir;
    return { dir: dir || "~/.kotone/models (mock)", moved: [], failed: [] };
  }
  return invoke<ModelsDirMigration>("set_models_dir", { dir });
}

/** 删除已下载模型/运行时；active 模型被删时回退默认 */
export async function deleteModel(id: string): Promise<DeleteOutcome> {
  if (!isTauri) {
    const m = mock.models.find((x) => x.id === id);
    if (m) m.downloaded = false;
    return { wasActive: false };
  }
  return invoke<DeleteOutcome>("delete_model", { id });
}

/** 在系统文件管理器中打开模型目录 */
export async function openModelsDir(): Promise<void> {
  if (!isTauri) {
    console.info("[mock] open_models_dir");
    return;
  }
  return invoke<void>("open_models_dir");
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

/** 切换引擎的活动模型（Running 时置 restartNeeded，不自动重启） */
export async function setActiveModel(engineId: string, modelId: string): Promise<void> {
  if (!isTauri) {
    const opts = (mock.settings.engineOptions[engineId] ??= {}) as Record<string, unknown>;
    opts.model = modelId;
    return;
  }
  return invoke<void>("set_active_model", { engineId, modelId });
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
