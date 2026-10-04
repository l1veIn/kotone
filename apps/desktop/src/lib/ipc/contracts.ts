/**
 * 前后端共享的 IPC 契约及纯配置辅助函数，camelCase 对齐 Rust serde。
 * 此模块不依赖 Tauri、浏览器状态或模拟数据。
 */

// ---------- 设置与配置 ----------
export interface HotkeyConfig {
  key: string;
  mode: "toggle" | "hold";
}

/** 交互模式预设（ADR-006；缺省 null = 由 hotkey.mode + autoSend 旧字段推导） */
export type InteractionMode = "push-to-talk" | "dictation" | "one-shot" | "solo";
export type AssistantInteractionMode = Exclude<InteractionMode, "solo">;

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

export interface PipelineConfig {
  id: string;
  displayName: string;
  steps: PostProcessStepConfig[];
}

export interface PostProcessingConfig {
  enabled: boolean;
  activePipelineId: string;
  pipelines: PipelineConfig[];
}

export function defaultPostProcessing(): PostProcessingConfig {
  return {
    enabled: true,
    activePipelineId: "blocklist",
    pipelines: [
      {
        id: "blocklist",
        displayName: "屏蔽词",
        steps: [
          {
            id: "blocklist-1",
            processorId: "builtin.blocklist-filter",
            enabled: true,
            config: {},
            timeoutMs: 5000,
            onError: "required",
          },
        ],
      },
    ],
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
  /** 当前活动模型；空则按 sttEngine + engineOptions.model 推导 */
  activeModelId: string;
  /** 各模型 configSchema 的用户值（密钥不进这里） */
  modelConfigs: Record<string, Record<string, unknown>>;
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
  /** 文字处理启用开关热键（默认空 = 关闭；按下切换 postProcessing.enabled） */
  togglePostProcessingHotkey: string;
  /** 切换文字处理流程热键（默认空 = 关闭；按下按声明顺序循环流程） */
  cyclePostProcessingHotkey: string;
  /** 启动 / 停止 Kotone 热键（默认空 = 关闭；应用级热键，runtime 停止时也生效） */
  runtimeToggleHotkey: string;
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
  /** 录音/发送音效提示 */
  soundFeedback: SoundFeedbackConfig;
  /** 诊断记录（流程事件）；默认开启 */
  diagnostics: DiagnosticsConfig;
  assistant: AssistantConfig;
}

export interface AssistantConfig {
  hotkey: string;
  /** 助手独立交互模式，不改变常规语音输入模式。 */
  interactionMode: AssistantInteractionMode;
  connectionId: string;
  defaultDelivery: ("voice" | "danmaku")[];
  voiceId: string;
  speechVolume: number;
  speechRate: number;
  speechEngine: "kokoro" | "system";
  kokoroVoiceId: string;
  danmakuDurationMs: number;
  danmakuSpeedPxPerSec: number;
  danmakuFontSize: number;
  /** 距当前屏幕顶部的逻辑/CSS像素；实际呈现会限制在屏幕内。 */
  danmakuTopOffsetPx: number;
  danmakuColor: string;
  danmakuEmote: "none" | "hello" | "cheering" | "pointing" | "amazed";
}

export interface DiagnosticsConfig {
  /** 是否写入流程事件。默认 true */
  recording: boolean;
}

/** 音效提示配置（config.json `soundFeedback` 段） */
export interface SoundFeedbackConfig {
  /** 录制音：按下热键开始录音时播放 */
  record: SoundFeedbackItem;
  /** 发送音：消息注入游戏成功时播放 */
  send: SoundFeedbackItem;
}

/** 单个音效（录制/发送）配置 */
export interface SoundFeedbackItem {
  /** 是否启用（默认 true） */
  enabled: boolean;
  /** 音量百分比 0-100（默认 60） */
  volume: number;
  /** 内置音效 id：record 用 rise/ding-up/chirp-up；send 用 fall/knock/ding-down */
  soundId: string;
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
/** Deep merge objects; arrays are replaced as a whole by the backend. */
type Patch<T> = T extends unknown[] ? T : T extends object ? { [K in keyof T]?: Patch<T[K]> } : T;

export type SettingsPatch = Patch<Settings>;

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
  placeholder?: string;
  presets?: { id: string; displayName: string; value: string }[];
  compatibleProviders?: string[];
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
  apiKeyUrl?: string | null;
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
export interface ModelConfigOption {
  value: string;
  label: string;
}

export interface ModelConfigField {
  key: string;
  label: string;
  kind: "string" | "enum" | "secret" | "connection" | string;
  default: string;
  options: ModelConfigOption[];
  required: boolean;
}

export interface ModelInfo {
  id: string;
  engineId: string;
  /** 展示名（清单内置文案） */
  displayName: string;
  sizeBytes: number;
  downloaded: boolean;
  io: "streaming" | "offline";
  backend: "sherpa" | "remote";
  recipe: string;
  configSchema: ModelConfigField[];
  /** 获取 API key 的控制台页面（在线模型） */
  apiKeyUrl?: string | null;
  /** 产品分组标签（不是引擎 id），例如 online-asr */
  tag?: string | null;
}

/** 自动下载失败后的手动安装指引 */
export interface ModelInstallFile {
  name: string;
  url: string;
  sizeBytes: number;
}

export interface ModelInstallGuide {
  id: string;
  displayName: string;
  destDir: string;
  destSubdir: string;
  pageUrl: string;
  files: ModelInstallFile[];
}

export function isDownloadCancelled(error: unknown): boolean {
  const text =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : error !== null && typeof error === "object" && "message" in error
          ? String((error as { message?: unknown }).message ?? "")
          : String(error);
  return text.includes("下载已取消");
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
  sourceText?: string | null;
  processDurationMs?: number | null;
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

/** 单个具名辅助热键的注册状态（未配置 / 未生效字段为 null） */
export interface NamedHotkeyStatus {
  /** 具名热键稳定 id（与设置页各分区对应） */
  id: string;
  /** 已生效组合；未配置 / 未生效为 null */
  key: string | null;
  /** 最近一次失败信息（如与录制/其他具名热键冲突；成功/未配置为 null） */
  error: string | null;
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
  /** 具名辅助热键状态（含未配置项；按声明顺序稳定输出） */
  named: NamedHotkeyStatus[];
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
  window: "24h" | "7d" | "all";
}

export type DiagnosticWindow = "24h" | "7d" | "all";

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

// ---------- 模型目录管理 ----------

export function modelsDirPathError(dir: string): string | null {
  return /[^\x00-\x7F]/.test(dir)
    ? "模型存储路径必须使用纯英文路径（可包含英文字母、数字、空格和英文符号），例如 D:\\KotoneModels"
    : null;
}

// ---------- 模型下载（引擎与模型页） ----------

/** `kotone://download` 事件 payload：模型/运行时下载进度 */
export interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number;
}
