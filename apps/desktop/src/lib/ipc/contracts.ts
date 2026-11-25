/**
 * 前后端共享的 IPC 契约及纯配置辅助函数，camelCase 对齐 Rust serde。
 * 此模块不依赖 Tauri、浏览器状态或模拟数据。
 */

import { m } from "../paraglide/messages.js";

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
  legacyUpgrade?: "notNeeded" | "pending" | "dismissed" | "optedIn";
  /** 打开 Kotone 时自动启动本地语音识别（在线识别打开即就绪，不看这一项）。 */
  autoStart: boolean;
  /** 新版本弹窗里选了「不再提醒」：启动时不自动弹出（关于页仍可手动检查） */
  updatePromptDisabled?: boolean;
  /** 打开软件时显示：`voiceInput` = 语音输入；空或其它任何值 = 聊天（ADR 028）。 */
  startPage?: string;
  /** 左侧栏自己排的顺序和隐藏的页面（ADR 034 第六节；空 = 默认）。 */
  sidebar?: { order: string[]; hidden: string[] };
}

export type StartPage = "chat" | "voiceInput";

/** 打开软件时落在哪：只认 `voiceInput`，其它（含以后新加、这一版不认识的值）都按聊天。 */
export function startPageOf(ui: UiConfig | undefined): StartPage {
  return ui?.startPage === "voiceInput" ? "voiceInput" : "chat";
}

export function usesLegacyVoiceInput(ui: UiConfig | undefined): boolean {
  return ui?.legacyUpgrade === "pending" || ui?.legacyUpgrade === "dismissed";
}

export function hasLegacyReminders(ui: UiConfig | undefined): boolean {
  return usesLegacyVoiceInput(ui) || ui?.legacyUpgrade === "optedIn";
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
        // i18n-ignore-start：默认流程名，存进配置（和后端的默认一致）
        displayName: "屏蔽词",
        // i18n-ignore-end
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
  lowerSpeakerVolumeWhileListening: boolean;
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
  /** 停止全部自动化（应用级热键；默认 Ctrl+Shift+F12，空 = 关闭） */
  automationStopHotkey: string;
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
  /** 本机 Agent 工具服务开关；桌面端启动时读取，默认关闭（docs/tool-service.md） */
  toolService: ToolServiceConfig;
  /** 当前 OCR 模型（统一模型清单 category=ocr；空 = 自动选第一个已安装的本地模型） */
  ocr: OcrConfig;
  /** 联网搜索（统一模型清单 category=search；空 = 自动：有 DeepSeek 连接就用 DeepSeek；off = 关）。旧配置没有时按默认补齐。 */
  search?: { activeModelId: string };
  /** 内置琴音 Agent（config.json `agent` 段；旧配置没有这一段时后端按默认补齐）。 */
  agent?: AgentConfig;
  /**
   * 内置浏览器（ADR 031；「设置 → 浏览器」，ADR 038）：`enabled` 是开关（新字段，旧配置没有时按开着读），
   * `extensions` 是玩家加的扩展（只经 browser_extension_* 命令改）。
   */
  browser?: BrowserSettings;
  /**
   * 插件（ADR 038）：每个插件一份，按插件 id。没写的项按插件清单的默认（旧的 `lol.enabled` 后端读进来时
   * 搬到 `plugins.lol.sources.client`；旧的 `voiceChat` 搬到 `plugins.game-voice`；旧的 `music`（音质、音量）
   * 搬到 `plugins.music.settings`）。
   */
  plugins?: Record<string, { enabled?: boolean; sources?: Record<string, boolean>; settings?: Record<string, unknown> }>;
  /**
   * 热键表（ADR 040 D8，后端 `kotone_core::hotkey_bindings`）。热键的旧字段（`hotkey.key`、`channelCycleHotkey`、
   * 呼叫键……）是它的镜像，后端每次保存照表写回；改旧字段也会改进表里。
   */
  hotkeys?: HotkeysConfig;
}

/** 表里的一条：`key` 加上 `command`（`<来源>.<命令 id>`）/ `tool` + `args` / `page` 三选一；读不懂的字段原样保留。 */
export interface HotkeyBinding {
  key: string;
  command?: string;
  tool?: string;
  args?: Record<string, unknown>;
  page?: string;
  [field: string]: unknown;
}

export interface HotkeysConfig {
  bindings: HotkeyBinding[];
}

