<script lang="ts">
  // 英雄技能（被动、Q、W、E、R）的图标（docs/design/lol-app.md 第一节 7）：方形 3 px 圆角，右下角压键位字母（深底白字小牌）；
  // 没有图（客户端没开、CommunityDragon 也没取到）或图挂了是灰格子 + 键位字母，R 是墨底（和原来的字母格子一样）。
  // 技能名放在鼠标停留提示里（“Q · 斩钢闪”“被动 · 浪客之道”）。
  import { m } from "../i18n.svelte";
  import type { Ability } from "../types";
  import Pic from "./Pic.svelte";

  let { k, ability, size = 24 }: { k: string; ability?: Ability | null; size?: number } = $props();

  const label = $derived(k === "P" ? m.plugin_lol_passive() : k);
  const title = $derived(ability?.name ? `${label} · ${ability.name}` : label);
</script>

<span class="ability" class:ult={k === "R"} class:small={size < 24} data-testid="lol-ability" data-key={k} data-icon={ability?.icon ? "1" : "0"}>
  <Pic path={ability?.icon} name={ability?.name || label} {title} fallback={k} badge={k} {size} />
</span>

<style>
  .ability {
    display: inline-flex;
    flex: none;
  }
  /* 没图时的字母：和原来的字母格子一样粗 */
  .ability :global(.ph) {
    font-size: var(--lol-fs-xs);
    font-weight: var(--lol-fw-strong);
  }
  /* 墨底只给字母格子（有图时图自己就是底） */
  .ability.ult :global(.ph) {
    background: var(--lol-ink-fill);
    color: var(--lol-ink-on);
  }
  /* 键位小牌：比等级小牌小一号，盖住的图少一点 */
  .ability :global(.lv) {
    right: 0;
    bottom: 0;
    min-width: 10px;
    height: 11px;
    padding: 0 2px;
    border-radius: 3px 0 0 0;
    font-size: 9px;
    line-height: 11px;
    font-weight: var(--lol-fw-strong);
  }
  .ability.small :global(.lv) {
    min-width: 9px;
    height: 10px;
    padding: 0 1px;
    font-size: 8px;
    line-height: 10px;
  }
</style>
