<script lang="ts">
  // 曲目列表（搜索结果、歌单、每日推荐）：照 YesPlayMusic 的 TrackListItem——序号 / 正在放、封面、歌名 + 歌手、专辑、时长。
  // 要 VIP、付费、没版权的灰掉并标出来（ADR 034 D6：她不放试听片段）；双击或点序号上的播放键从这首开始放这一列。
  import { AudioLines, Play } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { artistsLine, formatDuration, musicStore, playableBadge, type MusicTrack } from "../../../../lib/music";
  import MusicCover from "./MusicCover.svelte";

  let { tracks, onPlay, testid = "music-tracks" }: {
    tracks: MusicTrack[];
    onPlay: (index: number) => void;
    testid?: string;
  } = $props();

  const current = $derived($musicStore.player.track?.id ?? null);
  const playing = $derived($musicStore.player.status === "playing");
</script>

<div class="flex flex-col" role="list" data-testid={testid}>
  <div class="grid grid-cols-[2.5rem_minmax(0,1fr)_minmax(0,0.7fr)_3.5rem] items-center gap-3 border-b border-hairline px-2 pb-2 text-[11px] text-muted-foreground max-md:grid-cols-[2.5rem_minmax(0,1fr)_3.5rem]" aria-hidden="true">
    <span class="text-center">#</span><span>{m.music_col_title()}</span><span class="max-md:hidden">{m.music_col_album()}</span><span class="text-right">{m.music_col_duration()}</span>
  </div>
  {#each tracks as track, index (track.id + ":" + index)}
    {@const isCurrent = track.id === current}
    {@const badge = playableBadge(track)}
    <div role="listitem" data-testid="music-track" data-track-id={track.id} data-playable={track.playable} data-current={isCurrent}
      class="group grid grid-cols-[2.5rem_minmax(0,1fr)_minmax(0,0.7fr)_3.5rem] items-center gap-3 rounded-lg px-2 py-1.5 transition-colors max-md:grid-cols-[2.5rem_minmax(0,1fr)_3.5rem]
        {track.playable === 'full' ? 'hover:bg-muted/60' : 'opacity-55'} {isCurrent ? 'bg-primary/8' : ''}"
      ondblclick={() => track.playable === "full" && onPlay(index)}
      title={track.reason ?? undefined}>
      <div class="relative flex h-8 items-center justify-center text-xs tabular-nums text-muted-foreground">
        {#if isCurrent && playing}
          <AudioLines class="size-4 text-primary group-hover:hidden" />
        {:else}
          <span class="group-hover:hidden {isCurrent ? 'text-primary' : ''}">{index + 1}</span>
        {/if}
        {#if track.playable === "full"}
          <button type="button" class="absolute inset-0 hidden items-center justify-center rounded-md text-foreground hover:text-primary group-hover:flex focus-visible:flex"
            aria-label={m.music_play_track({ name: track.name })} data-testid="music-track-play" onclick={() => onPlay(index)}>
            <Play class="size-4 fill-current" />
          </button>
        {/if}
      </div>
      <div class="flex min-w-0 items-center gap-2.5">
        <MusicCover url={track.coverUrl} seed={track.name} size={80} class="size-9 rounded-md" />
        <div class="min-w-0">
          <p class="m-0 flex min-w-0 items-center gap-1.5 text-[13px] {isCurrent ? 'font-medium text-primary' : 'text-foreground'}">
            <span class="truncate">{track.name}</span>
            {#if badge}<span class="shrink-0 rounded border border-warning/50 px-1 text-[10px] leading-4 text-warning" data-testid="music-track-badge">{badge}</span>{/if}
          </p>
          <p class="m-0 truncate text-xs text-muted-foreground">{artistsLine(track)}</p>
        </div>
      </div>
      <p class="m-0 truncate text-xs text-muted-foreground max-md:hidden">{track.album}</p>
      <p class="m-0 text-right text-xs tabular-nums text-muted-foreground">{formatDuration(track.durationMs)}</p>
    </div>
  {:else}
    <p class="m-0 px-2 py-6 text-center text-xs text-muted-foreground" data-testid="music-tracks-empty">{m.music_tracks_empty()}</p>
  {/each}
</div>
