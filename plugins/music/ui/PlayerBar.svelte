<script lang="ts">
  // 音乐页底部的播放条（照 YesPlayMusic 的 Player.vue：顶上一条细进度、左边封面 + 歌名 + 红心、中间上一首 / 播放 / 下一首、
  // 右边播放列表和音量）。拖进度松手才跳（seek），拖音量马上生效。
  import { onDestroy } from "svelte";
  import { Heart, ListMusic, LoaderCircle, Pause, Play, SkipBack, SkipForward, Volume1, Volume2, VolumeX } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import {
    artistsLine, formatDuration, musicControl, musicPlay, musicQueue, musicStore, type MusicAction, type MusicTrack,
  } from "../../../../lib/music";
  import { errText, toast } from "../../../../lib/stores/ui";
  import MusicCover from "./MusicCover.svelte";

  const player = $derived($musicStore.player);
  const track = $derived(player.track);
  const loggedIn = $derived(!!$musicStore.account);

  /** 正在拖进度条：显示拖到的位置，不被 1 秒一次的状态盖掉。 */
  let dragging = $state<number | null>(null);
  const position = $derived(dragging ?? player.positionMs);
  let volumeDraft = $state<number | null>(null);
  const volume = $derived(volumeDraft ?? player.volume);
  let volumeTimer: ReturnType<typeof setTimeout> | undefined;
  onDestroy(() => clearTimeout(volumeTimer));

  async function act(action: MusicAction, value?: number | boolean) {
    try { await musicControl(action, value); }
    catch (error) { toast(false, m.music_control_failed({ error: errText(error) })); }
  }
  function seekEnd() {
    if (dragging === null) return;
    const ms = dragging;
    dragging = null;
    void act("seek", ms);
  }
  function setVolume(value: number) {
    volumeDraft = value;
    clearTimeout(volumeTimer);
    volumeTimer = setTimeout(() => {
      void act("volume", value).finally(() => { volumeDraft = null; });
    }, 120);
  }

  let queueOpen = $state(false);
  let queue = $state<{ tracks: MusicTrack[]; index: number } | null>(null);
  async function toggleQueue() {
    queueOpen = !queueOpen;
    if (!queueOpen) return;
    try { queue = await musicQueue(); }
    catch (error) { queue = { tracks: [], index: 0 }; toast(false, m.music_load_failed({ error: errText(error) })); }
  }
  async function playFromQueue(index: number) {
    if (!queue) return;
    try { await musicPlay({ tracks: queue.tracks, index }); queue = { ...queue, index }; }
    catch (error) { toast(false, m.music_play_failed({ error: errText(error) })); }
  }
  let queueEl = $state<HTMLElement>();
  function onWindowClick(event: MouseEvent) {
    if (queueOpen && queueEl && !queueEl.contains(event.target as Node)) queueOpen = false;
  }
  const VolumeIcon = $derived(volume === 0 ? VolumeX : volume < 50 ? Volume1 : Volume2);
</script>

<svelte:window onclick={onWindowClick} onkeydown={(e) => { if (e.key === "Escape") queueOpen = false; }} />

