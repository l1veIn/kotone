/**
 * dev:web 的音乐后端（ADR 034）：模拟网易云账号、扫码登录、每日推荐、歌单、搜索和一个会走进度的播放器。
 * 只依赖契约类型，不导入 Tauri。演示和 e2e 用 `window.__kotoneMusicPreview(step)` 推进：
 * "scanned" / "confirmed" / "expired" / "loggedIn" / "playing" / "reset"。
 * URL 里带 `mockMusic=loggedIn|playing` 直接从那一步开始；`mockMusicLogin=manual` 时扫码不自动往下走。
 */
import type { MusicAccount, MusicPlaylist, MusicState, MusicTrack, MusicPlayArgs, MusicQuality } from "../music";

type Listener = (state: MusicState) => void;
type Row = [name: string, artists: string[], album: string, seconds: number, playable: MusicTrack["playable"]];

const POOL: Row[] = [
  ["晴天（翻唱）", ["RyaVocal"], "晴天", 270, "full"],
  ["孤勇者", ["陈奕迅"], "孤勇者", 256, "vip"],
  ["Enemy", ["Imagine Dragons", "JID"], "Arcane League of Legends", 173, "vip"],
  ["起风了", ["买辣椒也用券"], "起风了", 325, "full"],
  ["夜空中最亮的星", ["逃跑计划"], "世界", 252, "full"],
  ["平凡之路", ["朴树"], "猎户星座", 301, "full"],
  ["Legends Never Die", ["Against The Current"], "Worlds 2017", 234, "full"],
  ["Rise", ["The Glitch Mob", "Mako", "The Word Alive"], "Worlds 2018", 189, "full"],
  ["稻香", ["周杰伦"], "魔杰座", 223, "unavailable"],
  ["海阔天空", ["Beyond"], "乐与怒", 326, "paid"],
  ["Lemon", ["米津玄師"], "STRAY SHEEP", 256, "vip"],
  ["光年之外", ["G.E.M.邓紫棋"], "光年之外", 235, "full"],
  ["演员", ["薛之谦"], "绅士", 261, "full"],
  ["New Boy", ["房东的猫"], "New Boy", 228, "full"],
  ["Warriors", ["2WEI", "Edda Hayes"], "Warriors", 172, "full"],
  ["孤勇者（钢琴版）", ["纪钧瀚"], "钢琴曲集", 238, "full"],
];
const REASONS: Record<MusicTrack["playable"], string | null> = {
  full: null, vip: "要 VIP 才能放整首", paid: "付费专辑", unavailable: "网易云没有版权",
};

function track(row: Row, index: number): MusicTrack {
  const [name, artists, album, seconds, playable] = row;
  return { id: `ne:${100000 + index}`, name, artists, album, coverUrl: null, durationMs: seconds * 1000, playable, reason: REASONS[playable] };
}
const TRACKS = POOL.map(track);
const rotate = (start: number, count: number) => Array.from({ length: count }, (_, i) => TRACKS[(start + i) % TRACKS.length]);

const ACCOUNT: MusicAccount = { id: "ne:10086", nickname: "开黑的猫", avatarUrl: null, vip: false };
const PLAYLISTS: { playlist: MusicPlaylist; tracks: MusicTrack[] }[] = [
  { playlist: { id: "ne:pl-liked", name: "开黑的猫喜欢的音乐", coverUrl: null, trackCount: 12, creator: "开黑的猫", liked: true, mine: true }, tracks: rotate(3, 12) },
  { playlist: { id: "ne:pl-game", name: "开黑歌单", coverUrl: null, trackCount: 8, creator: "开黑的猫", liked: false, mine: true }, tracks: [TRACKS[6], TRACKS[7], TRACKS[14], TRACKS[2], TRACKS[0], TRACKS[11], TRACKS[13], TRACKS[4]] },
  { playlist: { id: "ne:pl-night", name: "深夜 Lo-fi", coverUrl: null, trackCount: 6, creator: "开黑的猫", liked: false, mine: true }, tracks: rotate(9, 6) },
  { playlist: { id: "ne:pl-worlds", name: "英雄联盟 全球总决赛主题曲", coverUrl: null, trackCount: 4, creator: "英雄联盟", liked: false, mine: false }, tracks: [TRACKS[6], TRACKS[7], TRACKS[14], TRACKS[2]] },
  { playlist: { id: "ne:pl-classic", name: "华语经典", coverUrl: null, trackCount: 7, creator: "网易云音乐", liked: false, mine: false }, tracks: [TRACKS[5], TRACKS[9], TRACKS[8], TRACKS[4], TRACKS[12], TRACKS[3], TRACKS[11]] },
];
const DAILY = [TRACKS[0], TRACKS[3], TRACKS[1], TRACKS[4], TRACKS[11], TRACKS[13], TRACKS[6], TRACKS[12], TRACKS[5], TRACKS[15]];

