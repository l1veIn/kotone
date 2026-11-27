<script lang="ts">
  // 「游戏列表 → 本机的游戏」：照 Playnite 等启动器的做法只读注册表和启动器清单（通常 0.1 秒内），
  // 列出装了的游戏和它们的图标（主程序的图标）。第一次打开这一页自动检测一轮，之后点「重新检测」。
  // 每个游戏可以直接打开，或跳到对应的游戏配置（适配）。
  import { onMount, tick } from "svelte";
  import { ChevronRight, Folder, LoaderCircle, Play, RefreshCw } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { installedProfile } from "$lib/installedProfile";
  import { settingsStore } from "$lib/stores/ui";
  import { gameIcon, installedGames, launchInstalledGame, type GameProfile, type InstalledGame } from "../../../lib/ipc";
  import { errText, toast } from "../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { profiles, profileIcons, selected = $bindable(null) }: {
    profiles: GameProfile[];
    profileIcons: Record<string, string>;
    selected?: InstalledGame | null;
  } = $props();

  async function showDetail(game: InstalledGame) {
    selected = game;
    await tick();
    document.getElementById("installed-game-summary")?.focus();
  }

  const SOURCE_LABEL: Record<string, string> = {
    steam: "Steam", tencent: m.game_source_tencent(), riot: "Riot", hoyoplay: m.game_source_hoyoplay(), epic: "Epic", wegame: "WeGame",
    uninstall: m.game_installed_source_uninstall(),
  };
  const CACHE_KEY = "kotone.installedGames";

  // 先展示缓存；进入游戏列表时在后台校验本机安装情况。
  let games = $state<InstalledGame[] | null>(readCache());
  let icons = $state<Record<string, string>>({});
  let detecting = $state(false);
  let elapsedMs = $state<number | null>(null);
  let launching = $state<string | null>(null);

  function readCache(): InstalledGame[] | null {
    try { return JSON.parse(localStorage.getItem(CACHE_KEY) ?? "null"); } catch { return null; }
  }

  async function detect() {
    if (detecting) return;
    detecting = true;
    const started = performance.now();
    try {
      const found = await installedGames();
      elapsedMs = Math.round(performance.now() - started);
      games = found;
      try { localStorage.setItem(CACHE_KEY, JSON.stringify(found)); } catch { /* 只是缓存 */ }
      void loadIcons(found);
    } catch (error) {
      toast(false, m.game_detect_failed({ error: errText(error) }));
    } finally {
      detecting = false;
    }
  }

  async function loadIcons(list: InstalledGame[]) {
    for (const game of list) {
      if (icons[game.id]) continue;
      try {
        const bytes = await gameIcon(game.id);
        if (bytes?.length) {
          const binary = Array.from(bytes, byte => String.fromCharCode(byte)).join("");
          icons = { ...icons, [game.id]: `data:image/png;base64,${btoa(binary)}` };
        }
      } catch { /* 没有图标就用占位 */ }
    }
  }

  async function launch(game: InstalledGame) {
    launching = game.id;
    try {
      await launchInstalledGame(game.id);
      toast(true, m.game_installed_launching({ name: game.name }));
    } catch (error) {
      toast(false, m.game_installed_launch_failed({ error: errText(error) }));
    } finally {
      launching = null;
    }
  }

  onMount(() => {
    void detect();

  });
</script>

