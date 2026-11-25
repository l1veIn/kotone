// 跳到某个设置的位置（聊天里的工具卡片“琴音改了 xxx → 去看看”）。
// 位置用设置说明（crates/kotone-core/src/settings_guide.rs）里的写法：
// 「琴音 → 唤醒 → 呼叫键」「设置 → 模型 → 语音识别」「聊天页 → 她用的模型」「语音输入 → 悬浮窗」……
// 旧版本写进对话里的位置（「琴音 → 设置 → 呼叫」「设置 → 其他热键」「设置 → 怎么叫她」等）也认，跳到它们的新位置（ADR 028）。
// 设置窗口（Settings.svelte）订阅这里的请求并切页面 / 分页 / 滚到分组。
import { writable } from "svelte/store";
import { m } from "../paraglide/messages.js";

/**
 * 角色页（左侧栏「游戏陪玩」里她的那一项）的分页：聊天，角色 · 记忆 · 技能，以及从「设置」搬来的
 * 唤醒（`call`）· 交互（`reply`）· 权限 · 用量（交接第十一节第 8 条，2026-11-25）。分页 ID 沿用原来设置里的。
 */
export type CompanionSection = "call" | "reply" | "permissions" | "usage";
export type AssistantSection = "chat" | "identity" | "memory" | "skills" | CompanionSection;
export type VoiceInputPage = "general" | "processing" | "history";

export const COMPANION_SECTIONS: readonly CompanionSection[] = ["call", "reply", "permissions", "usage"];
export function isCompanionSection(value: string): value is CompanionSection {
  return (COMPANION_SECTIONS as readonly string[]).includes(value);
}

/** 聊天页输入框旁的“她用的模型”菜单（原「设置 → 她怎么回答 → 她用的模型」）。 */
export const CHAT_MODEL_ANCHOR = "chat-model";

export type PageTarget =
  | { page: "automation" | "game" | "about" | "legacy-reminders"; anchor?: string }
  /** 左侧栏的“全部页面”（排顺序、显示 / 隐藏；ADR 034 第六节）。 */
  | { page: "sidebar-pages" }
  /** 角色页的一个分页；`anchor` 是分页里的分组（聊天页上是 `chat-model`：打开选模型的菜单）。 */
  | { page: "assistant"; section: AssistantSection; anchor?: string }
  /** 「设置」的一页。旧的 `call` / `reply` / `permissions` / `usage` 也还认：落到角色页（见 normalizeTarget）。 */
  | { page: "advanced"; tab: string; anchor?: string }
  /** 左侧栏的「插件」：每个插件一张卡片（ADR 038）。 */
  | { page: "plugins" }
  /** 一个插件的 App（从「插件」的卡片点开）；`params` 是打开在 App 的哪里（插件自己认，例如语音输入的分页）。 */
  | { page: "plugin"; id: string; params?: Record<string, unknown> }
  /** 填 Key 的引导（语音输入的「让 Kotone 帮我配置」在她还没连上时）。 */
  | { page: "setup-key" };

/** 语音输入插件的 App 里的一页和分组（ADR 038：原来左侧栏的「语音输入」整块搬进了插件）。 */
export function voiceInputTarget(tab: VoiceInputPage = "general", anchor?: string, extra: Record<string, unknown> = {}): PageTarget {
  return { page: "plugin", id: "voice-input", params: { tab, ...(anchor ? { anchor } : {}), ...extra } };
}

/** 左侧栏的“全部页面”开着（侧栏的“⋯”、聊天里“去看看：左侧栏 → ⋯”）。 */
export const sidebarPagesOpen = writable(false);

