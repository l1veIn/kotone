<script lang="ts" module>
  /** 稀有度：客户端写 `kSilver` / `kGold` / `kPrismatic` / `kEventChoice`，设计文档写小写；两种都认。 */
  export function rarityOf(raw: string | null | undefined): "silver" | "gold" | "prismatic" | "event" {
    const r = String(raw ?? "").toLowerCase().replace(/^k/, "");
    return r === "gold" ? "gold" : r === "prismatic" ? "prismatic" : r.startsWith("event") ? "event" : "silver";
  }
</script>

<script lang="ts">
  // 海克斯：深色格子（图标是白色线稿，亮暗主题一样），边框表示稀有度——银 1 px、金 1.5 px、棱彩 1.5 px 渐变；
  // 图挂了显示名字第一个字（用稀有度的颜色）；名字和稀有度在 title（“棱彩 · 玻璃大炮”）。
  import { sources } from "../assets";
  import { firstChar } from "../format";
  import { m } from "../i18n.svelte";
  import type { Augment } from "../types";

  let { augment, size = 20 }: { augment: Augment; size?: number } = $props();
  const rarity = $derived(rarityOf(augment.rarity));
  const rarityName = $derived(
    {
      silver: m.plugin_lol_rarity_silver(),
      gold: m.plugin_lol_rarity_gold(),
      prismatic: m.plugin_lol_rarity_prismatic(),
      event: m.plugin_lol_rarity_event(),
    }[rarity],
  );
  const name = $derived(augment.name || String(augment.id));
  const list = $derived(sources(augment.icon));
  let attempt = $state(0);
</script>

<span class="aug {rarity}" style:--s="{size}px" title="{rarityName} · {name}">
  {#if list[attempt]}
    <img src={list[attempt]} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" onerror={() => (attempt += 1)} />
  {:else}
    <span class="ph" aria-hidden="true">{firstChar(name)}</span>
  {/if}
  {#if size >= 40}<span class="dot" aria-hidden="true"></span>{/if}
</span>

<style>
  .aug {
    --edge: var(--lol-aug-silver);
    position: relative;
    display: inline-grid;
    flex: none;
    place-items: center;
    width: var(--s);
    height: var(--s);
    overflow: hidden;
    border: 1px solid var(--edge);
    border-radius: var(--lol-r-xs);
    background: var(--lol-aug-tile);
  }
  .aug.gold {
    --edge: var(--lol-aug-gold);
    border-width: 1.5px;
  }
  .aug.event {
    --edge: var(--lol-aug-event);
  }
  .aug.prismatic {
    border: 1.5px solid transparent;
    background:
      linear-gradient(var(--lol-aug-tile), var(--lol-aug-tile)) padding-box,
      var(--lol-aug-prismatic) border-box;
  }
  img {
    display: block;
    width: 86%;
    height: 86%;
    object-fit: contain;
  }
  .ph {
    font-size: calc(var(--s) * 0.5);
    line-height: 1;
    font-weight: var(--lol-fw-medium);
    color: var(--edge);
  }
  .prismatic .ph {
    color: #c6b4ff;
  }
  .dot {
    position: absolute;
    right: 3px;
    bottom: 3px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--edge);
  }
  .prismatic .dot {
    background: var(--lol-aug-prismatic);
  }
</style>
