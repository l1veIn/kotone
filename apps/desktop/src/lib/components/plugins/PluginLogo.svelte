<script lang="ts">
  // 插件的 logo（ADR 039 D8，docs/design/lol-app.md 第五节）：一块自带底色的圆角方块，亮暗主题下都一样。
  // 有 plugins/<id>/logo.svg 就用它；没有就是首字块（名字第一个字 + 按 id 定的底色）。
  // 尺寸照设计：插件页卡片 40、App 顶上一条 20、自动卡片那一行 16、消息提示的角标 18。
  import { fallbackLogo, pluginLogo } from "$lib/pluginLogos";

  let { id, name, size = 40, off = false, class: className = "", testid }: {
    id: string;
    /** 插件的显示名（首字块用它的第一个字）。 */
    name: string;
    /** 边长，px。 */
    size?: number;
    /** 插件关着：变灰（卡片本身已经变淡，这里不再叠透明度）。 */
    off?: boolean;
    class?: string;
    testid?: string;
  } = $props();

  const src = $derived(pluginLogo(id));
  const fallback = $derived(fallbackLogo(id, name));
</script>

{#if src}
  <!-- 旁边总有名字，logo 只是装饰：alt 留空 -->
  <img {src} alt="" width={size} height={size} draggable="false" data-testid={testid} data-logo="file"
    class="shrink-0 select-none {off ? 'grayscale' : ''} {className}" style:width="{size}px" style:height="{size}px" />
{:else}
  <span aria-hidden="true" data-testid={testid} data-logo="fallback"
    class="grid shrink-0 place-items-center font-semibold leading-none text-white select-none {off ? 'grayscale' : ''} {className}"
    style:width="{size}px" style:height="{size}px" style:border-radius="25%" style:background={fallback.color}
    style:font-size="{Math.round(size * 0.46)}px" style:box-shadow="inset 0 0 0 1px rgb(255 255 255 / .14)">{fallback.text}</span>
{/if}
