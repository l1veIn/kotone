// 可切换的角色（琴音是默认的）：每个角色各一份名字、头像、性格、声音、唤醒词；记忆、技能、设置共用。
// 后端：apps/desktop/src-tauri/src/roles.rs（规则在 crates/kotone-agent/src/roles.rs，
// 唤醒词在 crates/kotone-stt/src/wake_word.rs）。角色卡格式照 Character Card V2。
// 浏览器预览用内存里的示例角色；唤醒词检查只做长度和汉字的粗查（真实的拼音检查在桌面版）。
import { invoke } from "@tauri-apps/api/core";
import { isTauri, updateSettings } from "./ipc";
import { m } from "./paraglide/messages.js";

export interface RoleWake { text: string; readings: string[] }
export interface SyllableCheck { character: string; reading: string; choices: string[]; tokens: string[]; missing: string[] }
export interface WakeWordCheck { text: string; syllables: SyllableCheck[]; tokens: string[]; ok: boolean; problems: string[]; warnings: string[] }
export interface RoleInfo {
  id: string; name: string; summary: string; builtin: boolean; avatar: string; voice: string;
  wake: RoleWake; wakeCheck: WakeWordCheck; greeting: string; current: boolean;
  /** 只有 rolesGet 带。 */
  soul?: string;
}
export interface RolePatch { name?: string; summary?: string; avatar?: string; voice?: string; wake?: RoleWake; greeting?: string; soul?: string }
export const DEFAULT_ROLE = "kotone";

// ---------- 浏览器预览 ----------
function previewCheck(text: string, readings: string[] = []): WakeWordCheck {
  const chars = [...text.replace(/\s+/g, "")];
  const problems: string[] = [];
  const warnings: string[] = [];
  if (!chars.length) problems.push(m.role_wake_empty());
  // i18n-ignore-start：汉字范围的正则、多音字读音数据
  else if (chars.some((c) => !/[一-鿿]/.test(c))) problems.push(m.role_wake_hanzi_only());
  else if (chars.length < 3) problems.push(m.role_wake_too_short());
  else if (chars.length > 8) problems.push(m.role_wake_too_long());
  else if (chars.length === 3) warnings.push(m.role_wake_three_chars());
  const choices: Record<string, string[]> = { 行: ["xíng", "háng"], 长: ["zhǎng", "cháng"] };
  // i18n-ignore-end
  const syllables = chars.map((c, i) => {
    const options = choices[c] ?? [`(${c})`];
    const reading = readings[i] && options.includes(readings[i]) ? readings[i] : options[0];
    return { character: c, reading, choices: options, tokens: [reading], missing: [] };
  });
  if (syllables.some((s) => s.choices.length > 1)) warnings.push(m.role_wake_polyphone());
  return { text: chars.join(""), syllables, tokens: syllables.map((s) => s.reading), ok: !problems.length, problems, warnings };
}
const previewSouls: Record<string, string> = {
  // i18n-ignore-start：角色内容（名字、性格、唤醒词规则，和后端一致），不是界面文字
  kotone: "# SOUL\n\n你是屏幕前玩家的游戏陪玩 Agent。说话温暖、轻快、简短；像一起开黑的朋友，不说教、不啰嗦。\n",
  dafeiyu: "# SOUL\n\n你是大肥鱼，一只蓝色长发的鲸鱼娘女仆，屏幕前玩家的游戏陪玩。\n",
  "claude-ojou": "# SOUL\n\n你是 Claude大小姐，优雅又有点傲娇的大小姐，屏幕前玩家的游戏陪玩。\n",
};
const preview: { current: string; roles: Omit<RoleInfo, "wakeCheck" | "current">[]; next: number } = {
  current: DEFAULT_ROLE,
  next: 3,
  roles: [
    { id: "kotone", name: "琴音", summary: "温暖、轻快，像一起开黑的朋友", builtin: true, avatar: "", voice: "", wake: { text: "", readings: [] }, greeting: "" },
    { id: "dafeiyu", name: "大肥鱼", summary: "蓝色鲸鱼娘女仆，贪吃软萌（社区二创，非官方）", builtin: true, avatar: "", voice: "deepseek/stella", wake: { text: "你好大肥鱼", readings: [] }, greeting: "" },
    { id: "claude-ojou", name: "Claude大小姐", summary: "傲娇优雅的大小姐，爱喝红茶（同人角色，非官方）", builtin: true, avatar: "", voice: "deepseek/stella", wake: { text: "你好大小姐", readings: [] }, greeting: "" },
  ],
};
function previewView(r: Omit<RoleInfo, "wakeCheck" | "current">): RoleInfo {
  const text = r.wake.text || (r.id === DEFAULT_ROLE && r.name === "琴音" ? "琴音琴音" : [...r.name].length === 2 ? r.name + r.name : r.name);
  // i18n-ignore-end
  return { ...r, wakeCheck: previewCheck(text, r.wake.readings), current: r.id === preview.current };
}
function previewRole(id: string) {
  const role = preview.roles.find((r) => r.id === id);
  if (!role) throw new Error(m.role_preview_not_found({ id }));
  return role;
}