export interface OcrConfig {
  activeModelId: string;
}

/** config.json 的 `browser` 段（Rust `BrowserConfig`）。 */
export interface BrowserSettings {
  /** 内置浏览器开着没有（默认开）。关着时右侧栏没有“网页”，她也没有 `browser.*`。 */
  enabled: boolean;
  extensions?: { id: string; path: string; name: string; enabled: boolean; installedId?: string }[];
  pendingUninstall?: string[];
}

/** 内置浏览器开着没有：旧配置没有这个字段时按开着（和后端一样）。 */
export function browserEnabled(settings: Pick<Settings, "browser"> | null | undefined): boolean {
  return settings?.browser?.enabled !== false;
}

export interface AssistantBindings {
  hotkey: string;
  connectionId: string;
}

export interface AssistantConfig extends AssistantBindings {
  /** 老用户主动启用琴音时单独设置，保留原提醒助手热键和连接。 */
  companionBindings?: AssistantBindings | null;
  /** 老配置补过默认呼叫键的标记（只补一次）。 */
  callHotkeyDefaulted?: boolean;
  /** 默认呼叫键的版本（1 = Ctrl+Backquote）；迁移标记，界面不用。 */
  callHotkeyRevision?: number;
  /** 旧的提醒助手热键方式；呼叫琴音固定为说完自动提交，只剩旧的窗口语音按钮在用。 */
  interactionMode: AssistantInteractionMode;
  defaultDelivery: ("voice" | "danmaku")[];
  voiceId: string;
  speechVolume: number;
  speechRate: number;
  /** 本地播报引擎（旧字段，只写旧版本认识的 system / kokoro） */
  speechEngine: "kokoro" | "system";
  /**
   * 语音合成：空 = 自动（默认：琴音用 DeepSeek 连接时用 DeepSeek 语音，否则 speechEngine）；
   * "local" = 明确用本地 speechEngine；deepseek-tts / openai-compat-tts / volcano-tts = 在线。
   */
  ttsModelId?: string;
  /** 语音唤醒（ADR 017；新字段，旧配置没有）。 */
  wake?: WakeConfig;
  /** 琴音的对话气泡（0.2.0 新字段，旧配置没有）。 */
  bubble?: BubbleConfig;
  /** 回答完接着听几秒（ADR 029）。ADR 033 起不再使用，只为旧版本保留。 */
  followUpSeconds?: number;
  /** 通话多久没人说话挂断，0 = 不通话（ADR 033 新字段，旧配置没有 = 30）。 */
  callIdleSeconds?: number;
  /** 通话里做回声消除：开着音箱也能开口打断她（ADR 033 新字段，旧配置没有 = true）。 */
  echoCancellation?: boolean;
  kokoroVoiceId: string;
  danmakuDurationMs: number;
  danmakuSpeedPxPerSec: number;
  danmakuFontSize: number;
  /** 距当前屏幕顶部的逻辑/CSS像素；实际呈现会限制在屏幕内。 */
  danmakuTopOffsetPx: number;
  danmakuColor: string;
  danmakuEmote: "none" | "hello" | "cheering" | "pointing" | "amazed";
  /** 请求去向：companion 为 Kotone 陪玩（默认；旧值 reminders 视同 companion）；agent 原文转交外部 Agent（需开启工具服务）。 */
  route?: "reminders" | "companion" | "agent";
  /** 陪玩：快捷消息、帮忙打字是否自动发送、AI 连接的测试结果（只用于显示） */
  companion: CompanionConfig;
}

/** 快捷指令（字段名沿用快捷消息，最多 8 条）：按下热键，要么等于替用户说了 text（交给 AI），
 *  要么直接运行已保存的自动化 automationId（带 args，不调用 AI）。 */
export interface QuickMessage {
  id: string;
  label: string;
  text: string;
  hotkey: string;
  /** 要直接运行的已保存自动化；空 / 缺省 = 交给 AI 的一句话 */
  automationId?: string;
  args?: Record<string, unknown>;
}

export interface CompanionCheck {
  connectionId: string;
  baseUrl: string;
  model: string;
  passed: boolean;
  checkedAtMs: number;
  message: string;
}

export interface CompanionConfig {
  quickMessages: QuickMessage[];
  /** 帮忙打字后自动按发送键（默认 true；关掉后只打进聊天框） */
  autoSend: boolean;
  check?: CompanionCheck | null;
}

