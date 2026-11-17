// 琴音的工作区（ADR 022 / 023）：她是谁（名字、头像、SOUL）、她记得什么（USER、MEMORY、游戏记忆）、她会什么（Skill）。
// 后端：apps/desktop/src-tauri/src/workspace_cmd.rs（规则在 crates/kotone-agent/src/workspace.rs）。
// 浏览器预览用内存里的示例数据，不读写任何文件。
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./ipc";

export interface FileSize { chars: number; limit: number }
export interface SkillInfo { name: string; description: string }
export interface WorkspaceOverview {
  name: string;
  defaultName: string;
  dir: string;
  files: { soul: FileSize; user: FileSize; memory: FileSize };
  gameLimit: number;
  games: { id: string; chars: number }[];
  skills: SkillInfo[];
}
/** 能直接编辑的文件：`SOUL.md`、`USER.md`、`MEMORY.md`、`games/<游戏 id>`。 */
export type WorkspaceFile = "SOUL.md" | "USER.md" | "MEMORY.md" | `games/${string}`;

const LIMITS: Record<string, number> = { "SOUL.md": 4000, "USER.md": 1375, "MEMORY.md": 2200 };
const GAME_LIMIT = 4000;
const preview = {
  files: new Map<string, string>([
    ["SOUL.md", "# SOUL\n\n你是屏幕前玩家的游戏陪玩 Agent。说话温暖、轻快、简短；像一起开黑的朋友，不说教、不啰嗦。\n"],
    ["USER.md", ""],
    ["MEMORY.md", "# MEMORY\n\n- 玩家习惯用中文语音交流\n"],
    ["games/pubg", "# 游戏记忆：pubg\n\n- 玩家喜欢跳 P 城\n"],
  ]),
  skills: [{ name: "pubg-callouts", description: "PUBG 报点用语与方位说法" }] as SkillInfo[],
  avatar: null as Uint8Array | null,
};

function limitOf(file: string): number {
  return file.startsWith("games/") ? GAME_LIMIT : LIMITS[file] ?? 1000;
}

export async function workspaceOverview(): Promise<WorkspaceOverview> {
  if (isTauri) return invoke("workspace_overview");
  const size = (file: string) => ({ chars: [...(preview.files.get(file) ?? "")].length, limit: limitOf(file) });
  return {
    name: "琴音", defaultName: "琴音", dir: "（浏览器预览）",
    files: { soul: size("SOUL.md"), user: size("USER.md"), memory: size("MEMORY.md") },
    gameLimit: GAME_LIMIT,
    games: [...preview.files.keys()].filter((k) => k.startsWith("games/")).map((k) => ({ id: k.slice(6), chars: size(k).chars })),
    skills: structuredClone(preview.skills),
  };
}

export async function workspaceRead(file: WorkspaceFile): Promise<string> {
  if (isTauri) return invoke("workspace_read", { file });
  return preview.files.get(file) ?? "";
}

export async function workspaceWrite(file: WorkspaceFile, text: string): Promise<void> {
  if (isTauri) return invoke("workspace_write", { file, text });
  if ([...text].length > limitOf(file)) throw new Error(`超过 ${limitOf(file)} 字`);
  preview.files.set(file, text);
}

export async function workspaceDeleteGame(id: string): Promise<void> {
  if (isTauri) return invoke("workspace_delete_game", { id });
  preview.files.delete(`games/${id}`);
}

/** 保存头像，返回文件名（写进设置 `agent.identity.avatar`）。 */
export async function workspaceSetAvatar(bytes: Uint8Array, ext: string): Promise<string> {
  if (isTauri) return invoke("workspace_set_avatar", { bytes: Array.from(bytes), ext });
  if (bytes.length > 2 * 1024 * 1024) throw new Error("头像不能超过 2 MB");
  preview.avatar = bytes;
  return `avatar.${ext.toLowerCase()}`;
}

export async function workspaceAvatar(name: string): Promise<Uint8Array | null> {
  if (!name) return null;
  if (isTauri) {
    const bytes = await invoke<number[] | null>("workspace_avatar", { name });
    return bytes ? new Uint8Array(bytes) : null;
  }
  return preview.avatar;
}

export async function workspaceDeleteSkill(name: string): Promise<void> {
  if (isTauri) return invoke("workspace_delete_skill", { name });
  preview.skills = preview.skills.filter((s) => s.name !== name);
}

export async function workspaceImportSkill(dir: string): Promise<SkillInfo> {
  if (isTauri) return invoke("workspace_import_skill", { dir });
  throw new Error("浏览器预览不能导入 Skill");
}

export async function workspaceOpenDir(skill?: string): Promise<void> {
  if (isTauri) return invoke("workspace_open_dir", { skill: skill ?? null });
}