// ---------- 接口 ----------
export async function rolesList(): Promise<{ current: string; roles: RoleInfo[] }> {
  if (isTauri) return invoke("roles_list");
  return { current: preview.current, roles: preview.roles.map(previewView) };
}
export async function rolesGet(id: string): Promise<RoleInfo> {
  if (isTauri) return invoke("roles_get", { id });
  return { ...previewView(previewRole(id)), soul: previewSouls[id] ?? "# SOUL\n" };
}
export async function rolesSwitch(id: string): Promise<void> {
  if (isTauri) { await invoke("roles_switch", { id }); return; }
  const role = previewRole(id);
  preview.current = id;
  await updateSettings({ agent: { roleId: id, identity: { name: role.name, avatar: role.avatar } } });
}
export async function rolesUpdate(id: string, patch: RolePatch): Promise<RoleInfo> {
  if (isTauri) return invoke("roles_update", { id, patch });
  const role = previewRole(id);
  const { soul, ...rest } = patch;
  if (rest.name !== undefined && !rest.name.trim()) throw new Error(m.role_preview_name_invalid());
  Object.assign(role, rest);
  if (soul !== undefined) previewSouls[id] = soul;
  if (preview.current === id) await updateSettings({ agent: { identity: { name: role.name, avatar: role.avatar } } });
  return rolesGet(id);
}
export async function rolesCreate(patch: RolePatch): Promise<RoleInfo> {
  if (isTauri) return invoke("roles_create", { patch });
  if (!patch.name?.trim()) throw new Error(m.role_preview_name_required());
  const id = `r${preview.next++}`;
  preview.roles.push({ id, name: patch.name.trim(), summary: patch.summary ?? "", builtin: false, avatar: "", voice: patch.voice ?? "", wake: patch.wake ?? { text: "", readings: [] }, greeting: "" });
  // i18n-ignore-start：角色内容（名字、性格、唤醒词规则，和后端一致），不是界面文字
  previewSouls[id] = patch.soul ?? `# SOUL\n\n你是${patch.name.trim()}，屏幕前玩家的游戏陪玩。\n`;
  // i18n-ignore-end
  return rolesGet(id);
}
export async function rolesDelete(id: string): Promise<void> {
  if (isTauri) { await invoke("roles_delete", { id }); return; }
  const role = previewRole(id);
  if (role.builtin) return;
  preview.roles = preview.roles.filter((r) => r.id !== id);
  if (preview.current === id) await rolesSwitch(DEFAULT_ROLE);
}
export async function rolesExport(id: string): Promise<unknown> {
  if (isTauri) return invoke("roles_export", { id });
  const role = previewRole(id);
  return { spec: "chara_card_v2", spec_version: "2.0", data: { name: role.name, description: previewSouls[id] ?? "", extensions: { kotone: { soulMd: true, roleId: id, voice: role.voice, wake: role.wake } } } };
}
export async function rolesImport(card: unknown): Promise<RoleInfo> {
  if (isTauri) return invoke("roles_import", { card });
  const data = (card as { data?: { name?: string; description?: string } })?.data ?? (card as { name?: string; description?: string });
  if (!data?.name) throw new Error(m.role_preview_card_no_name());
  return rolesCreate({ name: data.name, soul: data.description ?? "" });
}
export async function rolesCheckWake(text: string, readings: string[]): Promise<WakeWordCheck> {
  if (isTauri) return invoke("roles_check_wake", { text, readings });
  return previewCheck(text, readings);
}

