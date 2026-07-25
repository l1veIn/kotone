<script lang="ts">
  /*
   * 历史记录页（新 IPC：get_history / clear_history）：
   * 顶部 history.mode 三态 + includeAudio 开关 + 清空（二次确认）；
   * 列表 = 时间 / 识别文本 / 状态 / 引擎；空状态配 stickers/sleepy.png。
   */
  import { onMount } from "svelte";
  import {
    updateSettings,
    getHistory,
    clearHistory,
    type HistoryRecord,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import stickerSleepy from "../../../assets/brand/stickers/sleepy.png";

  let records = $state<HistoryRecord[] | null>(null);
  /** 清空二次确认：点一次进入确认态，3s 内再点执行 */
  let confirmingClear = $state(false);

  onMount(async () => {
    await refresh();
  });

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

  async function onIncludeAudioChange(v: boolean) {
    try {
      settingsStore.set(await updateSettings({ history: { includeAudio: v } }));
      toast(true, v ? "已开启随记录保存音频" : "已关闭随记录保存音频");
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
    <div class="flex-1">
      <Toggle
        checked={$settingsStore?.history.includeAudio ?? false}
        label="保存音频"
        desc="从评测录档复制 wav（需开启评测录档）"
        onchange={(v) => void onIncludeAudioChange(v)}
      />
    </div>
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
        <div class="kotone-card flex items-center gap-3 px-4 py-3">
          <span class="shrink-0 text-[11px] text-white/40 tabular-nums">{fmtTime(r.ts)}</span>
          <div class="min-w-0 flex-1">
            <p class="truncate text-[13px] {r.finalText ? 'text-white/90' : 'text-white/35'}">
              {r.finalText || (r.outcome === "cancelled" ? "（未说完成取消）" : "（无文本）")}
            </p>
            <p class="mt-0.5 text-[10px] text-white/35">
              {r.engineId}{r.profileId ? ` · ${r.profileId}` : ""} · {(r.audioMs / 1000).toFixed(1)}s
              {#if r.error}· <span class="text-kotone-pink/80">{r.error}</span>{/if}
            </p>
          </div>
          <span class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold {meta.cls}">
            {meta.text}
          </span>
        </div>
      {/each}
    </div>
    <p class="mt-3 text-[11px] text-white/35">共 {records.length} 条（新→旧）</p>
  {/if}
</div>
