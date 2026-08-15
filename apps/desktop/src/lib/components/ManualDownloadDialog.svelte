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
      toast(false, `打开下载页失败：${errText(e)}`);
    }
  }

  async function openFolder() {
    try {
      await openModelDestDir(guide.id);
    } catch (e) {
      toast(false, `打开目标文件夹失败：${errText(e)}`);
    }
  }

  async function copyPath() {
    if (copying) return;
    copying = true;
    try {
      await navigator.clipboard.writeText(guide.destDir);
      toastInfo("已复制目标目录路径");
    } catch (e) {
      toast(false, `复制失败：${errText(e)}`);
    } finally {
      copying = false;
    }
  }
</script>

<div
  class="fixed inset-0 z-[80] flex items-center justify-center bg-kotone-deep/80 p-6 backdrop-blur-sm"
  data-testid="manual-download-dialog"
>
  <div class="kotone-panel flex max-h-[90vh] w-full max-w-lg flex-col p-6 shadow-glow-cyan-lg">
    <div class="flex items-start gap-4">
      <img src={stickerThinking} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">自动下载失败，改手动安装</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-white/65">
          「{guide.displayName}」没能从镜像或官方源下完。用浏览器自己下载，再放进指定目录即可。
        </p>
      </div>
    </div>

    {#if error}
      <p class="mt-3 max-h-16 overflow-y-auto rounded-lg bg-kotone-pink/10 px-3 py-2 text-[11px] leading-relaxed break-all text-kotone-pink ring-1 ring-kotone-pink/35">
        {error}
      </p>
    {/if}

    <ol class="mt-4 flex list-decimal flex-col gap-2 pl-4 text-[12px] leading-relaxed text-white/70">
      <li>
        点「打开下载页」，从 Hugging Face / GitHub 下载下面这些文件（国内打不开就换网络或代理）。
      </li>
      <li>文件名和子目录不要改，例如 <span class="text-white/90">Qwen3-0.6B/merges.txt</span> 要保持这个相对路径。</li>
      <li>点「打开目标文件夹」，把文件放进去。</li>
      <li>回到这里点「我已放好」，Kotone 会重新检测。</li>
    </ol>

    <div class="mt-3 rounded-lg bg-white/5 p-3 ring-1 ring-white/10">
      <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
          <p class="text-[10px] font-semibold tracking-wide text-white/40">放到这个目录</p>
          <p class="mt-0.5 break-all font-mono text-[11px] text-white/80">{guide.destDir}</p>
        </div>
        <button
          class="shrink-0 rounded-lg bg-white/10 px-2.5 py-1 text-[11px] text-white/70 ring-1 ring-white/15 transition hover:bg-white/20"
          onclick={() => void copyPath()}
        >
          {copying ? "复制中…" : "复制路径"}
        </button>
      </div>
      <p class="mt-3 text-[10px] font-semibold tracking-wide text-white/40">需要这些文件</p>
      <ul class="mt-1 max-h-28 overflow-y-auto text-[11px] text-white/65">
        {#each guide.files as file}
          <li class="truncate py-0.5">
            {file.name}
            <span class="text-white/35"> · {formatSize(file.sizeBytes)}</span>
          </li>
        {/each}
      </ul>
    </div>

    <div class="mt-5 flex flex-wrap items-center justify-end gap-2">
      <button
        class="rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/75 ring-1 ring-white/15 transition hover:bg-white/20"
        onclick={onClose}
      >
        关闭
      </button>
      <button
        class="rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/75 ring-1 ring-white/15 transition hover:bg-white/20"
        onclick={() => void openFolder()}
      >
        打开目标文件夹
      </button>
      <button
        class="rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/75 ring-1 ring-white/15 transition hover:bg-white/20"
        onclick={() => void openPage()}
      >
        打开下载页
      </button>
      {#if onRecheck}
        <button
          class="rounded-lg bg-kotone-cyan/20 px-3 py-2 text-xs font-semibold text-kotone-cyan ring-1 ring-kotone-cyan/40 transition hover:bg-kotone-cyan/30"
          onclick={onRecheck}
        >
          我已放好
        </button>
      {/if}
      {#if onRetry}
        <button
          class="rounded-lg bg-kotone-cyan px-3 py-2 text-xs font-semibold text-kotone-deep transition hover:brightness-110 hover:shadow-glow-cyan active:scale-95"
          onclick={onRetry}
        >
          再试自动下载
        </button>
      {/if}
    </div>
  </div>
</div>
