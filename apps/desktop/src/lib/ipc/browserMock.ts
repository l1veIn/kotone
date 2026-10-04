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
  HistoryRecord,
  HotwordMergeReport,
  ElevationStatus,
  HotkeyStatus,
  InputEnvironmentCheck,
  ResourceUsage,
  DiagnosticExportResult,
  DiagnosticWindow,
} from "./contracts";
import { defaultPostProcessing, modelsDirPathError } from "./contracts";

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
    runAsAdminOnStart: false,
    adminPromptDismissed: false,
    autoAdminPromptDismissed: false,
    channelCycleHotkey: "Shift+CapsLock",
    resendLastHotkey: "",
    togglePostProcessingHotkey: "",
    cyclePostProcessingHotkey: "",
    runtimeToggleHotkey: "",
    interactionMode: "push-to-talk",
    vadSilenceMs: 700,
    vad: { threshold: 0.5, minSpeechMs: 50, minSilenceMs: 50 },
    hotwordsScore: 3.5,
    postProcessing: defaultPostProcessing(),
    connections: [],
    history: { mode: "capped", maxRecords: 1000, includeAudio: false, dir: "" },
    ui: { firstRunCompleted: true, autoStart: false },
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
    assistant: { hotkey: "", interactionMode: "push-to-talk", connectionId: "", defaultDelivery: ["voice", "danmaku"], voiceId: "", speechEngine: "system", kokoroVoiceId: "kokoro:zf_001", speechVolume: 80, speechRate: 0, danmakuDurationMs: 8000, danmakuSpeedPxPerSec: 160, danmakuFontSize: 32, danmakuTopOffsetPx: 120, danmakuColor: "#FFFFFF", danmakuEmote: "none" },
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
  return clone(mock.settings);
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

const mockConnectionPresets: ConnectionPreset[] = [
  {
    id: "dashscope",
    displayName: "通义千问（北京）",
    defaultBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    defaultModel: "qwen3-30b-a3b-instruct-2507",
    apiKeyUrl: "https://bailian.console.aliyun.com/?apiKey=1#/api-key",
  },
  {
    id: "custom",
    displayName: "自定义 OpenAI 兼容",
    defaultBaseUrl: "",
    defaultModel: "",
    apiKeyUrl: null,
  },
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
  const merged = clone(mock.settings) as unknown as Record<string, unknown>;
  mergeJson(merged, patch);
  mock.settings = merged as unknown as Settings;
  return clone(mock.settings);
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

/** 手动触发发送（调试/测试用） */
export async function simulateSend(text: string, profileId?: string): Promise<void> {
  console.info(`[mock] simulate_send: "${text}" (profile: ${profileId ?? "generic"})`);
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
export async function getProfileIcon(profileId: string): Promise<Uint8Array> {
  return new Uint8Array(0);
}

/** 游戏兼容状态：自身/游戏提权状态 + 激活 profile 的独占全屏状态 */
export async function getElevationStatus(): Promise<ElevationStatus> {
  return {
    elevated: false,
    activeGameElevated: null,
    activeGameFullscreen: null,
    supported: false,
  };
}

/** 以管理员身份重启（弹 UAC；成功后当前进程退出，调用方无需后续操作） */
export async function restartAsAdmin(): Promise<void> {
  console.info("[mock] restart_as_admin");
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
  return {
      registered: true,
      key: mock.settings.hotkey.key,
      error: null,
      backend: "llhook",
      named: [
        { id: "assistant", key: mock.settings.assistant.hotkey || null, error: null },
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

/** 静态键位冲突提示（向导热键步骤；常见游戏键位表） */
export async function detectHotkeyConflicts(key: string): Promise<string[]> {
  return [];
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

/** mock 的 restartNeeded 推导：Running 且快照 ≠ 当前配置 */
function mockRuntimeStatus(stage: string | null = null): RuntimeStatus {
  const s = mock.settings;
  const engineId = s.sttEngine;
  const modelId =
    (s.engineOptions[engineId] as Record<string, unknown> | undefined)?.model as string ??
    (engineId === "sherpa-streaming"
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
  return mockRuntimeStatus();
}

/** 启动：warmup 引擎 → 注册热键 → 显示悬浮窗；Running+restartNeeded 时等价重启 */
export async function startRuntime(): Promise<RuntimeStatus> {
  const engine = mock.engines.find((item) => item.id === mock.settings.sttEngine);
  if (!engine?.isReady) {
    throw new Error("X-ASR 模型未下载。请先下载模型再启动");
  }
  mockPhase = "running";
  const s = mockRuntimeStatus();
  mockStarted = { engineId: s.engineId ?? "", modelId: s.modelId ?? "" };
  return mockRuntimeStatus();
}

/** 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎；Stopped 幂等 */
export async function stopRuntime(): Promise<RuntimeStatus> {
  mockPhase = "stopped";
  mockStarted = null;
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
  const spec = mock.models.find((item) => item.id === modelId);
  const resolvedEngine = spec?.engineId ?? engineId;
  const opts = (mock.settings.engineOptions[resolvedEngine] ??= {}) as Record<string, unknown>;
  opts.model = modelId;
  mock.settings.activeModelId = modelId;
  mock.settings.sttEngine = resolvedEngine;
  return;
}

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
