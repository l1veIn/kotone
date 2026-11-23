<script lang="ts">
  // 左侧栏底部的小播放条（ADR 034）：在放的时候哪一页都看得见——歌名、播放 / 暂停、下一首；点歌名去音乐页。
  // 图标栏形态只剩一个按钮（点了去音乐页，悬停看歌名）。
  import { AudioLines, LoaderCircle, Music, Pause, Play, SkipForward } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import { artistsLine, hasTrack, musicControl, musicStore, type MusicAction } from "../../lib/music";
  import { errText, toast } from "../../lib/stores/ui";
  import MusicCover from "./pages/music/MusicCover.svelte";

  let { rail = false, onOpen }: { rail?: boolean; onOpen: () => void } = $props();

  const player = $derived($musicStore.player);
  const visible = $derived(hasTrack($musicStore));

  async function act(action: MusicAction) {
    try { await musicControl(action); } catch (error) { toast(false, m.music_control_failed({ error: errText(error) })); }
  }
</script>

{#if visible && player.track}
  {#if rail}
    <button type="button" class="relative mb-1 flex h-9 w-full items-center justify-center rounded-lg text-primary hover:bg-sidebar-accent/60"
      title={m.music_now_playing({ name: player.track.name })} aria-label={m.music_now_playing({ name: player.track.name })}
      data-testid="sidebar-mini-player" onclick={onOpen}>
      {#if player.status === "playing"}<AudioLines class="size-4" />{:else}<Music class="size-4" />{/if}
    </button>
  {:else}
    <div class="mb-1 flex items-center gap-2 rounded-xl border border-hairline bg-sidebar-accent/40 p-1.5" data-testid="sidebar-mini-player" data-status={player.status}>
      <button type="button" class="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left" onclick={onOpen}
        title={m.music_open_page()} data-testid="sidebar-mini-open">
        <MusicCover url={player.track.coverUrl} seed={player.track.name} size={64} class="size-8 rounded-md" />
        <span class="min-w-0">
          <span class="block truncate text-xs font-medium text-foreground" data-testid="sidebar-mini-title">{player.track.name}</span>
          <span class="block truncate text-[11px] text-sidebar-foreground/60">{artistsLine(player.track)}</span>
        </span>
      </button>
      <button type="button" class="flex size-7 shrink-0 items-center justify-center rounded-full text-foreground hover:bg-sidebar-accent"
        aria-label={player.status === "playing" ? m.music_pause() : m.music_resume()} title={player.status === "playing" ? m.music_pause() : m.music_resume()}
        data-testid="sidebar-mini-toggle" onclick={() => void act("toggle")}>
        {#if player.status === "loading"}<LoaderCircle class="size-3.5 animate-spin" />
        {:else if player.status === "playing"}<Pause class="size-3.5 fill-current" />
        {:else}<Play class="size-3.5 fill-current" />{/if}
      </button>
      <button type="button" class="flex size-7 shrink-0 items-center justify-center rounded-full text-foreground/80 hover:bg-sidebar-accent hover:text-foreground"
        aria-label={m.music_next()} title={m.music_next()} data-testid="sidebar-mini-next" onclick={() => void act("next")}>
        <SkipForward class="size-3.5 fill-current" />
      </button>
    </div>
  {/if}
{/if}
