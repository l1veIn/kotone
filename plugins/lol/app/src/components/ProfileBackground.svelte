<script lang="ts">
  import { sources } from "../assets";

  let { path }: { path: string } = $props();
  const urls = $derived(sources(path));
  let attempt = $state(0);
  let loaded = $state(false);
</script>

{#if urls[attempt]}
  <div class="art" class:loaded aria-hidden="true">
    <img src={urls[attempt]} alt="" decoding="async" referrerpolicy="no-referrer" data-testid="lol-home-background"
      onload={() => (loaded = true)} onerror={() => (attempt += 1)} />
  </div>
{/if}

<style>
  .art {
    position: absolute;
    inset: 0 0 auto;
    z-index: -1;
    height: min(100%, 335px);
    overflow: hidden;
    pointer-events: none;
    opacity: 0;
  }
  .art.loaded { opacity: 1; }
  .art img { display: block; width: 100%; height: 100%; object-fit: cover; object-position: center top; }
  .art::after {
    content: "";
    position: absolute;
    inset: 0;
    background:
      linear-gradient(180deg,
        color-mix(in srgb, var(--lol-bg) 16%, transparent),
        color-mix(in srgb, var(--lol-bg) 38%, transparent) 40%,
        color-mix(in srgb, var(--lol-bg) 94%, transparent) 76%,
        var(--lol-bg)),
      linear-gradient(90deg,
        color-mix(in srgb, var(--lol-bg) 95%, transparent) 1%,
        color-mix(in srgb, var(--lol-bg) 81%, transparent) 26%,
        color-mix(in srgb, var(--lol-bg) 9%, transparent) 55%,
        color-mix(in srgb, var(--lol-bg) 24%, transparent) 77%,
        color-mix(in srgb, var(--lol-bg) 78%, transparent));
  }
  @container (max-width: 460px) {
    .art img { object-position: 24% top; }
  }
</style>
