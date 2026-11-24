<script lang="ts">
  // 封面：网易云的图（换成 https、按显示大小取缩略图，照 YesPlayMusic 的 resizeImage）；没有或加载不了就用渐变占位。
  import { Heart, Music } from "@lucide/svelte";

  let { url, seed = "", size = 120, liked = false, class: className = "" }: {
    url: string | null;
    /** 占位渐变按它挑颜色（歌名 / 歌单 ID）。 */
    seed?: string;
    /** 取多大的缩略图（像素）。 */
    size?: number;
    /** “我喜欢的音乐”：红心占位。 */
    liked?: boolean;
    class?: string;
  } = $props();

  let failed = $state(false);
  $effect(() => { void url; failed = false; });

  const src = $derived.by(() => {
    if (!url) return null;
    const https = url.replace(/^http:\/\/([^/]*music\.126\.net)/, "https://$1");
    return /music\.126\.net/.test(https) && !https.includes("param=") ? `${https}${https.includes("?") ? "&" : "?"}param=${size}y${size}` : https;
  });
  const hue = $derived.by(() => {
    let h = 0;
    for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) % 360;
    return h;
  });
</script>

<div class="relative shrink-0 overflow-hidden bg-muted {className}" aria-hidden="true">
  {#if src && !failed}
    <img {src} alt="" class="size-full object-cover" loading="lazy" draggable="false" onerror={() => (failed = true)} />
  {:else}
    <div class="flex size-full items-center justify-center"
      style={liked
        ? "background: linear-gradient(135deg, oklch(0.62 0.2 15), oklch(0.5 0.22 350));"
        : `background: linear-gradient(135deg, oklch(0.55 0.12 ${hue}), oklch(0.38 0.1 ${(hue + 50) % 360}));`}>
      {#if liked}<Heart class="size-[38%] fill-white/90 text-white/90" />{:else}<Music class="size-[34%] text-white/70" />{/if}
    </div>
  {/if}
</div>