/** 「设置」的分页（页面 ID 不变）。 */
export const ADVANCED_TABS: Record<string, string> = {
  // i18n-ignore-start：设置位置的词：要和后端 settings_guide.rs 写的位置一字不差（玩家点“去看看”时据此跳转），不是界面文字
  "模型": "models", "API 连接": "connections", "声音": "sound", "调参": "tuning", "外观": "appearance", "系统": "system",
  "浏览器": "browser", "热键": "hotkeys",
  // 旧写法：「音效」；陪玩那一排（「设置 → 怎么叫她 / 她怎么回答 / 权限 / 用量」）搬进了角色页，normalizeTarget 落过去
  "音效": "sound", "怎么叫她": "call", "她怎么回答": "reply", "权限": "permissions", "用量": "usage",
};

/** 分页里的分组锚点（第三段）。 */
const ADVANCED_ANCHORS: Record<string, Record<string, string>> = {
  models: { "语音识别": "models-asr", "文字识别": "models-ocr", "语音合成": "models-tts", "联网搜索": "models-search" },
  system: { "停下": "system-stop", "打开软件时显示": "system-start-page" },
};

/** 角色页的唤醒、交互里的分组锚点（第三段）。“她用的模型”搬到了聊天页输入框旁（`chat-model`）。 */
const COMPANION_ANCHORS: Record<string, Record<string, string>> = {
  call: { "呼叫键": "settings-call-hotkey", "语音唤醒": "settings-call-wake", "快捷指令": "companion-title", "通话": "settings-call-hotkey" },
  reply: {
    "气泡": "settings-reply-bubble", "声音与弹幕": "settings-reply-output", "回答方式": "settings-reply-output",
    "自动卡片": "settings-reply-auto-cards", "消息提示": "settings-reply-auto-cards",
    "她用的模型": CHAT_MODEL_ANCHOR,
    // 旧写法（术语表 v2 之前的小节名；以前的对话里还引用着）
    "AI 连接": CHAT_MODEL_ANCHOR, "对话气泡": "settings-reply-bubble",
  },
};
/** 原来「设置 → 她怎么回答」里“她用的模型”那一组的锚点（旧的跳转请求里还有）。 */
const OLD_MODEL_ANCHOR = "settings-reply-ai";

/** 角色页的分页 + 分组；她用的模型落到聊天页，打开输入框旁的模型菜单。 */
export function companionTarget(section: CompanionSection, anchor?: string): PageTarget {
  if (anchor === CHAT_MODEL_ANCHOR || anchor === OLD_MODEL_ANCHOR) return { page: "assistant", section: "chat", anchor: CHAT_MODEL_ANCHOR };
  return { page: "assistant", section, ...(anchor ? { anchor } : {}) };
}

/**
 * 旧的页面目标落到新位置：「设置」里原来的 call / reply / permissions / usage（`ui.open ui://kotone/settings/call`、
 * 旧对话里的“去看看”、各处的“去设置”）→ 角色页的同名分页。所有跳转（openTarget、openLocation）都经过这里。
 */
export function normalizeTarget(target: PageTarget): PageTarget {
  if (target.page === "advanced" && isCompanionSection(target.tab)) return companionTarget(target.tab, target.anchor);
  return target;
}

/** 「语音输入」（插件的 App）的分页与分组。 */
const VOICE_TABS: Record<string, VoiceInputPage> = { "输入": "general", "文字处理": "processing", "历史": "history", "历史记录": "history" };
const VOICE_ANCHORS: Record<string, string> = {
  "其他热键": "voice-hotkeys", "悬浮窗": "voice-overlay", "交互模式": "voice-mode", "热键": "voice-hotkey",
};
/** 语音识别归本体（交接第十一节第 6 条）：换模型、启动 / 停止、启动热键、自动启动都在「设置 → 模型 → 语音识别」。
 * 旧位置「语音输入 → 输入 → 语音识别」（以前的对话里引用着）也跳到这里，不落在语音输入页那张只读的卡片上。 */
const ASR_SETTINGS: PageTarget = { page: "advanced", tab: "models", anchor: "models-asr" };

