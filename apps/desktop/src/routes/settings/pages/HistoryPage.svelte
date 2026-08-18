<script lang="ts">
  /*
   * 历史记录页（新 IPC：get_history / clear_history）：
   * 顶部 history.mode 三态 + 独立音频保留开关 + 清空（二次确认）；
   * 列表 = 时间 / 识别文本 / 时长 / 状态；技术字段收纳到高级页。
   * 音频回放手写 Web Audio 管线（decodeAudioData + AudioBufferSourceNode），
   * 不走 WebView2 的 <audio> 元素——此前「no supported source」的根源；
   * 波形用 canvas 手绘（峰值柱 + 进度高亮），点击可 seek。
   */
  import { onDestroy, onMount, tick } from "svelte";
  import Select from "../../../lib/components/Select.svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    updateSettings,
    getHistory,
    clearHistory,
    deleteHistoryRecord,
    readHistoryAudio,
    isTauri,
    type HistoryRecord,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import stickerSleepy from "../../../assets/brand/stickers/sleepy.webp";

  let records = $state<HistoryRecord[] | null>(null);
  /** 清空二次确认：点一次进入确认态，3s 内再点执行 */
  let confirmingClear = $state(false);
  /** 正在播放的记录 id（sessionId+ts）；同时只允许一条在播 */
  let playingId = $state<string | null>(null);
  /** 正在加载音频的记录 id（防止加载期间重复点击） */
  let loadingId = $state<string | null>(null);
  /** 正在删除的记录 id（防止重复点击） */
  let deletingId = $state<string | null>(null);
  let detail = $state<HistoryRecord | null>(null);
  /** 共享 AudioContext（懒创建；纯 Web Audio 播放，绕开 WebView2 <audio> 管线） */
  let ac: AudioContext | null = null;
  let src: AudioBufferSourceNode | null = null;
  /** 当前解码好的音频（seek 时复用，不重复解码） */
  let buf: AudioBuffer | null = null;
  /** 当前段起点：ac.currentTime 基准 + buffer 内偏移 */
  let startedAt = 0;
  let startOffset = 0;
  /** 播放进度重绘句柄（rAF） */
  let raf = 0;
  /** 行内波形画布（仅播放中的行渲染） */
  let waveCanvas: HTMLCanvasElement | undefined = $state();

  /** 运行中新增记录的即时刷新订阅（Rust 落账后发 kotone://history-updated） */
  let unlistenHistory: (() => void) | null = null;
  /** 组件已销毁标志（listen await 完成前被销毁时撤销订阅，防泄漏） */
  let disposed = false;
  /** 刷新序号：并发 refresh 只保留最后一次结果（旧快照不覆盖新快照） */
  let refreshSeq = 0;

  onMount(async () => {
    await refresh();
    if (!isTauri) return;
    // 会话终态落账后即时刷新：历史页停留期间新记录自动出现，无需切页再切回
    unlistenHistory = await listen("kotone://history-updated", () => void refresh());
    if (disposed) {
      // 订阅建立前组件已被销毁：立即撤销，避免泄漏
      unlistenHistory?.();
      unlistenHistory = null;
    }
  });

  onDestroy(() => {
    disposed = true;
    unlistenHistory?.();
    stopPlayback();
    void ac?.close();
    ac = null;
  });

  function recordId(r: HistoryRecord): string {
    return r.sessionId + r.ts;
  }

  /** 停止当前播放：停 rAF、停 source、清解码缓存 */
  function stopPlayback() {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    if (src) {
      src.onended = null;
      try {
        src.stop();
      } catch {
        /* 未 start 或已停止 */
      }
      src.disconnect();
      src = null;
    }
    buf = null;
    playingId = null;
    loadingId = null;
  }

  /** 播放 / 停止切换：切播先停上一条；播完（ended）自动停止。
   *  纯 Web Audio 管线：decodeAudioData 解码 + AudioBufferSourceNode 输出，
   *  不经过 <audio> 元素（WebView2 对本应用 16kHz WAV blob 会报
   *  「no supported source」，wavesurfer v7 播放层同样走 media element）。 */
  async function togglePlay(r: HistoryRecord) {
    if (!r.audioFile) return;
    const id = recordId(r);
    if (playingId === id) {
      stopPlayback();
      return;
    }
    if (loadingId === id) return;
    stopPlayback();
    loadingId = id;
    try {
      const bytes = await readHistoryAudio(r.audioFile);
      if (loadingId !== id) return; // 加载期间已切播/停止
      if (bytes.byteLength === 0) throw new Error("音频文件为空或已被清理");
      ac ??= new AudioContext();
      await ac.resume(); // 点击手势内调用，自动播放策略安全
      // 复制一份再解码（decodeAudioData 可能 detach 传入的 buffer）
      const decoded = await ac.decodeAudioData(
        bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ) as ArrayBuffer,
      );
      if (loadingId !== id) return;
      buf = decoded;
      playingId = id;
      await tick(); // 等行内画布渲染出来
      startFrom(0);
    } catch (e) {
      if (loadingId === id || playingId === id) {
        toast(false, `播放失败：${errText(e)}`);
        stopPlayback();
      }
    } finally {
      if (loadingId === id) loadingId = null;
    }
  }

  /** 从 buffer 偏移处起播（seek 复用）；自然播完时自动停止 */
  function startFrom(offset: number) {
    if (!ac || !buf) return;
    if (src) {
      src.onended = null;
      try {
        src.stop();
      } catch {
        /* 未 start 或已停止 */
      }
      src.disconnect();
    }
    const node = ac.createBufferSource();
    node.buffer = buf;
    node.connect(ac.destination);
    node.onended = () => {
      if (src === node) stopPlayback();
    };
    src = node;
    startedAt = ac.currentTime;
    startOffset = offset;
    node.start(0, offset);
    if (raf) cancelAnimationFrame(raf);
    drawProgress();
  }

  /** rAF 进度重绘（播放中持续刷新高亮与进度线） */
  function drawProgress() {
    if (!ac || !buf) return;
    const t = Math.min(startOffset + (ac.currentTime - startedAt), buf.duration);
    drawWave(t / buf.duration);
    raf = requestAnimationFrame(drawProgress);
  }

  /** 波形绘制：整条峰值柱（青 28%）+ 已播部分高亮（青）+ 品红进度线 */
  function drawWave(progress: number) {
    const canvas = waveCanvas;
    if (!canvas || !buf) return;
    const dpr = window.devicePixelRatio || 1;
    const cssW = canvas.clientWidth;
    const cssH = canvas.clientHeight;
    if (cssW === 0 || cssH === 0) return;
    if (canvas.width !== Math.round(cssW * dpr)) {
      canvas.width = Math.round(cssW * dpr);
      canvas.height = Math.round(cssH * dpr);
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);
    const data = buf.getChannelData(0);
    const bars = Math.max(48, Math.floor(cssW / 3));
    const barW = cssW / bars;
    const mid = cssH / 2;
    const played = Math.floor(bars * progress);
    for (let i = 0; i < bars; i++) {
      const from = Math.floor((i / bars) * data.length);
      const to = Math.max(from + 1, Math.floor(((i + 1) / bars) * data.length));
      let peak = 0;
      for (let j = from; j < to; j += 16) {
        const v = Math.abs(data[j]);
        if (v > peak) peak = v;
      }
      const h = Math.max(2, peak * (cssH - 6));
      ctx.fillStyle = i <= played ? "#00e5ff" : "rgba(0, 229, 255, 0.28)";
      ctx.beginPath();
      ctx.roundRect(i * barW + 0.5, mid - h / 2, Math.max(1, barW - 1), h, 1.5);
      ctx.fill();
    }
    if (progress > 0 && progress < 1) {
      ctx.fillStyle = "#ff2d78";
      ctx.fillRect(progress * cssW - 0.5, 2, 1, cssH - 4);
    }
  }

  /** 点击波形 seek（按比例换算 buffer 偏移重播） */
  function onWaveClick(e: MouseEvent) {
    if (!buf || !waveCanvas) return;
    const rect = waveCanvas.getBoundingClientRect();
    const ratio = Math.min(Math.max((e.clientX - rect.left) / rect.width, 0), 0.999);
    startFrom(buf.duration * ratio);
  }

  async function refresh() {
    const seq = ++refreshSeq;
    try {
      const next = await getHistory();
      if (seq !== refreshSeq) return; // 已有更新的刷新在途，丢弃过期结果
      records = next;
    } catch (e) {
      if (seq !== refreshSeq) return;
      toast(false, `读取历史失败：${errText(e)}`);
      records = [];
    }
  }

  /** 删除单条记录（带录音且不再被其他记录引用时一并删除 wav） */
  async function onDelete(r: HistoryRecord) {
    const id = recordId(r);
    if (deletingId === id) return;
    deletingId = id;
    try {
      await deleteHistoryRecord(r.sessionId, r.ts);
      // 正在播/正在加载这条时先停（wav 可能已物理删除，加载完成会播放空数据）
      if (playingId === id || loadingId === id) stopPlayback();
      toast(true, `已删除该条记录${r.audioFile ? "（含录音）" : ""}`);
      await refresh();
    } catch (e) {
      toast(false, `删除失败：${errText(e)}`);
    } finally {
      deletingId = null;
    }
  }

  async function onModeChange(mode: string) {
    try {
      settingsStore.set(await updateSettings({ history: { mode } }));
      toast(true, "历史记录模式已保存");
      if (mode === "off") records = [];
      else await refresh();
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function onIncludeAudioChange(includeAudio: boolean) {
    try {
      settingsStore.set(await updateSettings({ history: { includeAudio } }));
      toast(
        true,
        includeAudio
          ? "已开启历史音频保存"
          : "已关闭历史音频保存（已有音频保留）",
      );
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
      <div class="w-52">
        <Select
          id="history-mode"
          ariaLabel="记录模式"
          value={$settingsStore?.history.mode ?? "capped"}
          options={[
            { value: "capped", label: `保留最近 ${$settingsStore?.history.maxRecords ?? 1000} 条` },
            { value: "keep-all", label: "全部保留" },
            { value: "off", label: "不记录" },
          ]}
          onchange={(mode) => void onModeChange(mode)}
        />
      </div>
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

  <section class="kotone-panel mt-3 p-4">
    <Toggle
      checked={$settingsStore?.history.includeAudio ?? false}
      label="保存历史记录音频"
      desc="为之后的新记录独立保存 WAV，便于在本页回听；会增加磁盘占用"
      onchange={(value) => void onIncludeAudioChange(value)}
    />
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
        <div class="kotone-card px-4 py-3">
          <div class="flex items-center gap-3">
          <span class="shrink-0 text-[11px] text-white/40 tabular-nums">{fmtTime(r.ts)}</span>
          <div class="min-w-0 flex-1">
            <p class="truncate text-[13px] {r.finalText ? 'text-white/90' : 'text-white/35'}">
              {r.finalText || (r.outcome === "cancelled" ? "（未说完成取消）" : "（无文本）")}
            </p>
            <p class="mt-0.5 text-[10px] text-white/35">
              语音时长 {(r.audioMs / 1000).toFixed(1)} 秒
            </p>
          </div>
          {#if r.audioFile}
            <!-- 播放/暂停：播放中按钮旁显示简易声波动画 -->
            <div class="flex shrink-0 items-center gap-1.5">
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
          <button
            class="shrink-0 rounded bg-white/8 px-1.5 py-0.5 text-[10px] text-white/70 ring-1 ring-white/12 transition hover:bg-white/14"
            onclick={() => (detail = r)}
          >
            详情
          </button>
          <span class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold {meta.cls}">
            {meta.text}
          </span>
          <button
            class="flex h-6 w-6 items-center justify-center rounded-full bg-kotone-pink/15 text-[10px] text-kotone-pink ring-1 ring-kotone-pink/40 transition hover:bg-kotone-pink/25 active:scale-95 disabled:opacity-50"
            title="删除该条记录（含录音）"
            aria-label="删除该条记录"
            disabled={deletingId === id}
            onclick={() => void onDelete(r)}
          >
            {deletingId === id ? "…" : "✕"}
          </button>
          </div>
          {#if playingId === id}
            <!-- 行内波形（canvas 手绘峰值柱，点击可 seek） -->
            <canvas
              bind:this={waveCanvas}
              class="mt-2.5 h-10 w-full cursor-pointer rounded-lg bg-white/4 ring-1 ring-white/8"
              onclick={onWaveClick}
            ></canvas>
          {/if}
        </div>
      {/each}
    </div>
    <p class="mt-3 text-[11px] text-white/35">共 {records.length} 条（新→旧）</p>
  {/if}

  {#if detail}
    {@const meta = outcomeMeta[detail.outcome] ?? outcomeMeta.cancelled}
    <div
      class="fixed inset-0 z-40 flex items-center justify-center bg-black/55 p-4"
      onclick={() => (detail = null)}
      onkeydown={(event) => event.key === "Escape" && (detail = null)}
      role="presentation"
    >
      <div
        class="kotone-card max-h-[80vh] w-full max-w-lg overflow-y-auto p-4"
        onclick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="历史详情"
      >
        <div class="flex items-start justify-between gap-3">
          <div>
            <p class="text-sm font-semibold text-white/90">记录详情</p>
            <p class="mt-0.5 text-[11px] text-white/40">{fmtTime(detail.ts)} · {meta.text}</p>
          </div>
          <button
            class="rounded-md bg-white/8 px-2 py-1 text-[11px] text-white/70 ring-1 ring-white/12"
            onclick={() => (detail = null)}
          >
            关闭
          </button>
        </div>
        <dl class="mt-3 space-y-2 text-[12px]">
          <div>
            <dt class="text-[10px] text-white/40">发出</dt>
            <dd class="mt-0.5 break-all text-white/90">{detail.finalText || "（无文本）"}</dd>
          </div>
          <div>
            <dt class="text-[10px] text-white/40">识别原文</dt>
            <dd class="mt-0.5 break-all text-white/75">{detail.sourceText || "（旧记录无原文）"}</dd>
          </div>
          <div class="grid grid-cols-2 gap-2 text-[11px] text-white/65">
            <div>引擎 {detail.engineId}</div>
            <div>语音 {(detail.audioMs / 1000).toFixed(1)} 秒</div>
            <div>处理 {detail.processDurationMs != null ? `${detail.processDurationMs} ms` : "—"}</div>
            <div>识别收尾 {detail.finalizeLatencyMs != null ? `${detail.finalizeLatencyMs} ms` : "—"}</div>
          </div>
          {#if detail.error}
            <p class="text-[11px] text-kotone-pink/80">{detail.error}</p>
          {/if}
        </dl>
      </div>
    </div>
  {/if}
</div>
