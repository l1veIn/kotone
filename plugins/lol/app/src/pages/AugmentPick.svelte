<script lang="ts" module>
  /** 一张海克斯：判断照 AramHex 的规则（后端 plugins/lol/src/pushes/rules.rs）；`grade` 是这个英雄同稀有度里的 S–D；
   * `few`：样本偏少。认不出的那张只有 `unread`。 */
  export type PickSlot =
    | { id: number; name: string; rarity?: string; icon?: string; grade?: string | null; verdict: "best" | "ok" | "reroll" | "noData"; few?: boolean }
    | { unread: true };
  /** 海克斯三选一卡片的数据（推送 `augment-pick`，后端 plugins/lol/src/pushes/augment_pick.rs 的 `View::card`）：
   * 从左到右三格；`best` 首选在第几格；`weak` 三张都一般；`ended` 这一次选完了。 */
  export interface AugmentPickData {
    champion?: { id: number; short: string } | null;
    slots: PickSlot[];
    best?: number | null;
    weak?: boolean;
    source?: string;
    patch?: string;
    ended?: boolean;
  }
  export function isAugmentPick(value: unknown): value is AugmentPickData {
    return !!value && typeof value === "object" && Array.isArray((value as AugmentPickData).slots);
  }
</script>

<script lang="ts">
  // 海克斯三选一（`ui://kotone/lol/augment-pick`，ADR 043 D7 的 2026-11-26 晚修订）：玩家说“不需要单独给海克斯做一个UI，
  // 就直接推卡片就行啊，就像战绩一样推送，卡片上写清楚三个海克斯推荐程度就行”。只在推送里出现：三选一出现时插件推来，
  // 刷新了哪张同一张卡片原地变（宿主再推一次结果，这里跟着重画），选完了标“选好了”。
  // 三张从左到右，和游戏里的位置一样：图标、名字、评级（S–D）、判断（首选 绿 / 可选 蓝 / 可刷新 琥珀 / 没数据）。
  // 不写胜率数字（拳头不许显示海克斯的胜率，ADR 043 D7；判断本身也在那条里，风险写在插件的“风险和许可”里）。
  // 窄到悬浮窗的小窗（约 360 px）三列照样放得下。只学 AramHex 标什么，视觉是“记分纸”，不抄代码。
  import { onMount } from "svelte";
  import Augment from "../components/Augment.svelte";
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

  let pick = $state<AugmentPickData | null>(null);
  let loaded = $state(false);
  const full = $derived(display !== "inline");

  function show(result: ToolResult | null) {
    loaded = true;
    if (result && isAugmentPick(result.structuredContent)) pick = result.structuredContent;
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then(show);
    else loaded = true;
  });

  /** 判断的字（一个个写出来，terminology.spec 认得出用到了哪些）。 */
  function verdictText(verdict: string): string {
    switch (verdict) {
      case "best":
        return m.plugin_lol_pick_best();
      case "ok":
        return m.plugin_lol_pick_ok();
      case "reroll":
        return m.plugin_lol_pick_reroll();
      default:
        return m.plugin_lol_pick_no_data();
    }
  }
  const places = [m.plugin_lol_pick_place_left, m.plugin_lol_pick_place_middle, m.plugin_lol_pick_place_right];
  const slots = $derived((pick?.slots ?? []).slice(0, 3));
</script>

