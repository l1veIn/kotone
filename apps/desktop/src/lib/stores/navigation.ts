// 跳到某个设置的位置（聊天里的工具卡片“琴音改了 xxx → 去看看”）。
// 位置用设置说明里的写法：「琴音 → 设置 → 呼叫」「设置 → 模型 → 语音识别」「语音输入 → 悬浮窗」……
// 设置窗口（Settings.svelte）订阅这里的请求并切页面 / 分页 / 滚到分组。
import { writable } from "svelte/store";

export type PageTarget =
  | { page: "general" | "processing" | "history" | "automation" | "game" | "about" }
  | { page: "assistant"; section: "chat" | "identity" | "memory" | "skills" | "settings"; anchor?: string }
  | { page: "advanced"; tab: string };

const ASSISTANT_TABS: Record<string, "identity" | "memory" | "skills" | "settings"> = {
  "她是谁": "identity", "记忆": "memory", "技能": "skills", "设置": "settings",
};
const ASSISTANT_GROUPS: Record<string, string> = {
  "AI 连接": "companion-settings-ai",
  "呼叫": "companion-settings-call", "语音唤醒": "companion-settings-call", "快捷指令": "companion-settings-call",
  "回答": "companion-settings-reply", "对话气泡": "companion-settings-reply", "弹幕": "companion-settings-reply",
  "游戏语音": "companion-settings-teammates",
  "权限": "companion-settings-permissions",
};
const ADVANCED_TABS: Record<string, string> = {
  "系统": "system", "模型": "models", "API 连接": "connections", "其他热键": "hotkeys",
  "音效": "sound", "调参": "tuning", "外观": "appearance",
};
const PAGES: Record<string, "general" | "processing" | "history" | "automation" | "game" | "about"> = {
  "语音输入": "general", "通用": "general", "文字处理": "processing", "历史记录": "history",
  "自动化": "automation", "游戏列表": "game", "游戏": "game", "关于": "about",
};

/** 设置说明里的位置 → 页面目标；认不出返回 null。 */
export function parseLocation(where: string): PageTarget | null {
  const parts = where.split("→").map((part) => part.trim()).filter(Boolean);
  const [first, second, third] = parts;
  if (!first) return null;
  if (first === "琴音") {
    const section = ASSISTANT_TABS[second ?? "设置"] ?? "settings";
    return { page: "assistant", section, anchor: section === "settings" && third ? ASSISTANT_GROUPS[third] : undefined };
  }
  if (first === "设置") return { page: "advanced", tab: ADVANCED_TABS[second ?? "系统"] ?? "system" };
  const page = PAGES[first];
  return page ? { page } : null;
}

/** 改了东西、但结果里没有 effects.locations 的工具：按工具名给“改了什么、在哪”。只读的工具不给。 */
const TOOL_LOCATIONS: Record<string, [string, string]> = {
  "models.activate": ["模型", "设置 → 模型"],
  "models.download": ["模型", "设置 → 模型"],
  "models.delete": ["模型", "设置 → 模型"],
  "models.dir.set": ["模型目录", "设置 → 模型"],
  "speech.model.download": ["语音合成模型", "设置 → 模型"],
  "connections.upsert": ["API 连接", "设置 → API 连接"],
  "connections.delete": ["API 连接", "设置 → API 连接"],
  "connections.request_key": ["API 连接", "设置 → API 连接"],
  "voice_chat.install": ["游戏语音", "琴音 → 设置 → 游戏语音"],
  "voice_chat.default_mic": ["Windows 默认麦克风", "琴音 → 设置 → 游戏语音"],
  "wake.model.download": ["唤醒模型", "琴音 → 设置 → 呼叫"],
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
