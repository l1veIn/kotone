<script lang="ts">
  /*
   * 关于页（单卡片版）：一张项目卡片——左侧贴纸，右侧版本号、GitHub
   * 地址与「角色详情」入口（→ CharacterPage 全屏档案）。
   * 完整角色内容（立绘 / 故事 / 贴纸 / 海报）都在角色详情页；
   * 导出诊断包在「高级」页。
   */
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { isTauri, openExternal } from "../../../lib/ipc";
  import { toast, errText } from "../../../lib/stores/ui";
  import { checkForUpdates, type UpdateCheckResult } from "../../../lib/updater";
  import stickerProud from "../../../assets/brand/stickers/proud.webp";

  /** 角色详情页入口回调：由 Settings 切换至 CharacterPage 全屏档案视图 */
  let { onOpenCharacter }: { onOpenCharacter: () => void } = $props();

  /** 静态兜底版本（与 package.json 同步）；桌面端启动后替换为真实版本 */
  let version = $state("0.1.5");
  let checkingUpdate = $state(false);
  let updateResult = $state<UpdateCheckResult | null>(null);

  onMount(async () => {
    if (!isTauri) return;
    version = await getVersion().catch(() => version);
  });

  const GITHUB_URL = "https://github.com/l1veIn/kotone";

  /** webview 内 target=_blank 不会调起系统浏览器，走后端 open_external */
  async function openGitHub(e: MouseEvent) {
    e.preventDefault();
    try {
      await openExternal(GITHUB_URL);
    } catch (err) {
      toast(false, `打开链接失败：${errText(err)}`);
    }
  }

  async function checkUpdates() {
    if (checkingUpdate) return;
    checkingUpdate = true;
    updateResult = null;
    try {
      updateResult = await checkForUpdates();
    } finally {
      checkingUpdate = false;
    }
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">关于</h1>

  <!-- 项目卡片：左贴纸 + 右侧信息 -->
  <section class="kotone-card mt-4 flex items-center gap-4 p-4">
    <img
      src={stickerProud}
      alt="琴音贴纸 · 得意"
      class="h-20 w-20 shrink-0 object-contain drop-shadow-[0_0_18px_rgba(0,229,255,0.28)]"
    />
    <div class="min-w-0 flex-1">
      <p class="flex flex-wrap items-center gap-2 text-sm font-bold">
        Kotone <span class="kotone-gradient-text">琴音</span>
        <span
          class="rounded-full bg-white/8 px-2 py-0.5 text-[10px] font-normal text-white/50 ring-1 ring-white/12"
          >v{version}</span
        >
      </p>
      <p class="mt-1 text-[11px] text-white/45">
        本地优先 · 隐私安全 · 反馈群：1092354484
      </p>
      <div class="mt-2.5 flex flex-wrap items-center gap-2">
        <a
          class="inline-flex items-center gap-1.5 rounded-lg bg-white/8 px-2.5 py-1.5 text-[11px] text-white/70 ring-1 ring-white/12 transition hover:bg-white/15 hover:text-kotone-cyan"
          href={GITHUB_URL}
          target="_blank"
          rel="noreferrer"
          onclick={(e) => void openGitHub(e)}
        >
          <svg viewBox="0 0 24 24" fill="currentColor" class="h-3.5 w-3.5" aria-hidden="true">
            <path
              d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.27-.01-1.17-.02-2.12-3.2.7-3.88-1.36-3.88-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.03 1.76 2.69 1.25 3.35.96.1-.75.4-1.25.72-1.54-2.55-.29-5.23-1.28-5.23-5.68 0-1.26.45-2.28 1.18-3.09-.12-.29-.51-1.46.11-3.05 0 0 .96-.31 3.15 1.18a10.9 10.9 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.59.23 2.76.11 3.05.74.81 1.18 1.83 1.18 3.09 0 4.41-2.69 5.38-5.25 5.66.41.35.77 1.05.77 2.12 0 1.53-.01 2.76-.01 3.14 0 .31.21.68.8.56A10.52 10.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z"
            />
          </svg>
          github.com/l1veIn/kotone
        </a>
        <button
          class="group inline-flex items-center gap-1.5 rounded-lg bg-kotone-cyan/12 px-2.5 py-1.5 text-[11px] font-semibold text-kotone-cyan ring-1 ring-kotone-cyan/35 transition hover:shadow-glow-cyan"
          onclick={onOpenCharacter}
        >
          CHARACTER FILE · 角色详情
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.2"
            class="h-3 w-3 transition group-hover:translate-x-0.5"
            aria-hidden="true"
          >
            <path d="M9 6l6 6-6 6" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          data-testid="check-updates"
          class="inline-flex items-center gap-1.5 rounded-lg bg-kotone-pink/12 px-2.5 py-1.5 text-[11px] font-semibold text-kotone-pink ring-1 ring-kotone-pink/35 transition hover:brightness-125 disabled:cursor-wait disabled:opacity-60"
          disabled={checkingUpdate}
          onclick={() => void checkUpdates()}
        >
          {#if checkingUpdate}
            <span class="spinner inline-block h-3 w-3 rounded-full"></span>
            正在检查…
          {:else}
            检查更新
          {/if}
        </button>
      </div>
      <div class="mt-2 min-h-4 text-[11px]" aria-live="polite">
        {#if checkingUpdate}
          <p class="text-white/45">正在连接更新服务器…</p>
        {:else if updateResult?.status === "up-to-date"}
          <p class="text-emerald-300">✓ 已是最新版本（v{version}）</p>
        {:else if updateResult?.status === "available"}
          <p class="text-kotone-pink">发现新版本 v{updateResult.version}，尚未安装</p>
        {:else if updateResult?.status === "downloaded"}
          <p class="text-kotone-cyan">v{updateResult.version} 已下载，重启后生效</p>
        {:else if updateResult?.status === "error"}
          <p class="break-all text-red-300">检查更新失败：{updateResult.message}</p>
        {:else}
          <p class="text-white/35">支持启动时自动检查，也可以随时手动检查</p>
        {/if}
      </div>
    </div>
  </section>

  <p class="mt-4 text-center text-[10px] text-white/30">
    © 2026 Kotone 项目 · 第三方组件声明见 THIRD_PARTY_NOTICES
  </p>
</div>
