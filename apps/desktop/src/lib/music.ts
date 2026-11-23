// 音乐（ADR 034）：Kotone 自己的播放器（Rust：crates/kotone-music，先只接网易云音乐）。
// 这里是音乐页、左侧栏的小播放条、聊天页的扫码卡片共用的状态和命令；字段与后端 serde（camelCase）一致。
// 浏览器预览（dev:web）用 ipc/musicMock 里的内存状态。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readable, type Readable } from "svelte/store";
import { isTauri } from "./ipc";
import * as musicMock from "./ipc/musicMock";
import { m } from "./paraglide/messages.js";

/** 这首能不能完整放：要 VIP、要单独买（付费专辑）、网易云没有版权 / 下架了。 */
export type MusicPlayable = "full" | "vip" | "paid" | "unavailable";
export type MusicQuality = "standard" | "exhigh" | "lossless";
/**
 * 登录网易云音乐（ADR 034）：idle 没在登录；needed 她要放歌而你没登录（她不自己弹窗，聊天页出按钮）；
 * waiting 网易云官方登录页的窗口开着、等你用 App 扫码；expired 5 分钟没登录、窗口关了；failed 出错了。
 * scanned 是旧的自制二维码流程留下的，现在不会出现。
 */
export type MusicLoginStage = "idle" | "needed" | "waiting" | "scanned" | "expired" | "failed";
export type MusicStatus = "idle" | "loading" | "playing" | "paused" | "error";

export interface MusicTrack {
  /** 带来源前缀：“ne:1901371647”。 */
  id: string;
  name: string;
  artists: string[];
  album: string;
  coverUrl: string | null;
  durationMs: number;
  playable: MusicPlayable;
  reason: string | null;
}
export interface MusicPlaylist {
  id: string;
  name: string;
  coverUrl: string | null;
  trackCount: number;
  creator: string;
  /** “我喜欢的音乐”。 */
  liked: boolean;
  /** 自己建的（不是收藏的）。 */
  mine: boolean;
}
export interface MusicAccount { id: string; nickname: string; avatarUrl: string | null; vip: boolean }
/** qrSvg 现在总是 null（登录在网易云自己的页面里扫码）；message 是一句提示或出错原因。 */
export interface MusicLogin { stage: MusicLoginStage; qrSvg: string | null; message: string | null }
export interface MusicPlayer {
  status: MusicStatus;
  track: MusicTrack | null;
  positionMs: number;
  durationMs: number;
  /** 0–100 */
  volume: number;
  index: number;
  queueLength: number;
  liked: boolean;
  /** 在放的是哪一组：每日推荐 / 歌单名 / 搜的词。 */
  source: string | null;
  error: string | null;
}
export interface MusicState {
  provider: "netease";
  account: MusicAccount | null;
  login: MusicLogin;
  player: MusicPlayer;
  quality: MusicQuality;
}
export interface MusicHome { daily: MusicTrack[]; playlists: MusicPlaylist[] }
export type MusicFrom = "daily" | "liked" | "fm" | "playlist";
export interface MusicPlayArgs {
  tracks?: MusicTrack[];
  index?: number;
  from?: MusicFrom;
  playlistId?: string;
  query?: string;
}
export type MusicAction = "pause" | "resume" | "toggle" | "next" | "previous" | "stop" | "seek" | "volume" | "like";

export const MUSIC_EVENT = "kotone://music";
export const IDLE_PLAYER: MusicPlayer = {
  status: "idle", track: null, positionMs: 0, durationMs: 0, volume: 70, index: 0, queueLength: 0, liked: false, source: null, error: null,
};
export const IDLE_MUSIC: MusicState = {
  provider: "netease", account: null, login: { stage: "idle", qrSvg: null, message: null }, player: { ...IDLE_PLAYER }, quality: "exhigh",
};