const NEED_LOGIN = "要先用网易云音乐 App 扫码登录";

function query(name: string): string | null {
  if (typeof window === "undefined") return null;
  const fromSearch = new URLSearchParams(window.location.search).get(name);
  if (fromSearch) return fromSearch;
  const hash = window.location.hash;
  const at = hash.indexOf("?");
  return at >= 0 ? new URLSearchParams(hash.slice(at + 1)).get(name) : null;
}

const listeners = new Set<Listener>();
let state: MusicState = idle();
let queue: MusicTrack[] = [];
const liked = new Set<string>(PLAYLISTS[0].tracks.map((t) => t.id));
let loginTimers: ReturnType<typeof setTimeout>[] = [];
let tick: ReturnType<typeof setInterval> | null = null;
let started = false;

function idle(): MusicState {
  return {
    provider: "netease", account: null, login: { stage: "idle", qrSvg: null, message: null }, quality: "exhigh",
    player: { status: "idle", track: null, positionMs: 0, durationMs: 0, volume: 70, index: 0, queueLength: 0, liked: false, source: null, error: null },
  };
}

function emit() {
  state.player.liked = !!state.player.track && liked.has(state.player.track.id);
  const snapshot = structuredClone(state);
  for (const listener of listeners) listener(structuredClone(snapshot));
}

function ensureStarted() {
  if (started) return;
  started = true;
  const step = query("mockMusic");
  if (step === "loggedIn" || step === "playing") state.account = { ...ACCOUNT };
  if (step === "playing") void startAt(DAILY, 0, "每日推荐");
  if (typeof window !== "undefined") {
    (window as unknown as { __kotoneMusicPreview?: (s: string) => MusicState }).__kotoneMusicPreview = preview;
  }
}

/** 假二维码（25×25，三个定位角 + 按 key 散开的点）。真的二维码由后端生成。 */
function fakeQr(key: string): string {
  const size = 25;
  let seed = 0;
  for (const ch of key) seed = (seed * 31 + ch.charCodeAt(0)) >>> 0;
  const dark = (x: number, y: number) => {
    const finder = (fx: number, fy: number) => {
      const dx = x - fx, dy = y - fy;
      if (dx < 0 || dy < 0 || dx > 6 || dy > 6) return null;
      return dx === 0 || dy === 0 || dx === 6 || dy === 6 || (dx >= 2 && dx <= 4 && dy >= 2 && dy <= 4);
    };
    const f = finder(0, 0) ?? finder(size - 7, 0) ?? finder(0, size - 7);
    if (f !== null) return f;
    if ((x === 7 || y === 7) && (x < 8 || y < 8)) return false;
    seed = (seed * 1103515245 + 12345) >>> 0;
    return (seed >>> 16) % 2 === 0;
  };
  let rects = "";
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (dark(x, y)) rects += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 ${size + 4} ${size + 4}" shape-rendering="crispEdges"><rect x="-2" y="-2" width="${size + 4}" height="${size + 4}" fill="#fff"/><g fill="#111">${rects}</g></svg>`;
}

function clearLogin() {
  for (const t of loginTimers) clearTimeout(t);
  loginTimers = [];
}

