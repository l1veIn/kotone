/**
 * dev:web 的浏览器后端：模拟数据、内存状态与命令实现集中在这里。
 * 仅依赖契约，不导入 IPC 入口或 Tauri，避免模拟行为与真实 transport 耦合。
 */
import defaultLolBlocklistCsv from "../../../../../crates/kotone-postprocess/assets/lol-zh-cn-starter.csv?raw";
import type {
  PipelineConfig,
  PostProcessingTestStep,
  PostProcessingTestResult,
  Settings,
  RuntimePhase,
  RuntimeStatus,
  StartupOptions,
  ModelsDirInfo,
  ModelsDirMigration,
  DeleteOutcome,
  SettingsPatch,
  AudioDevice,
  EngineInfo,
  PostProcessorInfo,
  Connection,
  ConnectionInfo,
  ConnectionPreset,
  GameProfile,
  ModelInfo,
  ModelInstallGuide,
  CatalogModel,
  CatalogDeleteOutcome,
  ModelCategory,
  CompanionCheck,
  CompanionStatus,
  HistoryRecord,
  HotwordMergeReport,
  ElevationStatus,
  HotkeyStatus,
  HotkeyCheck,
  HotkeyTable,
  HotkeyBinding,
  HotkeyBindingOptions,
  InputEnvironmentCheck,
  ResourceUsage,
  DiagnosticExportResult,
  DiagnosticWindow,
  OverlayHealth,
  OverlayStateSnapshot,
} from "./contracts";
import { companionBindings, defaultPostProcessing, hasLegacyReminders, modelsDirPathError, usesLegacyVoiceInput } from "./contracts";
import type { BrowserAction, BrowserBounds, BrowserExtension, BrowserState, BrowserTab } from "../browser";
import {
  bindingOptions, checkHotkey as checkHotkeyInMock, confirmCustom, customStatus, hotkeyTable, presetStatus, removeCustom, saveCustom,
  setHotkey as setHotkeyInMock, type HotkeySources,
} from "./hotkeysMock";

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
    lowerSpeakerVolumeWhileListening: false,
    sttEngine: "sherpa-streaming",
    activeModelId: "",
    modelConfigs: {},
    engineOptions: {
      "sherpa-streaming": { provider: "cpu" },
    },
    autoSend: false,
    activeProfileId: "lol",
    language: "zh",
    evalRecording: false,
    runAsAdminOnStart: true,
    adminPromptDismissed: false,
    autoAdminPromptDismissed: false,
    adminPromptRevision: 0,
    channelCycleHotkey: "Shift+CapsLock",
    resendLastHotkey: "",
    togglePostProcessingHotkey: "",
    cyclePostProcessingHotkey: "",
    runtimeToggleHotkey: "",
    automationStopHotkey: "Ctrl+Shift+F12",
    interactionMode: "push-to-talk",
    vadSilenceMs: 700,
    vad: { threshold: 0.5, minSpeechMs: 50, minSilenceMs: 50 },
    hotwordsScore: 3.5,
    postProcessing: defaultPostProcessing(),
    connections: [],
    history: { mode: "capped", maxRecords: 1000, includeAudio: false, dir: "" },
    ui: { firstRunCompleted: true, autoStart: false, updatePromptDisabled: false },
    models: { dir: "" },
    download: { source: "auto", ghProxy: "https://ghfast.top/" },
    overlay: {
      visibility: "on_demand",
      style: "capsule",
      position: "auto",
      draggable: false,
      clickThrough: true,
    },
    soundFeedback: {
      record: { enabled: true, volume: 60, soundId: "rise" },
      send: { enabled: true, volume: 60, soundId: "fall" },
    },
    diagnostics: { recording: true },
    assistant: { hotkey: "Ctrl+Backquote", callHotkeyDefaulted: true, callHotkeyRevision: 1, interactionMode: "push-to-talk", connectionId: "", defaultDelivery: ["voice", "danmaku"], voiceId: "", speechEngine: "system", kokoroVoiceId: "kokoro:zf_001", speechVolume: 80, speechRate: 0, danmakuDurationMs: 8000, danmakuSpeedPxPerSec: 160, danmakuFontSize: 32, danmakuTopOffsetPx: 120, danmakuColor: "#FFFFFF", danmakuEmote: "none", route: "reminders", companion: { quickMessages: [], autoSend: true }, ttsModelId: "", wake: { enabled: false, phrase: "qinyin-qinyin", sensitivity: "normal", ack: "voice", noSpeechTimeoutMs: 5000 }, bubble: { enabled: true, corner: "bottom-right", offsetY: 0, alsoDanmaku: false }, followUpSeconds: 8, callIdleSeconds: 30, echoCancellation: true, callMode: "one-shot" },
    toolService: { enabled: false },
    ocr: { activeModelId: "" },
    search: { activeModelId: "" },
    browser: { enabled: true, extensions: [] },
    agent: { permissions: { mode: "default", rules: {} }, hiddenAutomations: [], inGame: true, progressSpeech: true, identity: { name: "", avatar: "" }, subagents: { model: "" }, compactAtTokens: 64000, roleId: "kotone", matchRecap: true, safetyCheck: true, gameEffort: "", chatEffort: "" },
    plugins: {},
    // 热键表（ADR 040 D8）：预览是老用户，旧字段里的键都在表里（含以前默认的 Shift+CapsLock 频道切换）
    hotkeys: {
      bindings: [
        { key: "Ctrl+Backquote", command: "kotone.call" },
        { key: "Ctrl+Shift+F12", command: "kotone.stop" },
        { key: "CapsLock", command: "voice-input.talk" },
        { key: "Shift+CapsLock", command: "voice-input.channel-cycle" },
      ],
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
      id: "sherpa-streaming",
      displayName: "sherpa 流式",
      capabilities: { streaming: true, hotwords: true, gpu: false, offline: true, languages: ["zh", "en"] },
      isReady: false,
    },
    {
      id: "sherpa-offline",
      displayName: "sherpa 非流式",
      capabilities: { streaming: false, hotwords: true, gpu: false, offline: true, languages: ["zh", "en", "ja", "ko", "yue"] },
      isReady: false,
    },
    {
      id: "volcano-asr",
      displayName: "火山引擎一句话识别",
      capabilities: { streaming: false, hotwords: false, gpu: false, offline: false, languages: ["zh"] },
      isReady: false,
    },
    {
      id: "iflytek-asr",
      displayName: "科大讯飞语音听写",
      capabilities: { streaming: true, hotwords: false, gpu: false, offline: false, languages: ["zh"] },
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
      preSendDelayMs: 100,
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
      preSendDelayMs: 100,
      preferClipboardPaste: false,
      icon: "lol-kotone.webp",
      hotwords: ["闪现", "大龙", "gank", "打野", "推塔", "回城"],
      allowAutomationInput: true,
      launchPath: "C:\\Riot Games\\Riot Client\\RiotClientServices.exe",
      channels: [
        { id: "team", displayName: "队伍", default: true },
        { id: "all", displayName: "所有人", openChatKey: "Shift+Enter" },
      ],
    },
  ],
  models: [
    {
      id: "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05",
      engineId: "sherpa-streaming",
      displayName: "X-ASR 流式中英标点（int8，480ms 低延迟）",
      sizeBytes: 169_000_000,
      downloaded: false,
      io: "streaming",
      backend: "sherpa",
      recipe: "zipformer-transducer",
      configSchema: [],
    },
    {
      id: "sense-voice-zh-en-ja-ko-yue-2024-07-17",
      engineId: "sherpa-offline",
      displayName: "sherpa SenseVoice 多语言（int8，非流式高准）",
      sizeBytes: 240_000_000,
      downloaded: false,
      io: "offline",
      backend: "sherpa",
      recipe: "sense-voice",
      configSchema: [
        {
          key: "language",
          label: "识别语言",
          kind: "enum",
          default: "auto",
          options: [
            { value: "auto", label: "自动" },
            { value: "zh", label: "中文" },
            { value: "en", label: "英语" },
            { value: "ja", label: "日语" },
            { value: "ko", label: "韩语" },
            { value: "yue", label: "粤语" },
          ],
          required: false,
        },
      ],
    },
    {
      id: "funasr-nano-int8-2025-12-30",
      engineId: "sherpa-offline",
      displayName: "FunASR-Nano 中英日（官方 Hugging Face，非流式，不支持热词）",
      sizeBytes: 1_010_000_000,
      downloaded: false,
      io: "offline",
      backend: "sherpa",
      recipe: "funasr-nano",
      configSchema: [],
    },
    {
      id: "funasr-nano-int8-modelscope",
      engineId: "sherpa-offline",
      displayName: "FunASR-Nano 中英日（魔搭社区，国内直下，不支持热词）",
      sizeBytes: 1_010_000_000,
      downloaded: false,
      io: "offline",
      backend: "sherpa",
      recipe: "funasr-nano",
      configSchema: [],
    },
    {
      id: "volcano-asr",
      engineId: "volcano-asr",
      displayName: "火山引擎语音识别（在线）",
      sizeBytes: 0,
      downloaded: true,
      io: "offline",
      backend: "remote",
      recipe: "volcano-asr",
      tag: "online-asr",
      apiKeyUrl: "https://console.volcengine.com/speech/service/15",
      configSchema: [
        { key: "appId", label: "App ID", kind: "string", default: "", options: [], required: true },
        { key: "accessToken", label: "Access Token", kind: "secret", default: "", options: [], required: true },
      ],
    },
    {
      id: "deepseek-asr",
      engineId: "deepseek-asr",
      displayName: "DeepSeek 语音识别（在线）",
      sizeBytes: 0,
      downloaded: true,
      io: "offline",
      backend: "remote",
      recipe: "deepseek-asr",
      tag: "online-asr",
      configSchema: [],
    },
    {
      id: "iflytek-asr",
      engineId: "iflytek-asr",
      displayName: "科大讯飞语音听写（在线）",
      sizeBytes: 0,
      downloaded: true,
      io: "streaming",
      backend: "remote",
      recipe: "iflytek-asr",
      tag: "online-asr",
      apiKeyUrl: "https://console.xfyun.cn/services/iat",
      configSchema: [
        { key: "appId", label: "APPID", kind: "string", default: "", options: [], required: true },
        { key: "apiSecret", label: "APISecret", kind: "secret", default: "", options: [], required: true },
        { key: "apiKey", label: "APIKey", kind: "string", default: "", options: [], required: true },
      ],
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
      // 发到了哪个游戏、哪个频道（ADR 043 D10；下面那条是旧记录，没有）
      game: { id: "lol", name: "英雄联盟" },
      channel: { id: "team", name: "队伍" },
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
  return "pong(mock)";
}

/** 当前进程的新手向导启动策略（CLI: --onboarding=auto|always|never）。 */
export async function getStartupOptions(): Promise<StartupOptions> {
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

export async function getSettings(): Promise<Settings> {
  initMockRuntime();
  applyElevationSeed();
  applyRiskSeed();
  if (!mock.settings.ui.sidebar) {
    const sidebar = loadMockSidebar();
    if (sidebar) mock.settings.ui.sidebar = sidebar;
  }
  const legacyMode = mockQuery("mockLegacyUpgrade");
  if ((legacyMode === "pending" || legacyMode === "dismissed") && mock.settings.ui.legacyUpgrade === undefined) {
    // 0.1.x 升级用户；dismissed = 上次选了“继续使用语音输入法”之后再打开（ADR 028 流程 B2）。
    mock.settings.ui.legacyUpgrade = legacyMode;
    if (legacyMode === "dismissed") mock.settings.ui.startPage = "voiceInput";
    mock.settings.assistant.hotkey = "";
    if (mockQuery("mockLegacyReminder") === "configured") {
      mock.settings.assistant.hotkey = "Alt+F9";
      mock.settings.assistant.connectionId = "original-reminder";
      mock.settings.connections.push({
        id: "original-reminder",
        displayName: "原来的提醒连接",
        kind: "remote",
        provider: "custom",
        baseUrl: "https://example.invalid/v1",
        model: "original-model",
      });
      mockConnectionSecrets.set("original-reminder", "test-legacy-key");
    }
  }
  return clone(mock.settings);
}

export async function getOverlayHealth(): Promise<OverlayHealth> {
  return { degraded: false, reason: null };
}

export async function getOverlayState(): Promise<OverlayStateSnapshot | null> {
  return null;
}

export async function listPostProcessors(): Promise<PostProcessorInfo[]> {
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
      developerOnly: true,
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
      description: "用词库把脏话换成能发的词。自带一份默认词库，也可以换成自己的。",
      category: "utility",
      developerOnly: false,
      networkAccess: "local",
      configFields: [
        {
          key: "csvPath",
          displayName: "自己的词库",
          description: "不选就用默认词库。选了文件就改用你的。",
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

// 浏览器预览：与后端 kotone-core connection::CONNECTION_PRESETS 同一份数据（29 家，按分组排列）。
const mockConnectionPresets: ConnectionPreset[] = [
  {id: "deepseek", displayName: "DeepSeek", group: "recommended", description: "深度求索官方接口，工具调用稳定，支持看图（V4.1 Flash）", defaultBaseUrl: "https://api.deepseek.com/v1", defaultModel: "deepseek-flash", models: ["deepseek-flash", "deepseek-v4-pro"], apiKeyUrl: "https://platform.deepseek.com/api_keys"},
  {id: "dashscope", displayName: "通义千问（北京）", group: "recommended", description: "阿里云百炼，国内访问快", defaultBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", defaultModel: "qwen-plus", models: ["qwen-plus", "qwen-max", "qwen-flash"], apiKeyUrl: "https://bailian.console.aliyun.com/?apiKey=1#/api-key"},
  {id: "moonshot-cn", displayName: "月之暗面 Kimi（国内）", group: "recommended", description: "Kimi 开放平台，长上下文，支持看图", defaultBaseUrl: "https://api.moonshot.cn/v1", defaultModel: "kimi-k3", models: ["kimi-k3", "kimi-k2.6", "kimi-k2.7-code"], apiKeyUrl: "https://platform.moonshot.cn/console/api-keys"},
  {id: "zhipu", displayName: "智谱 GLM（国内）", group: "recommended", description: "智谱开放平台按量计费接口", defaultBaseUrl: "https://open.bigmodel.cn/api/paas/v4", defaultModel: "glm-5.3", models: ["glm-5.3", "glm-5.3-flash", "glm-5.2"], apiKeyUrl: "https://open.bigmodel.cn/usercenter/apikeys"},
  {id: "minimax-cn", displayName: "MiniMax（国内）", group: "china", description: "MiniMax 开放平台（OpenAI 兼容接口）", defaultBaseUrl: "https://api.minimaxi.com/v1", defaultModel: "MiniMax-M3", models: ["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.7-highspeed"], apiKeyUrl: "https://platform.minimaxi.com/user-center/basic-information/interface-key"},
  {id: "xiaomi", displayName: "小米 MiMo", group: "china", description: "小米 MiMo 开放平台", defaultBaseUrl: "https://api.xiaomimimo.com/v1", defaultModel: "mimo-v2.6-pro", models: ["mimo-v2.6-pro", "mimo-v2.6-flash", "mimo-v2.5"], apiKeyUrl: null},
  {id: "zhipu-coding", displayName: "智谱 GLM Coding Plan", group: "china", description: "智谱编程套餐专用接口（需订阅套餐）", defaultBaseUrl: "https://open.bigmodel.cn/api/coding/paas/v4", defaultModel: "glm-5.3", models: ["glm-5.3", "glm-5.3-flash", "glm-4.6v"], apiKeyUrl: "https://open.bigmodel.cn/usercenter/apikeys"},
  {id: "kimi-coding", displayName: "Kimi For Coding", group: "china", description: "Kimi 编程套餐专用接口（需订阅套餐）", defaultBaseUrl: "https://api.kimi.com/coding/v1", defaultModel: "kimi-for-coding", models: ["kimi-for-coding", "k3"], apiKeyUrl: null},
  {id: "qwen-token-plan-cn", displayName: "百炼 Token Plan（国内）", group: "china", description: "阿里云百炼 Token Plan 套餐接口，可用多家模型", defaultBaseUrl: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1", defaultModel: "deepseek-v4.1-flash", models: ["deepseek-v4.1-flash", "glm-5.3", "kimi-k2.6", "deepseek-v4-pro"], apiKeyUrl: "https://bailian.console.aliyun.com/?apiKey=1#/api-key"},
  {id: "ant-ling", displayName: "蚂蚁百灵 Ling", group: "china", description: "蚂蚁百灵大模型", defaultBaseUrl: "https://api.ant-ling.com/v1", defaultModel: "Ling-2.6-1T", models: ["Ling-2.6-1T", "Ring-2.6-1T", "Ling-2.6-flash"], apiKeyUrl: null},
  {id: "openai", displayName: "OpenAI", group: "global", description: "OpenAI 官方接口", defaultBaseUrl: "https://api.openai.com/v1", defaultModel: "gpt-5", models: ["gpt-5", "gpt-5-mini", "gpt-4.1"], apiKeyUrl: "https://platform.openai.com/api-keys"},
  {id: "anthropic", displayName: "Anthropic Claude", group: "global", description: "Claude（经 Anthropic 的 OpenAI 兼容接口）", defaultBaseUrl: "https://api.anthropic.com/v1", defaultModel: "claude-sonnet-4-5", models: ["claude-sonnet-4-5", "claude-opus-5-5", "claude-haiku-5-5"], apiKeyUrl: "https://console.anthropic.com/settings/keys"},
  {id: "gemini", displayName: "Google Gemini", group: "global", description: "Gemini（经 Google 的 OpenAI 兼容接口）", defaultBaseUrl: "https://generativelanguage.googleapis.com/v1beta/openai/", defaultModel: "gemini-3.5-flash", models: ["gemini-3.5-flash", "gemini-3.1-pro-preview", "gemini-2.5-pro"], apiKeyUrl: "https://aistudio.google.com/apikey"},
  {id: "xai", displayName: "xAI Grok", group: "global", description: "xAI 官方接口", defaultBaseUrl: "https://api.x.ai/v1", defaultModel: "grok-4.7", models: ["grok-4.7", "grok-4.6", "grok-4.3"], apiKeyUrl: "https://console.x.ai/"},
  {id: "moonshot", displayName: "Kimi（国际）", group: "global", description: "Moonshot AI 国际站", defaultBaseUrl: "https://api.moonshot.ai/v1", defaultModel: "kimi-k3", models: ["kimi-k3", "kimi-k2.6", "kimi-k2.7-code"], apiKeyUrl: "https://platform.moonshot.ai/console/api-keys"},
  {id: "zai", displayName: "Z.AI GLM（国际）", group: "global", description: "智谱国际站按量计费接口", defaultBaseUrl: "https://api.z.ai/api/paas/v4", defaultModel: "glm-5.3", models: ["glm-5.3", "glm-5.3-flash", "glm-5.2"], apiKeyUrl: "https://z.ai/manage-apikey/apikey-list"},
  {id: "minimax", displayName: "MiniMax（国际）", group: "global", description: "MiniMax 国际站（OpenAI 兼容接口）", defaultBaseUrl: "https://api.minimax.io/v1", defaultModel: "MiniMax-M3", models: ["MiniMax-M3", "MiniMax-M2.7"], apiKeyUrl: null},
  {id: "dashscope-intl", displayName: "通义千问（国际）", group: "global", description: "阿里云 Model Studio 国际站", defaultBaseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", defaultModel: "qwen-plus", models: ["qwen-plus", "qwen-max", "qwen-flash"], apiKeyUrl: "https://modelstudio.console.alibabacloud.com/?tab=dashboard#/api-key"},
  {id: "mistral", displayName: "Mistral", group: "global", description: "Mistral 官方接口", defaultBaseUrl: "https://api.mistral.ai/v1", defaultModel: "mistral-medium-latest", models: ["mistral-medium-latest", "magistral-medium-latest", "ministral-14b-2512"], apiKeyUrl: "https://console.mistral.ai/api-keys"},
  {id: "groq", displayName: "Groq", group: "global", description: "Groq 高速推理", defaultBaseUrl: "https://api.groq.com/openai/v1", defaultModel: "openai/gpt-oss-120b", models: ["openai/gpt-oss-120b", "qwen/qwen3.8-27b", "llama-3.3-70b-versatile"], apiKeyUrl: "https://console.groq.com/keys"},
  {id: "cerebras", displayName: "Cerebras", group: "global", description: "Cerebras 高速推理", defaultBaseUrl: "https://api.cerebras.ai/v1", defaultModel: "gpt-oss-120b", models: ["gpt-oss-120b", "qwen-3.8-27b"], apiKeyUrl: "https://cloud.cerebras.ai/"},
  {id: "openrouter", displayName: "OpenRouter", group: "aggregator", description: "一个密钥用多家模型", defaultBaseUrl: "https://openrouter.ai/api/v1", defaultModel: "anthropic/claude-sonnet-4.5", models: ["anthropic/claude-sonnet-4.5", "anthropic/claude-opus-5.5", "anthropic/claude-haiku-5.5"], apiKeyUrl: "https://openrouter.ai/keys"},
  {id: "together", displayName: "Together AI", group: "aggregator", description: "开源模型推理平台", defaultBaseUrl: "https://api.together.ai/v1", defaultModel: "deepseek-ai/DeepSeek-V4.1-Flash", models: ["deepseek-ai/DeepSeek-V4.1-Flash", "moonshotai/Kimi-K3", "MiniMaxAI/MiniMax-M3", "Qwen/Qwen3.6-Plus"], apiKeyUrl: "https://api.together.ai/settings/api-keys"},
  {id: "fireworks", displayName: "Fireworks AI", group: "aggregator", description: "开源模型推理平台", defaultBaseUrl: "https://api.fireworks.ai/inference/v1", defaultModel: "accounts/fireworks/models/kimi-k3", models: ["accounts/fireworks/models/kimi-k3", "accounts/fireworks/models/glm-5p3", "accounts/fireworks/routers/glm-latest"], apiKeyUrl: "https://fireworks.ai/account/api-keys"},
  {id: "nvidia", displayName: "NVIDIA NIM", group: "aggregator", description: "NVIDIA 托管的模型接口", defaultBaseUrl: "https://integrate.api.nvidia.com/v1", defaultModel: "deepseek-ai/deepseek-v4.1-flash", models: ["deepseek-ai/deepseek-v4.1-flash", "moonshotai/kimi-k2.6", "nvidia/nemotron-3-ultra-550b-a55b"], apiKeyUrl: "https://build.nvidia.com/"},
  {id: "huggingface", displayName: "Hugging Face", group: "aggregator", description: "Hugging Face 推理路由", defaultBaseUrl: "https://router.huggingface.co/v1", defaultModel: "deepseek-ai/DeepSeek-V4.1-Flash", models: ["deepseek-ai/DeepSeek-V4.1-Flash", "deepseek-ai/DeepSeek-V4-Pro"], apiKeyUrl: "https://huggingface.co/settings/tokens"},
  {id: "baseten", displayName: "Baseten", group: "aggregator", description: "开源模型推理平台", defaultBaseUrl: "https://inference.baseten.co/v1", defaultModel: "deepseek-ai/DeepSeek-V4.1-Flash", models: ["deepseek-ai/DeepSeek-V4.1-Flash", "moonshotai/Kimi-K3", "deepseek-ai/DeepSeek-V4-Pro"], apiKeyUrl: null},
  {id: "vercel-ai-gateway", displayName: "Vercel AI Gateway", group: "aggregator", description: "Vercel 的多模型网关", defaultBaseUrl: "https://ai-gateway.vercel.sh/v1", defaultModel: "anthropic/claude-sonnet-4.5", models: ["anthropic/claude-sonnet-4.5"], apiKeyUrl: null},
  {id: "custom", displayName: "自定义 OpenAI 兼容", group: "custom", description: "任何兼容 OpenAI /chat/completions 的接口", defaultBaseUrl: "", defaultModel: "", models: [], apiKeyUrl: null},
];

const mockConnectionSecrets = new Map<string, string>();

export async function listConnectionPresets(): Promise<ConnectionPreset[]> {
  return mockConnectionPresets.map((item) => ({ ...item }));
}

export async function listConnections(): Promise<ConnectionInfo[]> {
  return mock.settings.connections.map((connection) => ({
    ...clone(connection),
    hasApiKey: mockConnectionSecrets.has(connection.id),
  }));
}

export async function upsertConnection(
  connection: Connection,
  apiKey?: string,
): Promise<Settings> {
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

export async function deleteConnection(id: string): Promise<Settings> {
  mock.settings.connections = mock.settings.connections.filter((item) => item.id !== id);
  mockConnectionSecrets.delete(id);
  return clone(mock.settings);
}

/**
 * 独立试跑一条 pipeline，不写历史、不触发输入注入，也不受总开关影响。
 * 显式传入 pipeline 可保证结果对应设置页当前展示的步骤快照。
 */
export async function testPostProcessing(
  text: string,
  pipeline?: PipelineConfig,
): Promise<PostProcessingTestResult> {
  const selected =
    pipeline ??
    mock.settings.postProcessing.pipelines.find(
      (item) => item.id === mock.settings.postProcessing.activePipelineId,
    ) ??
    mock.settings.postProcessing.pipelines[0];
  if (!selected) throw new Error("没有可试跑的处理流程");
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

/** 启动时损坏配置的恢复提示；只返回一次，浏览器演示模式恒为 null。 */
export async function getSettingsLoadWarning(): Promise<string | null> {
  return null;
}

export async function updateSettings(patch: SettingsPatch): Promise<Settings> {
  // 模型配置里的 Key 不留在设置里（ADR 048，和后端 model_secrets::take 一样）
  for (const [model, field] of [["volcano-asr", "accessToken"], ["iflytek-asr", "apiSecret"]] as const) {
    const config = (patch as { modelConfigs?: Record<string, Record<string, unknown> | undefined> }).modelConfigs?.[model];
    if (config && field in config) {
      if (String(config[field] ?? "").trim()) mockModelSecrets.add(model);
      delete config[field];
    }
  }
  const wasBrowserOn = mock.settings.browser?.enabled !== false;
  const merged = clone(mock.settings) as unknown as Record<string, unknown>;
  mergeJson(merged, patch);
  mock.settings = merged as unknown as Settings;
  if (patch.ui?.sidebar) saveMockSidebar(mock.settings.ui.sidebar);
  // 关了内置浏览器（ADR 038）：和桌面端一样，开着的网页都关掉
  if (wasBrowserOn && mock.settings.browser?.enabled === false) browserClearTabs();
  // 插件开关变了：和桌面端一样（关掉放歌插件就停下）；动态导入，免得和 pluginsMock 互相引用
  if (patch.plugins) void import("./pluginsMock").then((plugins) => plugins.settingsChanged());
  return clone(mock.settings);
}

// 左侧栏自己排的顺序（ADR 034）：预览里记在 localStorage，刷新后还在（真的在 config.json）。
const MOCK_SIDEBAR_KEY = "kotone.mock.sidebar";
function saveMockSidebar(sidebar: Settings["ui"]["sidebar"]) {
  try { localStorage.setItem(MOCK_SIDEBAR_KEY, JSON.stringify(sidebar ?? null)); } catch { /* 只是预览 */ }
}
function loadMockSidebar(): Settings["ui"]["sidebar"] | undefined {
  try {
    const raw = localStorage.getItem(MOCK_SIDEBAR_KEY);
    const value = raw ? JSON.parse(raw) : null;
    return value && Array.isArray(value.order) && Array.isArray(value.hidden) ? value : undefined;
  } catch { return undefined; }
}

/** 用户拖动悬浮窗后保存当前位置并切换为 custom。 */
export async function saveOverlayPosition(): Promise<Settings> {
  mock.settings.overlay.position = "custom";
  return clone(mock.settings);
}

export async function listAudioDevices(): Promise<AudioDevice[]> {
  return clone(mock.devices);
}

export async function setAudioDevice(id: string): Promise<void> {
  mock.settings.audioDeviceId = id;
  return;
}

export async function listSttEngines(): Promise<EngineInfo[]> {
  return clone(mock.engines);
}

export async function setSttEngine(id: string): Promise<void> {
  if (!mock.engines.some((e) => e.id === id)) throw new Error(`未注册的 STT 引擎: ${id}`);
  mock.settings.sttEngine = id;
  return;
}

export async function getEngineOptions(id: string): Promise<unknown> {
  return mock.settings.engineOptions[id] ?? null;
}

export async function listProfiles(): Promise<GameProfile[]> {
  return clone(mock.profiles);
}

export async function saveProfile(profile: GameProfile): Promise<void> {
  const i = mock.profiles.findIndex((p) => p.id === profile.id);
  if (i >= 0) mock.profiles[i] = clone(profile);
  else mock.profiles.push(clone(profile));
  return;
}

/** Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本） */
export async function confirmSend(): Promise<void> {
  return;
}

/** 取消当前会话（任意状态回 Idle） */
export async function cancelSession(): Promise<void> {
  return;
}

// ---------- 提权（UIPI 方案） ----------

/** 导出 profile 热词到 UTF-8 文本（每行一词条），返回条数 */
export async function exportHotwords(profileId: string, path: string): Promise<number> {
  const p = mock.profiles.find((x) => x.id === profileId);
  console.info(`[mock] export_hotwords: ${p?.hotwords.length ?? 0} 条 → ${path}`);
  return p?.hotwords.length ?? 0;
}

/** 导出内置屏蔽词表到 UTF-8 CSV（两列：屏蔽词,替换词），返回规则条数。 */
export async function exportBuiltinBlocklist(path: string): Promise<number> {
  console.info(`[mock] export_builtin_blocklist → ${path}`);
  return defaultLolBlocklistCsv
    .replace(/^\uFEFF/, "")
    .split(/\r?\n/)
    .slice(1)
    .filter((row) => row.trim() !== "").length;
}

/** 从 UTF-8 文本导入热词（合并去重），返回合并报告 */
export async function importHotwords(profileId: string, path: string): Promise<HotwordMergeReport> {
  return { added: 0, duplicates: 0, total: 0 };
}

/** 导出整包 profile 为 .zip 配置包（profile.json + icon.*） */
export async function exportProfile(profileId: string, path: string): Promise<void> {
  console.info(`[mock] export_profile: ${profileId} → ${path}`);
  return;
}

/** 导入 .zip 配置包：后端生成新随机 id 落盘，返回导入后的 profile */
export async function importProfile(path: string): Promise<GameProfile> {
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
    preSendDelayMs: 100,
    preferClipboardPaste: false,
    icon: null,
    hotwords: [],
  };
  mock.profiles.push(clone(profile));
  return profile;
}

/** 删除 profile：内置 = 恢复出厂；导入的 = 永久删除。返回动作类型 */
export async function deleteProfile(profileId: string): Promise<"reset" | "deleted"> {
  const i = mock.profiles.findIndex((p) => p.id === profileId);
  if (i >= 0) mock.profiles.splice(i, 1);
  return "deleted";
}

/** 读取 profile 图标字节（无图标 → 空）；前端按 icon 扩展名推断 mime 建 Blob URL */
const profileIcons = import.meta.glob<string>("../../../../../crates/kotone-core/resources/profiles/icons/*.webp", { query: "?url", import: "default", eager: true });
export async function getProfileIcon(profileId: string): Promise<Uint8Array> {
  const name = mock.profiles.find(p => p.id === profileId)?.icon;
  const url = profileIcons[`../../../../../crates/kotone-core/resources/profiles/icons/${name}`];
  if (!url) return new Uint8Array(0);
  return new Uint8Array(await (await fetch(url)).arrayBuffer());
}

/** 控制台通道（ADR 032）：浏览器预览里只记在内存，假装游戏装在 Steam 库、没在运行 */
const mockConsoleChat: Record<string, string | null> = {};

export async function consoleChatStatus(profileId: string): Promise<import("../ipc").ConsoleChatStatus> {
  const profile = mock.profiles.find((p) => p.id === profileId);
  const cc = profile?.consoleChat ?? null;
  const installDir = cc ? `D:\\SteamLibrary\\steamapps\\common\\${profile!.displayName}` : null;
  const writtenAt = mockConsoleChat[profileId] ?? null;
  return {
    supported: cc !== null,
    installDir,
    cfgDir: installDir && cc ? `${installDir}\\${cc.cfgDir.replaceAll("/", "\\")}` : null,
    enabled: writtenAt !== null,
    writtenAt,
    keyMismatch: false,
    triggerKey: cc?.triggerKey ?? "F23",
    gameRunning: false,
    needsRestart: false,
  };
}

export async function consoleChatSet(profileId: string, enabled: boolean): Promise<import("../ipc").ConsoleChatStatus> {
  const profile = mock.profiles.find((p) => p.id === profileId);
  if (!profile?.consoleChat) throw new Error("这个游戏配置不支持控制台通道");
  mockConsoleChat[profileId] = enabled ? "2026-11-22 01:00:00 +08:00" : null;
  return consoleChatStatus(profileId);
}

/**
 * 管理员身份（ADR 044）的预览和测试：页面加载前放 `window.__kotoneElevationSeed`：
 * `{ status?: Partial<ElevationStatus>, settings?: { adminPromptRevision?, runAsAdminOnStart? } }`。
 * 默认 `supported: false`（浏览器里没有管理员这回事），所以别的测试看不到说明框和提示。
 * 「以管理员身份重开」被点过几次记在 `window.__kotoneRestartAsAdminCalls`。
 */
type ElevationSeed = {
  status?: Partial<ElevationStatus>;
  settings?: Partial<Pick<Settings, "adminPromptRevision" | "runAsAdminOnStart">>;
};
function elevationSeed(): ElevationSeed {
  return (globalThis as { __kotoneElevationSeed?: ElevationSeed }).__kotoneElevationSeed ?? {};
}
// 测试读预览里的设置（看「好」/「先不用」写对了没有）
if (typeof window !== "undefined") {
  (window as unknown as { __kotoneMockSettings?: () => Settings }).__kotoneMockSettings = () => clone(mock.settings);
}
let elevationSeedApplied = false;
function applyElevationSeed() {
  if (elevationSeedApplied) return;
  elevationSeedApplied = true;
  Object.assign(mock.settings, elevationSeed().settings ?? {});
}

/**
 * 插件的“风险和许可”弹窗（ADR 043 D8）：预览里默认当作英雄联盟的看过了（别的测试打开英雄联盟的页面、打开有风险的开关时
 * 不被弹窗挡住）；`?mockRisk=unseen` 是从没看过（第一次打开插件页、第一次打开有风险的开关时弹）。
 */
let riskSeedApplied = false;
function applyRiskSeed() {
  if (riskSeedApplied) return;
  riskSeedApplied = true;
  if (mockQuery("mockRisk") === "unseen") return;
  const plugins = (mock.settings.plugins ??= {});
  plugins.lol = { ...plugins.lol, riskSeen: plugins.lol?.riskSeen ?? 1 };
}

/** 游戏兼容状态：自身/游戏提权状态 + 激活 profile 的独占全屏状态 */
export async function getElevationStatus(): Promise<ElevationStatus> {
  return {
    elevated: false,
    activeGameElevated: null,
    activeGameFullscreen: null,
    supported: false,
    adminPromptRevision: 1,
    notice: null,
    ...elevationSeed().status,
  };
}

/** 以管理员身份重启（弹 UAC；成功后当前进程退出，调用方无需后续操作） */
export async function restartAsAdmin(): Promise<void> {
  console.info("[mock] restart_as_admin");
  const w = globalThis as { __kotoneRestartAsAdminCalls?: number };
  w.__kotoneRestartAsAdminCalls = (w.__kotoneRestartAsAdminCalls ?? 0) + 1;
  return;
}

/** [mock] 浏览器模式下的开机自启状态（内存态，保证设置页行为与 e2e 表现一致） */
let mockAutostartEnabled = false;

/** 读取「开机自动启动」系统真实状态（HKCU Run 键 Kotone 值是否存在） */
export async function getAutostartEnabled(): Promise<boolean> {
  return mockAutostartEnabled;
}

/** 设置「开机自动启动」：写入 / 删除 HKCU Run 键（值名与 NSIS 卸载清理一致） */
export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  mockAutostartEnabled = enabled;
  console.info(`[mock] set_autostart_enabled ${enabled}`);
  return;
}

/** 试听内置提示音（设置页「音效」tab）：record/send + 音效 id（缺省按事件默认） */
export async function previewSfx(kind: "record" | "send", soundId?: string): Promise<void> {
  console.info(`[mock] preview_sfx ${kind} ${soundId ?? ""}`);
  return;
}

/** 热键注册状态（registered/key/error/backend + 具名辅助热键），设置页热键分区展示占用冲突 */
export async function getHotkeyStatus(): Promise<HotkeyStatus> {
  const registered = (await getRuntimeStatus()).phase === "running";
  return {
      registered,
      key: registered ? mock.settings.hotkey.key : null,
      error: null,
      backend: registered ? "llhook" : "none",
      named: [
        { id: "assistant", key: usesLegacyVoiceInput(mock.settings.ui) ? null : companionBindings(mock.settings.assistant)?.hotkey || null, error: null },
        ...(hasLegacyReminders(mock.settings.ui)
          ? [{ id: "legacy-reminder", key: mock.settings.assistant.hotkey || null, error: null }]
          : []),
        { id: "channel-cycle", key: mock.settings.channelCycleHotkey, error: null },
        { id: "resend-last", key: mock.settings.resendLastHotkey || null, error: null },
        {
          id: "post-process-toggle",
          key: mock.settings.togglePostProcessingHotkey || null,
          error: null,
        },
        {
          id: "post-process-cycle",
          key: mock.settings.cyclePostProcessingHotkey || null,
          error: null,
        },
        {
          id: "runtime-toggle",
          key: mock.settings.runtimeToggleHotkey || null,
          error: null,
        },
        {
          id: "automation-stop",
          key: mock.settings.automationStopHotkey || null,
          error: null,
        },
        // 自己加的每条用自己的 id 登记，接在后面（同后端：`custom.<id>`；要确认、按不了的不登记，写原因）
        ...customStatus(mock.settings),
        // 插件登记的调工具的命令（英雄联盟的一键喊话）：登记 id 是命令名
        ...presetStatus(hotkeySources()),
      ],
    };
}

/** 主动输入环境自检；可在引导页到达热键步骤时调用，不要求先启动录入。 */
export async function checkInputEnvironment(): Promise<InputEnvironmentCheck> {
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

/** 当前进程的 CPU / 内存占用（调用方按 ~2s 间隔轮询） */
export async function getResourceUsage(): Promise<ResourceUsage> {
  return { cpuPercent: 3.2, memoryBytes: 86 * 1024 * 1024 };
}

/** 导出诊断 ZIP；包内不含录音、识别文本或热词。 */
export async function exportDiagnostics(
  path: string,
  window: DiagnosticWindow = "all",
): Promise<DiagnosticExportResult> {
  return {
    reportId: "KT-MOCK",
    path: path.endsWith(".zip") ? path : `${path}.zip`,
    eventCount: 0,
    historyCount: mock.history.length,
    window,
  };
}

/** 清除本地流程事件与 kotone.log。stopRecording 时同时关闭后续记录。 */
export async function clearDiagnostics(stopRecording: boolean): Promise<void> {
  mock.settings.diagnostics.recording = stopRecording
    ? false
    : mock.settings.diagnostics.recording;
  return;
}

/** 将前端已处理/未处理异常写入后端持久日志；后端负责脱敏和截断。 */
export async function logFrontendError(context: string, message: string): Promise<void> {
  console.warn(`[frontend:${context}]`, message);
  return;
}

/** 热键表和改键前的冲突检查（ADR 040）：后端 hotkey_table 的内存版在 hotkeysMock.ts */
function hotkeySources(): HotkeySources {
  return {
    settings: mock.settings,
    profiles: mock.profiles,
    voiceInput: { id: "voice-input", name: "语音输入", enabled: mock.settings.plugins?.["voice-input"]?.enabled ?? true },
    lol: {
      id: "lol", name: "英雄联盟", enabled: mock.settings.plugins?.lol?.enabled ?? true,
      client: mock.settings.plugins?.lol?.sources?.client ?? true,
    },
  };
}

export async function getHotkeyTable(): Promise<HotkeyTable> {
  return clone(hotkeyTable(hotkeySources()));
}

export async function checkHotkey(key: string, except?: string): Promise<HotkeyCheck> {
  return clone(checkHotkeyInMock(hotkeySources(), key, except));
}

export async function setHotkey(id: string, key: string): Promise<Settings> {
  setHotkeyInMock(hotkeySources(), id, key);
  return clone(mock.settings);
}

export async function getHotkeyBindingOptions(): Promise<HotkeyBindingOptions> {
  return clone(bindingOptions(mock.settings));
}

export async function saveCustomHotkey(entry: HotkeyBinding, confirm: boolean): Promise<Settings> {
  saveCustom(hotkeySources(), clone(entry), confirm);
  return clone(mock.settings);
}

export async function removeCustomHotkey(id: string): Promise<Settings> {
  removeCustom(hotkeySources(), id);
  return clone(mock.settings);
}

export async function confirmHotkey(id: string): Promise<Settings> {
  confirmCustom(hotkeySources(), id);
  return clone(mock.settings);
}

/** 开始热键捕获（设置页「点击录入」）；结果经 kotone://hotkey-capture 事件推送 */
export async function startHotkeyCapture(): Promise<void> {
  throw new Error("浏览器调试环境不支持热键录入");
}

/** 取消进行中的热键捕获（组件销毁/重新点击的兜底） */
export async function cancelHotkeyCapture(): Promise<void> {
  return;
}

// ---------- 运行时「启动」开关 ----------

/** mock 运行时：启动快照（推导 restartNeeded 用；对齐壳侧语义） */
let mockStarted: { engineId: string; modelId: string } | null = null;

let mockPhase: RuntimePhase = "stopped";
let mockRuntimeError: string | null = null;
let mockRuntimeInitialized = false;
const runtimeListeners = new Set<(status: RuntimeStatus) => void>();
/** 在线识别（与后端 `kotone_stt::model::is_remote_model` 同一份清单）。 */
const REMOTE_ASR_IDS = new Set(["openai-compat-stt", "volcano-asr", "iflytek-asr", "deepseek-asr"]);

function mockActiveModelId(): string {
  const s = mock.settings;
  if (s.activeModelId.trim()) return s.activeModelId.trim();
  return (s.engineOptions[s.sttEngine] as Record<string, unknown> | undefined)?.model as string ??
    (s.sttEngine === "sherpa-streaming"
      ? "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
      : "default");
}

function mockAsrIsLocal(): boolean {
  return !REMOTE_ASR_IDS.has(mockActiveModelId());
}

const SENSE_VOICE_ID = "sense-voice-zh-en-ja-ko-yue-2024-07-17";
/** 能听懂外语的识别（自动判语种，或中英日韩都认得；和后端 `kotone_stt::listen` 同一条规矩）。 */
const FOREIGN_ASR_IDS = new Set(["deepseek-asr", "openai-compat-stt", SENSE_VOICE_ID]);

/** 同传用的识别（后端 `kotone_stt::listen::listen_model`）：你选的听得懂外语就用它，否则本地 SenseVoice。 */
function mockListenModel(): Pick<RuntimeStatus, "listenModelId" | "listenOnline" | "listenError"> {
  const active = mockActiveModelId();
  const id = FOREIGN_ASR_IDS.has(active) ? active : SENSE_VOICE_ID;
  const online = REMOTE_ASR_IDS.has(id);
  const downloaded = online || (mock.models.find((m) => m.id === id)?.downloaded ?? false);
  return downloaded
    ? { listenModelId: id, listenOnline: online, listenError: null }
    // 后端错误原文（kotone_stt::listen::NEED_SENSEVOICE）
    : { listenModelId: null, listenOnline: false, listenError: "同传要能听懂外语的识别：在「设置 → 模型 → 语音识别」下载 SenseVoice 多语言（约 230 MB），或者换成能自动判语种的在线识别（DeepSeek）" };
}

/** mock 的 restartNeeded 推导：Running 且快照 ≠ 当前配置 */
function mockRuntimeStatus(stage: string | null = null): RuntimeStatus {
  const s = mock.settings;
  const engineId = s.sttEngine;
  const modelId = mockActiveModelId();
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
    asrLocal: mockAsrIsLocal(),
    error: mockRuntimeError,
    ...mockListenModel(),
  };
}

function publishRuntime() {
  const status = mockRuntimeStatus();
  for (const listener of runtimeListeners) listener(status);
}

/** 浏览器预览里代替 `kotone://runtime` 事件：启动、停止、换识别模型后推送全量状态。 */
export function subscribeBrowserRuntime(listener: (status: RuntimeStatus) => void): () => void {
  runtimeListeners.add(listener);
  return () => runtimeListeners.delete(listener);
}

/** 在线识别打开即就绪（对齐壳侧 `runtime::after_asr_change` 与启动时的自动就绪）。 */
function reconcileMockRuntime(previousWasLocal: boolean | null) {
  const local = mockAsrIsLocal();
  if (!local) {
    mockRuntimeError = null;
    mockPhase = "running";
    mockStarted = { engineId: mock.settings.sttEngine, modelId: mockActiveModelId() };
  } else if (previousWasLocal === false) {
    // 从在线换成本地：立即启动一次（D9）；模型没下好时停在“出问题了”。
    startMockLocal();
  }
  publishRuntime();
}

function startMockLocal(): void {
  const modelId = mockActiveModelId();
  const model = mock.models.find((item) => item.id === modelId);
  const engine = mock.engines.find((item) => item.id === mock.settings.sttEngine);
  const ready = mockQuery("mockAsr") === "local-ready" || model?.downloaded || engine?.isReady;
  if (!ready) {
    mockPhase = "stopped";
    mockStarted = null;
    mockRuntimeError = `模型「${model?.displayName ?? modelId}」还没下载，请先在「语音输入」里下载`;
    throw new Error(mockRuntimeError);
  }
  mockRuntimeError = null;
  mockPhase = "running";
  mockStarted = { engineId: mock.settings.sttEngine, modelId };
}

function initMockRuntime() {
  if (mockRuntimeInitialized) return;
  mockRuntimeInitialized = true;
  // ?mockAsr=online：配好了 DeepSeek，用在线识别；?mockAsr=local-ready：本地模型已下载。
  if (mockQuery("mockAsr") === "online") {
    mock.settings.activeModelId = "deepseek-asr";
    mock.settings.sttEngine = "deepseek-asr";
    if (!mock.settings.connections.some((c) => c.id === "deepseek-mock")) {
      mock.settings.connections.push({ id: "deepseek-mock", displayName: "DeepSeek", kind: "remote", provider: "deepseek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-flash" });
      mockConnectionSecrets.set("deepseek-mock", "sk-mock");
    }
  }
  // ?mockAsr=local-ready：本地模型已下载（清单里也是下好的，设置 → 模型 → 语音识别 的启动按钮能点）。
  if (mockQuery("mockAsr") === "local-ready") {
    const model = mock.models.find((item) => item.id === mockActiveModelId());
    if (model) model.downloaded = true;
  }
  // ?mockCompanion=ready：角色已配好 DeepSeek（识别方式不动）。
  if (mockQuery("mockCompanion") === "ready" && !mock.settings.connections.some((c) => c.id === "deepseek-companion")) {
    mock.settings.connections.push({ id: "deepseek-companion", displayName: "DeepSeek", kind: "remote", provider: "deepseek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-flash" });
    mockConnectionSecrets.set("deepseek-companion", "sk-mock");
    mock.settings.assistant.connectionId = "deepseek-companion";
  }
  if (!mockAsrIsLocal()) reconcileMockRuntime(null);
  else if (mock.settings.ui.autoStart) {
    try { startMockLocal(); } catch { /* 停在“出问题了” */ }
  }
}

export async function getRuntimeStatus(): Promise<RuntimeStatus> {
  initMockRuntime();
  return mockRuntimeStatus();
}

/** 启动：warmup 引擎 → 注册热键 → 显示悬浮窗；Running+restartNeeded 时等价重启 */
export async function startRuntime(): Promise<RuntimeStatus> {
  initMockRuntime();
  if (mockAsrIsLocal()) {
    try { startMockLocal(); }
    finally { publishRuntime(); }
  } else {
    reconcileMockRuntime(null);
  }
  return mockRuntimeStatus();
}

/** 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎；Stopped 幂等 */
export async function stopRuntime(): Promise<RuntimeStatus> {
  mockPhase = "stopped";
  mockStarted = null;
  mockRuntimeError = null;
  publishRuntime();
  return mockRuntimeStatus();
}

export async function getModelsDir(): Promise<ModelsDirInfo> {
  const dir = mock.settings.models.dir.trim();
  return { dir: dir || "~/.kotone/models (mock)", isDefault: dir === "" };
}

/** 切换模型目录（后端先迁移旧目录内容再写配置；空串 = 恢复默认） */
export async function setModelsDir(dir: string): Promise<ModelsDirMigration> {
  const pathError = modelsDirPathError(dir.trim());
  if (pathError) throw new Error(pathError);
  mock.settings.models.dir = dir;
  return { dir: dir || "~/.kotone/models (mock)", moved: [], failed: [] };
}

/** 删除已下载模型/运行时；active 模型被删时回退默认 */
export async function deleteModel(id: string): Promise<DeleteOutcome> {
  const m = mock.models.find((x) => x.id === id);
  if (m) m.downloaded = false;
  return { wasActive: false };
}

/** 在系统文件管理器中打开模型目录 */
export async function openModelsDir(): Promise<void> {
  console.info("[mock] open_models_dir");
  return;
}

/** 在系统文件管理器中打开历史记录目录（P2-⑨） */
export async function openHistoryDir(): Promise<void> {
  console.info("[mock] open_history_dir");
  return;
}

/** 在系统默认浏览器打开外部链接（dev:web 直接 window.open 新标签页） */
export async function openExternal(url: string): Promise<void> {
  window.open(url, "_blank", "noreferrer");
  return;
}

/** 全部引擎的模型清单（downloaded 标记） */
export async function listModels(): Promise<ModelInfo[]> {
  // ?mockModels=none：刚装好、一个模型都没下载（新手引导用）
  if (mockQuery("mockModels") === "none") return clone(mock.models).map((m) => ({ ...m, downloaded: false }));
  return clone(mock.models);
}

/** 下载模型/运行时（进度经 kotone://download 事件推送：{ id, downloaded, total }） */
export async function downloadModel(id: string): Promise<void> {
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

/** 取消进行中的模型下载（幂等；已下载部分保留可续传） */
export async function cancelDownload(): Promise<void> {
  console.info("[mock] cancel_download");
  return;
}

function mockInstallGuide(id: string): ModelInstallGuide {
  const m = mock.models.find((x) => x.id === id);
  if (!m) throw new Error(`未知模型：${id}`);
  const destSubdir =
    m.id === "sense-voice-zh-en-ja-ko-yue-2024-07-17"
      ? "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
      : "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05";
  const pageUrl = m.id.includes("sense")
    ? "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
    : "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05.tar.bz2";
  return {
    id: m.id,
    displayName: m.displayName,
    destDir: `C:\\Users\\mock\\.kotone\\models\\${destSubdir}`,
    destSubdir,
    pageUrl,
    files: [
      {
        name: m.id.includes("sense") ? "model.int8.onnx" : "encoder.int8.onnx",
        url: pageUrl,
        sizeBytes: m.sizeBytes,
      },
    ],
  };
}

/** 自动下载失败后的手动安装指引（官方页 + 目标目录 + 文件清单） */
export async function getModelInstallGuide(id: string): Promise<ModelInstallGuide> {
  return mockInstallGuide(id);
}

/** 在资源管理器中打开该模型应放置的目录 */
export async function openModelDestDir(id: string): Promise<void> {
  console.info("[mock] open_model_dest_dir", id);
  return;
}

/** 切换引擎的活动模型（Running 时置 restartNeeded，不自动重启） */
export async function setActiveModel(engineId: string, modelId: string): Promise<void> {
  const wasLocal = mockAsrIsLocal();
  const spec = mock.models.find((item) => item.id === modelId);
  const resolvedEngine = spec?.engineId ?? engineId;
  const opts = (mock.settings.engineOptions[resolvedEngine] ??= {}) as Record<string, unknown>;
  opts.model = modelId;
  mock.settings.activeModelId = modelId;
  mock.settings.sttEngine = resolvedEngine;
  mockRuntimeError = null;
  try { reconcileMockRuntime(wasLocal); } catch { publishRuntime(); }
  return;
}

// ---------- 统一模型清单：OCR / TTS ----------

const mockCatalog: CatalogModel[] = [
  {
    category: "ocr", id: "ppocrv5-korean-mobile", engineId: "local-ppocrv5-korean",
    displayName: "PP-OCRv5 mobile Korean (local, CPU)", description: "韩文聊天（韩服）。本机 CPU 推理，不上传截图",
    sizeBytes: 18_308_324, backend: "local", downloaded: false, ready: false, state: "not-installed",
    active: false, recommended: false, languages: ["ko"], dataEgress: "none", configSchema: [], license: "Apache-2.0",
    files: [],
  },
  {
    category: "ocr", id: "deepseek-ocr", engineId: "deepseek-ocr", displayName: "DeepSeek OCR（在线）",
    description: "在线识别，多语种、版面更稳；每次识别都会把截图上传到 DeepSeek", sizeBytes: 0, backend: "remote",
    downloaded: true, ready: false, state: "not-configured", reason: "请选择一条 DeepSeek API 连接", active: false,
    recommended: false, languages: ["auto"], dataEgress: "selected-image",
    configSchema: [{ key: "connection", label: "API 连接（DeepSeek）", kind: "connection", default: "", options: [], required: true }],
    apiKeyUrl: "https://platform.deepseek.com/api_keys", license: "DeepSeek API terms", files: [],
  },
  {
    category: "tts", id: "system-speech", engineId: "system", displayName: "Windows 系统语音",
    description: "系统自带，无需下载；声音取决于系统已安装的语音包", sizeBytes: 0, backend: "local", downloaded: true,
    ready: true, state: "ready", active: true, recommended: false, languages: ["zh"], dataEgress: "none",
    configSchema: [], license: "Windows", files: [],
  },
  {
    category: "tts", id: "deepseek-tts", engineId: "deepseek", displayName: "DeepSeek 语音合成（在线，默认）",
    description: "DeepSeek 官方语音，默认用 AI 连接里的 DeepSeek 与密钥，声音海星（女声）/暗潮（男声）；按字数计费，每次播报都会把文字发给 DeepSeek，相同的话复用缓存；不支持调语速",
    sizeBytes: 0, backend: "remote", downloaded: true,
    ready: false, state: "not-configured", reason: "还没有 DeepSeek 连接：请在「设置 → API 连接」里添加 DeepSeek", active: false,
    recommended: true, languages: ["zh"], dataEgress: "text",
    configSchema: [
      { key: "connection", label: "DeepSeek 连接（留空 = 用 AI 连接里的 DeepSeek）", kind: "connection", default: "", options: [], required: false },
      { key: "voice", label: "声音", kind: "enum", default: "stella", options: [{ value: "stella", label: "海星（女声）" }, { value: "tide", label: "暗潮（男声）" }], required: true },
    ],
    apiKeyUrl: "https://platform.deepseek.com/api_keys", license: "服务商条款", files: [],
  },
  {
    category: "tts", id: "openai-compat-tts", engineId: "openai", displayName: "自定义（OpenAI 兼容接口）（在线）",
    description: "填地址、模型和声音名，接任何支持 /audio/speech 的服务；每次说话都会把文字发给它", sizeBytes: 0, backend: "remote", downloaded: true,
    ready: false, state: "not-configured", reason: "自定义语音接口还没有填地址", active: false,
    recommended: false, languages: ["zh"], dataEgress: "text",
    configSchema: [
      { key: "baseUrl", label: "接口地址", kind: "string", default: "", options: [], required: false },
      { key: "model", label: "合成模型", kind: "string", default: "", options: [], required: true },
      { key: "voice", label: "声音（服务商的声音名）", kind: "string", default: "", options: [], required: true },
    ],
    apiKeyUrl: null, license: "服务商条款", files: [],
  },
  {
    category: "tts", id: "kokoro-zh", engineId: "kokoro", displayName: "Kokoro 中文（本地神经语音）",
    description: "更自然的本地中文女声，约 147 MB；本机 CPU 合成，不上传文字", sizeBytes: 147_031_220, backend: "local",
    downloaded: false, ready: false, state: "not-installed", active: false, recommended: true, languages: ["zh"],
    dataEgress: "none", configSchema: [], license: "Apache-2.0", files: [],
  },
  {
    category: "search", id: "deepseek-search", engineId: "deepseek-search", displayName: "DeepSeek 联网搜索（在线）",
    description: "琴音查游戏版本、出装、海克斯强化、打法时用；搜索词和要读的网址会发给 DeepSeek，网页由 DeepSeek 的服务器去读。每次成功的搜索或读网页 0.005 元",
    sizeBytes: 0, backend: "remote", downloaded: true, ready: true, state: "ready", active: false, recommended: true, languages: ["auto"],
    dataEgress: "text", configSchema: [{ key: "connection", label: "API 连接（DeepSeek；留空 = 用琴音正在用的 DeepSeek 连接）", kind: "connection", default: "", options: [], required: false }],
    apiKeyUrl: "https://platform.deepseek.com/api_keys", license: "DeepSeek API terms", files: [],
  },
  {
    category: "search", id: "off", engineId: "off", displayName: "不联网", description: "琴音只凭自己的知识回答（游戏版本相关的内容可能过时）",
    sizeBytes: 0, backend: "local", downloaded: true, ready: true, state: "ready", active: false, recommended: false, languages: ["auto"],
    dataEgress: "none", configSchema: [], license: "", files: [],
  },
];

function refreshMockCatalog(): void {
  const ocrActive =
    mock.settings.ocr.activeModelId ||
    mockCatalog.find((m) => m.category === "ocr" && m.backend === "local" && m.ready)?.id ||
    "";
  const ttsActive = mock.settings.assistant.ttsModelId || (mock.settings.assistant.speechEngine === "kokoro" ? "kokoro-zh" : "system-speech");
  for (const m of mockCatalog) {
    const searchActive = mock.settings.search?.activeModelId || "deepseek-search";
    m.active = m.category === "ocr" ? m.id === ocrActive : m.category === "search" ? m.id === searchActive : m.id === ttsActive;
    if (m.id === "deepseek-ocr") {
      const connection = mock.settings.modelConfigs["deepseek-ocr"]?.connection;
      m.ready = typeof connection === "string" && connection !== "";
      m.state = m.ready ? "ready" : "not-configured";
    }
  }
}

export async function listCatalogModels(category: ModelCategory): Promise<CatalogModel[]> {
  refreshMockCatalog();
  return clone(mockCatalog.filter((m) => m.category === category));
}

export async function downloadCatalogModel(category: ModelCategory, id: string): Promise<void> {
  if (mockQuery("mockDownload") === "fail") throw new Error("网络连接失败：无法连接模型下载源");
  const m = mockCatalog.find((x) => x.category === category && x.id === id);
  if (m) Object.assign(m, { downloaded: true, ready: true, state: "ready" });
}

export async function importCatalogModel(category: ModelCategory, id: string, _dir: string): Promise<void> {
  return downloadCatalogModel(category, id);
}

export async function deleteCatalogModel(category: ModelCategory, id: string): Promise<CatalogDeleteOutcome> {
  const m = mockCatalog.find((x) => x.category === category && x.id === id);
  if (m) Object.assign(m, { downloaded: false, ready: false, state: "not-installed" });
  let wasActive = false;
  if (category === "ocr" && mock.settings.ocr.activeModelId === id) {
    mock.settings.ocr.activeModelId = "";
    wasActive = true;
  }
  if (category === "tts" && id === "kokoro-zh" && !mock.settings.assistant.ttsModelId && mock.settings.assistant.speechEngine === "kokoro") {
    mock.settings.assistant.speechEngine = "system";
    wasActive = true;
  }
  return { wasActive, settings: clone(mock.settings) };
}

export async function activateCatalogModel(category: ModelCategory, id: string): Promise<Settings> {
  const m = mockCatalog.find((x) => x.category === category && x.id === id);
  if (!m) throw new Error(`未知模型 ${id}`);
  if (m.backend === "local" && !m.downloaded) throw new Error(`${m.displayName} 尚未下载，请先下载再切换`);
  if (category === "ocr") mock.settings.ocr.activeModelId = id;
  else if (category === "search") mock.settings.search = { activeModelId: id };
  else if (SPEECH_ONLINE_IDS.includes(id)) mock.settings.assistant.ttsModelId = id;
  else {
    mock.settings.assistant.speechEngine = id === "kokoro-zh" ? "kokoro" : "system";
    // 明确选系统语音写 "local"，不再参与“自动”
    mock.settings.assistant.ttsModelId = id === "kokoro-zh" ? "" : "local";
  }
  return clone(mock.settings);
}

export async function getCatalogInstallGuide(category: ModelCategory, id: string): Promise<ModelInstallGuide> {
  const m = mockCatalog.find((x) => x.category === category && x.id === id);
  if (!m) throw new Error(`未知模型 ${id}`);
  return {
    id,
    displayName: m.displayName,
    destDir: `C:\\Users\\mock\\.kotone\\models\\ocr\\${id}`,
    destSubdir: `ocr/${id}`,
    pageUrl: "https://www.modelscope.cn/models/RapidAI/RapidOCR/files",
    files: m.files.map((f) => ({ name: f.name, url: f.url, sizeBytes: f.sizeBytes })),
  };
}

export async function openCatalogModelDir(category: ModelCategory, id: string): Promise<void> {
  console.info("[mock] open_catalog_model_dir", category, id);
}

// ---------- Kotone 陪玩 ----------

export async function companionSelfCheck(): Promise<CompanionCheck> {
  const id = companionBindings(mock.settings.assistant)?.connectionId;
  if (!id) throw new Error("还没有选 API 连接：请在聊天页输入框旁的「她用的模型」里选一个");
  return connectionSelfCheck(id);
}

export async function installedGames(query?: string): Promise<import("../ipc").InstalledGame[]> {
  const all = [
    { id: "tencent:lol", name: "英雄联盟（国服）", source: "tencent", installDir: "C:\\WeGameApps\\英雄联盟", launch: "\"C:\\WeGameApps\\英雄联盟\\WeGameLauncher\\launcher.exe\"", processNames: ["League of Legends.exe"], needsAdmin: true },
    { id: "hoyo:hk4e_cn", name: "原神", source: "hoyoplay", installDir: "D:\\miHoYo Launcher\\games\\Genshin Impact Game", launch: "\"D:\\miHoYo Launcher\\games\\Genshin Impact Game\\YuanShen.exe\"", processNames: ["YuanShen.exe"], needsAdmin: true },
    { id: "steam:570", name: "Dota 2", source: "steam", installDir: "D:\\SteamLibrary\\steamapps\\common\\dota 2 beta", launch: "steam://rungameid/570", processNames: [], needsAdmin: false },
  ];
  return query ? all.filter((g) => g.name.includes(query)) : all;
}

export async function connectionModels(id: string): Promise<{ supported: boolean; models: string[] }> {
  const connection = mock.settings.connections.find((c) => c.id === id);
  if (!connection) throw new Error(`连接 ${id} 不存在`);
  return { supported: true, models: [connection.model, `${connection.model}-pro`] };
}

export async function connectionSelfCheck(id: string): Promise<CompanionCheck> {
  const assistant = mock.settings.assistant;
  const connection = mock.settings.connections.find((c) => c.id === id);
  if (!connection) throw new Error(`连接 ${id} 不存在`);
  const check: CompanionCheck = {
    connectionId: id,
    baseUrl: connection?.baseUrl ?? "",
    model: connection?.model ?? "",
    passed: mockQuery("mockCompanionCheck") !== "fail",
    checkedAtMs: Date.now(),
    message: mockQuery("mockCompanionCheck") === "fail" ? `连得上，但服务商的模型列表里没有“${connection.model}”，请换一个模型` : "连接正常",
  };
  if (!usesLegacyVoiceInput(mock.settings.ui) && companionBindings(assistant)?.connectionId === id) assistant.companion.check = check;
  return clone(check);
}

export async function companionStatus(): Promise<CompanionStatus> {
  const assistant = mock.settings.assistant;
  const check = assistant.companion.check ?? null;
  const connectionId = companionBindings(assistant)?.connectionId ?? "";
  const ocr = mock.settings.ocr.activeModelId || null;
  const reason = !connectionId
    ? "还没有选 API 连接：请在「设置 → API 连接」里添加一条，再在聊天页输入框旁的「她用的模型」里选它"
    : !ocr
          ? "看游戏聊天需要文字识别模型：请在「设置 → 模型 → 文字识别」下载并选择一个"
          : null;
  return {
    ready: reason === null,
    reason,
    connectionId,
    check,
    ocr: { modelId: ocr, state: { ready: !!ocr, dataEgress: ocr === "deepseek-ocr" ? "selected-image" : "none" } },
    autoSend: assistant.companion.autoSend,
    quickMessages: clone(assistant.companion.quickMessages),
    profile: { id: mock.settings.activeProfileId ?? "generic", notes: "", chatRegion: null },
  };
}

export async function companionClearChatRegion(): Promise<void> {}

// ---------- 识别历史（历史记录页） ----------

/** 识别历史列表（新→旧） */
export async function getHistory(): Promise<HistoryRecord[]> {
  return clone(mock.history);
}

/** 清空全部识别历史（含音频文件；调用方负责二次确认） */
export async function clearHistory(): Promise<void> {
  mock.history = [];
  return;
}

/** 删除单条历史记录（带录音且不再被其他记录引用时一并删除对应 wav；记录不存在幂等） */
export async function deleteHistoryRecord(sessionId: string, ts: string): Promise<void> {
  const i = mock.history.findIndex((h) => h.sessionId === sessionId && h.ts === ts);
  if (i >= 0) mock.history.splice(i, 1);
  return;
}

/** 读取历史记录随附的音频（相对 history/audio/ 的文件名 → wav 字节） */
export async function readHistoryAudio(fileName: string): Promise<Uint8Array> {
  return new Uint8Array(0);
}

// ---------- 内置浏览器（ADR 031，聊天页右侧栏） ----------
// 命令可能不带参数地被调用（e2e 的兜底按命令名找这里的函数），参数都要可缺省。

const closedBrowser = (): BrowserState => ({
  open: false, url: null, title: null, site: null, loading: false, handover: null, media: null, busy: false, error: null,
  tabs: [], activeTab: null, notice: null,
});
const previewSongs = [
  { title: "晴天", artist: "周杰伦" },
  { title: "稻香", artist: "周杰伦" },
  { title: "夜曲", artist: "周杰伦" },
];
const browserPreview = {
  state: closedBrowser(),
  song: 0,
  nextTab: 1,
  bounds: null as BrowserBounds | null,
  listeners: new Set<(state: BrowserState) => void>(),
  extensions: [] as BrowserExtension[],
};

function hostOf(url: string): string | null {
  try { return new URL(url).host || null; } catch { return null; }
}

/** 预览里的“网页”：按网址给一个标题。 */
function previewTab(url: string | null, extra: Partial<BrowserTab> = {}): BrowserTab {
  const site = url ? hostOf(url) : null;
  return {
    id: `tab-${browserPreview.nextTab++}`, url, title: site ? `${site} 的页面` : null, site, loading: false,
    canGoBack: false, canGoForward: false, audible: false, agent: false, fullscreenElement: false, ...extra,
  };
}

/** 旧字段（url / title / site / loading）跟着当前标签页。 */
function syncActive(): void {
  const state = browserPreview.state;
  const tab = state.tabs.find((t) => t.id === state.activeTab) ?? null;
  state.open = state.tabs.length > 0;
  state.url = tab?.url ?? null;
  state.title = tab?.title ?? null;
  state.site = tab?.site ?? null;
  state.loading = tab?.loading ?? false;
}

function emitBrowser(): void {
  syncActive();
  for (const listener of browserPreview.listeners) listener(clone(browserPreview.state));
}

export function browserState(): BrowserState {
  syncActive();
  return clone(browserPreview.state);
}

/** 浏览器区域的位置（预览里只记下，e2e 读它核对）。 */
export function browserSetBounds(bounds?: BrowserBounds): null {
  browserPreview.bounds = bounds ? { ...bounds } : null;
  return null;
}
export function browserLastBounds(): BrowserBounds | null {
  return browserPreview.bounds ? { ...browserPreview.bounds } : null;
}

function activeTab(): BrowserTab | null {
  const state = browserPreview.state;
  return state.tabs.find((t) => t.id === state.activeTab) ?? null;
}

export function browserControl(args?: { action?: BrowserAction }): null {
  const state = browserPreview.state;
  const tab = activeTab();
  switch (args?.action) {
    case "close": browserPreview.state = closedBrowser(); break;
    // 重试（出错时的“重试”就是 reload）：预览里直接当作好了
    case "reload": state.error = null; if (tab) tab.loading = false; break;
    case "stop": if (tab) tab.loading = false; break;
    case "back": if (tab?.canGoBack) { tab.canGoBack = false; tab.canGoForward = true; } break;
    case "forward": if (tab?.canGoForward) { tab.canGoForward = false; tab.canGoBack = true; } break;
    default: break;
  }
  emitBrowser();
  return null;
}

/** 地址栏回车：像网址就打开，否则用必应搜（和后端的规则一样）。 */
export function browserNavigate(args?: { url?: string }): null {
  const input = (args?.url ?? "").trim();
  if (!input) return null;
  const looksLikeUrl = /^https?:\/\//i.test(input) || /^[\w-]+(\.[\w-]+)+(\/\S*)?$/.test(input);
  const url = looksLikeUrl ? (/^https?:\/\//i.test(input) ? input : `https://${input}`) : `https://cn.bing.com/search?q=${encodeURIComponent(input)}`;
  const state = browserPreview.state;
  let tab = activeTab();
  if (!tab) {
    tab = previewTab(url);
    state.tabs.push(tab);
    state.activeTab = tab.id;
  } else {
    const site = hostOf(url);
    Object.assign(tab, { url, site, title: site ? `${site} 的页面` : null, canGoBack: !!tab.url, canGoForward: false });
  }
  emitBrowser();
  return null;
}

export function browserTabNew(args?: { url?: string }): null {
  const state = browserPreview.state;
  if (state.tabs.length >= 8) return null;
  const tab = previewTab(args?.url ?? null);
  state.tabs.push(tab);
  state.activeTab = tab.id;
  emitBrowser();
  return null;
}

export function browserTabClose(args?: { id?: string }): null {
  const state = browserPreview.state;
  const index = state.tabs.findIndex((t) => t.id === args?.id);
  if (index < 0) return null;
  state.tabs.splice(index, 1);
  if (state.activeTab === args?.id) state.activeTab = state.tabs[Math.min(index, state.tabs.length - 1)]?.id ?? null;
  if (!state.tabs.length) { browserPreview.state = closedBrowser(); }
  emitBrowser();
  return null;
}

export function browserTabSelect(args?: { id?: string }): null {
  const state = browserPreview.state;
  if (state.tabs.some((t) => t.id === args?.id)) state.activeTab = args!.id!;
  emitBrowser();
  return null;
}

export function browserHandoverDone(): null {
  browserPreview.state.handover = null;
  emitBrowser();
  return null;
}

export function browserClearData(): null {
  browserClearTabs();
  return null;
}

/** 关掉所有标签页（清除登录、设置里关了内置浏览器）。 */
function browserClearTabs(): void {
  browserPreview.state = closedBrowser();
  emitBrowser();
}

export function browserExtensions(): BrowserExtension[] {
  return clone(browserPreview.extensions);
}
export function browserExtensionAdd(args?: { path?: string }): null {
  const path = (args?.path ?? "").trim();
  if (!path) throw new Error("没有选扩展文件夹");
  if (browserPreview.extensions.some((e) => e.path === path)) throw new Error("这个扩展已经加过了");
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  browserPreview.extensions.push({ id: `ext-${browserPreview.extensions.length + 1}`, name, enabled: true, path });
  return null;
}
export function browserExtensionRemove(args?: { id?: string }): null {
  browserPreview.extensions = browserPreview.extensions.filter((e) => e.id !== args?.id);
  return null;
}
export function browserExtensionToggle(args?: { id?: string; enabled?: boolean }): null {
  const ext = browserPreview.extensions.find((e) => e.id === args?.id);
  if (ext) ext.enabled = !!args?.enabled;
  return null;
}

export function subscribeBrowserPreview(callback: (state: BrowserState) => void): () => void {
  browserPreview.listeners.add(callback);
  return () => { browserPreview.listeners.delete(callback); };
}

/** dev:web 演示：没打开 → 网易云在放歌（两个标签页）→ 要登录（控制台里 `__kotoneBrowserPreview("playing")`）。 */
export function browserPreviewStep(step: "closed" | "playing" | "handover" | "error"): BrowserState {
  if (step === "closed") {
    browserPreview.state = closedBrowser();
  } else if (step === "error") {
    // 浏览器没能打开（例如 WebView2 建不出来）
    browserPreview.state = { ...closedBrowser(), error: "浏览器没能打开：WebView2 报错 0x80070057（参数错误）" };
  } else {
    browserPreview.song = 0;
    const music = previewTab("https://music.163.com/#/song?id=186016", {
      title: "晴天 - 周杰伦 - 单曲 - 网易云音乐", agent: true, audible: step !== "handover", canGoBack: true,
    });
    const guide = previewTab("https://www.bilibili.com/video/BV1xx411c7mD", { title: "海克斯大乱斗剑姬出装 - 哔哩哔哩", canGoBack: true });
    browserPreview.state = {
      ...closedBrowser(),
      tabs: [music, guide],
      activeTab: music.id,
      busy: false,
      handover: step === "handover" ? { reason: "网易云要登录一下" } : null,
      media: step === "handover" ? null : { playing: true, ...previewSongs[0] },
    };
  }
  emitBrowser();
  return clone(browserPreview.state);
}

if (typeof window !== "undefined" && import.meta.env?.DEV) {
  (window as unknown as { __kotoneBrowserPreview?: typeof browserPreviewStep }).__kotoneBrowserPreview = browserPreviewStep;
}


// ---------- 她的声音（ADR 048）：声音设置页的服务商、Key、声音、试听 ----------

/** 在线声音的几家（和后端 `kotone_tts::catalog::PROVIDERS` 同一个顺序）。 */
const SPEECH_ONLINE_IDS = ["deepseek-tts", "minimax-tts", "dashscope-tts", "siliconflow-tts", "volcano-tts", "openai-compat-tts"];
const speechProviderTable: Record<string, { provider: string; apiKeyUrl: string | null; defaultModel: string; defaultVoice: string; rate: boolean; regions: boolean; keyOnly: boolean; vendor?: string[] }> = {
  "deepseek-tts": { provider: "deepseek", apiKeyUrl: "https://platform.deepseek.com/api_keys", defaultModel: "", defaultVoice: "stella", rate: false, regions: false, keyOnly: true, vendor: ["deepseek"] },
  "minimax-tts": { provider: "minimax", apiKeyUrl: "https://platform.minimaxi.com/user-center/basic-information/interface-key", defaultModel: "speech-2.8-turbo", defaultVoice: "female-shaonv", rate: true, regions: true, keyOnly: true, vendor: ["minimax-cn", "minimax"] },
  "dashscope-tts": { provider: "dashscope", apiKeyUrl: "https://bailian.console.aliyun.com/?apiKey=1#/api-key", defaultModel: "qwen3-tts-flash", defaultVoice: "Cherry", rate: false, regions: true, keyOnly: true, vendor: ["dashscope", "dashscope-intl"] },
  "siliconflow-tts": { provider: "siliconflow", apiKeyUrl: "https://cloud.siliconflow.cn/account/ak", defaultModel: "FunAudioLLM/CosyVoice2-0.5B", defaultVoice: "FunAudioLLM/CosyVoice2-0.5B:claire", rate: true, regions: false, keyOnly: true },
  "volcano-tts": { provider: "volcano", apiKeyUrl: "https://console.volcengine.com/speech/app", defaultModel: "volcano_tts", defaultVoice: "BV700_streaming", rate: true, regions: false, keyOnly: false },
  "openai-compat-tts": { provider: "openai", apiKeyUrl: null, defaultModel: "", defaultVoice: "", rate: true, regions: false, keyOnly: false },
};
/** 预览里“存进凭据库”的声音 Key（只记有没有，不记值）。 */
const mockSpeechKeys = new Map<string, "cn" | "intl" | null>();
const mockSpeechVoices: Record<string, { id: string; name: string }[]> = {
  deepseek: [{ id: "deepseek/stella", name: "海星（女声）" }, { id: "deepseek/tide", name: "暗潮（男声）" }],
  minimax: [
    { id: "minimax/my-voice-01", name: "我的声音（复刻）· my-voice-01" },
    { id: "minimax/female-shaonv", name: "少女音色" }, { id: "minimax/female-yujie", name: "御姐音色" },
    { id: "minimax/Chinese (Mandarin)_Warm_Girl", name: "温暖少女" }, { id: "minimax/male-qn-qingse", name: "青涩青年音色" },
  ],
  dashscope: [{ id: "dashscope/Cherry", name: "芊悦（女）" }, { id: "dashscope/Serena", name: "苏瑶（女）" }, { id: "dashscope/Sunny", name: "四川话 · 晴儿（女）" }],
  siliconflow: [{ id: "siliconflow/FunAudioLLM/CosyVoice2-0.5B:claire", name: "温柔女声（claire）" }, { id: "siliconflow/FunAudioLLM/CosyVoice2-0.5B:alex", name: "沉稳男声（alex）" }],
  volcano: [{ id: "volcano/BV700_streaming", name: "灿灿（女声）" }, { id: "volcano/BV001_streaming", name: "通用女声" }, { id: "volcano/BV002_streaming", name: "通用男声" }],
  kokoro: [{ id: "kokoro/zf_001", name: "中文女声 001" }, { id: "kokoro/zf_003", name: "中文女声 003" }, { id: "kokoro/zf_017", name: "中文女声 017" }],
};

function speechConfig(id: string): Record<string, unknown> {
  return (mock.settings.modelConfigs[id] ?? {}) as Record<string, unknown>;
}

function mockSpeechProvider(id: string) {
  const spec = speechProviderTable[id];
  const config = speechConfig(id);
  let keySource: { kind: string; id?: string; name?: string } = { kind: "none" };
  let region: "cn" | "intl" | null = null;
  if (mockSpeechKeys.has(id)) {
    keySource = { kind: "own" };
    region = spec.regions ? mockSpeechKeys.get(id) ?? "cn" : null;
  } else if (spec.vendor) {
    const her = mock.settings.assistant.connectionId;
    const all = mock.settings.connections.filter((c) => spec.vendor!.includes(c.provider));
    const found = all.find((c) => c.id === her) ?? all[0];
    if (found) {
      keySource = { kind: "connection", id: found.id, name: found.displayName };
      region = spec.regions ? (found.provider.endsWith("intl") || found.provider === "minimax" ? "intl" : "cn") : null;
    }
  } else if (id === "openai-compat-tts") keySource = { kind: "notNeeded" };
  let reason: string | null = null;
  if (id === "openai-compat-tts") {
    if (!String(config.baseUrl ?? "").trim()) reason = "自定义语音接口还没有填地址";
    else if (!String(config.model ?? "").trim()) reason = "自定义语音接口还没有填模型名";
    else if (!String(config.voice ?? "").trim()) reason = "自定义语音接口还没有填声音名";
  } else if (keySource.kind === "connection" && !mockConnectionSecrets.has(keySource.id!)) reason = `API 连接「${keySource.name}」还没有保存 Key`;
  else if (keySource.kind === "none") reason = id === "volcano-tts" ? "缺少 Access Token：请填写，或先在语音识别里填写火山配置" : "还没有 Key";
  return {
    modelId: id, provider: spec.provider, displayName: id, description: "", billing: "", apiKeyUrl: spec.apiKeyUrl,
    defaultModel: spec.defaultModel, defaultVoice: spec.defaultVoice, rate: spec.rate, regions: spec.regions, keyOnly: spec.keyOnly,
    active: mock.settings.assistant.ttsModelId === id, ready: reason === null, reason, keySource, region,
  };
}

export async function speechProviders() {
  return SPEECH_ONLINE_IDS.map(mockSpeechProvider);
}

/** 她现在用的声音（预览里“自动”只认她的连接是不是 DeepSeek / MiniMax / 百炼、有没有 Key）。 */
export async function speechCurrent() {
  const assistant = mock.settings.assistant;
  const auto = !assistant.ttsModelId && assistant.speechEngine === "system";
  let modelId = assistant.ttsModelId && SPEECH_ONLINE_IDS.includes(assistant.ttsModelId) ? assistant.ttsModelId
    : assistant.speechEngine === "kokoro" ? "kokoro-zh" : "system-speech";
  if (auto) {
    const her = mock.settings.connections.find((c) => c.id === assistant.connectionId);
    const vendor = her && Object.entries(speechProviderTable).find(([, spec]) => spec.vendor?.includes(her.provider));
    if (her && vendor && mockConnectionSecrets.has(her.id) && vendor[0] !== "siliconflow-tts") modelId = vendor[0];
  }
  const online = SPEECH_ONLINE_IDS.includes(modelId);
  const provider = online ? mockSpeechProvider(modelId) : null;
  return {
    modelId, auto, online, ready: provider?.ready ?? true, reason: provider?.reason ?? null,
    keySource: provider?.keySource ?? { kind: "notNeeded" }, region: provider?.region ?? null, rate: provider?.rate ?? true,
  };
}

/** 保存 Key：预览不联网，“bad” 当作被拒的 Key。 */
export async function speechKeySave(modelId: string, key: string) {
  if (!speechProviderTable[modelId]) throw new Error(`不认识的声音服务商 ${modelId}`);
  if (!key.trim()) throw new Error("Key 是空的");
  if (key.trim() === "bad") return { ok: false, saved: false, message: "这个 Key 被拒了（预览）", region: null };
  const region = speechProviderTable[modelId].regions ? (key.includes("intl") ? "intl" : "cn") : null;
  mockSpeechKeys.set(modelId, region);
  return { ok: true, saved: true, message: "可以用了", region };
}

export async function speechKeyClear(modelId: string) {
  mockSpeechKeys.delete(modelId);
}

/** 当前引擎的声音（预览里系统语音读不到，返回空）。 */
export async function speechVoices() {
  const current = await speechCurrent();
  if (current.modelId === "kokoro-zh") return clone(mockSpeechVoices.kokoro);
  const provider = speechProviderTable[current.modelId]?.provider;
  return provider ? clone(mockSpeechVoices[provider] ?? []) : [];
}

/** 模型配置里的 Key 存了没有（预览：保存过一次就算有，值不留）。 */
const mockModelSecrets = new Set<string>();
export async function modelSecretSaved(modelId: string): Promise<boolean> {
  return mockModelSecrets.has(modelId);
}
