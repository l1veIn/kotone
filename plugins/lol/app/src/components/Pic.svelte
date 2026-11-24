<script lang="ts">
  // 一张图（英雄、召唤师头像、技能、装备、符文、海克斯）：按顺序试 Kotone 的素材层和 CommunityDragon；
  // 加载中是同形状的灰格子，都取不到时显示名字的第一个字。名字永远在 title 里（AramHex 的“先显示文字”）。
  // 不重试：下次打开页面再取。
  import type { Snippet } from "svelte";
  import { sources } from "../assets";
  import { firstChar } from "../format";

  let {
    path,
    name,
    size = 40,
    crop = false,
    round = false,
    title,
    fallback,
    cls = "",
    children,
  }: {
    /** 客户端资源路径；空 = 没有图，直接显示字。 */
    path: string | null | undefined;
    name: string;
    size?: number;
    /** 英雄头像：放大到 110% 裁掉原图四周的暗边。 */
    crop?: boolean;
    round?: boolean;
    /** 鼠标停上去的字，默认是名字。 */
    title?: string;
    /** 图挂了时显示的字，默认是名字的第一个字。 */
    fallback?: string;
    cls?: string;
    /** 叠在图上的东西（等级小牌）。 */
    children?: Snippet;
  } = $props();

  let failed = $state<Record<string, number>>({});
  const key = $derived(path ?? "");
  const list = $derived(sources(path));
  const src = $derived(list[failed[key] ?? 0] ?? "");
</script>

<span class="ci s{size} {cls}" class:crop class:round title={title ?? name}>
  {#if src}
    <img
      {src}
      alt=""
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      onerror={() => (failed[key] = (failed[key] ?? 0) + 1)}
    />
  {:else}
    <span class="ph" aria-hidden="true">{fallback ?? firstChar(name)}</span>
  {/if}
  {@render children?.()}
</span>