function loginStart(): MusicState {
  clearLogin();
  if (state.account) return structuredClone(state);
  state.login = { stage: "waiting", qrSvg: fakeQr(`k-${Date.now()}`), message: null };
  if (query("mockMusicLogin") !== "manual") {
    loginTimers.push(setTimeout(() => preview("scanned"), 3500));
    loginTimers.push(setTimeout(() => preview("confirmed"), 6000));
  }
  emit();
  return structuredClone(state);
}

function preview(step: string): MusicState {
  ensureStarted();
  switch (step) {
    case "scanned":
      if (state.login.stage === "waiting") state.login = { ...state.login, stage: "scanned" };
      break;
    case "confirmed":
    case "loggedIn":
      clearLogin();
      state.account = { ...ACCOUNT };
      state.login = { stage: "idle", qrSvg: null, message: null };
      break;
    case "expired":
      clearLogin();
      state.login = { ...state.login, stage: "expired", message: "二维码过期了" };
      break;
    case "playing":
      state.account = { ...ACCOUNT };
      state.login = { stage: "idle", qrSvg: null, message: null };
      void startAt(DAILY, 0, "每日推荐");
      return structuredClone(state);
    case "reset":
      clearLogin();
      stopTick();
      state = idle();
      queue = [];
      break;
  }
  emit();
  return structuredClone(state);
}

function stopTick() {
  if (tick) clearInterval(tick);
  tick = null;
}
function startTick() {
  stopTick();
  tick = setInterval(() => {
    if (state.player.status !== "playing") return;
    state.player.positionMs = Math.min(state.player.durationMs, state.player.positionMs + 1000);
    if (state.player.positionMs >= state.player.durationMs) {
      void step(1).catch(() => {});
      return;
    }
    emit();
  }, 1000);
}

/** 从 index 往后找第一首能完整放的（ADR 034 D6：不放试听片段）。 */
function firstFull(tracks: MusicTrack[], index: number): number {
  for (let i = Math.max(0, index); i < tracks.length; i++) if (tracks[i].playable === "full") return i;
  return -1;
}

async function startAt(tracks: MusicTrack[], index: number, source: string | null): Promise<MusicState> {
  const at = firstFull(tracks, index);
  if (at < 0) throw new Error(tracks[index]?.reason ? `${tracks[index].name}：${tracks[index].reason}` : "这一组里没有能完整放的歌");
  queue = tracks.slice();
  const t = queue[at];
  state.player = { ...state.player, status: "loading", track: t, positionMs: 0, durationMs: t.durationMs, index: at, queueLength: queue.length, source, error: null };
  emit();
  setTimeout(() => {
    if (state.player.track?.id !== t.id || state.player.status !== "loading") return;
    state.player.status = "playing";
    startTick();
    emit();
  }, 300);
  return structuredClone(state);
}

async function step(delta: 1 | -1): Promise<MusicState> {
  if (!queue.length) return structuredClone(state);
  let i = state.player.index + delta;
  while (i >= 0 && i < queue.length && queue[i].playable !== "full") i += delta;
  if (i < 0 || i >= queue.length) {
    stopTick();
    state.player = { ...state.player, status: "idle", positionMs: 0 };
    emit();
    return structuredClone(state);
  }
  return startAt(queue, i, state.player.source);
}

function search(text: string): MusicTrack[] {
  const q = text.trim().toLowerCase();
  if (!q) return [];
  const hits = TRACKS.filter((t) => t.name.toLowerCase().includes(q) || t.artists.some((a) => a.toLowerCase().includes(q)));
  // 原唱要 VIP / 没版权时，后面跟着几首能放的翻唱（像真的网易云搜索结果）
  const covers: MusicTrack[] = [0, 1, 2].map((i) => ({
    id: `ne:${900000 + i}-${q.length}`, name: `${text.trim()}（${["翻唱", "Live", "女声版"][i]}）`, artists: [["RyaVocal", "舟子", "GYBeat"][i]],
    album: text.trim(), coverUrl: null, durationMs: (240 + i * 17) * 1000, playable: "full", reason: null,
  }));
  return [...hits, ...covers];
}

