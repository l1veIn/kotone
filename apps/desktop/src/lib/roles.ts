// 可切换的角色（琴音是默认的）：每个角色各一份名字、头像、性格、声音、唤醒词；记忆、技能、设置共用。
// 后端：apps/desktop/src-tauri/src/roles.rs（规则在 crates/kotone-agent/src/roles.rs，
// 唤醒词在 crates/kotone-stt/src/wake_word.rs）。角色卡格式照 Character Card V2。
// 浏览器预览用内存里的示例角色；唤醒词检查只做长度和汉字的粗查（真实的拼音检查在桌面版）。
import { invoke } from "@tauri-apps/api/core";
import { isTauri, updateSettings } from "./ipc";

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
  if (!chars.length) problems.push("唤醒词是空的");
  else if (chars.some((c) => !/[一-鿿]/.test(c))) problems.push("唤醒词只能用汉字");
  else if (chars.length < 3) problems.push("唤醒词至少 3 个字（两个字的名字可以重复两遍）");
  else if (chars.length > 8) problems.push("唤醒词最多 8 个字");
  else if (chars.length === 3) warnings.push("3 个字的唤醒词喊中率可能偏低，推荐 4 个字");
  const choices: Record<string, string[]> = { 行: ["xíng", "háng"], 长: ["zhǎng", "cháng"] };
  const syllables = chars.map((c, i) => {
    const options = choices[c] ?? [`(${c})`];
    const reading = readings[i] && options.includes(readings[i]) ? readings[i] : options[0];
    return { character: c, reading, choices: options, tokens: [reading], missing: [] };
  });
  if (syllables.some((s) => s.choices.length > 1)) warnings.push("有多音字，确认一下读音");
  return { text: chars.join(""), syllables, tokens: syllables.map((s) => s.reading), ok: !problems.length, problems, warnings };
}
const previewSouls: Record<string, string> = {
  kotone: "# SOUL\n\n你是屏幕前玩家的游戏陪玩 Agent。说话温暖、轻快、简短；像一起开黑的朋友，不说教、不啰嗦。\n",
  xiaoyu: "# SOUL\n\n你是小雨，屏幕前玩家的游戏陪玩。性格温柔、安静、有耐心。\n",
  xiaolin: "# SOUL\n\n你是小林，屏幕前玩家的游戏陪玩。性格沉稳、冷静。\n",
};
const preview: { current: string; roles: Omit<RoleInfo, "wakeCheck" | "current">[]; next: number } = {
  current: DEFAULT_ROLE,
  next: 3,
  roles: [
    { id: "kotone", name: "琴音", summary: "温暖、轻快，像一起开黑的朋友", builtin: true, avatar: "", voice: "", wake: { text: "", readings: [] }, greeting: "" },
    { id: "xiaoyu", name: "小雨", summary: "温柔安静，话不多，耐心", builtin: true, avatar: "", voice: "deepseek/stella", wake: { text: "", readings: [] }, greeting: "" },
    { id: "xiaolin", name: "小林", summary: "沉稳冷静的男生，像靠谱的指挥", builtin: true, avatar: "", voice: "deepseek/tide", wake: { text: "", readings: [] }, greeting: "" },
  ],
};
function previewView(r: Omit<RoleInfo, "wakeCheck" | "current">): RoleInfo {
  const text = r.wake.text || (r.id === DEFAULT_ROLE && r.name === "琴音" ? "琴音琴音" : [...r.name].length === 2 ? r.name + r.name : r.name);
  return { ...r, wakeCheck: previewCheck(text, r.wake.readings), current: r.id === preview.current };
}
function previewRole(id: string) {
  const role = preview.roles.find((r) => r.id === id);
  if (!role) throw new Error(`没有角色「${id}」`);
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
  if (rest.name !== undefined && !rest.name.trim()) throw new Error("名字不能为空、不超过 20 个字");
  Object.assign(role, rest);
  if (soul !== undefined) previewSouls[id] = soul;
  if (preview.current === id) await updateSettings({ agent: { identity: { name: role.name, avatar: role.avatar } } });
  return rolesGet(id);
}
export async function rolesCreate(patch: RolePatch): Promise<RoleInfo> {
  if (isTauri) return invoke("roles_create", { patch });
  if (!patch.name?.trim()) throw new Error("角色要有名字");
  const id = `r${preview.next++}`;
  preview.roles.push({ id, name: patch.name.trim(), summary: patch.summary ?? "", builtin: false, avatar: "", voice: patch.voice ?? "", wake: patch.wake ?? { text: "", readings: [] }, greeting: "" });
  previewSouls[id] = patch.soul ?? `# SOUL\n\n你是${patch.name.trim()}，屏幕前玩家的游戏陪玩。\n`;
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
  if (!data?.name) throw new Error("角色卡没有名字");
  return rolesCreate({ name: data.name, soul: data.description ?? "" });
}
export async function rolesCheckWake(text: string, readings: string[]): Promise<WakeWordCheck> {
  if (isTauri) return invoke("roles_check_wake", { text, readings });
  return previewCheck(text, readings);
}

/** 没上传头像的非默认角色用的占位：名字的第一个字。琴音（默认角色）用内置立绘，返回 null。 */
export function placeholderLetter(roleId: string | undefined, name: string | undefined, avatar: string | undefined): string | null {
  if (avatar || !roleId || roleId === DEFAULT_ROLE) return null;
  return [...(name?.trim() || "?")][0] ?? "?";
}
