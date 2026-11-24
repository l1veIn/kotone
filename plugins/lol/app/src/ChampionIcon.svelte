<script lang="ts">
  // 英雄头像：按顺序试 Kotone 的素材和 CommunityDragon，都不行就显示名字的第一个字。
  import { championIcon } from "./assets";

  let { id, label, size = 28 }: { id: number; label: string; size?: number } = $props();
  const sources = $derived(championIcon(id));
  let attempt = $state(0);
</script>

{#if attempt < sources.length}
  <img
    class="icon"
    src={sources[attempt]}
    alt={label}
    width={size}
    height={size}
    loading="lazy"
    decoding="async"
    referrerpolicy="no-referrer"
    onerror={() => (attempt += 1)}
  />
{:else}
  <span class="icon fallback" style:width={`${size}px`} style:height={`${size}px`} role="img" aria-label={label}>{label.slice(0, 1)}</span>
{/if}

<style>
  .icon {
    flex: none;
    border-radius: 6px;
    background: var(--surface-2);
    object-fit: cover;
  }
  .fallback {
    display: inline-grid;
    place-items: center;
    font-size: 12px;
    color: var(--muted);
  }
</style>