/** 陪玩是否可用（companion_status） */
export interface CompanionStatus {
  ready: boolean;
  reason: string | null;
  connectionId: string;
  check: CompanionCheck | null;
  ocr: { modelId: string | null; state: { ready: boolean; reason?: string | null; backend?: string; dataEgress?: string } };
  autoSend: boolean;
  quickMessages: QuickMessage[];
  profile: { id: string; notes: string; chatRegion: ChatRegion | null };
}

export interface ChatRegion {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 内置琴音 Agent 的设置（ADR 014 / 015）。权限与隐藏列表要用专门的命令整体修改（见 lib/agent.ts）。 */
export interface AgentConfig {
  permissions: { mode: "default" | "allow-all" | "read-only"; rules: Record<string, "allow" | "ask" | "deny"> };
  /** 不交给 AI 的已保存自动化 id。 */
  hiddenAutomations: string[];
  /** 呼叫键与快捷指令交给琴音（默认开；关闭时用旧的单次陪玩）。 */
  inGame: boolean;
  /** 游戏内交给琴音后 1.5 秒还没出声，先说一句“好，我看看”（默认开）。 */
  progressSpeech?: boolean;
  /** 名字与头像（空 = “琴音”与内置形象）。头像是工作区里的文件名，用 workspaceSetAvatar 上传。 */
  identity?: { name: string; avatar: string };
  /** 子会话（同传等）用的模型 id；空 = 和琴音同一个模型。 */
  subagents?: { model: string };
  /** 一段对话估到这么多 token 时整理成摘要（上下文压缩）；0 = 不整理。默认 64000。 */
  compactAtTokens?: number;
  /** 当前角色（默认 kotone = 琴音）。换角色用 lib/roles.ts 的 rolesSwitch（同时换名字、头像）。 */
  roleId?: string;
  /** 游戏退出时自动复盘（默认开）。 */
  matchRecap?: boolean;
  /** DeepSeek 内容审核（默认开；ADR 026）。 */
  safetyCheck?: boolean;
  /** 游戏里 / 聊天窗口里的思考强度（只对 DeepSeek）：low / high / max / none；空 = 服务商默认。 */
  gameEffort?: string;
  chatEffort?: string;
  /** 自动卡片和消息提示（ADR 039 D3、D4；默认都开，旧配置没有这一项）。 */
  autoCards?: { enabled: boolean; popup: boolean };
}

/** 语音唤醒设置（config.json `assistant.wake`）。 */
export type BubbleCorner = "bottom-right" | "bottom-left" | "top-right" | "top-left";

export interface BubbleConfig {
  enabled: boolean;
  corner: BubbleCorner;
  /** 离屏幕上下边缘再挪开多少逻辑像素（0–800） */
  offsetY: number;
  /** 回答同时用弹幕 */
  alsoDanmaku: boolean;
}

export interface WakeConfig {
  enabled: boolean;
  phrase: "qinyin-qinyin" | "nihao-qinyin";
  sensitivity: "low" | "normal" | "high";
  ack: "voice" | "tone" | "none";
  /** 唤醒后一直不说话多久就结束（毫秒，2000–15000） */
  noSpeechTimeoutMs: number;
}

/** 本机工具服务配置（config.json `toolService` 段）；修改后需重启桌面端 */
export interface ToolServiceConfig {
  enabled: boolean;
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

/** 当前进程对悬浮窗显示异常采取的自动降级。 */
export interface OverlayHealth {
  degraded: boolean;
  reason: string | null;
}

/** 新创建的窗口补读最近一条状态，恢复预览或错误内容。 */
export interface OverlayStateSnapshot {
  state: string;
  payload: unknown;
  caseId?: string;
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
  /** 当前语音识别是本地模型（有「启动」概念）；在线识别为 false，打开即就绪。 */
  asrLocal?: boolean;
  /** 最近一次启动失败的原因；成功启动、停止或换识别模型后清空。 */
  error?: string | null;
  /** 同传用的识别（模型 id）：你选的识别听得懂外语就是它，否则本地 SenseVoice。现在用不了时为 null，原因在 listenError。 */
  listenModelId?: string | null;
  /** 同传用的是在线识别（队友的声音会发给识别服务商，每句计费）。 */
  listenOnline?: boolean;
  listenError?: string | null;
}

/**
 * 语音识别就绪状态（ADR 028）：
 * - 在线识别：`online-ready` / `online-preparing` / `online-error`，界面上不出现「启动」；
 * - 本地识别：未启动 / 准备中 / 已启动 / 出问题了。
 */
export type AsrReadiness =
  | "unknown"
  | "online-ready" | "online-preparing" | "online-error"
  | "local-stopped" | "local-starting" | "local-running" | "local-error";

export function asrReadiness(status: RuntimeStatus | null | undefined): AsrReadiness {
  if (!status) return "unknown";
  const local = status.asrLocal ?? true;
  if (status.phase === "running") return local ? "local-running" : "online-ready";
  if (status.phase === "starting") return local ? "local-starting" : "online-preparing";
  if (status.error) return local ? "local-error" : "online-error";
  if (status.phase === "stopping") return local ? "local-starting" : "online-preparing";
  return local ? "local-stopped" : "online-preparing";
}

/** 本地识别四种状态的文案（标题栏、语音输入卡片、聊天页共用）。 */
export const LOCAL_ASR_LABELS: Record<"local-stopped" | "local-starting" | "local-running" | "local-error", string> = {
  "local-stopped": m.shell_asr_state_stopped(),
  "local-starting": m.shell_asr_state_starting(),
  "local-running": m.shell_asr_state_running(),
  "local-error": m.shell_asr_state_error(),
};

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

export function companionBindings(config: AssistantConfig | undefined): AssistantBindings | undefined {
  return config?.companionBindings ?? config;
}

export function companionBindingPatch(
  config: AssistantConfig | undefined, patch: Partial<AssistantBindings>,
): SettingsPatch["assistant"] {
  return config?.companionBindings ? { companionBindings: patch } : patch;
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

/** 预置提供商的分组（界面按这个顺序展示）。 */
export type ConnectionPresetGroup = "recommended" | "china" | "global" | "aggregator" | "custom";

export interface ConnectionPreset {
  id: string;
  displayName: string;
  group: ConnectionPresetGroup;
  /** 一句话说明。 */
  description: string;
  defaultBaseUrl: string;
  defaultModel: string;
  /** 可选的模型（第一个一般就是默认模型；也可以手填）。 */
  models: string[];
  /** 控制台里创建 API key 的页面；null = 不确定，不显示“获取密钥”。 */
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
  /** 走控制台通道时这个频道的命令（ADR 032）：say_team 队伍 / say 所有人；缺省 = 只走键盘 */
  consoleCommand?: "say_team" | "say";
}

/** Source 2 控制台通道（ADR 032 决定 3）：只有实测通过的游戏，内置配置才带这一段 */
export interface ConsoleChat {
  /** 游戏的 cfg 目录，相对 Steam 安装目录（如 game/dota/cfg） */
  cfgDir: string;
  /** 游戏里绑了 exec kotone_say 的键（F13–F23，缺省 F23） */
  triggerKey?: string;
}

/** 控制台通道开关的状态（开关就是游戏 autoexec.cfg 里 Kotone 的那一行） */
export interface ConsoleChatStatus {
  supported: boolean;
  /** Steam 安装目录；null = 没找到 */
  installDir: string | null;
  cfgDir: string | null;
  /** autoexec.cfg 里有 Kotone 那一行 */
  enabled: boolean;
  writtenAt: string | null;
  /** 玩家把那一行绑的键改成了别的 */
  keyMismatch: boolean;
  triggerKey: string;
  /** 游戏开着（这时不能切换） */
  gameRunning: boolean;
  /** 游戏是在打开开关之前启动的，重启游戏后生效 */
  needsRestart: boolean;
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
  /** 陪玩的游戏说明（可选，写进提示词），如“聊天在左下角，[队伍] 前缀是队友” */
  companionNotes?: string;
  /** 陪玩记录的聊天框位置（窗口客户区比例）；缺省 = 整窗识别 */
  chatRegion?: ChatRegion | null;
  /** 允许自动化程序对这个游戏按键、点鼠标（缺省 true；新字段，旧版本忽略） */
  allowAutomationInput?: boolean;
  /** 打开这个游戏用的程序、快捷方式或 steam:// 之类的链接（空 = 琴音不能打开它） */
  launchPath?: string;
  /**
   * 让琴音一步一步操作这个游戏（game.act，ADR 019）：ask 先问（缺省）/ allow 一直允许 / deny 不允许。
   */
  computerUse?: "ask" | "allow" | "deny";
  /** 支持 Source 2 控制台通道（ADR 032）；缺省 = 只走键盘 */
  consoleChat?: ConsoleChat | null;
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

/** 统一模型清单的类别：asr 语音识别 / ocr 文字识别 / tts 语音合成 */
export type ModelCategory = "asr" | "ocr" | "tts" | "search";

export interface CatalogFile {
  name: string;
  url: string;
  sha256: string;
  sizeBytes: number;
}

/** OCR / TTS 清单条目（ASR 仍用 ModelInfo） */
export interface CatalogModel {
  category: ModelCategory;
  id: string;
  engineId: string;
  displayName: string;
  description: string;
  sizeBytes: number;
  backend: "local" | "remote";
  downloaded: boolean;
  ready: boolean;
  state: "ready" | "not-installed" | "invalid" | "not-configured" | "unavailable" | string;
  reason?: string | null;
  active: boolean;
  recommended: boolean;
  languages: string[];
  /** none / selected-image（每次识别上传截图）/ text（上传文字） */
  dataEgress: "none" | "selected-image" | "text" | string;
  configSchema: ModelConfigField[];
  apiKeyUrl?: string | null;
  license: string;
  files: CatalogFile[];
}

export interface CatalogDeleteOutcome {
  wasActive: boolean;
  settings: Settings;
}

export interface ModelInfo {
  /** 统一模型清单类别；本清单恒为 asr */
  category?: "asr";
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
  // i18n-ignore-start：匹配后端的错误文字
  return text.includes("下载已取消");
  // i18n-ignore-end
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

/**
 * 热键表（ADR 040 第 1 期，后端 `kotone_core::hotkey_table`）：一个占着某个键的东西。
 * core = Kotone 自己的；plugin = 插件的；custom = 玩家自己加的（快捷指令）；fixed = Esc；
 * game = 游戏配置里往游戏里按的键（不进表，只参与冲突检查）；unknown = 表里有、这个版本找不到的命令（不登记）。
 */
export type KeySource = "core" | "plugin" | "custom" | "fixed" | "game" | "unknown";
export type GameKeyKind = "open-chat" | "channel" | "send" | "console";

export interface KeyUse {
  /** 稳定 id：热键同设置页 / `named[].id`（assistant、record、quick-message-3…）；Esc 是 escape；游戏的键是 game.<配置 id>.<哪个键> */
  id: string;
  /** 绑的命令（`kotone.call`、`voice-input.talk`）；提醒助手热键、快捷指令、Esc、游戏的键没有 */
  command?: string;
  /** 设置里写的键；空 = 没设 */
  key: string;
  /** 规范写法（和录键得到的一样，Ctrl+Alt+Shift+主键）；没设 / 认不出为空 */
  combo: string;
  source: KeySource;
  /** 插件 id 或游戏配置 id */
  owner?: string;
  /** 插件名或游戏名 */
  ownerName?: string;
  gameKey?: GameKeyKind;
  /** 后端的中文名（给 Agent 看）；界面按 id 用 messages 里的名字 */
  label: string;
  /** 快捷指令说的话 / 运行的自动化、频道名、Esc 做什么 */
  detail?: string;
  /** 在哪改（设置说明里的位置，openLocation 认得）；Esc 没有 */
  page?: string;
  field?: string;
  /** false = 插件关着，这个键现在没登记 */
  enabled: boolean;
  /** 和谁冲突（别的项的 id，可能是 gameKeys 里的） */
  conflicts?: string[];
}

/** get_hotkey_table：「设置 → 热键」那一页 */
export interface HotkeyTable {
  bindings: KeyUse[];
  gameKeys: KeyUse[];
}

/** check_hotkey：改一个键之前查一次 */
export interface HotkeyCheck {
  key: string;
  /** 会撞上的；有就不该用这个键 */
  conflicts: KeyUse[];
  /** 提示（不拦）：常见的游戏 / 系统键 */
  warnings: string[];
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
    ? m.shell_models_dir_ascii_only()
    : null;
}

// ---------- 模型下载（引擎与模型页） ----------

/** `kotone://download` 事件 payload：模型/运行时下载进度 */
export interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number;
}
