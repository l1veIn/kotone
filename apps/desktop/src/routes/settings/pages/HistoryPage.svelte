<script lang="ts">
  /*
   * 历史记录页（新 IPC：get_history / clear_history）：
   * 顶部 history.mode 三态 + 清空（二次确认）；
   * 列表 = 时间 / 识别文本 / 时长 / 状态；技术字段收纳到高级页。
   */
  import { onDestroy, onMount } from "svelte";
  import {
    updateSettings,
    getHistory,
    clearHistory,
    readHistoryAudio,
    type HistoryRecord,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import stickerSleepy from "../../../assets/brand/stickers/sleepy.png";

  let records = $state<HistoryRecord[] | null>(null);
  /** 清空二次确认：点一次进入确认态，3s 内再点执行 */
  let confirmingClear = $state(false);
  /** 正在播放的记录 id（sessionId+ts）；同时只允许一条在播 */
  let playingId = $state<string | null>(null);
  /** 正在加载音频的记录 id（防止加载期间重复点击） */
  let loadingId = $state<string | null>(null);
  let audioEl: HTMLAudioElement | null = null;
  let audioUrl: string | null = null;

  onMount(async () => {
    await refresh();
  });

  onDestroy(() => {
    stopPlayback();
  });

  function recordId(r: HistoryRecord): string {
    return r.sessionId + r.ts;
  }

  /** 停止当前播放并释放 objectURL */
  function stopPlayback() {
    audioEl?.pause();
    audioEl = null;
    if (audioUrl) {
      URL.revokeObjectURL(audioUrl);
      audioUrl = null;
    }
    playingId = null;
    loadingId = null;
  }

  /** 播放 / 暂停切换：切播先停上一条；播完（ended）自动停止 */
  async function togglePlay(r: HistoryRecord) {
    if (!r.audioFile) return;
    const id = recordId(r);
    if (playingId === id || loadingId === id) {
      stopPlayback();
      return;
    }
    stopPlayback();
    loadingId = id;
    try {
      const bytes = await readHistoryAudio(r.audioFile);
      if (loadingId !== id) return; // 加载期间已切播/停止
      const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: "audio/wav" }));
      const el = new Audio(url);
      el.onended = () => {
        if (playingId === id) stopPlayback();
      };
      audioEl = el;
      audioUrl = url;
      playingId = id;
      await el.play();
    } catch (e) {
      if (loadingId === id) {
        toast(false, `播放失败：${errText(e)}`);
        stopPlayback();
      }
    } finally {
      if (loadingId === id) loadingId = null;
    }
  }

  async function refresh() {
    try {
      records = await getHistory();
    } catch (e) {
      toast(false, `读取历史失败：${errText(e)}`);
      records = [];
    }
  }

  async function onModeChange(e: Event) {
    const mode = (e.target as HTMLSelectElement).value;
    try {
      settingsStore.set(await updateSettings({ history: { mode } }));
      toast(true, "历史记录模式已保存");
      if (mode === "off") records = [];
      else await refresh();
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function onClear() {
    if (!confirmingClear) {
      confirmingClear = true;
      setTimeout(() => (confirmingClear = false), 3000);
      return;
    }
    confirmingClear = false;
    stopPlayback(); // 音频文件随记录一起删除，先停掉在播条目
    try {
      await clearHistory();
      records = [];
      toast(true, "已清空全部识别历史");
    } catch (e) {
      toast(false, `清空失败：${errText(e)}`);
    }
  }

  const outcomeMeta: Record<string, { text: string; cls: string }> = {
    sent: { text: "已发送", cls: "bg-kotone-cyan/15 text-kotone-cyan" },
    cancelled: { text: "已取消", cls: "bg-white/10 text-white/55" },
    error: { text: "失败", cls: "bg-kotone-pink/15 text-kotone-pink" },
  };

  function fmtTime(ts: string): string {
    // "2026-07-25T10:14:43Z" → "07-25 10:14"
    const m = ts.match(/^\d{4}-(\d{2}-\d{2})T(\d{2}:\d{2})/);
    return m ? `${m[1]} ${m[2]}` : ts;
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">历史记录</h1>
  <p class="mt-0.5 text-[11px] text-white/45">每一次开口，都有迹可循</p>

  <!-- 配置条 -->
  <section class="kotone-panel mt-4 flex items-center gap-4 p-4">
    <div class="flex items-center gap-2">
      <label class="text-xs text-white/60" for="history-mode">记录模式</label>
      <select
        id="history-mode"
        class="rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
        value={$settingsStore?.history.mode ?? "capped"}
        onchange={(e) => void onModeChange(e)}
      >
        <option value="capped">保留最近 {$settingsStore?.history.maxRecords ?? 1000} 条</option>
        <option value="keep-all">全部保留</option>
        <option value="off">不记录</option>
      </select>
    </div>
    <div class="flex-1"></div>
    <button
      class="shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 {confirmingClear
        ? 'bg-kotone-pink text-white ring-kotone-pink shadow-glow-pink'
        : 'bg-white/8 text-white/70 ring-white/15 hover:bg-white/15'}"
      onclick={() => void onClear()}
    >
      {confirmingClear ? "再点一次确认清空" : "清空历史"}
    </button>
  </section>

  <!-- 记录列表 -->
  {#if records === null}
    <p class="mt-8 text-center text-sm text-white/50">读取中…</p>
  {:else if records.length === 0}
    <div class="mt-8 flex flex-col items-center gap-3 py-6">
      <img src={stickerSleepy} alt="空空如也" class="h-28 w-28 object-contain opacity-90" />
      <p class="text-sm font-medium text-white/70">还没有语音记录</p>
      <p class="text-[12px] text-white/40">你的语音输入记录将显示在这里</p>
    </div>
  {:else}
    <div class="mt-4 flex flex-col gap-2">
      {#each records as r (r.sessionId + r.ts)}
        {@const meta = outcomeMeta[r.outcome] ?? outcomeMeta.cancelled}
        {@const id = recordId(r)}
        <div class="kotone-card flex items-center gap-3 px-4 py-3">
          <span class="shrink-0 text-[11px] text-white/40 tabular-nums">{fmtTime(r.ts)}</span>
          <div class="min-w-0 flex-1">
            <p class="truncate text-[13px] {r.finalText ? 'text-white/90' : 'text-white/35'}">
              {r.finalText || (r.outcome === "cancelled" ? "（未说完成取消）" : "（无文本）")}
            </p>
            <p class="mt-0.5 text-[10px] text-white/35">
              语音时长 {(r.audioMs / 1000).toFixed(1)} 秒
              {#if r.error}· <span class="text-kotone-pink/80">{r.error}</span>{/if}
            </p>
          </div>
          {#if r.audioFile}
            <!-- 播放/暂停：播放中按钮旁显示简易声波动画 -->
            <div class="flex shrink-0 items-center gap-1.5">
              {#if playingId === id}
                <span class="flex h-3.5 items-end gap-[2px]" aria-hidden="true">
                  {#each [0, 1, 2, 3] as i}
                    <span class="wave-bar" style:animation-delay="{i * 0.12}s"></span>
                  {/each}
                </span>
              {/if}
              <button
                class="flex h-6 w-6 items-center justify-center rounded-full bg-kotone-cyan/15 text-[10px] text-kotone-cyan ring-1 ring-kotone-cyan/40 transition hover:bg-kotone-cyan/25 active:scale-95 disabled:opacity-50"
                title={playingId === id ? "停止播放" : "播放录音"}
                aria-label={playingId === id ? "停止播放" : "播放录音"}
                disabled={loadingId === id}
                onclick={() => void togglePlay(r)}
              >
                {loadingId === id ? "…" : playingId === id ? "⏸" : "▶"}
              </button>
            </div>
          {/if}
          <span class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold {meta.cls}">
            {meta.text}
          </span>
        </div>
      {/each}
    </div>
    <p class="mt-3 text-[11px] text-white/35">共 {records.length} 条（新→旧）</p>
  {/if}
</div>

<style>
  /* 播放中的简易声波动画：四根竖条高低跳动（纯 CSS，暂停/停止即移除节点） */
  .wave-bar {
    width: 2px;
    height: 30%;
    border-radius: 9999px;
    background: var(--color-kotone-cyan, #00e5ff);
    animation: wave-bounce 0.9s ease-in-out infinite;
  }
  @keyframes wave-bounce {
    0%,
    100% {
      height: 30%;
    }
    50% {
      height: 100%;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wave-bar {
      animation: none;
      height: 60%;
    }
  }
</style>
