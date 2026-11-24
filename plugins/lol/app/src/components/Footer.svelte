<script lang="ts">
  // 数据出处、声明和风险提示（设计第一节 11；交接第十一节第 15 条）：全屏每页最下面一行小字；卡片里只有一句短的。
  // - 拳头的 “Legal Jibber Jabber” 政策要求写在显眼处；
  // - 风险提示跟着 App 走（Claude 这类宿主里没有 Kotone 的设置页，也要看得到）：文案就是插件两个数据开关的风险原文
  //   （`plugin.toml` 的 client / control 的 risk，和设置页里的同一句），点开才展开；只提示，不拦着。
  import { m } from "../i18n.svelte";

  let {
    data,
    card = false,
    extra = "",
    onLink,
  }: {
    /** 第一句：数据从哪来（这局、战绩：客户端；强度榜：腾讯 101……）。卡片里空 = “数据：客户端 · 素材归 Riot Games”。 */
    data: string;
    card?: boolean;
    /** 卡片里放在前面的一句（“还在查 1 人”）。 */
    extra?: string;
    onLink?: (url: string) => void;
  } = $props();

  const CDRAGON = "https://www.communitydragon.org/";
  const ISSUE_523 = "https://github.com/LeagueAkari/LeagueAkari/issues/523";
  function open(event: MouseEvent, url: string) {
    if (!onLink) return;
    event.preventDefault();
    onLink(url);
  }
</script>

{#snippet risk()}
  <details class="risk" data-testid="lol-app-risk">
    <summary>{m.plugin_lol_risk_title()}</summary>
    <p>{m.plugin_lol_source_client_risk()}</p>
    <p>{m.plugin_lol_source_control_risk()}</p>
    <p><a href={ISSUE_523} target="_blank" rel="noopener noreferrer" onclick={(e) => open(e, ISSUE_523)}>github.com/LeagueAkari/LeagueAkari/issues/523</a></p>
  </details>
{/snippet}

{#if card}
  <div class="card-foot">
    {[extra, data || m.plugin_lol_card_foot()].filter(Boolean).join(" · ")}
    {@render risk()}
  </div>
{:else}
  <footer class="foot" data-testid="lol-app-foot">
    {#if data}<span>{data}</span>{/if}
    <span>{m.plugin_lol_foot_images()} <a href={CDRAGON} target="_blank" rel="noopener noreferrer" onclick={(e) => open(e, CDRAGON)}>CommunityDragon</a></span>
    <span>{m.plugin_lol_foot_riot()}</span>
    {@render risk()}
  </footer>
{/if}

<style>
  .risk {
    flex-basis: 100%;
    color: var(--lol-text-3);
  }
  .card-foot .risk {
    display: inline;
  }
  .card-foot .risk[open] {
    display: block;
  }
  .risk summary {
    display: inline;
    cursor: pointer;
    list-style: none;
    color: var(--lol-warn);
  }
  .card-foot .risk summary::before {
    content: " · ";
    color: var(--lol-text-3);
  }
  .risk summary::-webkit-details-marker {
    display: none;
  }
  .risk p {
    margin: 4px 0 0;
    color: var(--lol-text-2);
  }
  .risk a {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--lol-line-2);
    text-underline-offset: 2px;
  }
</style>
