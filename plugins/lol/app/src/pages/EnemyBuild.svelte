<script lang="ts" module>
  export interface BuildItem { id: number; name: string; icon?: string; new?: boolean }
  export interface BuildChampion { id: number; short: string }
  /** 你阵亡时那张卡片的数据（推送 `my-death`，后端 plugins/lol/src/pushes/alerts.rs 的 `death_card`）：第几次阵亡、几点、
   * 几秒复活；对面每个人按身上装备的总价（`gold`，客户端资料里的总价）从高到低，`new` 是上次你阵亡以后新出的。 */
  export interface EnemyBuildData {
    deaths: number;
    at: string;
    respawn: number;
    me?: { champion: BuildChampion; level: number; kills: number; deaths: number; assists: number; gold: number } | null;
    enemies: {
      champion: BuildChampion;
      level: number;
      kills: number;
      deaths: number;
      assists: number;
      gold: number;
      items: BuildItem[];
      dead?: boolean;
      hidden?: boolean;
    }[];
    goldKnown?: boolean;
  }
  export function isEnemyBuild(value: unknown): value is EnemyBuildData {
    return !!value && typeof value === "object" && Array.isArray((value as EnemyBuildData).enemies);
  }
</script>

<script lang="ts">
  // 对面的装备（`ui://kotone/lol/enemy-build`，ADR 043 B 轮第 2 条“你阵亡时：出装调整（对面谁肥了、出了什么）”）：
  // 只在推送里出现，一局一张，每次阵亡原地更新（悬浮窗再弹一次）。只写数字和装备：对面每个人的等级、KDA、身上的装备，
  // 按装备总价排（谁身上的钱多一眼看得到），上次你阵亡以后新出的那几件描一圈、标“新”。怎么出装由你和她判断（ADR 043 D2）。
  // 窄到悬浮窗的小窗（约 360 px）时装备换到名字下面一行。
  import { onMount } from "svelte";
  import ChampIcon from "../components/ChampIcon.svelte";
  import Pic from "../components/Pic.svelte";
  import SourceTag from "../components/SourceTag.svelte";
  import type { DisplayMode, Session, ToolResult } from "../host";
  import { m } from "../i18n.svelte";

  let {
    session,
    initial,
    display,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
  } = $props();

  let card = $state<EnemyBuildData | null>(null);
  let loaded = $state(false);
  const full = $derived(display !== "inline");

  function show(result: ToolResult | null) {
    loaded = true;
    if (result && isEnemyBuild(result.structuredContent)) card = result.structuredContent;
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then(show);
    else loaded = true;
  });

  /** 金币：一千以上写成 8.2k。 */
  const gold = (value: number) => (value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value));
  const kda = (k: number, d: number, a: number) => `${k}/${d}/${a}`;
</script>

<div class="app" data-testid="lol-app-enemy-build" data-deaths={card?.deaths ?? 0}>
  {#if card}
    <div class="eb" class:full>
      <div class="card-head">
        <span class="t">{m.plugin_lol_page_enemy_build()}</span>
        <span class="sub">{m.plugin_lol_death_sub({ count: card.deaths, at: card.at, respawn: card.respawn })}</span>
      </div>
      <div class="rows" data-testid="lol-enemy-build-rows">
        {#each card.enemies as row, i (i)}
          <div class="row stripe enemy" class:dead={row.dead} data-testid="lol-enemy-build-row">
            <ChampIcon id={row.champion.id} name={row.champion.short} size={24} level={row.level} />
            <span class="who">
              <span class="name">{row.champion.short}</span>
              {#if row.hidden}<span class="chip">{m.plugin_lol_identity_hidden()}</span>{/if}
              {#if row.dead}<span class="chip">{m.plugin_lol_death_dead()}</span>{/if}
            </span>
            <span class="kda num">{kda(row.kills, row.deaths, row.assists)}</span>
            {#if card.goldKnown}<span class="gold num" title={m.plugin_lol_death_gold()}>{gold(row.gold)}</span>{/if}
            <span class="items">
              {#each row.items as item (item.id)}
                <span class="item" class:new={item.new} data-new={item.new ? "true" : "false"} title={item.new ? `${item.name} · ${m.plugin_lol_death_new_hint()}` : item.name}>
                  <Pic path={item.icon} name={item.name || String(item.id)} size={20} />
                  {#if item.new}<span class="sr-only">{m.plugin_lol_death_new()}</span>{/if}
                </span>
              {/each}
            </span>
          </div>
        {/each}
      </div>
      {#if card.me}
        <p class="me" data-testid="lol-enemy-build-me">
          <!-- 客户端资料里没有装备价钱时不写总价（对面那一列也不画），免得写成“装备 0” -->
          {card.goldKnown
            ? m.plugin_lol_death_me({ level: card.me.level, kda: kda(card.me.kills, card.me.deaths, card.me.assists), gold: gold(card.me.gold) })
            : m.plugin_lol_death_me_no_gold({ level: card.me.level, kda: kda(card.me.kills, card.me.deaths, card.me.assists) })}
        </p>
      {/if}
      <div class="card-foot">
        {#if card.goldKnown}<span class="order">{m.plugin_lol_death_order()} · </span>{/if}<SourceTag id="client" onLink={session.openLink} />
      </div>
    </div>
  {:else if loaded}
    {#if full}
      <div class="empty-state" data-testid="lol-app-status"><p>{m.plugin_lol_death_empty()}</p></div>
    {:else}
      <p class="card-line" data-testid="lol-app-status">{m.plugin_lol_death_empty()}</p>
    {/if}
  {/if}
</div>

<style>
  .eb.full {
    max-width: 640px;
    padding: var(--lol-sp-4) 0;
  }
  .row {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) 5.2em 3.4em auto;
    grid-template-areas: "icon who kda gold items";
    align-items: center;
    column-gap: 8px;
    min-height: var(--lol-row-card);
    padding: 4px 12px 4px 14px;
  }
  .row + .row {
    box-shadow: inset 0 1px 0 var(--lol-line);
  }
  .row.dead {
    color: var(--lol-text-2);
  }
  .row > :global(:first-child) {
    grid-area: icon;
    align-self: start;
  }
  .who {
    grid-area: who;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--lol-fw-strong);
  }
  .kda {
    grid-area: kda;
    text-align: right;
    white-space: nowrap;
  }
  .gold {
    grid-area: gold;
    text-align: right;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .items {
    grid-area: items;
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    justify-content: flex-end;
  }
  .item {
    position: relative;
    display: inline-flex;
    border-radius: var(--lol-r-xs);
  }
  /* 上次阵亡以后新出的：墨线描一圈 */
  .item.new {
    box-shadow: 0 0 0 1.5px var(--lol-text);
  }
  .me {
    margin: 6px 14px 0;
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .card-foot {
    padding-top: 8px;
  }
  .order {
    vertical-align: middle;
  }
  /* 窄（悬浮窗的小窗）：装备换到名字下面一行 */
  @container (max-width: 480px) {
    .row {
      grid-template-columns: 24px minmax(0, 1fr) 5.2em 3.4em;
      grid-template-areas:
        "icon who kda gold"
        "icon items items items";
      row-gap: 3px;
      padding: 6px 10px 6px 12px;
    }
    .items {
      justify-content: flex-start;
    }
  }
</style>
