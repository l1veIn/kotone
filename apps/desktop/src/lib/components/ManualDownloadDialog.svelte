<script lang="ts">
  /*
   * 自动下载失败后的手动安装指引。
   * 告诉用户：去哪个官方页下文件、放到哪个目录、保持哪些文件名。
   */
  import {
    openExternal,
    openModelDestDir,
    type ModelInstallGuide,
  } from "../ipc";
  import { toast, toastInfo, errText } from "../stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import stickerThinking from "../../assets/brand/stickers/thinking.webp";

  let {
    guide,
    error = "",
    onClose,
    onRetry,
    onRecheck,
  }: {
    guide: ModelInstallGuide;
    error?: string;
    onClose: () => void;
    onRetry?: () => void;
    onRecheck?: () => void;
  } = $props();

  let copying = $state(false);

  function formatSize(bytes: number): string {
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  async function openPage() {
    try {
      await openExternal(guide.pageUrl);
    } catch (e) {
      toast(false, m.system_manual_download_open_page_failed({ error: errText(e) }));
    }
  }

  async function openFolder() {
    try {
      await openModelDestDir(guide.id);
    } catch (e) {
      toast(false, m.system_manual_download_open_folder_failed({ error: errText(e) }));
    }
  }

  async function copyPath() {
    if (copying) return;
    copying = true;
    try {
      await navigator.clipboard.writeText(guide.destDir);
      toastInfo(m.system_manual_download_path_copied());
    } catch (e) {
      toast(false, m.system_manual_download_copy_failed({ error: errText(e) }));
    } finally {
      copying = false;
    }
  }
</script>

<div
  class="fixed inset-0 z-[80] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm"
  data-testid="manual-download-dialog"
>
  <div class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl flex max-h-[90vh] w-full max-w-lg flex-col p-6">
    <div class="flex items-start gap-4">
      <img src={stickerThinking} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">{m.system_manual_download_title()}</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
          {m.system_manual_download_body({ name: guide.displayName })}
        </p>
      </div>
    </div>

    {#if error}
      <p class="mt-3 max-h-16 overflow-y-auto rounded-lg bg-brand-pink/10 px-3 py-2 text-[11px] leading-relaxed break-all text-brand-pink ring-1 ring-brand-pink/35">
        {error}
      </p>
    {/if}

    <ol class="mt-4 flex list-decimal flex-col gap-2 pl-4 text-[12px] leading-relaxed text-muted-foreground">
      <li>
        {m.system_manual_download_step_open_page()}
      </li>
      <li>{m.system_manual_download_step_keep_names_before()} <span class="text-foreground/90">Qwen3-0.6B/merges.txt</span> {m.system_manual_download_step_keep_names_after()}</li>
      <li>{m.system_manual_download_step_open_folder()}</li>
      <li>{m.system_manual_download_step_recheck()}</li>
    </ol>

    <div class="mt-3 rounded-lg bg-muted/40 p-3 ring-1 ring-hairline">
      <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
          <p class="text-[10px] font-semibold tracking-wide text-muted-foreground/80">{m.system_manual_download_dest_label()}</p>
          <p class="mt-0.5 break-all font-mono text-[11px] text-foreground/90">{guide.destDir}</p>
        </div>
        <button
          class="shrink-0 rounded-lg bg-muted/70 px-2.5 py-1 text-[11px] text-muted-foreground ring-1 ring-hairline transition hover:bg-muted"
          onclick={() => void copyPath()}
        >
          {copying ? m.system_manual_download_copying() : m.system_manual_download_copy_path_button()}
        </button>
      </div>
      <p class="mt-3 text-[10px] font-semibold tracking-wide text-muted-foreground/80">{m.system_manual_download_files_label()}</p>
      <ul class="mt-1 max-h-28 overflow-y-auto text-[11px] text-muted-foreground">
        {#each guide.files as file}
          <li class="truncate py-0.5">
            {file.name}
            <span class="text-muted-foreground/80"> · {formatSize(file.sizeBytes)}</span>
          </li>
        {/each}
      </ul>
    </div>

    <div class="mt-5 flex flex-wrap items-center justify-end gap-2">
      <button
        class="rounded-lg bg-muted/70 px-3 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted"
        onclick={onClose}
      >
        {m.system_common_close()}
      </button>
      <button
        class="rounded-lg bg-muted/70 px-3 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted"
        onclick={() => void openFolder()}
      >
        {m.system_manual_download_open_folder_button()}
      </button>
      <button
        class="rounded-lg bg-muted/70 px-3 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted"
        onclick={() => void openPage()}
      >
        {m.system_common_open_download_page()}
      </button>
      {#if onRecheck}
        <button
          class="rounded-lg bg-primary/20 px-3 py-2 text-xs font-semibold text-primary ring-1 ring-primary/40 transition hover:bg-primary/30"
          onclick={onRecheck}
        >
          {m.system_manual_download_recheck_button()}
        </button>
      {/if}
      {#if onRetry}
        <button
          class="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95"
          onclick={onRetry}
        >
          {m.system_manual_download_retry_button()}
        </button>
      {/if}
    </div>
  </div>
</div>
