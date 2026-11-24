// 跳到某个设置的位置（聊天里的工具卡片“琴音改了 xxx → 去看看”）。
// 位置用设置说明（crates/kotone-core/src/settings_guide.rs）里的写法：
// 「设置 → 怎么叫她」「设置 → 模型 → 语音识别」「语音输入 → 悬浮窗」……
// 旧版本写进对话里的位置（「琴音 → 设置 → 呼叫」「设置 → 其他热键」等）也认，跳到它们的新位置（ADR 028）。
// 设置窗口（Settings.svelte）订阅这里的请求并切页面 / 分页 / 滚到分组。
import { writable } from "svelte/store";
import { m } from "../paraglide/messages.js";

export type AssistantSection = "chat" | "identity" | "memory" | "skills";
export type VoiceInputPage = "general" | "processing" | "history";

export type PageTarget =
  | { page: VoiceInputPage | "automation" | "game" | "music" | "about" | "legacy-reminders"; anchor?: string }
  /** 左侧栏的“全部页面”（排顺序、显示 / 隐藏；ADR 034 第六节）。 */
  | { page: "sidebar-pages" }
  | { page: "assistant"; section: AssistantSection }
  | { page: "advanced"; tab: string; anchor?: string }
  /** 插件的页面（左侧栏「插件」组，ADR 038）。 */
  | { page: "plugin"; id: string };

/** 「设置」的分页（页面 ID 不变；陪玩组是 ADR 028 新加的分页）。 */
/** 左侧栏的“全部页面”开着（侧栏的“⋯”、聊天里“去看看：左侧栏 → ⋯”）。 */
export const sidebarPagesOpen = writable(false);

export const ADVANCED_TABS: Record<string, string> = {
  // i18n-ignore-start：设置位置的词：要和后端 settings_guide.rs 写的位置一字不差（玩家点“去看看”时据此跳转），不是界面文字
  "怎么叫她": "call", "她怎么回答": "reply", "权限": "permissions", "用量": "usage",
  "模型": "models", "API 连接": "connections", "声音": "sound", "调参": "tuning", "外观": "appearance", "系统": "system",
  "插件": "plugins",
  // 旧写法
  "音效": "sound",
};

/** 分页里的分组锚点（第三段）。 */
const ADVANCED_ANCHORS: Record<string, Record<string, string>> = {
  call: { "呼叫键": "settings-call-hotkey", "语音唤醒": "settings-call-wake", "快捷指令": "companion-title" },
  reply: {
    "她用的模型": "settings-reply-ai", "气泡": "settings-reply-bubble", "声音与弹幕": "settings-reply-output", "回答方式": "settings-reply-output",
    // 旧写法（术语表 v2 之前的小节名；以前的对话里还引用着）
    "AI 连接": "settings-reply-ai", "对话气泡": "settings-reply-bubble",
  },
  models: { "语音识别": "models-asr", "文字识别": "models-ocr", "语音合成": "models-tts", "联网搜索": "models-search" },
  system: { "停下": "system-stop", "打开软件时显示": "system-start-page" },
};

/** 「语音输入」子板块的分页与分组。 */
const VOICE_TABS: Record<string, VoiceInputPage> = { "输入": "general", "文字处理": "processing", "历史": "history", "历史记录": "history" };
const VOICE_ANCHORS: Record<string, string> = {
  "语音识别": "voice-asr", "其他热键": "voice-hotkeys", "悬浮窗": "voice-overlay", "交互模式": "voice-mode", "热键": "voice-hotkey",
};

const ROLE_TABS: Record<string, AssistantSection> = {
  "她是谁": "identity", "角色": "identity", "记忆": "memory", "技能": "skills", "聊天": "chat",
};

