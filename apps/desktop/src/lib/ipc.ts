/*
 * 所有 invoke 调用的类型化封装（docs/development.md §5.3 IPC 契约）。
 * 与 src-tauri/src/lib.rs 注册的命令一一对应。
 *
 * 浏览器（vite dev:web 纯前端调试）环境下自动降级为内存 mock，
 * 便于脱离 Tauri 调试 UI；Tauri 环境直接走 invoke。
 */

import { invoke } from "@tauri-apps/api/core";
import defaultLolBlocklistCsv from "../../../../crates/kotone-postprocess/assets/lol-zh-cn-starter.csv?raw";

/** 是否运行在 Tauri WebView 中（否则为纯浏览器调试） */
export const isTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// ---------- 设置与配置 ----------
export interface HotkeyConfig {
  key: string;
  mode: "toggle" | "hold";
}

/** 交互模式预设（ADR-006；缺省 null = 由 hotkey.mode + autoSend 旧字段推导） */
export type InteractionMode = "push-to-talk" | "dictation" | "one-shot" | "solo";

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
  /** 是否随记录独立保存音频（不依赖评测录档） */
  includeAudio: boolean;
  /** 自定义历史目录（含音频）；空 = 默认 ~/.kotone/history */
  dir: string;
}

export type PostProcessFailurePolicy = "required" | "best-effort";

export interface PostProcessStepConfig {
  id: string;
  processorId: string;
  enabled: boolean;
  config: unknown;
  timeoutMs: number;
  onError: PostProcessFailurePolicy;
}

export interface PostProcessingConfig {
  enabled: boolean;
  pipeline: {
    id: string;
    steps: PostProcessStepConfig[];
  };
}

export interface PostProcessingTestStep {
  stepId: string;
  processorId: string;
  displayName: string;
  durationMs: number;
  outcome: "succeeded" | "failed";
  error?: string;
}

export interface PostProcessingTestResult {
  sourceText: string;
  finalText: string;
  pipelineId: string;
  durationMs: number;
  steps: PostProcessingTestStep[];
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
  /** 「建议以管理员运行」提示已被用户关闭（不再重复提示） */
  adminPromptDismissed: boolean;
  /** 提权重启成功后的「启动时自动请求管理员权限」询问已被用户关闭（不再提醒） */
  autoAdminPromptDismissed: boolean;
  /** 频道切换热键（默认 Shift+CapsLock；按声明顺序循环当前 profile 的聊天频道） */
  channelCycleHotkey: string;
  /** 重发最近一条热键（默认空 = 关闭；Idle 时重发历史最新一条发送文本） */
  resendLastHotkey: string;
  /** 交互模式预设（null = 旧字段推导） */
  interactionMode: InteractionMode | null;
  /** VAD 静音判停阈值 ms（one-shot 生效，200-5000） */
  vadSilenceMs: number;
  /** silero VAD 内部参数（与 vadSilenceMs 判停阈值独立；one-shot/solo 生效） */
  vad: VadConfig;
  /** 热词命中加分（默认 3.5；越高越易命中热词，也越易把噪声识别成热词） */
  hotwordsScore: number;
  /** STT final 与预览/发送之间的有序后处理 pipeline。 */
  postProcessing: PostProcessingConfig;
  /** 后处理在线连接目录（不含 API key）。 */
  connections: Connection[];
  history: HistoryConfig;
  ui: UiConfig;
  models: ModelsConfig;
  /** 模型下载源配置（auto 默认：ModelScope/镜像优先，失败回退官方） */
  download: DownloadConfig;
  /** 悬浮窗配置 */
  overlay: OverlayConfig;
}

/** 悬浮窗配置（config.json `overlay` 段） */
export interface OverlayConfig {
  /** 显示模式：always 常驻 / on_demand 用时浮现 / never 完全不显示 */
  visibility: "always" | "on_demand" | "never";
  /** 样式：capsule 胶囊（默认）/ card 卡片 */
  style: "card" | "capsule";
  /** 固定位置；custom 为用户拖动后保存的位置。 */
  position:
    | "auto"
    | "top_left"
    | "top_center"
    | "top_right"
    | "center"
    | "bottom_left"
    | "bottom_center"
    | "bottom_right"
    | "custom";
  /** 是否允许直接拖动悬浮窗。 */
  draggable: boolean;
  /** 鼠标事件穿透到游戏；开启后悬浮窗自身不可点击/拖动。 */
  clickThrough: boolean;
  customX?: number;
  customY?: number;
}

