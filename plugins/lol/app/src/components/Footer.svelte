<script lang="ts">
  // 数据出处和声明（设计第一节 11）：全屏每页最下面一行小字；卡片里只有一句短的（点开全屏看完整声明）。
  // 拳头的 “Legal Jibber Jabber” 政策要求写在显眼处。
  import { m } from "../i18n.svelte";

  let {
    data,
    card = false,
    extra = "",
    onLink,
  }: {
    /** 第一句：数据从哪来（这局、战绩：客户端；强度榜：腾讯 101……）。 */
    data: string;
    card?: boolean;
    /** 卡片里放在前面的一句（“还在查 1 人”）。 */
    extra?: string;
    onLink?: (url: string) => void;
  } = $props();

  const CDRAGON = "https://www.communitydragon.org/";
  function open(event: MouseEvent) {
    if (!onLink) return;
    event.preventDefault();
    onLink(CDRAGON);
  }
</script>

{#if card}
  <div class="card-foot">{[extra, m.plugin_lol_card_foot()].filter(Boolean).join(" · ")}</div>
{:else}
  <footer class="foot" data-testid="lol-app-foot">
    <span>{data}</span>
    <span>{m.plugin_lol_foot_images()} <a href={CDRAGON} target="_blank" rel="noopener noreferrer" onclick={open}>CommunityDragon</a></span>
    <span>{m.plugin_lol_foot_riot()}</span>
  </footer>
{/if}