{#snippet cover(game: InstalledGame, large = false)}
  {@const profile = installedProfile(game, profiles)}
  {@const image = icons[game.id] || (profile && profileIcons[profile.id])}
  <span class="cover {large ? 'large' : ''}" aria-hidden="true">
    {#if image}<img src={image} alt="" class="size-full object-contain" data-testid="installed-game-icon" />
    {:else}{Array.from(game.name.trim())[0]}{/if}
  </span>
{/snippet}

{#snippet play(game: InstalledGame, full = false)}
  {#if game.launch}
    <Button size="sm" variant={full ? "default" : "ghost"} aria-label={m.game_installed_launch_aria({ name: game.name })} title={m.game_installed_launch_title()} disabled={launching !== null}
      onclick={() => void launch(game)} data-testid="installed-game-launch">
      {#if launching === game.id}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Play class="size-3.5" />{/if}
      {#if full}{m.game_installed_launch_title()}{/if}
    </Button>
  {/if}
{/snippet}

{#if selected}
  <section tabindex="-1" id="installed-game-summary" aria-label={selected.name} class="flex flex-wrap items-center gap-4 rounded-xl border border-hairline bg-card/70 px-5 py-5" data-testid="installed-game-summary">
    {@render cover(selected, true)}
    <div class="min-w-32 flex-1">
      <p class="text-sm font-semibold">{SOURCE_LABEL[selected.source] ?? selected.source}</p>
      {#if selected.installDir}<p class="mt-1 flex items-start gap-1.5 text-xs text-muted-foreground"><Folder class="mt-px size-3.5 shrink-0" /><span class="break-all">{selected.installDir}</span></p>{/if}
    </div>
    {@render play(selected, true)}
  </section>
{:else}
  <section data-testid="installed-games">
    <div class="flex flex-wrap items-center justify-between gap-2 border-b border-hairline pb-3">
      <h2 class="text-sm font-semibold">{m.game_installed_title()}{#if games}<span class="ml-2 text-xs font-normal tabular-nums text-muted-foreground">{games.length}</span>{/if}</h2>
      <Button size="sm" variant="ghost" disabled={detecting} onclick={() => void detect()} title={elapsedMs === null ? m.game_installed_description() : m.game_installed_elapsed({ms: elapsedMs})} data-testid="installed-games-detect">
        {#if detecting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<RefreshCw class="size-3.5" />{/if}{m.game_installed_redetect()}
      </Button>
    </div>
    {#if games === null}
      <p class="py-5 text-sm text-muted-foreground">{detecting ? m.game_installed_detecting() : m.game_catalog_unavailable()}</p>
    {:else if !games.length}
      <p class="py-5 text-sm text-muted-foreground" data-testid="installed-games-empty">{m.game_installed_empty()}</p>
    {:else}
      <ul class="m-0 list-none divide-y divide-hairline border-b border-hairline p-0">
        {#each games as game (game.id)}
          {@const profile = installedProfile(game, profiles)}
          <li class="flex items-center gap-2 py-1" data-testid="installed-game" data-game-id={game.id}>
            <button type="button" class="game-link" id={`game-link-${game.id}`} onclick={() => void showDetail(game)} data-testid="installed-game-detail">
              {@render cover(game)}
              <span class="min-w-0 flex-1"><strong class="block truncate text-[15px] font-semibold">{game.name}</strong>
                <span class="mt-1 block text-xs text-muted-foreground">{SOURCE_LABEL[game.source] ?? game.source}</span>
              </span>
              {#if profile}<span class="text-xs {profile.id === $settingsStore?.activeProfileId ? 'text-primary' : 'text-muted-foreground'}">{profile.id === $settingsStore?.activeProfileId ? m.game_catalog_current() : m.game_catalog_matched()}</span>
              {:else}<span class="text-xs text-muted-foreground">{m.game_catalog_default()}</span>{/if}
              <ChevronRight class="size-4 shrink-0 text-muted-foreground" />
            </button>
            {@render play(game)}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .game-link { display: flex; align-items: center; gap: .85rem; flex: 1; min-width: 0; padding: .7rem .4rem; text-align: left; border-radius: .5rem; }
  .game-link:hover { background: var(--muted); }
  .cover { width: 3rem; height: 3rem; display: grid; place-items: center; flex: none; overflow: hidden; border-radius: .55rem; background: var(--muted); color: var(--muted-foreground); font-size: 1.15rem; font-weight: 600; }
  .cover.large { width: 4rem; height: 4rem; font-size: 1.6rem; }
</style>