/** 没上传头像、也没有自带形象的非默认角色用的占位：名字的第一个字。琴音（默认角色）用内置立绘，返回 null。 */
export function placeholderLetter(roleId: string | undefined, name: string | undefined, avatar: string | undefined): string | null {
  if (avatar || !roleId || roleId === DEFAULT_ROLE || BUNDLED_FACES[roleId]) return null;
  return [...(name?.trim() || "?")][0] ?? "?";
}

// ---------- 自带形象的内置角色（社区二创，CC BY-NC-SA 4.0；见 src/assets/roles/README.md） ----------
import fishIdle from "../assets/roles/dafeiyu/idle.webp";
import fishListening from "../assets/roles/dafeiyu/listening.webp";
import fishThinking from "../assets/roles/dafeiyu/thinking.webp";
import fishSpeaking from "../assets/roles/dafeiyu/speaking.webp";
import fishAsking from "../assets/roles/dafeiyu/asking.webp";
import fishNote from "../assets/roles/dafeiyu/note.webp";
import fishError from "../assets/roles/dafeiyu/error.webp";
import ojouIdle from "../assets/roles/claude-ojou/idle.webp";
import ojouListening from "../assets/roles/claude-ojou/listening.webp";
import ojouThinking from "../assets/roles/claude-ojou/thinking.webp";
import ojouSpeaking from "../assets/roles/claude-ojou/speaking.webp";
import ojouAsking from "../assets/roles/claude-ojou/asking.webp";
import ojouNote from "../assets/roles/claude-ojou/note.webp";
import ojouError from "../assets/roles/claude-ojou/error.webp";

/** 气泡的几种状态（和 bubble.ts 的 BubblePhase 一致，外加 idle = 平时的头像）。 */
export type FaceMood = "idle" | "listening" | "recognizing" | "thinking" | "speaking" | "asking" | "note" | "error";
export type RoleFaces = Record<FaceMood, string>;

const face = (idle: string, listening: string, thinking: string, speaking: string, asking: string, note: string, error: string): RoleFaces =>
  ({ idle, listening, recognizing: listening, thinking, speaking, asking, note, error });

/** 角色 id → 自带的表情头像（没有的角色：琴音用内置立绘，其他用名字首字占位）。 */
export const BUNDLED_FACES: Record<string, RoleFaces> = {
  dafeiyu: face(fishIdle, fishListening, fishThinking, fishSpeaking, fishAsking, fishNote, fishError),
  "claude-ojou": face(ojouIdle, ojouListening, ojouThinking, ojouSpeaking, ojouAsking, ojouNote, ojouError),
};

/** 自带形象的署名（CC BY-NC-SA 4.0 要求署名；界面上显示）。 */
export const FACE_CREDITS: Record<string, string> = {
  // i18n-ignore-start：立绘的版权署名，跟着素材走
  dafeiyu: "形象：社区二创 DeepSeek「大肥鱼」（非官方）· 原型 @上善无形 · 设计 @ZipZipPipe · 素材 PC2005-cloud/dsh-pet · CC BY-NC-SA 4.0（非商用）",
  "claude-ojou": "素材：Claude大小姐的日记本（github.com/opsCar/claude-daxiaojie-assets）· 角色设定：ZipZipPipe（胖次狐）· 设定图：lmy414/ai-chan-sheets · CC BY-NC-SA 4.0（非商用）· 同人作品，与 Anthropic 无关",
  // i18n-ignore-end
};

/** 这个角色自带的表情头像；玩家上传了头像（`avatar` 非空）时不用。 */
export function bundledFaces(roleId: string | undefined, avatar: string | undefined): RoleFaces | null {
  if (avatar || !roleId) return null;
  return BUNDLED_FACES[roleId] ?? null;
}

