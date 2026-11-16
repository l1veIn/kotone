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
        <h2 class="text-base font-bold">发送被系统拦截了</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
          Kotone 发出文字时，所有模拟按键都被系统拦截（0
          次落地）。这不是目标程序的问题——此刻打开记事本也发不进去。通常是有软件在系统层面拦截模拟输入。
        </p>
      </div>
    </div>

    <div class="mt-4 rounded-lg bg-muted/40 p-3 ring-1 ring-hairline">
      <p class="text-xs font-semibold text-foreground/90">按概率从高到低排查：</p>
      <ol class="mt-2 flex list-decimal flex-col gap-1.5 pl-4 text-[12px] leading-relaxed text-muted-foreground">
        <li>
          <span class="font-semibold text-foreground/90">安全软件的「主动防御 / 键盘保护」</span>（电脑管家、360、火绒等）——退出该软件，或在设置里把
          Kotone 加入信任列表
        </li>
        <li>
          <span class="font-semibold text-foreground/90">游戏反作弊</span>（如 Riot
          Vanguard）——从任务栏托盘完全退出后重试
        </li>
        <li>
          <span class="font-semibold text-foreground/90">远程控制软件</span>（ToDesk、向日葵等）的输入保护——退出后重试
        </li>
      </ol>
    </div>

    <p class="mt-3 text-[11px] leading-relaxed text-muted-foreground">
      快速验证：打开记事本，再试一次发送。每退出一个可疑软件就试一次，能发进记事本就说明找到它了。
    </p>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={onClose}
      >
        知道了
      </button>
      {#if onRetry}
        <button
          class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
          disabled={busy}
          onclick={() => void handleRetry()}
        >
          {busy ? "重试中…" : "重试发送"}
        </button>
      {/if}
    </div>
  </div>
</div>