const text = (value: unknown): string | null => (typeof value === "string" && value.trim() ? value : null);
const num = (value: unknown, fallback = 0): number => (typeof value === "number" && Number.isFinite(value) ? value : fallback);
const PLAYABLE: MusicPlayable[] = ["full", "vip", "paid", "unavailable"];
const STAGES: MusicLoginStage[] = ["idle", "needed", "waiting", "scanned", "expired", "failed"];
const STATUSES: MusicStatus[] = ["idle", "loading", "playing", "paused", "error"];
const QUALITIES: MusicQuality[] = ["standard", "exhigh", "lossless"];

export function normalizeTrack(value: Partial<MusicTrack> | null | undefined): MusicTrack | null {
  if (!value || typeof value !== "object" || typeof value.id !== "string") return null;
  return {
    id: value.id,
    name: text(value.name) ?? value.id,
    artists: Array.isArray(value.artists) ? value.artists.filter((a): a is string => typeof a === "string" && !!a) : [],
    album: typeof value.album === "string" ? value.album : "",
    coverUrl: text(value.coverUrl),
    durationMs: num(value.durationMs),
    playable: PLAYABLE.includes(value.playable as MusicPlayable) ? (value.playable as MusicPlayable) : "full",
    reason: text(value.reason),
  };
}
export function normalizeTracks(value: unknown): MusicTrack[] {
  return Array.isArray(value) ? value.map((t) => normalizeTrack(t)).filter((t): t is MusicTrack => !!t) : [];
}
export function normalizePlaylist(value: Partial<MusicPlaylist> | null | undefined): MusicPlaylist | null {
  if (!value || typeof value !== "object" || typeof value.id !== "string") return null;
  return {
    id: value.id, name: text(value.name) ?? value.id, coverUrl: text(value.coverUrl), trackCount: num(value.trackCount),
    creator: typeof value.creator === "string" ? value.creator : "", liked: !!value.liked, mine: !!value.mine,
  };
}

/** 后端给的状态补齐缺省字段（旧后端、模拟数据少字段也能显示）。 */
export function normalizeMusicState(value: Partial<MusicState> | null | undefined): MusicState {
  if (!value || typeof value !== "object") return structuredClone(IDLE_MUSIC);
  const login = value.login ?? IDLE_MUSIC.login;
  const player = (value.player ?? {}) as Partial<MusicPlayer>;
  const account = value.account && typeof value.account.id === "string"
    ? { id: value.account.id, nickname: text(value.account.nickname) ?? "", avatarUrl: text(value.account.avatarUrl), vip: !!value.account.vip }
    : null;
  return {
    provider: "netease",
    account,
    login: {
      stage: STAGES.includes(login.stage) ? login.stage : "idle",
      qrSvg: text(login.qrSvg),
      message: text(login.message),
    },
    player: {
      status: STATUSES.includes(player.status as MusicStatus) ? (player.status as MusicStatus) : "idle",
      track: normalizeTrack(player.track),
      positionMs: num(player.positionMs),
      durationMs: num(player.durationMs),
      volume: Math.max(0, Math.min(100, num(player.volume, 70))),
      index: num(player.index),
      queueLength: num(player.queueLength),
      liked: !!player.liked,
      source: text(player.source),
      error: text(player.error),
    },
    quality: QUALITIES.includes(value.quality as MusicQuality) ? (value.quality as MusicQuality) : "exhigh",
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  // 参数可能是界面里的响应式对象（$state 代理）：先变成普通 JSON（Tauri 本来也要序列化）
  const plain = args ? (JSON.parse(JSON.stringify(args)) as Record<string, unknown>) : undefined;
  if (!isTauri) return musicMock.invoke(command, plain ?? {}) as Promise<T>;
  return invoke<T>(command, plain);
}

export async function musicState(): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_state"));
}
export async function musicLoginStart(): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_login_start"));
}
export async function musicLoginCancel(): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_login_cancel"));
}
export async function musicLogout(): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_logout"));
}
export async function musicSearch(query: string, offset = 0): Promise<{ tracks: MusicTrack[]; hasMore: boolean }> {
  const r = await call<{ tracks: unknown; hasMore: unknown }>("music_search", { query, offset });
  return { tracks: normalizeTracks(r?.tracks), hasMore: !!r?.hasMore };
}
export async function musicHome(): Promise<MusicHome> {
  const r = await call<{ daily: unknown; playlists: unknown }>("music_home");
  const playlists = Array.isArray(r?.playlists)
    ? r.playlists.map((p) => normalizePlaylist(p as Partial<MusicPlaylist>)).filter((p): p is MusicPlaylist => !!p) : [];
  // “我喜欢的音乐”排第一（后端已经这样排；这里再保证一次）
  playlists.sort((a, b) => Number(b.liked) - Number(a.liked));
  return { daily: normalizeTracks(r?.daily), playlists };
}
export async function musicPlaylist(id: string): Promise<{ playlist: MusicPlaylist; tracks: MusicTrack[] }> {
  const r = await call<{ playlist: unknown; tracks: unknown }>("music_playlist", { id });
  const playlist = normalizePlaylist(r?.playlist as Partial<MusicPlaylist>);
  if (!playlist) throw new Error(m.music_load_failed({ error: id }));
  return { playlist, tracks: normalizeTracks(r?.tracks) };
}
export async function musicPlay(args: MusicPlayArgs): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_play", { ...args }));
}
export async function musicControl(action: MusicAction, value?: number | boolean): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_control", value === undefined ? { action } : { action, value }));
}
export async function musicQueue(): Promise<{ tracks: MusicTrack[]; index: number }> {
  const r = await call<{ tracks: unknown; index: unknown }>("music_queue");
  return { tracks: normalizeTracks(r?.tracks), index: num(r?.index) };
}
export async function musicSetQuality(quality: MusicQuality): Promise<MusicState> {
  return normalizeMusicState(await call<MusicState>("music_set_quality", { quality }));
}

