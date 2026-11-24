<script lang="ts">
  // 登录网易云音乐（ADR 034 D5：第一次放歌先登录）。登录在网易云音乐**官方的登录页**里做：点按钮后后端开一个
  // “登录网易云音乐”窗口（music.163.com），你用网易云音乐 App 扫里面的二维码；后端从窗口读到登录状态就关窗。
  // （我们自己生成的二维码会被网易云判成“设备环境异常”，官方网页登录不会。）
  // 音乐页里是一张大卡片；聊天页输入框上面是窄的一条：她要放歌而你没登录（needed）时出现——她不自己弹窗，免得游戏里抢焦点。
  import { LoaderCircle, LogIn, RefreshCw, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import { musicLoginCancel, musicLoginStart, musicStore } from "./music";
  import { errText, toast } from "$lib/stores/ui";

  let { compact = false }: { compact?: boolean } = $props();

  const login = $derived($musicStore.login);
  const waiting = $derived(login.stage === "waiting" || login.stage === "scanned");
  let starting = $state(false);

  async function start() {
    starting = true;
    try { await musicLoginStart(); }
    catch (error) { toast(false, m.plugin_music_login_failed({ error: errText(error) })); }
    finally { starting = false; }
  }
  async function cancel() {
    try { await musicLoginCancel(); } catch { /* 关掉卡片不提示 */ }
  }

  /** 卡片上的那一行：在等窗口里登录 / 她要放歌 / 过期 / 出错；没在登录时（音乐页）不写。 */
  const line = $derived(
    waiting ? m.plugin_music_login_waiting()
      : login.stage === "needed" ? m.plugin_music_login_needed()
        : login.stage === "expired" ? m.plugin_music_login_expired()
          : login.stage === "failed" ? (login.message ? m.plugin_music_login_failed({ error: login.message }) : m.plugin_music_login_failed_unknown())
            : "",
  );
  const lineTone = $derived(login.stage === "expired" || login.stage === "failed" ? "text-warning" : waiting ? "text-primary" : "text-muted-foreground");
</script>

{#snippet actions(size: "sm" | "default")}
  {#if waiting}
    <Button {size} variant="outline" data-testid="music-login-reopen" disabled={starting} onclick={() => void start()}>
      {#if starting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<RefreshCw class="size-3.5" />{/if}{m.plugin_music_login_reopen()}
    </Button>
    <Button {size} variant="ghost" data-testid="music-login-cancel" onclick={() => void cancel()}>{m.plugin_music_login_cancel()}</Button>
  {:else}
    <Button {size} data-testid="music-login-start" disabled={starting} onclick={() => void start()}>
      {#if starting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<LogIn class="size-3.5" />{/if}{m.plugin_music_login_button()}
    </Button>
  {/if}
{/snippet}

{#if compact}
  <div class="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-2xl border border-primary/35 bg-composer px-3.5 py-2.5" role="status" data-testid="music-login-card" data-stage={login.stage}>
    <div class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-rose-500/15 text-rose-500"><LogIn class="size-4" /></div>
    <!-- 窄的一条只写一句（按钮上已经是“登录网易云音乐”，不再重复标题） -->
    <div class="min-w-0 flex-1">
      {#if line}<p class="m-0 text-[13px] leading-snug font-medium {lineTone === 'text-muted-foreground' ? 'text-foreground' : lineTone}" data-testid="music-login-stage">{line}</p>
      {:else}<p class="m-0 text-[13px] font-medium text-foreground">{m.plugin_music_login_title()}</p>{/if}
    </div>
    <div class="flex shrink-0 items-center gap-1.5">
      {@render actions("sm")}
      {#if !waiting}
        <button type="button" class="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          aria-label={m.plugin_music_login_dismiss()} title={m.plugin_music_login_dismiss()} data-testid="music-login-dismiss" onclick={() => void cancel()}><X class="size-3.5" /></button>
      {/if}
    </div>
  </div>
{:else}
  <section class="flex flex-col items-center gap-5 rounded-2xl border border-hairline bg-card/60 px-6 py-8 text-center sm:flex-row sm:text-left" data-testid="music-login-card" data-stage={login.stage}>
    <div class="flex size-28 shrink-0 items-center justify-center rounded-2xl"
      style="background: linear-gradient(135deg, oklch(0.62 0.2 15), oklch(0.5 0.22 350));">
      {#if waiting}<LoaderCircle class="size-10 animate-spin text-white/90" />{:else}<LogIn class="size-10 text-white/90" />{/if}
    </div>
    <div class="min-w-0 flex-1">
      <h2 class="m-0 text-lg font-semibold">{m.plugin_music_login_title()}</h2>
      <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">{m.plugin_music_login_desc()}</p>
      {#if line}<p class="mt-3 mb-0 text-sm font-medium {lineTone}" data-testid="music-login-stage" role="status">{line}</p>{/if}
      <div class="mt-4 flex flex-wrap items-center justify-center gap-2 sm:justify-start">{@render actions("default")}</div>
      <p class="mt-3 mb-0 text-xs text-muted-foreground">{m.plugin_music_login_privacy()}</p>
    </div>
  </section>
{/if}
