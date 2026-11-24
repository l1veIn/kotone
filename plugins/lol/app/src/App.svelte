<script lang="ts">
  // 英雄联盟 App 的外壳：和宿主握手，按 <meta name="kotone:page"> 选最先显示的页面（几个页面共用一个打包），
  // 之后在 App 里自己换页：顶栏的分页、点一个人看战绩、点一局看单局详情。显示方式（inline 聊天卡片 /
  // fullscreen 插件页）跟着宿主上下文变（docs/design/lol-app.md 第一节 2）。
  import { onMount } from "svelte";
  import type { PageId } from "./components/TopBar.svelte";
  import { connect, currentPage, displayOf, type DisplayMode, type Session } from "./host";
  import { m } from "./i18n.svelte";
  import Game from "./pages/Game.svelte";
  import Match from "./pages/Match.svelte";
  import Player from "./pages/Player.svelte";
  import Tier from "./pages/Tier.svelte";

  interface View {
    page: PageId;
    puuid?: string;
    gameId?: number;
    /** 第几次换页：0 是宿主打开的那一页（只有它用宿主推来的结果）。 */
    key: number;
  }

  const PAGES: PageId[] = ["match", "player", "game", "tier"];
  const first = currentPage();

  let session = $state<Session | null>(null);
  let error = $state("");
  let display = $state<DisplayMode>("inline");
  let view = $state<View>({ page: (PAGES as string[]).includes(first) ? (first as PageId) : "tier", key: 0 });
  /** 单局详情左上角“← 战绩”回到的那一页。 */
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

  function go(next: Omit<View, "key">) {
    back = next.page === "game" && view.page === "player" ? view : null;
    view = { ...next, key: view.key + 1 };
    window.scrollTo(0, 0);
  }
  const tab = (page: PageId) => go({ page });
  const player = (puuid: string) => go({ page: "player", puuid });
  const game = (gameId: number) => go({ page: "game", gameId });
  const initial = $derived(session && view.key === 0 ? session.initial : null);
</script>

{#if error}
  <p class="card-line danger" role="alert">{m.plugin_lol_failed({ error })}</p>
{:else if session}
  {#key view.key}
    {#if view.page === "match"}
      <Match {session} {initial} {display} onTab={tab} onPlayer={player} onGame={game} />
    {:else if view.page === "player"}
      <Player {session} {initial} {display} puuid={view.puuid} onTab={tab} onGame={game} />
    {:else if view.page === "game"}
      <Game {session} {initial} {display} gameId={view.gameId} onTab={tab} onPlayer={player} onBack={back ? () => back && go(back) : undefined} />
    {:else}
      <Tier {session} {initial} {display} onTab={tab} />
    {/if}
  {/key}
{/if}