/** 模型下载配置（config.json `download` 段） */
export interface DownloadConfig {
  /** 下载源：auto（魔搭/镜像优先+回退）/ official（仅官方）/ mirror（仅镜像） */
  source: "auto" | "official" | "mirror";
  /** GitHub 加速代理前缀（公益代理不稳定，做成可配置） */
  ghProxy: string;
}

/** silero VAD 内部参数（config.json `vad` 段；与 vadSilenceMs 判停阈值独立） */
export interface VadConfig {
  /** 语音判定阈值（0.1-0.9，默认 0.5）；调高 → 噪声更难被误判成语音 */
  threshold: number;
  /** 最短语音时长 ms（20-500，默认 50）；拉长可过滤短促噪声突发 */
  minSpeechMs: number;
  /** 最短静音时长 ms（20-500，默认 50） */
  minSilenceMs: number;
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

export type OnboardingLaunchMode = "auto" | "always" | "never";

export interface StartupOptions {
  onboarding: OnboardingLaunchMode;
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

/** 后端注册表发现到的后处理模块；设置页不应硬编码此列表。 */
export interface PostProcessorInfo {
  id: string;
  displayName: string;
  description: string;
  category: "writing" | "translation" | "utility";
  developerOnly: boolean;
  networkAccess: "none" | "local" | "internet";
  configFields: PostProcessorConfigField[];
}

export interface PostProcessorConfigField {
  key: string;
  displayName: string;
  description: string;
  kind: "text" | "file" | "connection";
  required: boolean;
  fileExtensions: string[];
}

export type ConnectionKind = "remote" | "attach" | "managed";

/** 可持久化的连接公开记录；API key 不在此结构里。 */
export interface Connection {
  id: string;
  displayName: string;
  kind: ConnectionKind;
  provider: string;
  baseUrl: string;
  model: string;
}

export interface ConnectionInfo extends Connection {
  hasApiKey: boolean;
}

export interface ConnectionPreset {
  id: string;
  displayName: string;
  defaultBaseUrl: string;
  defaultModel: string;
}

// ---------- 游戏 profile ----------
/** 聊天频道声明（ADR-008）：按键策略与前缀策略正交，可同时设置 */
export interface ProfileChannel {
  id: string;
  displayName: string;
  /** 该频道专属的开聊天框按键（缺省沿用 profile.openChatKey） */
  openChatKey?: string;
  /** 发送时拼在文本前的前缀（如 "/all "；不污染用户原文） */
  textPrefix?: string;
  /** 默认频道（每 profile 恰好一个；缺省取第一个） */
  default?: boolean;
}

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
  /** 图标文件名（相对 ~/.kotone/profiles/icons/；null = 无图标，UI 用占位） */
  icon?: string | null;
  hotwords: string[];
  /** 用户从内置热词中移除的词条（保留内置集，仅记录差集；可选） */
  removedBuiltinHotwords?: string[];
  /** 聊天频道列表（ADR-008；缺省/空 = 单频道，行为与旧版一致） */
  channels?: ProfileChannel[];
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
  /** 当前激活 profile 的游戏是否处于独占全屏；null = 未运行或无法判断 */
  activeGameFullscreen: boolean | null;
  /** 当前平台是否支持提权方案（非 Windows 为 false） */
  supported: boolean;
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
  /** 已生效的频道切换热键（未配置/未生效为 null） */
  cycleKey: string | null;
  /** 频道切换热键最近一次失败信息（如与录制热键冲突） */
  cycleError: string | null;
  /** 已生效的重发最近一条热键（未配置/未生效为 null） */
  resendKey: string | null;
  /** 重发热键最近一次失败信息（如与录制/频道切换热键冲突） */
  resendError: string | null;
}

/** 录入/启动前的低级键盘钩子与 SendInput 环境自检结果。 */
export interface InputEnvironmentCheck {
  /** false = hook 安装失败或 SendInput 明确少发事件，应提前提示安全软件信任区。 */
  available: boolean;
  /** 合成探测事件是否完整回到本进程的 WH_KEYBOARD_LL。 */
  hookVerified: boolean;
  observed: number;
  expected: number;
  /** 诊断细节；可用但未闭环时也可能存在。 */
  detail: string | null;
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
    hotkey: { key: "CapsLock", mode: "toggle" },
    hotkeyBackend: "auto",
    audioDeviceId: "default",
    sttEngine: "sherpa-onnx-x-asr-zh-en",
    engineOptions: {
      "sherpa-onnx-x-asr-zh-en": { provider: "cpu" },
    },
    autoSend: false,
    activeProfileId: "lol",
    language: "zh",
    evalRecording: false,
    runAsAdminOnStart: false,
    adminPromptDismissed: false,
    autoAdminPromptDismissed: false,
    channelCycleHotkey: "Shift+CapsLock",
    resendLastHotkey: "",
    interactionMode: "push-to-talk",
    vadSilenceMs: 700,
    vad: { threshold: 0.5, minSpeechMs: 50, minSilenceMs: 50 },
    hotwordsScore: 3.5,
    postProcessing: { enabled: false, pipeline: { id: "default", steps: [] } },
    connections: [],
    history: { mode: "capped", maxRecords: 1000, includeAudio: false, dir: "" },
    ui: { firstRunCompleted: true, autoStart: false },
    models: { dir: "" },
    download: { source: "auto", ghProxy: "https://ghfast.top/" },
    overlay: {
      visibility: "on_demand",
      style: "capsule",
      position: "auto",
      draggable: true,
      clickThrough: false,
    },
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
      icon: "lol-kotone.webp",
      hotwords: ["闪现", "大龙", "gank", "打野", "推塔", "回城"],
      channels: [
        { id: "team", displayName: "队伍", default: true },
        { id: "all", displayName: "所有人", openChatKey: "Shift+Enter" },
      ],
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

const mockDefaultBlocklistRules = defaultLolBlocklistCsv
  .replace(/^\uFEFF/, "")
  .split(/\r?\n/)
  .slice(1)
  .filter(Boolean)
  .map((row) => {
    const separator = row.indexOf(",");
    const pattern = separator < 0 ? row : row.slice(0, separator);
    const replacement = separator < 0 ? "" : row.slice(separator + 1);
    return {
      pattern,
      replacement: replacement || "*".repeat(Array.from(pattern).length),
    };
  })
  .sort((left, right) => Array.from(right.pattern).length - Array.from(left.pattern).length);

function applyMockDefaultBlocklist(text: string): string {
  let result = "";
  for (let cursor = 0; cursor < text.length; ) {
    const rule = mockDefaultBlocklistRules.find(({ pattern }) => text.startsWith(pattern, cursor));
    if (rule) {
      result += rule.replacement;
      cursor += rule.pattern.length;
    } else {
      const codePoint = text.codePointAt(cursor);
      if (codePoint === undefined) break;
      const value = String.fromCodePoint(codePoint);
      result += value;
      cursor += value.length;
    }
  }
  return result;
}

function mockQuery(name: string): string | null {
  if (typeof window === "undefined") return null;
  const searchValue = new URLSearchParams(window.location.search).get(name);
  const hashQuery = window.location.hash.includes("?")
    ? window.location.hash.slice(window.location.hash.indexOf("?") + 1)
    : "";
  return new URLSearchParams(hashQuery).get(name) ?? searchValue;
}

// ================================================================
// 命令封装（与 lib.rs invoke_handler 注册名一致）
// ================================================================

/** 冒烟测试：验证 IPC 通路 */
export async function ping(): Promise<string> {
  if (!isTauri) return "pong(mock)";
  return invoke<string>("ping");
}

/** 当前进程的新手向导启动策略（CLI: --onboarding=auto|always|never）。 */
export async function getStartupOptions(): Promise<StartupOptions> {
  if (!isTauri) {
    const searchMode = new URLSearchParams(window.location.search).get("onboarding");
    const hashQuery = window.location.hash.includes("?")
      ? window.location.hash.slice(window.location.hash.indexOf("?") + 1)
      : "";
    const hashMode = new URLSearchParams(hashQuery).get("onboarding");
    const requested = hashMode ?? searchMode;
    return {
      onboarding:
        requested === "always" || requested === "never" || requested === "auto"
          ? requested
          : "auto",
    };
  }
  return invoke<StartupOptions>("get_startup_options");
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri) return clone(mock.settings);
  return invoke<Settings>("get_settings");
}

export async function listPostProcessors(): Promise<PostProcessorInfo[]> {
  if (!isTauri) {
    return [
      {
        id: "writing.openai-compat",
        displayName: "AI 润色",
        description: "用在线大模型去掉口癖、修正口误，并尽量保持原意和游戏术语。",
        category: "writing",
        developerOnly: false,
        networkAccess: "internet",
        configFields: [
          {
            key: "connectionId",
            displayName: "API 连接",
            description: "使用「API 连接」里已保存的在线接口。",
            kind: "connection",
            required: true,
            fileExtensions: [],
          },
        ],
      },
      {
        id: "translation.qwen-mt",
        displayName: "翻译（Qwen-MT）",
        description: "用通义 Qwen-MT 把识别文本译成目标语言，并尽量保住游戏术语。",
        category: "translation",
        developerOnly: false,
        networkAccess: "internet",
        configFields: [
          {
            key: "connectionId",
            displayName: "API 连接",
            description: "请使用通义兼容端点；模型建议 qwen-mt-lite。",
            kind: "connection",
            required: true,
            fileExtensions: [],
          },
          {
            key: "targetLang",
            displayName: "目标语言",
            description: "例如 English、Japanese、Korean。",
            kind: "text",
            required: true,
            fileExtensions: [],
          },
        ],
      },
      {
        id: "builtin.blocklist-filter",
        displayName: "屏蔽词过滤",
        description: "过滤国服对局常见辱骂；可选择自定义 CSV 完整覆盖内置词表。",
        category: "utility",
        developerOnly: false,
        networkAccess: "local",
        configFields: [
          {
            key: "csvPath",
            displayName: "自定义屏蔽词 CSV",
            description: "可选。UTF-8 CSV，每行“屏蔽词,替换词”；第二列留空时替换为等长星号。",
            kind: "file",
            required: false,
            fileExtensions: ["csv"],
          },
        ],
      },
      {
        id: "mock.append-exclamation",
        displayName: "Mock · 句尾叹号",
        description: "在文本末尾追加一个全角叹号，用于验证后处理链路。",
        category: "utility",
        developerOnly: true,
        networkAccess: "none",
        configFields: [],
      },
      {
        id: "mock.wrap-brackets",
        displayName: "Mock · 方括号包裹",
        description: "用全角方括号包裹文本，用于验证多步骤顺序。",
        category: "utility",
        developerOnly: true,
        networkAccess: "none",
        configFields: [],
      },
    ];
  }
  return invoke<PostProcessorInfo[]>("list_post_processors");
}

const mockConnectionPresets: ConnectionPreset[] = [
  {
    id: "dashscope",
    displayName: "通义千问（北京）",
    defaultBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    defaultModel: "qwen-turbo",
  },
  {
    id: "custom",
    displayName: "自定义 OpenAI 兼容",
    defaultBaseUrl: "",
    defaultModel: "",
  },
];

const mockConnectionSecrets = new Map<string, string>();

export async function listConnectionPresets(): Promise<ConnectionPreset[]> {
  if (!isTauri) return mockConnectionPresets.map((item) => ({ ...item }));
  return invoke<ConnectionPreset[]>("list_connection_presets");
}

export async function listConnections(): Promise<ConnectionInfo[]> {
  if (!isTauri) {
    return mock.settings.connections.map((connection) => ({
      ...clone(connection),
      hasApiKey: mockConnectionSecrets.has(connection.id),
    }));
  }
  return invoke<ConnectionInfo[]>("list_connections");
}

export async function upsertConnection(
  connection: Connection,
  apiKey?: string,
): Promise<Settings> {
  if (!isTauri) {
    if (!connection.id.trim() || !connection.displayName.trim() || !connection.baseUrl.trim()) {
      throw new Error("连接名称和接口地址不能为空");
    }
    const existed = mock.settings.connections.some((item) => item.id === connection.id);
    const secret = apiKey?.trim();
    if (!existed && !secret) throw new Error("新建连接必须填写 API key");
    if (secret) mockConnectionSecrets.set(connection.id, secret);
    const next = clone(connection);
    const index = mock.settings.connections.findIndex((item) => item.id === next.id);
    if (index >= 0) mock.settings.connections[index] = next;
    else mock.settings.connections.push(next);
    return clone(mock.settings);
  }
  return invoke<Settings>("upsert_connection", { connection, apiKey: apiKey ?? null });
}

export async function deleteConnection(id: string): Promise<Settings> {
  if (!isTauri) {
    mock.settings.connections = mock.settings.connections.filter((item) => item.id !== id);
    mockConnectionSecrets.delete(id);
    return clone(mock.settings);
  }
  return invoke<Settings>("delete_connection", { id });
}

/**
 * 独立试跑一条 pipeline，不写历史、不触发输入注入，也不受总开关影响。
 * 显式传入 pipeline 可保证结果对应设置页当前展示的步骤快照。
 */
export async function testPostProcessing(
  text: string,
  pipeline?: PostProcessingConfig["pipeline"],
): Promise<PostProcessingTestResult> {
  if (!isTauri) {
    const selected = pipeline ?? mock.settings.postProcessing.pipeline;
    const descriptors = await listPostProcessors();
    const started = performance.now();
    let current = text;
    const steps: PostProcessingTestStep[] = [];

    for (const step of selected.steps.filter((item) => item.enabled)) {
      const descriptor = descriptors.find((item) => item.id === step.processorId);
      if (!descriptor) throw new Error(`未注册后处理模块：${step.processorId}`);
      const stepStarted = performance.now();
      if (step.processorId === "builtin.blocklist-filter") {
        const config = step.config as Record<string, unknown> | null;
        if (typeof config?.csvPath === "string" && config.csvPath.trim()) {
          throw new Error("浏览器试跑无法读取本地 CSV，请在桌面应用中试跑自定义词表");
        }
        current = applyMockDefaultBlocklist(current);
      } else if (step.processorId === "mock.append-exclamation") current += "！";
      else if (step.processorId === "mock.wrap-brackets") current = `【${current}】`;
      else throw new Error(`浏览器试跑暂不支持：${step.processorId}`);
      steps.push({
        stepId: step.id,
        processorId: step.processorId,
        displayName: descriptor.displayName,
        durationMs: Math.round(performance.now() - stepStarted),
        outcome: "succeeded",
      });
    }

    return {
      sourceText: text,
      finalText: current,
      pipelineId: selected.id,
      durationMs: Math.round(performance.now() - started),
      steps,
    };
  }
  return invoke<PostProcessingTestResult>("test_post_processing", { text, pipeline });
}

/** 启动时损坏配置的恢复提示；只返回一次，浏览器演示模式恒为 null。 */
export async function getSettingsLoadWarning(): Promise<string | null> {
  if (!isTauri) return null;
  return invoke<string | null>("get_settings_load_warning");
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

/** 用户拖动悬浮窗后保存当前位置并切换为 custom。 */
export async function saveOverlayPosition(): Promise<Settings> {
  if (!isTauri) {
    mock.settings.overlay.position = "custom";
    return clone(mock.settings);
  }
  return invoke<Settings>("save_overlay_position");
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

/** 导出整包 profile 为 .zip 配置包（profile.json + icon.*） */
export async function exportProfile(profileId: string, path: string): Promise<void> {
  if (!isTauri) {
    console.info(`[mock] export_profile: ${profileId} → ${path}`);
    return;
  }
  return invoke<void>("export_profile", { profileId, path });
}

/** 导入 .zip 配置包：后端生成新随机 id 落盘，返回导入后的 profile */
export async function importProfile(path: string): Promise<GameProfile> {
  if (!isTauri) {
    // dev:web 模拟：从文件名造一条带随机 id 的 profile
    const name = path.split(/[\\/]/).pop() ?? "imported.zip";
    const profile: GameProfile = {
      id: `p-mock-${Math.floor(Math.random() * 1e9)}`,
      displayName: name.replace(/\.zip$/i, "") || "导入的 profile",
      processNames: [],
      windowTitlePatterns: [],
      openChatKey: "Enter",
      sendKey: "Enter",
      preOpenDelayMs: 20,
      prePasteDelayMs: 20,
      preSendDelayMs: 20,
      preferClipboardPaste: false,
      icon: null,
      hotwords: [],
    };
    mock.profiles.push(clone(profile));
    return profile;
  }
  return invoke<GameProfile>("import_profile", { path });
}

/** 删除 profile：内置 = 恢复出厂；导入的 = 永久删除。返回动作类型 */
export async function deleteProfile(profileId: string): Promise<"reset" | "deleted"> {
  if (!isTauri) {
    const i = mock.profiles.findIndex((p) => p.id === profileId);
    if (i >= 0) mock.profiles.splice(i, 1);
    return "deleted";
  }
  const out = await invoke<{ kind: "reset" | "deleted" }>("delete_profile", { profileId });
  return out.kind;
}

/** 读取 profile 图标字节（无图标 → 空）；前端按 icon 扩展名推断 mime 建 Blob URL */
export async function getProfileIcon(profileId: string): Promise<Uint8Array> {
  if (!isTauri) return new Uint8Array(0);
  const bytes = await invoke<number[]>("get_profile_icon", { profileId });
  return new Uint8Array(bytes);
}

/** 游戏兼容状态：自身/游戏提权状态 + 激活 profile 的独占全屏状态 */
export async function getElevationStatus(): Promise<ElevationStatus> {
  if (!isTauri) {
    return {
      elevated: false,
      activeGameElevated: null,
      activeGameFullscreen: null,
      supported: false,
    };
  }
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
    return {
      registered: true,
      key: mock.settings.hotkey.key,
      error: null,
      backend: "llhook",
      cycleKey: mock.settings.channelCycleHotkey,
      cycleError: null,
      resendKey: mock.settings.resendLastHotkey || null,
      resendError: null,
    };
  return invoke<HotkeyStatus>("get_hotkey_status");
}

/** 主动输入环境自检；可在引导页到达热键步骤时调用，不要求先启动录入。 */
export async function checkInputEnvironment(): Promise<InputEnvironmentCheck> {
  if (!isTauri) {
    if (mockQuery("mockInputEnvironment") === "blocked")
      return {
        available: false,
        hookVerified: false,
        observed: 0,
        expected: 2,
        detail: "SendInput 探测事件被系统拦截（0/2 成功）",
      };
    return {
      available: true,
      hookVerified: true,
      observed: 2,
      expected: 2,
      detail: null,
    };
  }
  return invoke<InputEnvironmentCheck>("check_input_environment");
}

// ---------- 资源占用（标题栏运行中展示） ----------

/** get_resource_usage 返回值（camelCase 对齐 serde） */
export interface ResourceUsage {
  /** CPU 占用百分比（依赖后端两次采样间隔，需周期性轮询才有意义） */
  cpuPercent: number;
  /** 进程内存占用（字节） */
  memoryBytes: number;
}

/** 脱敏诊断包导出结果。 */
export interface DiagnosticExportResult {
  reportId: string;
  path: string;
  eventCount: number;
  historyCount: number;
}

/** 当前进程的 CPU / 内存占用（调用方按 ~2s 间隔轮询） */
export async function getResourceUsage(): Promise<ResourceUsage> {
  if (!isTauri) return { cpuPercent: 3.2, memoryBytes: 86 * 1024 * 1024 };
  return invoke<ResourceUsage>("get_resource_usage");
}

/** 导出诊断 ZIP；包内不含录音、识别文本或热词。 */
export async function exportDiagnostics(path: string): Promise<DiagnosticExportResult> {
  if (!isTauri) {
    return {
      reportId: "KT-MOCK",
      path: path.endsWith(".zip") ? path : `${path}.zip`,
      eventCount: 0,
      historyCount: mock.history.length,
    };
  }
  return invoke<DiagnosticExportResult>("export_diagnostics", { path });
}

/** 将前端已处理/未处理异常写入后端持久日志；后端负责脱敏和截断。 */
export async function logFrontendError(context: string, message: string): Promise<void> {
  if (!isTauri) {
    console.warn(`[frontend:${context}]`, message);
    return;
  }
  return invoke<void>("log_frontend_error", { context, message });
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

/** 静态键位冲突提示（向导热键步骤；常见游戏键位表） */
export async function detectHotkeyConflicts(key: string): Promise<string[]> {
  if (!isTauri) return [];
  return invoke<string[]>("detect_hotkey_conflicts", { key });
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
    const engine = mock.engines.find((item) => item.id === mock.settings.sttEngine);
    if (!engine?.isReady) {
      throw new Error("X-ASR 模型未下载。请先下载模型再启动");
    }
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

export function modelsDirPathError(dir: string): string | null {
  return /[^\x00-\x7F]/.test(dir)
    ? "模型存储路径必须使用纯英文路径（可包含英文字母、数字、空格和英文符号），例如 D:\\KotoneModels"
    : null;
}

export async function getModelsDir(): Promise<ModelsDirInfo> {
  if (!isTauri) {
    const dir = mock.settings.models.dir.trim();
    return { dir: dir || "~/.kotone/models (mock)", isDefault: dir === "" };
  }
  return invoke<ModelsDirInfo>("get_models_dir");
}

/** 切换模型目录（后端先迁移旧目录内容再写配置；空串 = 恢复默认） */
export async function setModelsDir(dir: string): Promise<ModelsDirMigration> {
  const pathError = modelsDirPathError(dir.trim());
  if (pathError) throw new Error(pathError);
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

/** 在系统文件管理器中打开历史记录目录（P2-⑨） */
export async function openHistoryDir(): Promise<void> {
  if (!isTauri) {
    console.info("[mock] open_history_dir");
    return;
  }
  return invoke<void>("open_history_dir");
}

/** 在系统默认浏览器打开外部链接（dev:web 直接 window.open 新标签页） */
export async function openExternal(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noreferrer");
    return;
  }
  return invoke<void>("open_external", { url });
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
    if (mockQuery("mockDownload") === "fail") {
      throw new Error("网络连接失败：无法连接模型下载源");
    }
    const m = mock.models.find((x) => x.id === id);
    if (m) {
      m.downloaded = true;
      const engine = mock.engines.find((item) => item.id === m.engineId);
      if (engine) engine.isReady = true;
    }
    return;
  }
  return invoke<void>("download_model", { id });
}

/** 取消进行中的模型下载（幂等；已下载部分保留可续传） */
export async function cancelDownload(): Promise<void> {
  if (!isTauri) {
    console.info("[mock] cancel_download");
    return;
  }
  return invoke<void>("cancel_download");
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

/** 删除单条历史记录（带录音且不再被其他记录引用时一并删除对应 wav；记录不存在幂等） */
export async function deleteHistoryRecord(sessionId: string, ts: string): Promise<void> {
  if (!isTauri) {
    const i = mock.history.findIndex((h) => h.sessionId === sessionId && h.ts === ts);
    if (i >= 0) mock.history.splice(i, 1);
    return;
  }
  return invoke<void>("delete_history_record", { sessionId, ts });
}

/** 读取历史记录随附的音频（相对 history/audio/ 的文件名 → wav 字节） */
export async function readHistoryAudio(fileName: string): Promise<Uint8Array> {
  if (!isTauri) return new Uint8Array(0);
  const bytes = await invoke<number[]>("read_history_audio", { fileName });
  return new Uint8Array(bytes);
}
