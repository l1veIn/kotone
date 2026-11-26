<script lang="ts">
  // 选人卡片上“我的英雄”那块和“一键导入”（ADR 043 B 轮；后端 plugins/lol/src/pushes/import.rs）。
  // 峡谷：腾讯 101 这一路的符文（局数最多的那页）、出门 / 鞋 / 核心三件 / 后期、加点、召唤师技能；
  // 海斗：aramhex-data 的最好的海克斯、核心三件、召唤师技能（没有符文）。卡片上画的就是导入时写的那一份。
  // 一键导入调只给页面的 `lol.import`：“操作客户端”关着时不调，说清楚为什么、带玩家去插件设置（不替玩家打开开关）。
  // 学的是 LeagueAkari 英雄数据窗“写进客户端”的做法（只学做法，视觉是“记分纸”，docs/design/lol-app.md）。
  import Augment from "../../components/Augment.svelte";
  import ChampIcon from "../../components/ChampIcon.svelte";
  import Pic from "../../components/Pic.svelte";
  import SourceTag from "../../components/SourceTag.svelte";
  import { rate } from "../../format";
  import { PAGE_SETTINGS, type Session } from "../../host";
  import { m } from "../../i18n.svelte";
  import type { ImportResult, ItemGroup, MineBlock, Written } from "./types";

  /** 对外（MCP）的工具名（`lol.import`）。 */
  const TOOL_IMPORT = "kotone_lol_import";

  let {
    mine,
    control,
    ended,
    session,
  }: { mine: MineBlock; control: boolean; ended: boolean; session: Session } = $props();

  let busy = $state(false);
  let result = $state<ImportResult | null>(null);
  const mayhem = $derived(mine.source === "aramhex");
  const can = $derived(mine.importable.runes || mine.importable.items || mine.importable.spells);

  function groupName(g: ItemGroup): string {
    switch (g.kind) {
      case "starting":
        return m.plugin_lol_build_starting();
      case "boots":
        return m.plugin_lol_build_boots();
      case "late":
        return m.plugin_lol_cs_build_late();
      default:
        // 海斗的两套核心三件各有胜率：和峡谷一样写在右边那一列（写进标签里 4.6em 放不下，压在图标上）
        return m.plugin_lol_build_core();
    }
  }

  function parse(text: string): ImportResult {
    try {
      const v = JSON.parse(text) as ImportResult;
      if (v && typeof v.status === "string") return v;
    } catch {
      // 不是 JSON：当成一句出错的话
    }
    return { status: "error", text };
  }

  async function run() {
    if (!control) {
      // 关着：不调工具、不替玩家打开，带玩家去插件设置（打开“操作客户端”时有风险和许可的弹窗）
      session.openPage(PAGE_SETTINGS, {});
      return;
    }
    busy = true;
    result = null;
    try {
      const r = await session.call(TOOL_IMPORT, {});
      const block = r.content?.find((c) => c.type === "text");
      result = parse(block && "text" in block ? String(block.text) : "");
    } catch (error) {
      result = { status: "error", text: String((error as Error)?.message ?? error) };
    } finally {
      busy = false;
    }
  }

  function runesLine(w: Written): string {
    switch (w.result) {
      case "created":
        return m.plugin_lol_cs_runes_created({ name: w.name ?? "" });
      case "updated":
        return m.plugin_lol_cs_runes_updated({ name: w.name ?? "" });
      case "replaced":
        return m.plugin_lol_cs_runes_replaced({ old: w.old ?? "" });
      case "no_data":
        return m.plugin_lol_cs_runes_no_data();
      case "no_room":
        return m.plugin_lol_cs_runes_no_room();
      default:
        return m.plugin_lol_cs_runes_error({ reason: w.error ?? "" });
    }
  }
  function itemsLine(w: Written): string {
    switch (w.result) {
      case "written":
        return m.plugin_lol_cs_items_written({ name: w.name ?? "", blocks: w.blocks ?? 0 });
      case "no_data":
        return m.plugin_lol_cs_items_no_data();
      default:
        return m.plugin_lol_cs_items_error({ reason: w.error ?? "" });
    }
  }
  function spellsLine(w: Written): string {
    switch (w.result) {
      case "written":
        return m.plugin_lol_cs_spells_written();
      case "no_data":
        return m.plugin_lol_cs_spells_no_data();
      case "not_now":
        return m.plugin_lol_cs_spells_not_now();
      default:
        return m.plugin_lol_cs_spells_error({ reason: w.error ?? "" });
    }
  }
  const okResult = (w: Written) => ["created", "updated", "replaced", "written"].includes(w.result);
  const lines = $derived.by(() => {
    const r = result;
    if (!r) return [];
    if (r.status === "disabled" && r.reason === "control_off") return [{ text: m.plugin_lol_cs_import_off(), ok: false, off: true }];
    if (r.status === "not_now" && r.reason === "not_champ_select") return [{ text: m.plugin_lol_cs_import_not_now(), ok: false, off: false }];
    if (!r.runes && !r.items && !r.spells) return [{ text: m.plugin_lol_cs_import_failed({ reason: r.text ?? "" }), ok: false, off: false }];
    const out: { text: string; ok: boolean; off: boolean }[] = [];
    // 海斗本来就没有符文：不写那一行
    if (r.runes && !(mayhem && r.runes.result === "no_data")) out.push({ text: runesLine(r.runes), ok: okResult(r.runes), off: false });
    if (r.items) out.push({ text: itemsLine(r.items), ok: okResult(r.items), off: false });
    if (r.spells) out.push({ text: spellsLine(r.spells), ok: okResult(r.spells), off: false });
    return out;
  });
