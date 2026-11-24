<script lang="ts">
  // 英雄联盟 App 的外壳：和宿主握手，按 <meta name="kotone:page"> 选最先显示的页面（几个页面共用一个打包），
  // 之后在 App 里自己换页：顶栏的分页（这局 · 战绩 · 英雄 · 强度榜）、点一个人看战绩、点一局看单局详情、点一个英雄看
  // 英雄页。在 Kotone 的插件页里和在 Claude 这类宿主里都是同一套导航（插件的设置是宿主的页面）。
  // 显示方式（inline 聊天卡片 / fullscreen 插件页）跟着宿主上下文变（docs/design/lol-app.md 第一节 2）。
  import { onMount } from "svelte";
  import type { PageId } from "./components/TopBar.svelte";
  import { connect, currentPage, displayOf, type DisplayMode, type Session } from "./host";
  import { m } from "./i18n.svelte";
  import Champion from "./pages/Champion.svelte";
  import Game from "./pages/Game.svelte";
  import Match from "./pages/Match.svelte";
  import Player from "./pages/Player.svelte";
  import Tier from "./pages/Tier.svelte";

  interface View {
    page: PageId;
    args: Record<string, unknown>;
    /** 第几次换页：0 是宿主打开的那一页（只有它用宿主推来的结果）。 */
    key: number;
  }

  const PAGES: PageId[] = ["match", "player", "game", "champion", "tier"];
  const first = currentPage();

  let session = $state<Session | null>(null);
  let error = $state("");
  let display = $state<DisplayMode>("inline");
  let view = $state<View>({ page: (PAGES as string[]).includes(first) ? (first as PageId) : "tier", args: {}, key: 0 });
  /** 点进来的页（单局详情、别的英雄）左上角“←”回到的那一页。 */
  let back = $state<View | null>(null);

  onMount(() => {
    connect()
      .then((s) => {
        display = displayOf(s.context);
        s.onContext((context) => (display = displayOf(context)));
        session = s;
      })
      .catch((e: unknown) => (error = e instanceof Error ? e.message : String(e)));
  });

  function go(page: PageId, args: Record<string, unknown> = {}) {
    back = page === "game" && (view.page === "player" || view.page === "match") ? view : null;
    view = { page, args, key: view.key + 1 };
    window.scrollTo(0, 0);
  }
  const tab = (page: PageId) => go(page);
  const player = (puuid: string) => go("player", puuid ? { puuid } : {});
  const game = (gameId: number, justEnded = false) => go("game", justEnded ? { gameId, justEnded } : { gameId });
  const champion = (args: Record<string, string>) => go("champion", args);
  const initial = $derived(session && view.key === 0 ? session.initial : null);
  function goBack() {
    if (!back) return;
    const to = back;
    back = null;
    view = { ...to, key: view.key + 1 };
  }
</script>

{#if error}
  <p class="card-line danger" role="alert">{m.plugin_lol_failed({ error })}</p>
{:else if session}
  {#key view.key}
    {#if view.page === "match"}
      <Match {session} {initial} {display} onTab={tab} onPlayer={player} onGame={game} />
    {:else if view.page === "player"}
      <Player {session} {initial} {display} args={view.args} onTab={tab} onGame={game} />
    {:else if view.page === "game"}
      <Game {session} {initial} {display} args={view.args} onTab={tab} onPlayer={player} onBack={back ? goBack : undefined} backTo={back?.page} />
    {:else if view.page === "champion"}
      <Champion {session} {initial} {display} args={view.args} onTab={tab} onChampion={champion} />
    {:else}
      <Tier {session} {initial} {display} args={view.args} onTab={tab} onChampion={champion} />
    {/if}
  {/key}
{/if}