/** 旧版「琴音 → 设置 → 分组」/「琴音 → 交流」：搬进「设置」之后的位置。 */
const OLD_COMPANION_GROUPS: Record<string, { tab: string; anchor?: string }> = {
  "她用的模型": { tab: "reply", anchor: "settings-reply-ai" },
  "AI 连接": { tab: "reply", anchor: "settings-reply-ai" },
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

/** 插件的中文名 → 插件 id（「插件 → 英雄联盟」）：名字从插件自己的文案读（plugin_<id>_name）。 */
const PLUGIN_NAMES: Record<string, string> = Object.fromEntries(
  Object.entries(import.meta.glob<Record<string, string>>("../../../../../plugins/*/messages/zh-CN.json", { eager: true, import: "default" }))
    .map(([path, messages]) => {
      const id = path.split("/plugins/")[1]?.split("/")[0] ?? "";
      return [messages[`plugin_${id.replace(/-/g, "_")}_name`] ?? id, id];
    }),
);

const PAGES: Record<string, "automation" | "game" | "music" | "about" | "legacy-reminders"> = {
  "自动化": "automation", "游戏列表": "game", "游戏": "game", "游戏适配": "game", "音乐": "music", "关于": "about", "提醒助手": "legacy-reminders",
};

/** 旧版「设置 → 其他热键」里的热键现在分散在三处；按第三段（热键名）分，认不出就去语音输入的其他热键。 */
function oldHotkeysTarget(name: string | undefined): PageTarget {
  if (name && /启动|停止 Kotone|本地语音识别/.test(name) && !/自动化|停下/.test(name)) return { page: "general", anchor: "voice-asr" };
  if (name && /自动化|停下/.test(name)) return { page: "advanced", tab: "system", anchor: "system-stop" };
  return { page: "general", anchor: "voice-hotkeys" };
}

function parseOne(where: string): PageTarget | null {
  const parts = where.split("→").map((part) => part.trim()).filter(Boolean);
  const [first, second, third] = parts;
  if (!first) return null;
  // 游戏语音搬进了插件（ADR 038 第 4 步）：旧位置「设置 → 她怎么回答 → 游戏语音」「琴音 → 设置 → 游戏语音」跳到插件页
  if (third === "游戏语音" && (second === "她怎么回答" || second === "设置")) return { page: "plugin", id: "game-voice" };
  if (first === "琴音" || first === "琴音陪玩" || first === "角色") {
    if (second === "设置") {
      const group = third ? OLD_COMPANION_GROUPS[third] : undefined;
      return { page: "advanced", tab: group?.tab ?? "call", anchor: group?.anchor };
    }
    if (second === "交流") return { page: "advanced", tab: "call", anchor: "settings-call-hotkey" };
    return { page: "assistant", section: ROLE_TABS[second ?? "她是谁"] ?? "identity" };
  }
  // 「左侧栏 → ⋯」= 全部页面（ADR 034）
  if (first === "左侧栏" || first === "侧边栏") return { page: "sidebar-pages" };
  // 「插件 → 英雄联盟」= 插件的页面；只写「插件」= 设置里的插件列表（ADR 038）
  if (first === "插件") {
    // 「插件 → 英雄联盟 / 游戏语音」：取第一个认得出的
    const id = second?.split("/").map((name) => PLUGIN_NAMES[name.trim()]).find(Boolean);
    return id ? { page: "plugin", id } : { page: "advanced", tab: "plugins" };
  }
  if (first === "设置") {
    if (second === "其他热键") return oldHotkeysTarget(third);
    // 旧位置「设置 → 权限 → 放歌」（浏览器放歌时选网站的地方）：放歌现在在音乐页（ADR 034）
    if (second === "权限" && third === "放歌") return { page: "music" };
    const tab = ADVANCED_TABS[second ?? "系统"] ?? "system";
    return { page: "advanced", tab, anchor: third ? ADVANCED_ANCHORS[tab]?.[third] : undefined };
  }
  if (first === "语音输入" || first === "通用") {
    const page = VOICE_TABS[second ?? "输入"];
    if (page) return { page, anchor: page === "general" && third ? VOICE_ANCHORS[third] : undefined };
    return { page: "general", anchor: second ? VOICE_ANCHORS[second] : undefined };
  }
  if (first === "文字处理") return { page: "processing" };
  if (first === "历史记录") return { page: "history" };
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
  "wake.model.download": [m.shell_nav_changed_wake_model(), "设置 → 怎么叫她 → 语音唤醒"],
  "skill.manage": [m.shell_nav_changed_skills(), "琴音 → 技能"],
  "memory": [m.shell_nav_changed_memory(), "琴音 → 记忆"],
  "soul.write": [m.shell_nav_changed_soul(), "琴音 → 她是谁"],
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

/** 直接打开某个页面目标（侧栏以外的入口：聊天页的“去设置”、语音输入的“让 Kotone 帮我配置”等）。 */
export function openTarget(target: PageTarget) {
  navigationRequest.set({ target, n: ++counter });
}
