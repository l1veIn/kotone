// 跳到某个设置的位置（聊天里的工具卡片“琴音改了 xxx → 去看看”）。
// 位置用设置说明（crates/kotone-core/src/settings_guide.rs）里的写法：
// 「设置 → 怎么叫她」「设置 → 模型 → 语音识别」「语音输入 → 悬浮窗」……
// 旧版本写进对话里的位置（「琴音 → 设置 → 呼叫」「设置 → 其他热键」等）也认，跳到它们的新位置（ADR 028）。
// 设置窗口（Settings.svelte）订阅这里的请求并切页面 / 分页 / 滚到分组。
import { writable } from "svelte/store";

export type AssistantSection = "chat" | "identity" | "memory" | "skills";
export type VoiceInputPage = "general" | "processing" | "history";

export type PageTarget =
  | { page: VoiceInputPage | "automation" | "game" | "about" | "legacy-reminders"; anchor?: string }
  | { page: "assistant"; section: AssistantSection }
  | { page: "advanced"; tab: string; anchor?: string };

/** 「设置」的分页（页面 ID 不变；陪玩组是 ADR 028 新加的分页）。 */
export const ADVANCED_TABS: Record<string, string> = {
  "怎么叫她": "call", "她怎么回答": "reply", "权限": "permissions", "用量": "usage",
  "模型": "models", "API 连接": "connections", "声音": "sound", "调参": "tuning", "外观": "appearance", "系统": "system",
  // 旧写法
  "音效": "sound",
};

/** 分页里的分组锚点（第三段）。 */
const ADVANCED_ANCHORS: Record<string, Record<string, string>> = {
  call: { "呼叫键": "settings-call-hotkey", "语音唤醒": "settings-call-wake", "快捷指令": "companion-title" },
  reply: { "AI 连接": "settings-reply-ai", "对话气泡": "settings-reply-bubble", "声音与弹幕": "settings-reply-output", "游戏语音": "settings-reply-teammates" },
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
  "AI 连接": { tab: "reply", anchor: "settings-reply-ai" },
  "呼叫": { tab: "call", anchor: "settings-call-hotkey" },
  "语音唤醒": { tab: "call", anchor: "settings-call-wake" },
  "快捷指令": { tab: "call", anchor: "companion-title" },
  "回答": { tab: "reply", anchor: "settings-reply-output" },
  "对话气泡": { tab: "reply", anchor: "settings-reply-bubble" },
  "弹幕": { tab: "reply", anchor: "settings-reply-output" },
  "游戏语音": { tab: "reply", anchor: "settings-reply-teammates" },
  "权限": { tab: "permissions" },
  "用量": { tab: "usage" },
};

const PAGES: Record<string, "automation" | "game" | "about" | "legacy-reminders"> = {
  "自动化": "automation", "游戏列表": "game", "游戏": "game", "游戏适配": "game", "关于": "about", "提醒助手": "legacy-reminders",
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
  if (first === "琴音" || first === "琴音陪玩" || first === "角色") {
    if (second === "设置") {
      const group = third ? OLD_COMPANION_GROUPS[third] : undefined;
      return { page: "advanced", tab: group?.tab ?? "call", anchor: group?.anchor };
    }
    if (second === "交流") return { page: "advanced", tab: "call", anchor: "settings-call-hotkey" };
    return { page: "assistant", section: ROLE_TABS[second ?? "她是谁"] ?? "identity" };
  }
  if (first === "设置") {
    if (second === "其他热键") return oldHotkeysTarget(third);
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
  "models.activate": ["模型", "设置 → 模型"],
  "models.download": ["模型", "设置 → 模型"],
  "models.delete": ["模型", "设置 → 模型"],
  "models.dir.set": ["模型目录", "设置 → 模型"],
  "speech.model.download": ["语音合成模型", "设置 → 模型 → 语音合成"],
  "connections.upsert": ["API 连接", "设置 → API 连接"],
  "connections.delete": ["API 连接", "设置 → API 连接"],
  "connections.request_key": ["API 连接", "设置 → API 连接"],
  "voice_chat.install": ["游戏语音", "设置 → 她怎么回答 → 游戏语音"],
  "voice_chat.default_mic": ["Windows 默认麦克风", "设置 → 她怎么回答 → 游戏语音"],
  "wake.model.download": ["唤醒模型", "设置 → 怎么叫她 → 语音唤醒"],
  "skill.manage": ["技能", "琴音 → 技能"],
  "memory": ["记忆", "琴音 → 记忆"],
  "soul.write": ["性格", "琴音 → 她是谁"],
  "automation.save": ["自动化", "自动化"],
};

/** 一次完成的工具调用改了哪些设置、在界面上的哪里（聊天里显示成可点的链接）。 */
export function changedLocations(tool: string, content: string | null | undefined, args?: string): { label: string; page: string }[] {
  // 换 / 下载模型：按类别说清是哪种模型
  if (tool.startsWith("models.") && tool !== "models.list") {
    let category = "";
    try { category = String((JSON.parse(args ?? "{}") as { category?: string }).category ?? ""); } catch { category = ""; }
    const kind = ({ asr: "语音识别", ocr: "文字识别", tts: "语音合成", search: "联网搜索" } as Record<string, string>)[category];
    if (kind) return [{ label: kind === "联网搜索" ? kind : `${kind}模型`, page: `设置 → 模型 → ${kind}` }];
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
