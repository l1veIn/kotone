<script lang="ts">
  /** 琴音风格更新弹窗：发现新版、下载安装、重启确认共用同一张状态卡。 */
  import { tick } from "svelte";
  import { answerUpdateDialog, updateDialogStore } from "../updater";
  import stickerProud from "../../assets/brand/stickers/proud.webp";
  import stickerCheering from "../../assets/brand/stickers/cheering.webp";

  let dialogElement = $state<HTMLDivElement>();
  let focusedPhase = $state<string | null>(null);

  const progressPercent = $derived.by(() => {
    const dialog = $updateDialogStore;
    if (dialog?.phase !== "downloading" || !dialog.totalBytes) return null;
    return Math.min(100, Math.round((dialog.downloadedBytes / dialog.totalBytes) * 100));
  });

  $effect(() => {
    const phase = $updateDialogStore?.phase ?? null;
    if (phase === null) {
      focusedPhase = null;
      return;
    }
    if (phase === focusedPhase) return;
    focusedPhase = phase;
    void tick().then(() => {
      const primary = dialogElement?.querySelector<HTMLElement>("[data-update-primary]");
      (primary ?? dialogElement)?.focus();
    });
  });

  function handleKeydown(event: KeyboardEvent) {
    const phase = $updateDialogStore?.phase;
    if (event.key === "Escape" && (phase === "available" || phase === "ready")) {
      event.preventDefault();
      answerUpdateDialog("later");
      return;
    }
    if (event.key !== "Tab" || !dialogElement) return;
    const focusable = Array.from(
      dialogElement.querySelectorAll<HTMLElement>("button:not(:disabled), [tabindex='0']"),
    );
    if (focusable.length === 0) {
      event.preventDefault();
      dialogElement.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if $updateDialogStore}
  {@const dialog = $updateDialogStore}
  <div
    class="fixed inset-0 z-[90] flex items-center justify-center bg-kotone-deep/85 p-6 backdrop-blur-md"
    data-testid="update-dialog"
  >
    <div
      bind:this={dialogElement}
      class="kotone-panel relative w-full max-w-lg overflow-hidden p-6 shadow-glow-cyan-lg"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="update-dialog-title"
      aria-describedby="update-dialog-description"
    >
      <div
        class="pointer-events-none absolute -top-20 -right-16 h-48 w-48 rounded-full bg-kotone-cyan/12 blur-3xl"
      ></div>
      <div
        class="pointer-events-none absolute -bottom-24 -left-16 h-48 w-48 rounded-full bg-kotone-pink/10 blur-3xl"
      ></div>

      <div class="relative flex items-start gap-4">
        <div
          class="flex h-16 w-16 shrink-0 items-center justify-center rounded-2xl bg-kotone-cyan/10 ring-1 ring-kotone-cyan/25"
        >
          <img
            src={dialog.phase === "ready" ? stickerCheering : stickerProud}
            alt=""
            class="h-15 w-15 object-contain"
          />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h2 id="update-dialog-title" class="text-lg font-bold">
              {#if dialog.phase === "available"}
                发现琴音新版本
              {:else if dialog.phase === "downloading"}
                正在更新琴音
              {:else if dialog.phase === "ready"}
                更新已经准备好
              {:else}
                正在重新启动
              {/if}
            </h2>
            <span
              class="rounded-full bg-kotone-pink/15 px-2.5 py-1 text-[11px] font-bold text-kotone-pink ring-1 ring-kotone-pink/30"
            >
              v{dialog.version}
            </span>
          </div>
          <p id="update-dialog-description" class="mt-1.5 text-[13px] leading-relaxed text-white/60">
            {#if dialog.phase === "available"}
              新版本已经就绪，可以直接从当前版本更新到最新版。
            {:else if dialog.phase === "downloading"}
              正在下载并安装完整更新包，请保持琴音运行。
            {:else if dialog.phase === "ready"}
              安装已完成。重新启动琴音后，新版本就会生效。
            {:else}
              马上回来，请稍候片刻…
            {/if}
          </p>
        </div>
      </div>

      {#if dialog.phase === "available" || dialog.phase === "ready"}
        <div class="relative mt-5 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
          <div class="mb-2 flex items-center gap-2">
            <span class="h-1.5 w-1.5 rounded-full bg-kotone-cyan shadow-glow-cyan"></span>
            <p class="text-xs font-bold text-white/80">本次更新</p>
          </div>
          {#if dialog.notes}
            <div
              class="max-h-48 overflow-y-auto whitespace-pre-wrap break-words pr-2 text-[12px] leading-relaxed text-white/60"
              data-testid="update-notes"
            >{dialog.notes}</div>
          {:else}
            <p class="text-[12px] text-white/45">本次发布暂未提供更新说明。</p>
          {/if}
        </div>
      {:else}
        <div class="relative mt-6" aria-live="polite">
          <div class="mb-2 flex items-center justify-between text-[11px] font-semibold">
            <span class="text-white/55">
              {dialog.phase === "downloading" ? "下载安装中" : "正在重新启动"}
            </span>
            {#if dialog.phase === "downloading" && progressPercent !== null}
              <span class="text-kotone-cyan" data-testid="update-progress">{progressPercent}%</span>
            {/if}
          </div>
          <div class="h-2 overflow-hidden rounded-full bg-white/8 ring-1 ring-white/8">
            {#if dialog.phase === "downloading" && progressPercent !== null}
              <div
                class="h-full rounded-full bg-gradient-to-r from-kotone-cyan to-kotone-pink shadow-glow-cyan transition-[width] duration-300"
                style:width={`${progressPercent}%`}
              ></div>
            {:else}
              <div
                class="update-indeterminate h-full w-2/5 rounded-full bg-gradient-to-r from-kotone-cyan to-kotone-pink shadow-glow-cyan"
              ></div>
            {/if}
          </div>
        </div>
      {/if}

      {#if dialog.phase === "available"}
        <div class="relative mt-6 flex items-center justify-between gap-4">
          <p class="hidden text-[10px] text-white/35 sm:block">无需逐个版本安装</p>
          <div class="ml-auto flex items-center gap-3">
            <button
              class="rounded-lg bg-white/8 px-4 py-2 text-xs font-semibold text-white/70 ring-1 ring-white/15 transition hover:bg-white/15 hover:text-white"
              onclick={() => answerUpdateDialog("later")}
            >
              稍后更新
            </button>
            <button
              data-update-primary
              class="rounded-lg bg-kotone-pink px-5 py-2 text-xs font-bold text-white shadow-glow-pink transition hover:brightness-110 active:scale-95"
              onclick={() => answerUpdateDialog("primary")}
            >
              立即更新
            </button>
          </div>
        </div>
      {:else if dialog.phase === "ready"}
        <div class="relative mt-6 flex items-center justify-end gap-3">
          <button
            class="rounded-lg bg-white/8 px-4 py-2 text-xs font-semibold text-white/70 ring-1 ring-white/15 transition hover:bg-white/15 hover:text-white"
            onclick={() => answerUpdateDialog("later")}
          >
            稍后重启
          </button>
          <button
            data-update-primary
            class="rounded-lg bg-kotone-cyan px-5 py-2 text-xs font-bold text-kotone-deep shadow-glow-cyan transition hover:brightness-110 active:scale-95"
            onclick={() => answerUpdateDialog("primary")}
          >
            立即重启
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .update-indeterminate {
    animation: update-slide 1.15s ease-in-out infinite alternate;
  }

  @keyframes update-slide {
    from {
      transform: translateX(-20%);
    }
    to {
      transform: translateX(170%);
    }
  }
</style>