/** 角色页的分页（「琴音 → 唤醒」）。“她是谁”是“角色”的旧名；“怎么叫她”“她怎么回答”是唤醒、交互在「设置」里时的旧名。 */
const ROLE_TABS: Record<string, AssistantSection> = {
  "她是谁": "identity", "角色": "identity", "记忆": "memory", "技能": "skills", "聊天": "chat",
  "唤醒": "call", "交互": "reply", "权限": "permissions", "用量": "usage",
  "怎么叫她": "call", "她怎么回答": "reply",
};

/** 旧版「琴音 → 设置 → 分组」/「琴音 → 交流」：现在的位置（角色页的分页；她用的模型在聊天页）。 */
const OLD_COMPANION_GROUPS: Record<string, { tab: CompanionSection; anchor?: string }> = {
  "她用的模型": { tab: "reply", anchor: CHAT_MODEL_ANCHOR },
  "AI 连接": { tab: "reply", anchor: CHAT_MODEL_ANCHOR },
  "呼叫": { tab: "call", anchor: "settings-call-hotkey" },
  "语音唤醒": { tab: "call", anchor: "settings-call-wake" },
  "快捷指令": { tab: "call", anchor: "companion-title" },
  "回答": { tab: "reply", anchor: "settings-reply-output" },
  "回答方式": { tab: "reply", anchor: "settings-reply-output" },
  "气泡": { tab: "reply", anchor: "settings-reply-bubble" },
  "对话气泡": { tab: "reply", anchor: "settings-reply-bubble" },
  "弹幕": { tab: "reply", anchor: "settings-reply-output" },
  "权限": { tab: "permissions" },
  "用量": { tab: "usage" },
};

/** 插件的中文名 → 插件 id（「插件 → 英雄联盟」）：名字从插件自己的文案读（plugin_<id>_name）。
 * 改过名的插件，旧名字（以前的对话里引用着）也认：「音乐」→「网易云音乐」（交接第十一节第 7 条，id 还是 music）。 */
const PLUGIN_NAMES: Record<string, string> = {
  "音乐": "music",
  ...Object.fromEntries(
    Object.entries(import.meta.glob<Record<string, string>>("../../../../../plugins/*/messages/zh-CN.json", { eager: true, import: "default" }))
      .map(([path, messages]) => {
        const id = path.split("/plugins/")[1]?.split("/")[0] ?? "";
        return [messages[`plugin_${id.replace(/-/g, "_")}_name`] ?? id, id];
      }),
  ),
};

const PAGES: Record<string, "automation" | "game" | "about" | "legacy-reminders"> = {
  "自动化": "automation", "游戏列表": "game", "游戏": "game", "游戏适配": "game", "关于": "about", "提醒助手": "legacy-reminders",
};

/** 旧版「设置 → 其他热键」里的热键现在分散在三处（「设置 → 热键」一页列出全部，ADR 040）；按第三段（热键名）分，认不出就去语音输入的其他热键。 */
function oldHotkeysTarget(name: string | undefined): PageTarget {
  if (name && /启动|停止 Kotone|本地语音识别/.test(name) && !/自动化|停下/.test(name)) return ASR_SETTINGS;
  if (name && /自动化|停下/.test(name)) return { page: "advanced", tab: "system", anchor: "system-stop" };
  return voiceInputTarget("general", "voice-hotkeys");
}

/** 「语音输入 → 分页 → 分组」或「语音输入 → 分组」。 */
function voiceTarget(second: string | undefined, third: string | undefined): PageTarget {
  if (second === "语音识别" || (VOICE_TABS[second ?? ""] === "general" && third === "语音识别")) return ASR_SETTINGS;
  const tab = VOICE_TABS[second ?? "输入"];
  if (tab) return voiceInputTarget(tab, tab === "general" && third ? VOICE_ANCHORS[third] : undefined);
  return voiceInputTarget("general", second ? VOICE_ANCHORS[second] : undefined);
}