</script>

<section class="blk" data-testid="lol-cs-mine" data-source={mine.source}>
  <header class="bh">
    <span class="who">
      <ChampIcon id={mine.champion.id} icon={mine.champion.icon} name={mine.champion.short} size={20} />
      <span class="bt">{mine.champion.short}</span>
      {#if mine.lane}<span class="sub">{mine.lane}</span>{/if}
      {#if !mine.locked}<span class="chip">{m.plugin_lol_cs_mine_prepick()}</span>{/if}
    </span>
    <SourceTag id={mine.source} date={mine.patch} onLink={session.openLink} />
  </header>

  {#if mine.augments.length}
    <div class="row" data-testid="lol-cs-mine-augments">
      <!-- 标签列 4.6em：“最好的海克斯”放不下（窄的时候压在图标上），写“海克斯”，右边的 +8.8 已经说明是最好的几个 -->
      <span class="k">{m.plugin_lol_augments_title()}</span>
      <span class="augs">
        {#each mine.augments as a (a.id)}
          <span class="aug" title={a.name}><Augment augment={a} size={20} /><span class="d num" class:good={a.delta > 0}>{`${a.delta > 0 ? "+" : ""}${a.delta.toFixed(1)}`}</span></span>
        {/each}
      </span>
    </div>
  {/if}

  {#if mine.runes}
    <div class="row" data-testid="lol-cs-mine-runes">
      <span class="k">{m.plugin_lol_runes()}</span>
      <span class="icons">
        {#each mine.runes.perks as p, i (i)}
          <Pic path={p.icon} name={p.name || String(p.id)} size={i === 0 ? 22 : i < 6 ? 18 : 14} round />
        {/each}
      </span>
      <span class="r num">{rate(mine.runes.winRate)}</span>
    </div>
  {/if}

  {#each mine.items as g, i (i)}
    <div class="row" data-testid="lol-cs-mine-items" data-kind={g.kind}>
      <span class="k">{groupName(g)}</span>
      <span class="icons">{#each g.items as x, j (j)}<Pic path={x.icon} name={x.name || String(x.id)} size={20} />{/each}</span>
      {#if g.winRate !== null}<span class="r num">{rate(g.winRate)}</span>{/if}
    </div>
  {/each}

  {#if mine.skills && mine.skills.priority.length}
    <div class="row" data-testid="lol-cs-mine-skills">
      <span class="k">{m.plugin_lol_skills()}</span>
      <span class="keys num">{m.plugin_lol_skill_priority({ order: mine.skills.priority.join(" > ") })}{#if mine.skills.order.length}<span class="sub">{` · ${mine.skills.order.join(" ")}`}</span>{/if}</span>
    </div>
  {/if}

  {#if mine.spells.length}
    <div class="row" data-testid="lol-cs-mine-spells">
      <span class="k">{m.plugin_lol_spells()}</span>
      <span class="icons">{#each mine.spells as s, i (i)}<Pic path={s.icon} name={s.name || String(s.id)} size={20} />{/each}
        <span class="sub">{mine.spells.map((s) => s.name).join(" + ")}</span></span>
      {#if mine.spellsWinRate !== null}<span class="r num">{rate(mine.spellsWinRate)}</span>{/if}
    </div>
  {/if}

  {#if can}
    <div class="imp" data-testid="lol-cs-import" data-control={control ? "on" : "off"}>
      {#if ended}
        <p class="note-line">{m.plugin_lol_cs_import_ended()}</p>
      {:else}
        <div class="imp-row">
          <button type="button" class="btn" class:primary={control} class:locked={!control} disabled={busy}
            data-testid="lol-cs-import-button" onclick={run}>{busy ? m.plugin_lol_cs_import_busy() : m.plugin_lol_cs_import()}</button>
          <span class="note-line">{control ? (mayhem ? m.plugin_lol_cs_import_what_mayhem() : m.plugin_lol_cs_import_what()) : m.plugin_lol_cs_import_off()}</span>
        </div>
        {#if !control}
          <button type="button" class="link" data-testid="lol-cs-import-settings" onclick={() => session.openPage(PAGE_SETTINGS, {})}>{m.plugin_lol_source_off_open()}</button>
        {/if}
      {/if}
      {#if lines.length}
        <ul class="result" data-testid="lol-cs-import-result" data-status={result?.status ?? ""}>
          {#each lines as l, i (i)}<li class:good={l.ok} class:bad={!l.ok && !l.off}>{l.text}</li>{/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .blk {
    margin: 6px 0 0;
    padding: 6px 12px 4px;
    border-top: 1px solid var(--lol-line);
  }
  .bh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 24px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    white-space: nowrap;
  }
  .bt {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--lol-fs-sm);
    font-weight: var(--lol-fw-strong);
  }
  .sub {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .row {
    display: grid;
    grid-template-columns: 4.6em minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 8px;
    min-height: 26px;
  }
  .k {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-3);
    white-space: nowrap;
  }
  .icons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 3px;
    min-width: 0;
  }
  .icons .sub {
    margin-left: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .r {
    font-size: var(--lol-fs-xs);
    color: var(--lol-text-2);
  }
  .keys {
    font-size: var(--lol-fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .augs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 8px;
  }
  .aug {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .d {
    font-size: var(--lol-fs-2xs);
    color: var(--lol-text-2);
  }
  .d.good {
    color: var(--lol-win);
  }
  .imp {
    margin: 4px 0 2px;
  }
  .imp-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }
  .btn.locked {
    color: var(--lol-text-2);
    border-style: dashed;
  }
  .note-line {
    margin: 0;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
    color: var(--lol-text-3);
  }
  .link {
    margin: 2px 0 0;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    font-size: var(--lol-fs-xs);
    color: var(--lol-ally);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .result {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
  }
  .result li::before {
    content: "· ";
    color: var(--lol-text-3);
  }
  .result .good {
    color: var(--lol-win);
  }
  .result .bad {
    color: var(--lol-loss);
  }
</style>
