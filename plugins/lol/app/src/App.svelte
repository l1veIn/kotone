<script lang="ts">
  // 英雄联盟 App 的外壳：和宿主握手，按 <meta name="kotone:page"> 选页面（几个页面共用一个打包）。
  import { onMount } from "svelte";
  import { connect, currentPage, type Session } from "./host";
  import { t } from "./i18n.svelte";
  import Tier from "./pages/Tier.svelte";

  const page = currentPage();
  let session = $state<Session | null>(null);
  let error = $state("");

  onMount(() => {
    connect()
      .then((s) => (session = s))
      .catch((e: unknown) => (error = e instanceof Error ? e.message : String(e)));
  });
</script>

{#if error}
  <p class="failed" role="alert">{t("plugin_lol_failed", { error })}</p>
{:else if session && page === "tier"}
  <Tier {session} />
{:else if session}
  <p class="failed" role="alert">{t("plugin_lol_failed", { error: page })}</p>
{/if}

<style>
  .failed {
    margin: 12px;
    font-size: 12px;
    color: var(--hot);
  }
</style>
