<script lang="ts">
  /*
   * 历史记录页（新 IPC：get_history / clear_history）：
   * 顶部 history.mode 三态 + 独立音频保留开关 + 清空（二次确认）；
   * 列表 = 时间 / 识别文本 / 时长、发到哪个游戏哪个频道（ADR 043 D10，旧记录没有就不显示） / 状态；技术字段收纳到高级页。
   * 音频回放手写 Web Audio 管线（decodeAudioData + AudioBufferSourceNode），
   * 不走 WebView2 的 <audio> 元素——此前「no supported source」的根源；
   * 波形用 canvas 手绘（峰值柱 + 进度高亮），点击可 seek。
   */
  import { onDestroy, onMount, tick } from "svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { listen } from "@tauri-apps/api/event";
  import {
    updateSettings,
    getHistory,
    clearHistory,
    deleteHistoryRecord,
    readHistoryAudio,
    isTauri,
    type HistoryRecord,
  } from "$lib/ipc";
  import { settingsStore, toast, errText } from "$lib/stores/ui";
  import stickerSleepy from "$lib/../assets/brand/stickers/sleepy.webp";
  import { m } from "$lib/paraglide/messages.js";

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
      if (bytes.byteLength === 0) throw new Error(m.voice_history_audio_empty());
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
        toast(false, m.voice_history_play_failed({ error: errText(e) }));
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
    // 波形颜色跟随当前主题（深色 / 浅色、各配色都清楚）
    const style = getComputedStyle(canvas);
    const primary = style.getPropertyValue("--primary").trim() || "#00e5ff";
    const pink = style.getPropertyValue("--brand-pink").trim() || style.getPropertyValue("--destructive").trim() || "#ff2d78";
    for (let i = 0; i < bars; i++) {
      const from = Math.floor((i / bars) * data.length);
      const to = Math.max(from + 1, Math.floor(((i + 1) / bars) * data.length));
      let peak = 0;
      for (let j = from; j < to; j += 16) {
        const v = Math.abs(data[j]);
        if (v > peak) peak = v;
      }
      const h = Math.max(2, peak * (cssH - 6));
      ctx.fillStyle = primary;
      ctx.globalAlpha = i <= played ? 1 : 0.3;
      ctx.beginPath();
      ctx.roundRect(i * barW + 0.5, mid - h / 2, Math.max(1, barW - 1), h, 1.5);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    if (progress > 0 && progress < 1) {
      ctx.fillStyle = pink;
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
      toast(false, m.voice_history_load_failed({ error: errText(e) }));
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
      toast(true, r.audioFile ? m.voice_history_deleted_with_audio() : m.voice_history_deleted());
      await refresh();
    } catch (e) {
      toast(false, m.voice_history_delete_failed({ error: errText(e) }));
    } finally {
      deletingId = null;
    }
  }

  async function onModeChange(mode: string) {
    if (mode !== "capped" && mode !== "keep-all" && mode !== "off") return;
    try {
      settingsStore.set(await updateSettings({ history: { mode } }));
      toast(true, m.voice_history_mode_saved());
      if (mode === "off") records = [];
      else await refresh();
    } catch (err) {
      toast(false, m.voice_save_failed({ error: errText(err) }));
    }
  }

  async function onIncludeAudioChange(includeAudio: boolean) {
    try {
      settingsStore.set(await updateSettings({ history: { includeAudio } }));
      toast(
        true,
        includeAudio
          ? m.voice_history_audio_on()
          : m.voice_history_audio_off(),
      );
    } catch (err) {
      toast(false, m.voice_save_failed({ error: errText(err) }));
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
      toast(true, m.voice_history_cleared());
    } catch (e) {
      toast(false, m.voice_history_clear_failed({ error: errText(e) }));
    }
  }

  const outcomeMeta: Record<string, { text: string; cls: string }> = {
    sent: { text: m.voice_history_outcome_sent(), cls: "bg-primary/12 text-primary" },
    cancelled: { text: m.voice_history_outcome_cancelled(), cls: "bg-muted text-muted-foreground" },
    error: { text: m.voice_history_outcome_error(), cls: "bg-brand-pink/12 text-brand-pink" },
  };

  /** 发到了哪里（“英雄联盟 · 队伍”）；没发出去、旧记录没有时 null。 */
  function sentTo(r: HistoryRecord): string | null {
    if (!r.game && !r.channel) return null;
    return m.plugin_voice_input_history_sent_to({ game: r.game?.name ?? "", channel: r.channel?.name ?? "" });
  }

  function fmtTime(ts: string): string {
    // "2026-07-25T10:14:43Z" → "07-25 10:14"
    const m = ts.match(/^\d{4}-(\d{2}-\d{2})T(\d{2}:\d{2})/);
    return m ? `${m[1]} ${m[2]}` : ts;
  }