function parseOne(where: string): PageTarget | null {
  const parts = where.split("→").map((part) => part.trim()).filter(Boolean);
  const [first, second, third] = parts;
  if (!first) return null;
  // 游戏语音搬进了插件（ADR 038 第 4 步）：旧位置「设置 → 她怎么回答 → 游戏语音」「琴音 → 设置 → 游戏语音」跳到插件页
  if (third === "游戏语音" && (second === "她怎么回答" || second === "设置")) return { page: "plugin", id: "game-voice" };
  // 音乐页成了放歌插件的页面（ADR 038 第 4 步）：旧位置「音乐」跳到插件页
  if (first === "音乐") return { page: "plugin", id: "music" };
  if (first === "琴音" || first === "琴音陪玩" || first === "角色") {
    if (second === "设置") {
      const group = third ? OLD_COMPANION_GROUPS[third] : undefined;
      return companionTarget(group?.tab ?? "call", group?.anchor);
    }
    if (second === "交流") return companionTarget("call", "settings-call-hotkey");
    // 「琴音 → 唤醒 → 呼叫键」「琴音 → 交互 → 气泡」「琴音 → 记忆」
    const section = ROLE_TABS[second ?? "角色"] ?? "identity";
    if (isCompanionSection(section)) return companionTarget(section, third ? COMPANION_ANCHORS[section]?.[third] : undefined);
    return { page: "assistant", section };
  }
  // 「聊天页 → 她用的模型」：输入框旁的模型菜单（2026-11-25 从「设置 → 她怎么回答」搬回来）
  if (first === "聊天页" || first === "聊天") {
    return second ? { page: "assistant", section: "chat", anchor: CHAT_MODEL_ANCHOR } : { page: "assistant", section: "chat" };
  }
  // 「左侧栏 → ⋯」= 全部页面（ADR 034）
  if (first === "左侧栏" || first === "侧边栏") return { page: "sidebar-pages" };
  // 「插件 → 英雄联盟」= 插件的 App；只写「插件」= 左侧栏的「插件」（ADR 038）
  if (first === "插件") {
    // 「插件 → 英雄联盟 / 游戏语音」：取第一个认得出的
    const id = second?.split("/").map((name) => PLUGIN_NAMES[name.trim()]).find(Boolean);
    // 「插件 → 语音输入 → 文字处理 / 悬浮窗」：App 里的分页和分组
    if (id === "voice-input" && third) return voiceTarget(third, parts[3]);
    return id ? { page: "plugin", id } : { page: "plugins" };
  }
  if (first === "设置") {
    if (second === "其他热键") return oldHotkeysTarget(third);
    // 旧位置「设置 → 插件（→ 英雄联盟）」：插件搬到了左侧栏的「插件」（ADR 038）
    if (second === "插件") {
      const id = third?.split("/").map((name) => PLUGIN_NAMES[name.trim()]).find(Boolean);
      return id ? { page: "plugin", id } : { page: "plugins" };
    }
    // 旧位置「设置 → 权限 → 放歌」（浏览器放歌时选网站的地方）：放歌现在在放歌插件的音乐页（ADR 034、038）
    if (second === "权限" && third === "放歌") return { page: "plugin", id: "music" };
    const tab = ADVANCED_TABS[second ?? "系统"] ?? "system";
    // 旧位置「设置 → 怎么叫她 / 她怎么回答 / 权限 / 用量」：角色页的唤醒 / 交互 / 权限 / 用量
    if (isCompanionSection(tab)) return companionTarget(tab, third ? COMPANION_ANCHORS[tab]?.[third] : undefined);
    return { page: "advanced", tab, anchor: third ? ADVANCED_ANCHORS[tab]?.[third] : undefined };
  }
  // 原来左侧栏的「语音输入」（现在是语音输入插件的 App，ADR 038）
  if (first === "语音输入" || first === "通用") return voiceTarget(second, third);
  if (first === "文字处理") return voiceInputTarget("processing");
  if (first === "历史记录") return voiceInputTarget("history");
  const page = PAGES[first];
  return page ? { page } : null;
}

