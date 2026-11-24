<script lang="ts">
  // 这局（`ui://kotone/lol/match`，`lol.scout` 的页面）：全屏是插件页（顶栏、阶段牌、四种状态，每几秒自己更新），
  // inline 是聊天卡片“开局战绩”（她调 `lol.scout` 时、自动卡片；是那一刻的快照，不自己更新）。
  // 点一个人看他的战绩；结算后点“看这局详情”打开单局详情。
  import { onMount } from "svelte";
  import Footer from "../components/Footer.svelte";
  import Icon from "../components/Icon.svelte";
  import StateBlock from "../components/StateBlock.svelte";
  import TopBar, { type PageId } from "../components/TopBar.svelte";
  import { PAGE_SETTINGS, resultStatus, TOOL_SCOUT, type DisplayMode, type Session, type ToolResult } from "../host";
  import { stateView, type ToolState } from "../status";
  import { m } from "../i18n.svelte";
  import { isMatchData, type MatchData, type MatchPlayer } from "../types";
  import MatchCard from "./match/MatchCard.svelte";
  import MatchFull from "./match/MatchFull.svelte";
  import { phaseOf } from "./match/match";

  let {
    session,
    initial,
    display,
    onTab,
    onPlayer,
    onGame,
  }: {
    session: Session;
    /** 宿主推来的结果（只有最先打开的那一页有）。 */
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
    onTab: (page: PageId) => void;
    onPlayer: (puuid: string) => void;
    /** 打开单局详情；`justEnded`：从结算的“看这局详情”点进去。 */
    onGame: (gameId: number, justEnded?: boolean) => void;
  } = $props();

  let match = $state<MatchData | null>(null);
  let status = $state<ToolState | null>(null);
  let error = $state("");
  let loading = $state(false);
  let loaded = $state(false);
  let now = $state(Date.now());

  const full = $derived(display !== "inline");
  const phase = $derived(match ? phaseOf(match) : null);
  const canOpen = $derived(!!session.context?.availableDisplayModes?.includes("fullscreen"));

  function show(result: ToolResult) {
    loaded = true;
    if (isMatchData(result.structuredContent)) {
      match = result.structuredContent;
      status = null;
    } else {
      // 不在对局里、客户端没开：留着上一局（设计第四节 1“客户端关闭”），只换阶段牌
      status = resultStatus(result);
    }
  }

  async function load() {
    if (loading) return;
    loading = true;
    error = "";
    try {
      show(await session.call(TOOL_SCOUT));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      loaded = true;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then((result) => (result ? show(result) : load()));
    else void load();
  });

  // 全屏时自己更新：在对局里每 5 秒，不在对局里每 10 秒（客户端开了、进了下一局都会自己出来）。卡片是快照，不更新。
  $effect(() => {
    if (!full) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      if (stopped) return;
      if (document.visibilityState === "visible") await load();
      if (!stopped) timer = setTimeout(tick, match && !status ? 5000 : 10000);
    };
    timer = setTimeout(tick, 5000);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  });

  // 阶段牌上的“进加载 N 秒”
  $effect(() => {
    if (phase !== "loading") return;
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });

  const pill = $derived.by(() => {
    if (status && status.status !== "ok") {
      const view = stateView(status);
      return { dot: view.dot, label: view.pill || m.plugin_lol_phase_none(), more: "" };
    }
    if (!match) return null;
    const label =
      phase === "champ_select"
        ? m.plugin_lol_phase_select()
        : phase === "loading"
          ? m.plugin_lol_phase_loading()
          : phase === "in_game"
            ? m.plugin_lol_phase_in_game()
            : phase === "post_match"
              ? m.plugin_lol_phase_post()
              : m.plugin_lol_phase_live();
    const since = match.phaseSince;
    const secs =
      phase === "loading" && typeof since === "number" && since > 0
        ? m.plugin_lol_phase_loading_secs({ seconds: Math.max(0, Math.round((now - since) / 1000)) })
        : "";
    return { dot: phase === "post_match" ? "idle" : "", label, more: [match.mode, secs].filter(Boolean).join(" · ") };
  });

  function openPlayer(p: MatchPlayer) {
    if (p.puuid) onPlayer(p.puuid);
  }
</script>

{#snippet skeleton(rows: number)}
  {#each Array.from({ length: rows }, (_, i) => i) as i (i)}
    <div class="sk-row">
      <span class="ci s40"></span>
      <span class="sk-lines"><span class="skel" style:width="{120 + ((i * 37) % 60)}px"></span><span class="skel" style:width="{180 + ((i * 53) % 80)}px"></span></span>
    </div>
  {/each}
{/snippet}

{#if full}
  <div class="app" data-testid="lol-app-match" data-phase={phase ?? ""}>
    <TopBar current="match" {onTab} onRefresh={() => void load()} {loading}>
      {#snippet end()}
        {#if pill}
          <span class="phase" data-testid="lol-match-phase">
            <span class="dot {pill.dot}"></span><b>{pill.label}</b>{#if pill.more}<span class="more">· {pill.more}</span>{/if}
          </span>
        {/if}
      {/snippet}
    </TopBar>
    <main class="page">
      {#if error}
        <p class="banner danger" role="alert"><Icon name="alert" />{m.plugin_lol_match_failed({ error })}<button type="button" class="btn" onclick={() => void load()}>{m.plugin_lol_retry()}</button></p>
      {/if}
      {#if match}
        {#if status}<p class="banner" data-testid="lol-app-status-banner" title={status.text}><Icon name="info" />{stateView(status).text}</p>{/if}
        <MatchFull {match} onPlayer={openPlayer} onGame={(id) => onGame(id, true)} onMine={() => onPlayer("")} />
      {:else if status}
        <StateBlock state={status} onSettings={() => session.openPage(PAGE_SETTINGS, {})}>
          {#if status.status !== "disabled"}
            <button type="button" class="btn" data-testid="lol-app-see-mine" onclick={() => onPlayer("")}>{m.plugin_lol_see_mine()}</button>
          {/if}
        </StateBlock>
      {:else if !loaded}
        <div class="sk-teams" aria-busy="true" aria-label={m.plugin_lol_loading()}>
          <div class="team">{@render skeleton(5)}</div>
          <div class="team">{@render skeleton(5)}</div>
        </div>
      {/if}
      <Footer data={m.plugin_lol_foot_client()} onLink={session.openLink} />
    </main>
  </div>
{:else}
  <div class="app" data-testid="lol-app-match" data-phase={phase ?? ""}>
    {#if match}
      <MatchCard {match} {canOpen} onOpen={() => void session.requestDisplayMode("fullscreen")} onLink={session.openLink} />
    {:else if error}
      <p class="card-line danger" role="alert">{m.plugin_lol_match_failed({ error })}</p>
    {:else if status}
      <StateBlock state={status} inline />
    {:else}
      <div aria-busy="true" aria-label={m.plugin_lol_loading()}>{@render skeleton(3)}</div>
    {/if}
  </div>
{/if}

<style>
  .sk-teams {
    display: grid;
    gap: var(--lol-sp-5);
    grid-template-columns: minmax(0, 1fr);
  }
  @container (min-width: 880px) {
    .sk-teams {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
  }
  .sk-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: var(--lol-row-player);
    padding: 8px 12px 8px 14px;
  }
  .sk-row + .sk-row {
    border-top: 1px solid var(--lol-line);
  }
  .sk-lines {
    display: grid;
    gap: 8px;
  }
</style>
