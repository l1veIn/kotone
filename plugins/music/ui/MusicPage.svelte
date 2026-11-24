<script lang="ts">
  // 音乐页（ADR 034 第五节）：信息结构照 YesPlayMusic（MIT）——顶部搜歌和账号；发现里每日推荐、私人 FM、我的歌单
  // （“我喜欢的音乐”排第一）；搜索结果和歌单是曲目列表；底部播放条。没登录时先扫码（D5）。
  // 歌在 Kotone 自己的播放器里放（kotone-music），换页面、关窗口都不停。
  import { untrack } from "svelte";
  import {
    ArrowLeft, CalendarDays, Check, ChevronDown, LoaderCircle, LogOut, Play, Radio, Search, SkipForward, X,
  } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import { m } from "$lib/paraglide/messages.js";
  import {
    loginPending, musicControl, musicHome, musicLoginStart, musicLogout, musicPlay, musicPlaylist, musicSearch,
    musicSetQuality, musicStore, qualityLabel,
    type MusicHome, type MusicPlayArgs, type MusicPlaylist, type MusicQuality, type MusicTrack,
  } from "../../../lib/music";
  import { errText, toast } from "../../../lib/stores/ui";
  import MusicCover from "./music/MusicCover.svelte";
  import MusicLoginCard from "./music/MusicLoginCard.svelte";
  import PlayerBar from "./music/PlayerBar.svelte";
  import TrackList from "./music/TrackList.svelte";

  type View =
    | { kind: "home" }
    | { kind: "search"; query: string }
    | { kind: "list"; from: "daily" | "playlist"; playlist: MusicPlaylist | null };

  const music = $derived($musicStore);
  const accountId = $derived(music.account?.id ?? null);
  const pending = $derived(loginPending(music));
  // 后端在放私人 FM 时 source 是“私人 FM”（不随界面语言变）：按里面的 FM 认
  const fmPlaying = $derived(/FM/i.test(music.player.source ?? "") && !!music.player.track);

  let view = $state<View>({ kind: "home" });
  let home = $state<MusicHome | null>(null);
  let homeError = $state("");
  let homeLoading = $state(false);
  let list = $state<MusicTrack[]>([]);
  let listLoading = $state(false);
  let listError = $state("");
  let searchText = $state("");
  let results = $state<MusicTrack[]>([]);
  let hasMore = $state(false);
  let searching = $state(false);
  let searchError = $state("");
  let searchSeq = 0;

  async function loadHome() {
    homeLoading = true;
    homeError = "";
    try { home = await musicHome(); }
    catch (error) { home = null; homeError = m.music_load_failed({ error: errText(error) }); }
    finally { homeLoading = false; }
  }
  // 登录 / 换账号 / 退出：重新读“发现”
  $effect(() => {
    const id = accountId;
    untrack(() => {
      if (id) void loadHome();
      else { home = null; if (view.kind === "list") view = { kind: "home" }; }
    });
  });

  async function runSearch(more = false) {
    const query = searchText.trim();
    if (!query) { view = { kind: "home" }; return; }
    const seq = ++searchSeq;
    view = { kind: "search", query };
    searching = true;
    searchError = "";
    try {
      const r = await musicSearch(query, more ? results.length : 0);
      if (seq !== searchSeq) return;
      results = more ? [...results, ...r.tracks] : r.tracks;
      hasMore = r.hasMore;
    } catch (error) {
      if (seq === searchSeq) searchError = m.music_search_failed({ error: errText(error) });
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }
  function clearSearch() {
    searchText = "";
    searchSeq++;
    results = [];
    view = { kind: "home" };
  }

  async function openList(from: "daily" | "playlist", playlist: MusicPlaylist | null) {
    view = { kind: "list", from, playlist };
    listError = "";
    if (from === "daily") { list = home?.daily ?? []; return; }
    if (!playlist) return;
    list = [];
    listLoading = true;
    try { list = (await musicPlaylist(playlist.id)).tracks; }
    catch (error) { listError = m.music_load_failed({ error: errText(error) }); }
    finally { listLoading = false; }
  }

  /** 放歌；没登录就先开始扫码（ADR 034 D5），卡片在上面。 */
  async function play(args: MusicPlayArgs) {
    try { await musicPlay(args); }
    catch (error) {
      if (!$musicStore.account) {
        try { await musicLoginStart(); } catch { /* 卡片上能重试 */ }
        toast(false, m.music_need_login());
      } else {
        toast(false, m.music_play_failed({ error: errText(error) }));
      }
    }
  }
  function playList(index: number) {
    if (view.kind === "search") void play({ tracks: results, index });
    else if (view.kind === "list" && view.from === "daily") void play({ from: "daily", index });
    else if (view.kind === "list" && view.playlist) void play({ from: "playlist", playlistId: view.playlist.id, index });
  }

  async function setQuality(quality: MusicQuality) {
    try { await musicSetQuality(quality); toast(true, m.music_quality_saved({ quality: qualityLabel(quality) })); }
    catch (error) { toast(false, m.music_control_failed({ error: errText(error) })); }
  }
  async function logout() {
    try { await musicLogout(); toast(true, m.music_logged_out()); }
    catch (error) { toast(false, m.music_control_failed({ error: errText(error) })); }
  }
  async function startLogin() {
    try { await musicLoginStart(); } catch (error) { toast(false, m.music_login_failed({ error: errText(error) })); }
  }

  const today = new Date().getDate();
  const QUALITIES: MusicQuality[] = ["standard", "exhigh", "lossless"];
</script>

<div class="ui-scope flex min-h-full flex-col text-foreground" data-testid="music-page">
  <header class="sticky top-0 z-20 flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-hairline bg-background/88 px-6 py-3 backdrop-blur-md">
    {#if view.kind !== "home"}
      <button type="button" class="flex size-8 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label={m.music_back()} title={m.music_back()} data-testid="music-back" onclick={() => (view.kind === "search" ? clearSearch() : (view = { kind: "home" }))}>
        <ArrowLeft class="size-4" />
      </button>
    {/if}
    <h1 class="m-0 text-lg font-semibold tracking-tight">{m.music_title()}</h1>
    <form class="flex h-9 min-w-48 flex-1 items-center gap-2 rounded-full bg-muted/70 px-3.5 focus-within:ring-1 focus-within:ring-primary/50 sm:max-w-md"
      role="search" onsubmit={(e) => { e.preventDefault(); void runSearch(); }}>
      <Search class="size-4 shrink-0 text-muted-foreground" />
      <input class="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-muted-foreground" type="search"
        placeholder={m.music_search_placeholder()} aria-label={m.music_search_placeholder()} bind:value={searchText} data-testid="music-search-input" />
      {#if searchText}
        <button type="button" class="flex size-5 items-center justify-center rounded-full text-muted-foreground hover:text-foreground"
          aria-label={m.music_search_clear()} onclick={clearSearch}><X class="size-3.5" /></button>
      {/if}
    </form>
    <div class="ml-auto flex items-center gap-2">
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <button {...props} type="button" class="flex h-8 items-center gap-1 rounded-full px-3 text-xs text-muted-foreground hover:bg-muted hover:text-foreground" data-testid="music-quality">
              {m.music_quality_label({ quality: qualityLabel(music.quality) })}<ChevronDown class="size-3" />
            </button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="end" class="w-48">
          {#each QUALITIES as quality (quality)}
            <DropdownMenu.Item data-testid={`music-quality-${quality}`} onSelect={() => void setQuality(quality)}>
              <Check class="size-3.5 {music.quality === quality ? '' : 'invisible'}" />
              <span class="flex-1">{qualityLabel(quality)}</span>
              {#if quality === "lossless"}<span class="text-[10px] text-muted-foreground">{m.music_quality_vip_hint()}</span>{/if}
            </DropdownMenu.Item>
          {/each}
        </DropdownMenu.Content>
      </DropdownMenu.Root>
      {#if music.account}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <button {...props} type="button" class="flex h-8 items-center gap-2 rounded-full pr-3 pl-1 text-xs hover:bg-muted" data-testid="music-account">
                <MusicCover url={music.account?.avatarUrl ?? null} seed={music.account?.nickname ?? ""} size={64} class="size-6 rounded-full" />
                <span class="max-w-32 truncate">{music.account?.nickname}</span>
                {#if music.account?.vip}<span class="rounded bg-rose-500/15 px-1 text-[10px] font-medium text-rose-500">VIP</span>{/if}
              </button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end" class="w-44">
            <DropdownMenu.Item data-testid="music-logout" onSelect={() => void logout()}><LogOut class="size-3.5" />{m.music_logout()}</DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {:else if !pending && view.kind !== "home"}
        <!-- 首页没登录时卡片上已经有“扫码登录”：顶栏不再重复 -->
        <Button size="sm" variant="outline" class="h-8 rounded-full" data-testid="music-login-button" onclick={() => void startLogin()}>{m.music_login_button()}</Button>
      {/if}
    </div>
  </header>

  <div class="mx-auto w-full max-w-5xl flex-1 px-6 pt-6 pb-10">
    {#if pending && view.kind !== "home"}
      <div class="mb-5"><MusicLoginCard compact /></div>
    {/if}

    {#if view.kind === "home"}
      {#if !music.account}
        <MusicLoginCard />
        <p class="mt-3 mb-0 text-xs leading-relaxed text-muted-foreground">{m.music_logged_out_hint()}</p>
      {:else}
        <section aria-labelledby="music-for-you">
          <h2 id="music-for-you" class="mt-0 mb-3 text-base font-semibold">{m.music_for_you()}</h2>
          <div class="grid gap-4 md:grid-cols-2">
            <!-- 每日推荐：点卡片看列表，点播放键从第一首放 -->
            <div class="group relative h-44 overflow-hidden rounded-2xl text-left text-white shadow-sm" data-testid="music-daily-card"
              style="background: linear-gradient(135deg, oklch(0.55 0.19 18), oklch(0.42 0.17 330) 60%, oklch(0.32 0.12 290));">
              <button type="button" class="absolute inset-0 flex flex-col justify-between p-5 text-left" data-testid="music-daily-open"
                onclick={() => void openList("daily", null)} aria-label={m.music_daily_open()}>
                <span class="flex items-center gap-2 text-sm font-medium text-white/85"><CalendarDays class="size-4" />{m.music_daily_title()}</span>
                <span><span class="block text-6xl leading-none font-bold tracking-tight">{today}</span>
                  <span class="mt-2 block text-xs text-white/80">{m.music_daily_desc({ count: home?.daily.length ?? 0 })}</span></span>
              </button>
              <button type="button" class="absolute right-5 bottom-5 flex size-11 items-center justify-center rounded-full bg-white/90 text-neutral-900 shadow-md transition-transform hover:scale-105"
                aria-label={m.music_daily_play()} title={m.music_daily_play()} data-testid="music-daily-play" onclick={() => void play({ from: "daily" })}>
                <Play class="ml-0.5 size-5 fill-current" />
              </button>
            </div>
            <!-- 私人 FM -->
            <div class="flex h-44 items-center gap-4 rounded-2xl border border-hairline bg-card/70 p-5" data-testid="music-fm-card">
              {#if fmPlaying && music.player.track}
                <MusicCover url={music.player.track.coverUrl} seed={music.player.track.name} size={240} class="size-32 rounded-xl shadow" />
              {:else}
                <div class="flex size-32 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-primary/40 to-primary/10"><Radio class="size-10 text-primary" /></div>
              {/if}
              <div class="flex min-w-0 flex-1 flex-col justify-between self-stretch">
                <div class="min-w-0">
                  <p class="m-0 flex items-center gap-1.5 text-xs text-muted-foreground"><Radio class="size-3.5" />{m.music_fm_title()}</p>
                  {#if fmPlaying && music.player.track}
                    <p class="mt-1.5 mb-0 truncate text-base font-semibold">{music.player.track.name}</p>
                    <p class="m-0 truncate text-xs text-muted-foreground">{music.player.track.artists.join(" / ")}</p>
                  {:else}
                    <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">{m.music_fm_desc()}</p>
                  {/if}
                </div>
                <div class="flex items-center gap-2">
                  <Button size="sm" class="h-8 rounded-full" data-testid="music-fm-play"
                    onclick={() => (fmPlaying ? void musicControl("toggle") : void play({ from: "fm" }))}>
                    <Play class="size-3.5 fill-current" />{fmPlaying ? (music.player.status === "playing" ? m.music_pause() : m.music_resume()) : m.music_fm_play()}
                  </Button>
                  {#if fmPlaying}
                    <Button size="icon" variant="ghost" class="size-8 rounded-full" aria-label={m.music_next()} title={m.music_next()} data-testid="music-fm-next"
                      onclick={() => void musicControl("next")}><SkipForward class="size-4 fill-current" /></Button>
                  {/if}
                </div>
              </div>
            </div>
          </div>
        </section>

        <section class="mt-8" aria-labelledby="music-playlists">
          <h2 id="music-playlists" class="mt-0 mb-3 text-base font-semibold">{m.music_playlists_title()}</h2>
          {#if homeLoading && !home}
            <p class="m-0 flex items-center gap-2 text-xs text-muted-foreground"><LoaderCircle class="size-3.5 animate-spin" />{m.music_loading()}</p>
          {:else if homeError}
            <p class="m-0 text-xs text-destructive" role="alert" data-testid="music-home-error">{homeError}
              <button type="button" class="ml-2 text-primary underline-offset-2 hover:underline" onclick={() => void loadHome()}>{m.music_retry()}</button></p>
          {:else}
            <div class="grid grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] gap-x-4 gap-y-5" data-testid="music-playlists">
              {#each home?.playlists ?? [] as playlist (playlist.id)}
                <div class="group min-w-0" data-testid="music-playlist-card" data-playlist-id={playlist.id} data-liked={playlist.liked}>
                  <div class="relative">
                    <button type="button" class="block w-full overflow-hidden rounded-xl" onclick={() => void openList("playlist", playlist)}
                      aria-label={m.music_playlist_open({ name: playlist.liked ? m.music_liked_title() : playlist.name })}>
                      <MusicCover url={playlist.coverUrl} seed={playlist.id} liked={playlist.liked} size={300}
                        class="aspect-square w-full rounded-xl transition-transform duration-200 group-hover:scale-[1.03]" />
                    </button>
                    <button type="button" class="absolute right-2 bottom-2 flex size-9 items-center justify-center rounded-full bg-white/90 text-neutral-900 opacity-0 shadow-md transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                      aria-label={m.music_playlist_play({ name: playlist.name })} data-testid="music-playlist-play"
                      onclick={() => void play({ from: "playlist", playlistId: playlist.id })}><Play class="ml-0.5 size-4 fill-current" /></button>
                  </div>
                  <p class="mt-2 mb-0 line-clamp-2 text-[13px] leading-snug font-medium">{playlist.liked ? m.music_liked_title() : playlist.name}</p>
                  <p class="m-0 truncate text-[11px] text-muted-foreground">{m.music_playlist_meta({ count: playlist.trackCount, creator: playlist.creator })}</p>
                </div>
              {:else}
                <p class="m-0 text-xs text-muted-foreground">{m.music_playlists_empty()}</p>
              {/each}
            </div>
          {/if}
        </section>
      {/if}
    {:else if view.kind === "search"}
      <div class="mb-4 flex items-baseline gap-2">
        <h2 class="m-0 text-base font-semibold" data-testid="music-search-title">{m.music_search_results({ query: view.query })}</h2>
        {#if searching}<LoaderCircle class="size-3.5 animate-spin text-muted-foreground" />{/if}
      </div>
      {#if searchError}
        <p class="m-0 text-xs text-destructive" role="alert">{searchError}</p>
      {:else if !searching || results.length}
        <TrackList tracks={results} onPlay={playList} testid="music-search-results" />
        {#if hasMore}
          <div class="mt-4 flex justify-center"><Button size="sm" variant="outline" disabled={searching} onclick={() => void runSearch(true)}>{m.music_load_more()}</Button></div>
        {/if}
      {/if}
    {:else}
      {@const title = view.from === "daily" ? m.music_daily_title() : view.playlist?.liked ? m.music_liked_title() : (view.playlist?.name ?? "")}
      <div class="mb-6 flex items-end gap-5">
        {#if view.from === "daily"}
          <div class="flex size-36 shrink-0 flex-col items-center justify-center rounded-2xl text-white shadow"
            style="background: linear-gradient(135deg, oklch(0.55 0.19 18), oklch(0.42 0.17 330));">
            <CalendarDays class="size-6 text-white/80" /><span class="mt-1 text-4xl font-bold">{today}</span>
          </div>
        {:else}
          <MusicCover url={view.playlist?.coverUrl ?? null} seed={view.playlist?.id ?? ""} liked={!!view.playlist?.liked} size={300} class="size-36 rounded-2xl shadow" />
        {/if}
        <div class="min-w-0">
          <p class="m-0 text-xs text-muted-foreground">{view.from === "daily" ? m.music_for_you() : m.music_playlist_kind()}</p>
          <h2 class="mt-1 mb-1 truncate text-2xl font-bold tracking-tight" data-testid="music-list-title">{title}</h2>
          <p class="m-0 text-xs text-muted-foreground">
            {view.from === "daily" ? m.music_daily_desc({ count: list.length }) : m.music_playlist_meta({ count: view.playlist?.trackCount ?? list.length, creator: view.playlist?.creator ?? "" })}
          </p>
          <Button size="sm" class="mt-3 h-8 rounded-full" data-testid="music-list-play" disabled={!list.length}
            onclick={() => (view.kind === "list" && view.from === "daily" ? void play({ from: "daily" }) : view.kind === "list" && view.playlist ? void play({ from: "playlist", playlistId: view.playlist.id }) : undefined)}>
            <Play class="size-3.5 fill-current" />{m.music_play_all()}
          </Button>
        </div>
      </div>
      {#if listLoading}
        <p class="m-0 flex items-center gap-2 text-xs text-muted-foreground"><LoaderCircle class="size-3.5 animate-spin" />{m.music_loading()}</p>
      {:else if listError}
        <p class="m-0 text-xs text-destructive" role="alert">{listError}</p>
      {:else}
        <TrackList tracks={list} onPlay={playList} testid="music-list-tracks" />
      {/if}
    {/if}
  </div>

  <PlayerBar />
</div>