{#if track}
  <div class="sticky bottom-0 z-20 border-t border-hairline bg-background/92 backdrop-blur-md" data-testid="music-player-bar" data-status={player.status}>
    <input type="range" min="0" max={Math.max(1, player.durationMs)} step="1000" value={position}
      class="music-range absolute -top-[7px] left-0 h-3.5 w-full cursor-pointer" aria-label={m.music_progress()}
      data-testid="music-player-progress"
      oninput={(e) => { dragging = Number((e.currentTarget as HTMLInputElement).value); }}
      onchange={seekEnd} style={`--fill: ${player.durationMs ? (position / player.durationMs) * 100 : 0}%`} />
    <div class="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 px-5 py-2.5">
      <div class="flex min-w-0 items-center gap-3">
        <MusicCover url={track.coverUrl} seed={track.name} size={96} class="size-11 rounded-md shadow-sm" />
        <div class="min-w-0">
          <p class="m-0 truncate text-[13px] font-medium" data-testid="music-player-title">{track.name}</p>
          <p class="m-0 truncate text-xs text-muted-foreground">{artistsLine(track)}{#if player.source}<span class="text-muted-foreground/70"> · {player.source}</span>{/if}</p>
        </div>
        {#if loggedIn}
          <button type="button" class="flex size-8 shrink-0 items-center justify-center rounded-full hover:bg-muted {player.liked ? 'text-rose-500' : 'text-muted-foreground hover:text-foreground'}"
            aria-label={player.liked ? m.music_unlike() : m.music_like()} title={player.liked ? m.music_unlike() : m.music_like()}
            aria-pressed={player.liked} data-testid="music-player-like" onclick={() => void act("like", !player.liked)}>
            <Heart class="size-4 {player.liked ? 'fill-current' : ''}" />
          </button>
        {/if}
      </div>

      <div class="flex items-center gap-1.5">
        <button type="button" class="flex size-9 items-center justify-center rounded-full text-foreground/80 hover:bg-muted hover:text-foreground"
          aria-label={m.music_previous()} title={m.music_previous()} data-testid="music-player-prev" onclick={() => void act("previous")}><SkipBack class="size-4 fill-current" /></button>
        <button type="button" class="flex size-11 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-[0_0_14px_color-mix(in_oklch,var(--color-primary)_45%,transparent)] transition-transform hover:scale-105"
          aria-label={player.status === "playing" ? m.music_pause() : m.music_resume()} title={player.status === "playing" ? m.music_pause() : m.music_resume()}
          data-testid="music-player-toggle" onclick={() => void act("toggle")}>
          {#if player.status === "loading"}<LoaderCircle class="size-5 animate-spin" />
          {:else if player.status === "playing"}<Pause class="size-5 fill-current" />
          {:else}<Play class="ml-0.5 size-5 fill-current" />{/if}
        </button>
        <button type="button" class="flex size-9 items-center justify-center rounded-full text-foreground/80 hover:bg-muted hover:text-foreground"
          aria-label={m.music_next()} title={m.music_next()} data-testid="music-player-next" onclick={() => void act("next")}><SkipForward class="size-4 fill-current" /></button>
      </div>

      <div class="flex min-w-0 items-center justify-end gap-2">
        <span class="text-[11px] tabular-nums text-muted-foreground max-sm:hidden" data-testid="music-player-time">{formatDuration(position)} / {formatDuration(player.durationMs)}</span>
        <div class="relative" bind:this={queueEl}>
          <button type="button" class="flex size-8 items-center justify-center rounded-full hover:bg-muted {queueOpen ? 'text-primary' : 'text-muted-foreground hover:text-foreground'}"
            aria-label={m.music_queue()} title={m.music_queue()} aria-expanded={queueOpen} data-testid="music-player-queue"
            onclick={(e) => { e.stopPropagation(); void toggleQueue(); }}><ListMusic class="size-4" /></button>
          {#if queueOpen}
            <div class="absolute right-0 bottom-11 z-30 w-80 overflow-hidden rounded-xl border border-hairline bg-popover shadow-xl" data-testid="music-queue">
              <p class="m-0 border-b border-hairline px-3 py-2 text-xs font-medium">{m.music_queue_title({ count: queue?.tracks.length ?? player.queueLength })}</p>
              <div class="max-h-80 overflow-y-auto p-1">
                {#each queue?.tracks ?? [] as item, index (item.id + ":" + index)}
                  <button type="button" class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-muted disabled:opacity-50
                    {index === player.index ? 'text-primary' : ''}" disabled={item.playable !== "full"}
                    data-testid="music-queue-item" data-current={index === player.index} onclick={() => void playFromQueue(index)}>
                    <span class="w-5 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground">{index + 1}</span>
                    <span class="min-w-0 flex-1"><span class="block truncate text-xs">{item.name}</span><span class="block truncate text-[11px] text-muted-foreground">{artistsLine(item)}</span></span>
                  </button>
                {:else}
                  <p class="m-0 px-2 py-4 text-center text-xs text-muted-foreground">{m.music_queue_empty()}</p>
                {/each}
              </div>
            </div>
          {/if}
        </div>
        <div class="flex items-center gap-1.5 max-md:hidden">
          <button type="button" class="flex size-8 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label={volume === 0 ? m.music_unmute() : m.music_mute()} title={volume === 0 ? m.music_unmute() : m.music_mute()}
            onclick={() => setVolume(volume === 0 ? 70 : 0)}><VolumeIcon class="size-4" /></button>
          <input type="range" min="0" max="100" step="1" value={volume} class="music-range h-3.5 w-24 cursor-pointer" aria-label={m.music_volume()}
            data-testid="music-player-volume" style={`--fill: ${volume}%`}
            oninput={(e) => setVolume(Number((e.currentTarget as HTMLInputElement).value))} />
        </div>
      </div>
    </div>
    {#if player.error}
      <p class="m-0 border-t border-hairline px-5 py-1.5 text-xs text-destructive" role="alert" data-testid="music-player-error">{player.error}</p>
    {/if}
  </div>
{/if}

<style>
  /* 细进度条 / 音量条：主色填充、悬停时显出圆点（WebView2 = Chromium） */
  .music-range {
    appearance: none;
    background: transparent;
  }
  .music-range::-webkit-slider-runnable-track {
    height: 3px;
    border-radius: 999px;
    background: linear-gradient(to right, var(--color-primary) var(--fill), color-mix(in oklch, var(--color-muted-foreground) 30%, transparent) var(--fill));
  }
  .music-range::-webkit-slider-thumb {
    appearance: none;
    width: 11px;
    height: 11px;
    margin-top: -4px;
    border-radius: 999px;
    background: var(--color-primary);
    opacity: 0;
    transition: opacity 120ms;
  }
  .music-range:hover::-webkit-slider-thumb,
  .music-range:focus-visible::-webkit-slider-thumb {
    opacity: 1;
  }
</style>
