<script lang="ts">
  // 「游戏列表 → 本机的游戏」：照 Playnite 等启动器的做法只读注册表和启动器清单（通常 0.1 秒内），
  // 列出装了的游戏和它们的图标（主程序的图标）。第一次打开这一页自动检测一轮，之后点「重新检测」。
  // 每个游戏可以直接打开，或跳到对应的游戏配置（适配）。
  import { onMount } from "svelte";
  import { Gamepad2, LoaderCircle, Play, RefreshCw } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { gameIcon, installedGames, launchInstalledGame, type GameProfile, type InstalledGame } from "../../../lib/ipc";
  import { errText, toast } from "../../../lib/stores/ui";

  let { profiles, onOpenProfile }: {
    profiles: GameProfile[];
    /** 打开这个游戏的配置（适配）。 */
    onOpenProfile: (profile: GameProfile) => void;
  } = $props();

  const SOURCE_LABEL: Record<string, string> = {
    steam: "Steam", tencent: "腾讯", riot: "Riot", hoyoplay: "米哈游", epic: "Epic", wegame: "WeGame", uninstall: "已安装程序",
  };
  const CACHE_KEY = "kotone.installedGames";

  // 设置窗口开着期间只自动检测一次（切走再回来不重复）。
  let games = $state<InstalledGame[] | null>(readCache());
  let icons = $state<Record<string, string>>({});
  let detecting = $state(false);
  let elapsedMs = $state<number | null>(null);
  let launching = $state<string | null>(null);

  function readCache(): InstalledGame[] | null {
    try { return JSON.parse(localStorage.getItem(CACHE_KEY) ?? "null"); } catch { return null; }
  }

  function profileFor(game: InstalledGame): GameProfile | undefined {
    const procs = game.processNames.map((p) => p.toLowerCase());
    const norm = (s: string) => s.split(/[（(]/)[0].toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
    return profiles.find((p) => p.processNames.some((name) => procs.includes(name.toLowerCase())))
      ?? profiles.find((p) => p.processNames.length > 0 && norm(p.displayName) && norm(game.name).includes(norm(p.displayName)));
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
      toast(false, `检测失败：${errText(error)}`);
    } finally {
      detecting = false;
    }
  }

  async function loadIcons(list: InstalledGame[]) {
    for (const game of list) {
      if (icons[game.id]) continue;
      try {
        const bytes = await gameIcon(game.id);
        if (bytes) icons = { ...icons, [game.id]: URL.createObjectURL(new Blob([bytes as BlobPart], { type: "image/png" })) };
      } catch { /* 没有图标就用占位 */ }
    }
  }

  async function launch(game: InstalledGame) {
    launching = game.id;
    try {
      await launchInstalledGame(game.id);
      toast(true, `正在打开 ${game.name}`);
    } catch (error) {
      toast(false, `打开失败：${errText(error)}`);
    } finally {
      launching = null;
    }
  }

  onMount(() => {
    void detect();
    return () => { for (const url of Object.values(icons)) URL.revokeObjectURL(url); };
  });
</script>

<SettingsSection title="本机的游戏" testid="installed-games"
  description="Steam、英雄联盟、Riot、米哈游、Epic、WeGame 里装了的游戏。只读启动器的记录，不扫硬盘。">
  {#snippet actions()}
    {#if elapsedMs !== null && !detecting}<span class="text-[11px] text-muted-foreground" data-testid="installed-games-elapsed">用时 {elapsedMs} ms</span>{/if}
    <Button size="sm" variant="outline" disabled={detecting} onclick={() => void detect()} data-testid="installed-games-detect">
      {#if detecting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<RefreshCw class="size-3.5" />{/if}重新检测
    </Button>
  {/snippet}
  <div class="py-3.5">
    {#if games === null}
      <p class="m-0 text-xs text-muted-foreground">正在检测…</p>
    {:else if !games.length}
      <p class="m-0 text-xs text-muted-foreground" data-testid="installed-games-empty">没有检测到。没装在这些启动器里的游戏，可以在下面的游戏配置里手动填启动路径。</p>
    {:else}
      <ul class="m-0 grid list-none grid-cols-1 gap-2 p-0 sm:grid-cols-2">
        {#each games as game (game.id)}
          {@const profile = profileFor(game)}
          <li class="flex items-center gap-3 rounded-xl border border-hairline bg-background/40 px-3 py-2.5" data-testid="installed-game" data-game-id={game.id}>
            <span class="flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-muted">
              {#if icons[game.id]}<img src={icons[game.id]} alt="" class="size-8 object-contain" data-testid="installed-game-icon" />
              {:else}<Gamepad2 class="size-5 text-muted-foreground" />{/if}
            </span>
            <div class="min-w-0 flex-1">
              <p class="m-0 truncate text-[13px] font-medium" title={game.name}>{game.name}</p>
              <p class="m-0 mt-0.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <span>{SOURCE_LABEL[game.source] ?? game.source}</span>
                {#if profile}
                  <button type="button" class="rounded bg-primary/10 px-1.5 text-primary hover:bg-primary/20" onclick={() => onOpenProfile(profile)}
                    data-testid="installed-game-profile">已适配：{profile.displayName}</button>
                {/if}
              </p>
            </div>
            {#if game.launch}
              <Button size="sm" variant="ghost" aria-label={`打开 ${game.name}`} title="打开游戏" disabled={launching !== null}
                onclick={() => void launch(game)} data-testid="installed-game-launch">
                {#if launching === game.id}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Play class="size-3.5" />{/if}
              </Button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</SettingsSection>