export async function subscribeMusic(callback: (state: MusicState) => void): Promise<UnlistenFn> {
  if (!isTauri) return musicMock.subscribe((state) => callback(normalizeMusicState(state)));
  return listen<MusicState>(MUSIC_EVENT, (e) => callback(normalizeMusicState(e.payload)));
}

/** 音乐状态：先读一次，之后跟着事件更新（有人订阅时才连上）。 */
export const musicStore: Readable<MusicState> = readable<MusicState>(structuredClone(IDLE_MUSIC), (set) => {
  let alive = true;
  let unlisten: UnlistenFn | undefined;
  let events = 0;
  void subscribeMusic((state) => { events += 1; set(state); })
    .then((un) => { if (alive) unlisten = un; else un(); })
    .catch(() => {});
  const seen = events;
  void musicState()
    .then((state) => { if (alive && events === seen) set(state); })
    .catch(() => {});
  return () => { alive = false; unlisten?.(); };
});

/** 要登录（聊天页显示登录卡片）：她要放歌、登录窗口开着、过期了、出错了。 */
export function loginPending(state: MusicState): boolean {
  const stage = state.login.stage;
  return !state.account && stage !== "idle";
}
/** 有歌（在放、暂停、在加载）：左侧栏显示小播放条、音乐页显示播放条。 */
export function hasTrack(state: MusicState): boolean {
  return !!state.player.track && state.player.status !== "idle";
}

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}
export function artistsLine(track: Pick<MusicTrack, "artists">): string {
  return track.artists.join(" / ");
}
/** 放不了的原因（曲目列表上的小标签）；能完整放的返回空。 */
export function playableBadge(track: MusicTrack): string {
  switch (track.playable) {
    case "vip": return m.music_badge_vip();
    case "paid": return m.music_badge_paid();
    case "unavailable": return m.music_badge_unavailable();
    default: return "";
  }
}
export function qualityLabel(quality: MusicQuality): string {
  return quality === "standard" ? m.music_quality_standard() : quality === "lossless" ? m.music_quality_lossless() : m.music_quality_exhigh();
}
