<script lang="ts">
  /*
   * 模拟输入被系统拦截的排查引导弹窗（0.1.6）。
   *
   * 触发条件：注入时 SendInput 0/N 全部未落地（InjectError.inputBlocked），
   * 即系统层面有软件拦截了全部合成输入——此时连记事本都发不进去，
   * 与目标程序/Profile/权限无关，继续重试不会好转。
   *
   * 只回答一个问题：「谁在拦，怎么排查」。给出按概率排序的排查清单，
   * 并提供一个快速验证路径（记事本）。可选 onRetry（如引导页的注入测试）。
   */
  import stickerAmazed from "../../assets/brand/stickers/amazed.webp";
  import { m } from "$lib/paraglide/messages.js";

  let {
    onClose,
    onRetry,
  }: { onClose: () => void; onRetry?: () => void } = $props();

  let busy = $state(false);

  async function handleRetry() {
    if (!onRetry || busy) return;
    busy = true;
    try {
      onRetry();
    } finally {
      busy = false;
    }
  }
</script>

<div
  class="absolute inset-0 z-[70] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm"
  data-testid="injection-blocked-dialog"
>
  <div class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl w-full max-w-md p-6">
    <div class="flex items-start gap-4">
      <img src={stickerAmazed} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">{m.game_injection_blocked_title()}</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
          {m.game_injection_blocked_body()}
        </p>
      </div>
    </div>

    <div class="mt-4 rounded-lg bg-muted/40 p-3 ring-1 ring-hairline">
      <p class="text-xs font-semibold text-foreground/90">{m.game_injection_blocked_checklist_title()}</p>
      <ol class="mt-2 flex list-decimal flex-col gap-1.5 pl-4 text-[12px] leading-relaxed text-muted-foreground">
        <li>
          <span class="font-semibold text-foreground/90">{m.game_injection_blocked_security_software()}</span>{m.game_injection_blocked_security_software_hint()}
        </li>
        <li>
          <span class="font-semibold text-foreground/90">{m.game_injection_blocked_anticheat()}</span>{m.game_injection_blocked_anticheat_hint()}
        </li>
        <li>
          <span class="font-semibold text-foreground/90">{m.game_injection_blocked_remote_control()}</span>{m.game_injection_blocked_remote_control_hint()}
        </li>
      </ol>
    </div>

    <p class="mt-3 text-[11px] leading-relaxed text-muted-foreground">
      {m.game_injection_blocked_quick_check()}
    </p>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={onClose}
      >
        {m.game_injection_blocked_close()}
      </button>
      {#if onRetry}
        <button
          class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
          disabled={busy}
          onclick={() => void handleRetry()}
        >
          {busy ? m.game_injection_blocked_retrying() : m.game_injection_blocked_retry()}
        </button>
      {/if}
    </div>
  </div>
</div>