<div class="app" data-testid="lol-app-augment-pick" data-ended={pick?.ended ? "true" : "false"}>
  {#if pick}
    <div class="pick" class:full>
      <div class="card-head">
        <span class="t">{m.plugin_lol_page_augment_pick()}</span>
        <span class="sub">{pick.ended ? m.plugin_lol_pick_ended() : pick.champion ? m.plugin_lol_pick_for({ champion: pick.champion.short }) : ""}</span>
      </div>
      <div class="cols" data-testid="lol-augment-pick-cols">
        {#each slots as slot, i (i)}
          {#if "unread" in slot}
            <div class="col unread" data-testid="lol-augment-pick-slot" data-verdict="unread" data-best="false">
              <span class="place">{places[i]()}</span>
              <span class="ph" aria-hidden="true">?</span>
              <span class="name muted">{m.plugin_lol_pick_unread()}</span>
            </div>
          {:else}
            <div class="col" class:best={pick.best === i} data-testid="lol-augment-pick-slot" data-verdict={slot.verdict} data-best={pick.best === i ? "true" : "false"}>
              <span class="place">{places[i]()}</span>
              <Augment augment={slot} size={36} />
              <span class="name" title={slot.name}>{slot.name}</span>
              <span class="tags">
                {#if slot.grade}<span class="grade" data-grade={slot.grade}>{slot.grade}</span>{/if}
                <span class="verdict" data-verdict={slot.verdict}>{verdictText(slot.verdict)}</span>
              </span>
              {#if slot.few}<span class="few">{m.plugin_lol_pick_few()}</span>{/if}
            </div>
          {/if}
        {/each}
      </div>
      {#if pick.weak && !pick.ended}<p class="weak" data-testid="lol-augment-pick-weak">{m.plugin_lol_pick_weak()}</p>{/if}
      <div class="card-foot"><SourceTag id="aramhex" date={pick.patch ?? ""} onLink={session.openLink} /></div>
    </div>
  {:else if loaded}
    {#if full}
      <div class="empty-state" data-testid="lol-app-status"><p>{m.plugin_lol_pick_empty()}</p></div>
    {:else}
      <p class="card-line" data-testid="lol-app-status">{m.plugin_lol_pick_empty()}</p>
    {/if}
  {/if}
</div>

<style>
  .pick.full {
    max-width: 560px;
    padding: var(--lol-sp-4) 0;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
    padding: 0 12px;
  }
  .col {
    display: grid;
    justify-items: center;
    align-content: start;
    gap: 4px;
    min-width: 0;
    padding: 8px 6px 9px;
    border-radius: var(--lol-r-md);
    background: var(--lol-bg-2);
    text-align: center;
  }
  /* 首选：墨线圈出来（和选英雄卡片“值得换”的那个一样，不用金色） */
  .col.best {
    box-shadow: inset 0 0 0 1.5px var(--lol-text);
  }
  .place {
    font-size: var(--lol-fs-2xs);
    line-height: 1;
    color: var(--lol-text-3);
  }
  .ph {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border: 1px dashed var(--lol-line-2);
    border-radius: var(--lol-r-xs);
    color: var(--lol-text-3);
    font-weight: var(--lol-fw-medium);
  }
  .name {
    display: -webkit-box;
    max-width: 100%;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    font-size: var(--lol-fs-sm);
    line-height: 1.3;
    font-weight: var(--lol-fw-strong);
    word-break: break-all;
  }
  .name.muted {
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-3);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 4px;
  }
  /* 判断：首选 胜色、可选 我方蓝、可刷新 提醒色、没数据 灰；字本身就能分，颜色只是帮着看 */
  .verdict {
    --v: var(--lol-text-3);
    display: inline-grid;
    place-items: center;
    height: 18px;
    padding: 0 6px;
    border-radius: var(--lol-r-xs);
    font-size: var(--lol-fs-xs);
    font-weight: 700;
    line-height: 1;
    white-space: nowrap;
    color: var(--v);
    background: color-mix(in srgb, var(--v) 12%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--v) 45%, transparent);
  }
  .verdict[data-verdict="best"] { --v: var(--lol-win); }
  .verdict[data-verdict="ok"] { --v: var(--lol-ally); }
  .verdict[data-verdict="reroll"] { --v: var(--lol-warn); }
  /* 评级字母小牌（S–D），和选英雄卡片一样 */
  .grade {
    --g: var(--lol-text);
    display: inline-grid;
    place-items: center;
    min-width: 18px;
    height: 18px;
    padding: 0 3px;
    border-radius: var(--lol-r-xs);
    font-size: var(--lol-fs-xs);
    font-weight: 700;
    line-height: 1;
    color: var(--g);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--g) 45%, transparent);
  }
  .grade[data-grade="S"] { --g: var(--lol-win); }
  .grade[data-grade="A"] { --g: var(--lol-ally); }
  .grade[data-grade="C"] { --g: var(--lol-warn); }
  .grade[data-grade="D"] { --g: var(--lol-loss); }
  .few {
    font-size: var(--lol-fs-2xs);
    line-height: 1.2;
    color: var(--lol-text-3);
  }
  .weak {
    margin: 8px 14px 0;
    font-size: var(--lol-fs-xs);
    color: var(--lol-warn);
  }
  .card-foot {
    padding-top: 8px;
  }
  @container (max-width: 400px) {
    .cols {
      gap: 4px;
      padding: 0 8px;
    }
    .col {
      padding: 7px 4px 8px;
    }
  }
</style>