/** 设置说明里的位置 → 页面目标；认不出返回 null。「A，或 B」只取第一个认得出的。 */
export function parseLocation(where: string): PageTarget | null {
  for (const option of where.split(/，或|,\s*或|；或/)) {
    const target = parseOne(option);
    if (target) return target;
  }
  return null;
}

/** 改了东西、但结果里没有 effects.locations 的工具：按工具名给“改了什么、在哪”。只读的工具不给。 */
const TOOL_LOCATIONS: Record<string, [string, string]> = {
  "models.activate": [m.shell_nav_changed_models(), "设置 → 模型"],
  "models.download": [m.shell_nav_changed_models(), "设置 → 模型"],
  "models.delete": [m.shell_nav_changed_models(), "设置 → 模型"],
  "models.dir.set": [m.shell_nav_changed_models_dir(), "设置 → 模型"],
  "speech.model.download": [m.shell_nav_changed_tts_model(), "设置 → 模型 → 语音合成"],
  "connections.upsert": [m.shell_nav_changed_connections(), "设置 → API 连接"],
  "connections.delete": [m.shell_nav_changed_connections(), "设置 → API 连接"],
  "connections.request_key": [m.shell_nav_changed_connections(), "设置 → API 连接"],
  "voice_chat.install": [m.shell_nav_changed_voice_chat(), "插件 → 游戏语音"],
  "voice_chat.default_mic": [m.shell_nav_changed_default_mic(), "插件 → 游戏语音"],
  "wake.model.download": [m.shell_nav_changed_wake_model(), "琴音 → 唤醒 → 语音唤醒"],
  "skill.manage": [m.shell_nav_changed_skills(), "琴音 → 技能"],
  "memory": [m.shell_nav_changed_memory(), "琴音 → 记忆"],
  "soul.write": [m.shell_nav_changed_soul(), "琴音 → 角色"],
  "automation.save": [m.shell_nav_changed_automation(), "自动化"],
};

/** 一次完成的工具调用改了哪些设置、在界面上的哪里（聊天里显示成可点的链接）。 */
export function changedLocations(tool: string, content: string | null | undefined, args?: string): { label: string; page: string }[] {
  // 换 / 下载模型：按类别说清是哪种模型
  if (tool.startsWith("models.") && tool !== "models.list") {
    let category = "";
    try { category = String((JSON.parse(args ?? "{}") as { category?: string }).category ?? ""); } catch { category = ""; }
    const kind = ({ asr: "语音识别", ocr: "文字识别", tts: "语音合成", search: "联网搜索" } as Record<string, string>)[category];
    const label = ({
      asr: m.shell_nav_changed_asr_model(), ocr: m.shell_nav_changed_ocr_model(),
      tts: m.shell_nav_changed_tts_model(), search: m.shell_nav_changed_search(),
    } as Record<string, string>)[category];
    if (kind && label) return [{ label, page: `设置 → 模型 → ${kind}` }];
  // i18n-ignore-end
  }
  let parsed: unknown = null;
  try { parsed = content ? JSON.parse(content) : null; } catch { parsed = null; }
  const locations = (parsed as { effects?: { locations?: { label: string; page: string }[] } } | null)?.effects?.locations;
  if (Array.isArray(locations) && locations.length) return locations.filter((l) => parseLocation(l.page));
  const known = TOOL_LOCATIONS[tool];
  return known ? [{ label: known[0], page: known[1] }] : [];
}

export const navigationRequest = writable<{ target: PageTarget; n: number } | null>(null);
let counter = 0;

export function openLocation(where: string): boolean {
  const target = parseLocation(where);
  if (!target) return false;
  navigationRequest.set({ target, n: ++counter });
  return true;
}

/** 直接打开某个页面目标（侧栏以外的入口：聊天页的“去设置”、语音输入的“让 Kotone 帮我配置”、`ui.open` 等）。旧目标落到新位置。 */
export function openTarget(target: PageTarget) {
  navigationRequest.set({ target: normalizeTarget(target), n: ++counter });
}
