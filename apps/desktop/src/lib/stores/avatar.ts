// 玩家上传的琴音头像（设置 agent.identity.avatar → 工作区里的图片）→ object URL；没有 = null。
import { derived, type Readable } from "svelte/store";
import { bundledFaces, placeholderLetter, type RoleFaces } from "../roles";
import { workspaceAvatar } from "../workspace";
import { settingsStore } from "./ui";

/** 当前角色自带的表情头像（没上传头像时）。 */
export const roleFaces: Readable<RoleFaces | null> = derived(settingsStore, ($settings) =>
  bundledFaces($settings?.agent?.roleId, $settings?.agent?.identity?.avatar));

/** 当前角色没有头像、又不是琴音时的占位字（名字的第一个字）。 */
export const avatarLetter: Readable<string | null> = derived(settingsStore, ($settings) =>
  placeholderLetter($settings?.agent?.roleId, $settings?.agent?.identity?.name, $settings?.agent?.identity?.avatar));

let current: { name: string; url: string | null } = { name: "", url: null };

export const customAvatar: Readable<string | null> = derived(settingsStore, ($settings, set) => {
  const name = $settings?.agent?.identity?.avatar ?? "";
  if (name === current.name) { set(current.url); return; }
  if (!name) {
    if (current.url) URL.revokeObjectURL(current.url);
    current = { name: "", url: null };
    set(null);
    return;
  }
  let cancelled = false;
  void workspaceAvatar(name).then((bytes) => {
    if (cancelled) return;
    if (current.url) URL.revokeObjectURL(current.url);
    current = { name, url: bytes ? URL.createObjectURL(new Blob([bytes as BlobPart])) : null };
    set(current.url);
  }).catch(() => { if (!cancelled) set(null); });
  return () => { cancelled = true; };
}, null as string | null);
