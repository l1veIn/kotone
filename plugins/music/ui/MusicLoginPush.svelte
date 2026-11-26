<script lang="ts">
  // 推送“登录过期”（ADR 043 D11）：网易云音乐的登录过期了（以前登录过、网易云说要重新登录），聊天里一张卡片，上面是
  // “登录网易云音乐”按钮——和音乐页、聊天页那个按钮是同一个（打开网易云官方的登录页，用网易云音乐 App 扫码）。
  // 在等扫码时照音乐的状态写一句、按钮换成“重新打开窗口”。登好了后端把这张卡片原地换成“已登录”；后端那次没推出去时，
  // 看现在的状态有账号也算登好了。聊天里的卡片和点开以后右侧栏里是同一个；按钮上的点击不往外传（卡片本身点了是打开右侧栏）。
  import { Check, LoaderCircle, LogIn, RefreshCw } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import type { PluginViewProps } from "$lib/pluginUi";
  import { errText } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import { musicStore, type LoginCardData } from "./music";

  let { data, bridge }: PluginViewProps = $props();
  const card = $derived(data && typeof data === "object" && "stage" in data ? (data as LoginCardData) : null);
  const account = $derived($musicStore.account);
  const done = $derived(card?.stage === "done" || !!account);
  const nickname = $derived(card?.stage === "done" ? (card.nickname ?? "") : (account?.nickname ?? ""));
  const waiting = $derived($musicStore.login.stage === "waiting" || $musicStore.login.stage === "scanned");

  let busy = $state(false);
  let failed = $state("");

  async function start(event: Event) {
    event.stopPropagation();
    if (busy) return;
    busy = true;
    failed = "";
    try {
      await bridge.call("music.login_start");
    } catch (error) {
      failed = errText(error);
    } finally {
      busy = false;
    }
  }
</script>

<div class="min-w-0" data-testid="music-login-push" data-state={done ? "done" : waiting ? "waiting" : "expired"}>
  {#if !card}
    <p class="m-0 text-xs text-muted-foreground">{m.plugin_music_card_empty()}</p>
  {:else if done}
    <p class="m-0 flex items-center gap-1.5 text-[13px] text-success" data-testid="music-login-push-done">
      <Check class="size-3.5 shrink-0" />{nickname ? m.plugin_music_card_done_named({ nickname }) : m.plugin_music_card_done()}
    </p>
  {:else}
    <p class="m-0 break-words text-[13px] text-foreground" data-testid="music-login-push-expired">{m.plugin_music_card_expired()}</p>
    {#if waiting}
      <p class="m-0 mt-1 break-words text-xs text-primary" data-testid="music-login-push-waiting">{m.plugin_music_login_waiting()}</p>
    {/if}
    <div class="mt-1.5 flex flex-wrap items-center gap-2">
      <Button size="xs" variant={waiting ? "outline" : "default"} disabled={busy} data-testid="music-login-push-start"
        onclick={start} onkeydown={(e: KeyboardEvent) => e.stopPropagation()}>
        {#if busy}<LoaderCircle class="size-3.5 animate-spin" />{:else if waiting}<RefreshCw class="size-3.5" />{:else}<LogIn class="size-3.5" />{/if}
        {waiting ? m.plugin_music_login_reopen() : m.plugin_music_login_button()}
      </Button>
      {#if failed}
        <span class="min-w-0 break-words text-[11px] text-warning" data-testid="music-login-push-failed">{m.plugin_music_login_failed({ error: failed })}</span>
      {/if}
    </div>
  {/if}
</div>