function requireLogin() {
  if (!state.account) throw new Error(NEED_LOGIN);
}

async function play(args: MusicPlayArgs): Promise<MusicState> {
  requireLogin();
  if (args.tracks?.length) return startAt(args.tracks, args.index ?? 0, null);
  if (args.query) return startAt(search(args.query), 0, args.query);
  switch (args.from) {
    case "daily": return startAt(DAILY, args.index ?? 0, "每日推荐");
    case "liked": return startAt(PLAYLISTS[0].tracks, args.index ?? 0, PLAYLISTS[0].playlist.name);
    case "fm": return startAt(rotate(5, 8), 0, "私人 FM");
    case "playlist": {
      const found = PLAYLISTS.find((p) => p.playlist.id === args.playlistId);
      if (!found) throw new Error("没找到这个歌单");
      return startAt(found.tracks, args.index ?? 0, found.playlist.name);
    }
  }
  throw new Error("要放什么？");
}

async function control(action: string, value: unknown): Promise<MusicState> {
  const p = state.player;
  switch (action) {
    case "pause": if (p.status === "playing") { p.status = "paused"; } break;
    case "resume": if (p.status === "paused") { p.status = "playing"; startTick(); } break;
    case "toggle":
      if (p.status === "playing") p.status = "paused";
      else if (p.status === "paused") { p.status = "playing"; startTick(); }
      break;
    case "next": return step(1);
    case "previous": return step(-1);
    case "stop": stopTick(); state.player = { ...p, status: "idle", track: null, positionMs: 0, durationMs: 0, queueLength: 0, source: null }; queue = []; break;
    case "seek": if (p.track && typeof value === "number") p.positionMs = Math.max(0, Math.min(p.durationMs, Math.round(value))); break;
    case "volume": if (typeof value === "number") p.volume = Math.max(0, Math.min(100, Math.round(value))); break;
    case "like":
      requireLogin();
      if (p.track) {
        if (value === false || (value === undefined && liked.has(p.track.id))) liked.delete(p.track.id);
        else liked.add(p.track.id);
      }
      break;
    default: throw new Error(`不认识的动作：${action}`);
  }
  emit();
  return structuredClone(state);
}

export async function invoke(command: string, args: Record<string, unknown>): Promise<unknown> {
  ensureStarted();
  switch (command) {
    case "music_state": return structuredClone(state);
    case "music_login_start": return loginStart();
    case "music_login_cancel": clearLogin(); state.login = { stage: "idle", qrSvg: null, message: null }; emit(); return structuredClone(state);
    case "music_logout": clearLogin(); stopTick(); state = { ...idle(), quality: state.quality }; queue = []; emit(); return structuredClone(state);
    case "music_search": {
      const all = search(String(args.query ?? ""));
      const offset = Number(args.offset ?? 0);
      return { tracks: all.slice(offset, offset + 30), hasMore: all.length > offset + 30 };
    }
    case "music_home":
      requireLogin();
      return { daily: structuredClone(DAILY), playlists: PLAYLISTS.map((p) => structuredClone(p.playlist)) };
    case "music_playlist": {
      requireLogin();
      const found = PLAYLISTS.find((p) => p.playlist.id === args.id);
      if (!found) throw new Error("没找到这个歌单");
      return structuredClone(found);
    }
    case "music_play": return play(args as MusicPlayArgs);
    case "music_control": return control(String(args.action ?? ""), args.value);
    case "music_queue": return { tracks: structuredClone(queue), index: state.player.index };
    case "music_set_quality": {
      const q = String(args.quality ?? "") as MusicQuality;
      if (q === "standard" || q === "exhigh" || q === "lossless") state.quality = q;
      emit();
      return structuredClone(state);
    }
  }
  throw new Error(`预览里没有 ${command}`);
}

export function subscribe(listener: Listener): () => void {
  ensureStarted();
  listeners.add(listener);
  return () => listeners.delete(listener);
}