</script>

<SettingsPage embedded title={m.voice_tab_history()} description={m.voice_history_page_desc()} testid="history-page">
  {#snippet actions()}
    <Button size="sm" variant={confirmingClear ? "destructive" : "outline"} onclick={() => void onClear()}>
      {confirmingClear ? m.voice_history_clear_confirm() : m.voice_history_clear()}
    </Button>
  {/snippet}

  <SettingsSection>
    <SettingsRow label={m.voice_history_mode_label()} description={m.voice_history_mode_desc()} for="history-mode">
      {#snippet control()}
        <div class="w-52">
          <SelectField
            id="history-mode"
            ariaLabel={m.voice_history_mode_label()}
            value={$settingsStore?.history.mode ?? "capped"}
            options={[
              { value: "capped", label: m.voice_history_mode_capped({ count: $settingsStore?.history.maxRecords ?? 1000 }) },
              { value: "keep-all", label: m.voice_history_mode_keep_all() },
              { value: "off", label: m.voice_history_mode_off() },
            ]}
            onchange={(mode) => void onModeChange(mode)}
          />
        </div>
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField
        checked={$settingsStore?.history.includeAudio ?? false}
        label={m.voice_history_audio_label()}
        desc={m.voice_history_audio_desc()}
        onchange={(value) => void onIncludeAudioChange(value)}
      />
    </div>
  </SettingsSection>

  <!-- 记录列表 -->
  {#if records === null}
    <p class="py-8 text-center text-sm text-muted-foreground">{m.voice_history_loading()}</p>
  {:else if records.length === 0}
    <div class="flex flex-col items-center gap-2 py-8">
      <img src={stickerSleepy} alt={m.voice_history_empty_alt()} class="size-28 object-contain opacity-90" />
      <p class="m-0 text-sm font-medium text-foreground/80">{m.voice_history_empty_title()}</p>
      <p class="m-0 text-xs text-muted-foreground">{m.voice_history_empty_desc()}</p>
    </div>
  {:else}
    <SettingsSection title={m.voice_history_records_title()} description={m.voice_history_records_count({ count: records.length })}>
      {#each records as r (r.sessionId + r.ts)}
        {@const meta = outcomeMeta[r.outcome] ?? outcomeMeta.cancelled}
        {@const id = recordId(r)}
        <div class="py-3" data-testid="history-record">
          <div class="flex items-center gap-3">
            <span class="shrink-0 text-[11px] text-muted-foreground tabular-nums">{fmtTime(r.ts)}</span>
            <div class="min-w-0 flex-1">
              <p class="m-0 truncate text-[13px] {r.finalText ? 'text-foreground' : 'text-muted-foreground'}">
                {r.finalText || (r.outcome === "cancelled" ? m.voice_history_cancelled_unfinished() : m.voice_history_no_text())}
              </p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {m.voice_history_duration({ seconds: (r.audioMs / 1000).toFixed(1) })}
                {#if sentTo(r)}<span data-testid="history-record-sent-to"> · {sentTo(r)}</span>{/if}
              </p>
            </div>
            {#if r.audioFile}
              <!-- 播放/暂停：播放中下方显示波形 -->
              <button
                class="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary/12 text-[11px] text-primary ring-1 ring-primary/35 transition hover:bg-primary/20 active:scale-95 disabled:opacity-50"
                title={playingId === id ? m.voice_history_stop_playback() : m.voice_history_play()}
                aria-label={playingId === id ? m.voice_history_stop_playback() : m.voice_history_play()}
                disabled={loadingId === id}
                onclick={() => void togglePlay(r)}
              >
                {loadingId === id ? "…" : playingId === id ? "⏸" : "▶"}
              </button>
            {/if}
            <Button size="xs" variant="outline" onclick={() => (detail = r)}>{m.voice_history_details()}</Button>
            <span class="shrink-0 rounded-md px-1.5 py-0.5 text-[11px] font-medium {meta.cls}">{meta.text}</span>
            <button
              class="flex size-7 shrink-0 items-center justify-center rounded-full text-[11px] text-muted-foreground transition hover:bg-destructive/12 hover:text-destructive active:scale-95 disabled:opacity-50"
              title={m.voice_history_delete_title()}
              aria-label={m.voice_history_delete_aria()}
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
              class="mt-2.5 h-10 w-full cursor-pointer rounded-lg bg-muted/50 ring-1 ring-hairline"
              onclick={onWaveClick}
            ></canvas>
          {/if}
        </div>
      {/each}
    </SettingsSection>
  {/if}
</SettingsPage>

<Dialog.Root open={detail !== null} onOpenChange={(open) => { if (!open) detail = null; }}>
  <Dialog.Content class="ui-scope max-h-[80vh] overflow-y-auto sm:max-w-lg" aria-label={m.voice_history_detail_aria()}>
    {#if detail}
      {@const meta = outcomeMeta[detail.outcome] ?? outcomeMeta.cancelled}
      <Dialog.Header>
        <Dialog.Title>{m.voice_history_detail_title()}</Dialog.Title>
        <Dialog.Description>{fmtTime(detail.ts)} · {meta.text}</Dialog.Description>
      </Dialog.Header>
      <dl class="m-0 space-y-3 text-[13px]">
        <div>
          <dt class="text-[11px] text-muted-foreground">{m.voice_history_detail_sent()}</dt>
          <dd class="m-0 mt-0.5 break-all text-foreground">{detail.finalText || m.voice_history_no_text()}</dd>
        </div>
        <div>
          <dt class="text-[11px] text-muted-foreground">{m.voice_history_detail_source()}</dt>
          <dd class="m-0 mt-0.5 break-all text-foreground/80">{detail.sourceText || m.voice_history_detail_no_source()}</dd>
        </div>
        {#if sentTo(detail)}
          <div>
            <dt class="text-[11px] text-muted-foreground">{m.plugin_voice_input_history_detail_sent_to()}</dt>
            <dd class="m-0 mt-0.5 text-foreground/80" data-testid="history-detail-sent-to">{sentTo(detail)}</dd>
          </div>
        {/if}
        <div class="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
          <div>{m.voice_history_detail_engine({ engine: detail.engineId })}</div>
          <div>{m.voice_history_detail_audio({ seconds: (detail.audioMs / 1000).toFixed(1) })}</div>
          <div>{m.voice_history_detail_process({ value: detail.processDurationMs != null ? `${detail.processDurationMs} ms` : "—" })}</div>
          <div>{m.voice_history_detail_finalize({ value: detail.finalizeLatencyMs != null ? `${detail.finalizeLatencyMs} ms` : "—" })}</div>
        </div>
        {#if detail.error}
          <p class="m-0 text-xs text-destructive">{detail.error}</p>
        {/if}
      </dl>
      <Dialog.Footer>
        <Button size="sm" variant="outline" onclick={() => (detail = null)}>{m.voice_action_close()}</Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>
