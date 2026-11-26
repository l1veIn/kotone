<script lang="ts">
  // 出处（设计第一节 11，ADR 043 D8）：
  // - 卡片最底下只有来源标签（“客户端”“腾讯 101”……）和一句要紧的话（“还在查 1 人”）；风险说明不放在卡片下面
  //   （玩家：“卡片下面的风险提示，去掉”），收进插件的“风险和许可”弹窗；
  // - 全屏每页最下面一行小字：这一页的来源标签（有的块在自己的标题旁边已经有标签了，就不重复）、图片出处、
  //   拳头 “Legal Jibber Jabber” 政策要求写在显眼处的那句（只在全屏页上写一行；卡片上不写，弹窗里有）。
  import { m } from "../i18n.svelte";
  import type { SourceId } from "../types";
  import SourceTag from "./SourceTag.svelte";

  let {
    sources = [],
    card = false,
    extra = "",
    onLink,
  }: {
    /** 这一页 / 这张卡片的数据从哪来（只写出处，不能换的那种）。 */
    sources?: { id: SourceId; date?: string; stale?: boolean }[];
    card?: boolean;
    /** 卡片里放在前面的一句（“还在查 1 人”）。 */
    extra?: string;
    onLink?: (url: string) => void;
  } = $props();

  const CDRAGON = "https://www.communitydragon.org/";
  function open(event: MouseEvent, url: string) {
    if (!onLink) return;
    event.preventDefault();
    onLink(url);
  }
</script>

{#if card}
  <div class="card-foot" data-testid="lol-card-foot">
    {#if extra}<span class="extra">{extra}</span>{/if}
    {#each sources as s (s.id)}<SourceTag id={s.id} date={s.date} stale={s.stale} {onLink} />{/each}
  </div>
{:else}
  <footer class="foot" data-testid="lol-app-foot">
    {#each sources as s (s.id)}<SourceTag id={s.id} date={s.date} stale={s.stale} {onLink} />{/each}
    <span>{m.plugin_lol_foot_images()} <a href={CDRAGON} target="_blank" rel="noopener noreferrer" onclick={(e) => open(e, CDRAGON)}>CommunityDragon</a></span>
    <span>{m.plugin_lol_foot_riot()}</span>
  </footer>
{/if}

<style>
  .card-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }
  .extra {
    color: var(--lol-warn);
  }
  .foot {
    align-items: center;
  }
</style>
