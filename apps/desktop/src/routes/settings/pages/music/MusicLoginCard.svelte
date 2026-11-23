<script lang="ts">
  // 扫码登录网易云音乐（ADR 034 D5：第一次放歌先扫码）。音乐页里是一张大卡片；聊天页输入框上面是窄的一条
  // （她要放歌而你没登录时出现）。二维码是后端生成的 SVG。
  import { LoaderCircle, QrCode, RefreshCw, Smartphone, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import { musicLoginCancel, musicLoginStart, musicStore } from "../../../../lib/music";
  import { errText, toast } from "../../../../lib/stores/ui";

  let { compact = false }: { compact?: boolean } = $props();

  const login = $derived($musicStore.login);
  let starting = $state(false);

  async function start() {
    starting = true;
    try { await musicLoginStart(); }
    catch (error) { toast(false, m.music_login_failed({ error: errText(error) })); }
    finally { starting = false; }
  }
  async function cancel() {
    try { await musicLoginCancel(); } catch { /* 关掉卡片不提示 */ }
  }

  const stageText = $derived(
    login.stage === "scanned" ? m.music_login_scanned()
      : login.stage === "expired" ? m.music_login_expired()
        : login.stage === "failed" ? (login.message ? m.music_login_failed({ error: login.message }) : m.music_login_failed_unknown())
          : m.music_login_waiting(),
  );
  const showQr = $derived(!!login.qrSvg && (login.stage === "waiting" || login.stage === "scanned"));
</script>

{#snippet qr(sizeClass: string)}
  <div class="relative shrink-0 overflow-hidden rounded-xl bg-white p-1.5 {sizeClass}" data-testid="music-login-qr">
    {#if showQr}
      <!-- 后端（kotone-music）生成的二维码 SVG，不含外部内容 -->
      <div class="size-full [&>svg]:size-full">{@html login.qrSvg}</div>
      {#if login.stage === "scanned"}
        <div class="absolute inset-0 flex items-center justify-center bg-white/85"><Smartphone class="size-1/3 text-neutral-800" /></div>
      {/if}
    {:else}
      <button type="button" class="flex size-full flex-col items-center justify-center gap-1 text-neutral-700" data-testid="music-login-refresh"
        disabled={starting} onclick={() => void start()} aria-label={m.music_login_refresh()}>
        {#if starting}<LoaderCircle class="size-6 animate-spin" />{:else}<RefreshCw class="size-6" /><span class="text-[11px]">{m.music_login_refresh()}</span>{/if}
      </button>
    {/if}
  </div>
{/snippet}

{#if compact}
  <div class="flex items-center gap-3 rounded-2xl border border-primary/35 bg-composer px-3 py-2.5" role="status" data-testid="music-login-card" data-stage={login.stage}>
    {@render qr("size-20")}
    <div class="min-w-0 flex-1">
      <p class="m-0 text-[13px] font-medium text-foreground">{m.music_login_title()}</p>
      <p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground" data-testid="music-login-stage">{stageText}</p>
    </div>
    <button type="button" class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
      aria-label={m.music_login_cancel()} title={m.music_login_cancel()} data-testid="music-login-cancel" onclick={() => void cancel()}><X class="size-3.5" /></button>
  </div>
{:else}
  <section class="flex flex-col items-center gap-5 rounded-2xl border border-hairline bg-card/60 px-6 py-8 text-center sm:flex-row sm:text-left" data-testid="music-login-card" data-stage={login.stage}>
    {#if login.stage === "idle" || (!login.qrSvg && login.stage !== "expired" && login.stage !== "failed")}
      <div class="flex size-40 shrink-0 items-center justify-center rounded-2xl bg-muted/70"><QrCode class="size-14 text-muted-foreground" /></div>
    {:else}
      {@render qr("size-40")}
    {/if}
    <div class="min-w-0 flex-1">
      <h2 class="m-0 text-lg font-semibold">{m.music_login_title()}</h2>
      <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">{m.music_login_desc()}</p>
      {#if login.stage === "idle"}
        <Button class="mt-4" data-testid="music-login-start" disabled={starting} onclick={() => void start()}>
          {#if starting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<QrCode class="size-3.5" />{/if}{m.music_login_button()}
        </Button>
      {:else}
        <p class="mt-4 mb-0 text-sm font-medium {login.stage === 'scanned' ? 'text-primary' : login.stage === 'expired' || login.stage === 'failed' ? 'text-warning' : 'text-foreground'}"
          data-testid="music-login-stage" role="status">{stageText}</p>
        <p class="mt-1.5 mb-0 text-xs text-muted-foreground">{m.music_login_privacy()}</p>
      {/if}
    </div>
  </section>
{/if}
