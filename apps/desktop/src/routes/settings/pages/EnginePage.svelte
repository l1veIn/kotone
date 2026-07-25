<script lang="ts">
  /*
   * 引擎与模型页：引擎卡片（能力标签 + 切换）+ 模型下载状态。
   * 加载/空状态配 stickers/curious.png。
   */
  import { onMount } from "svelte";
  import {
    listSttEngines,
    setSttEngine,
    listModels,
    downloadModel,
    type EngineInfo,
    type ModelInfo,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";
  import stickerCurious from "../../../assets/brand/stickers/curious.png";

  let engines = $state<EngineInfo[] | null>(null);
  let models = $state<ModelInfo[]>([]);
  let downloading = $state<string | null>(null);

  onMount(async () => {
    try {
      [engines, models] = await Promise.all([listSttEngines(), listModels()]);
    } catch (e) {
      toast(false, `加载引擎信息失败：${errText(e)}`);
      engines = [];
    }
  });

  async function onEngineSwitch(id: string) {
    try {
      await setSttEngine(id);
      settingsStore.update((s) => (s ? { ...s, sttEngine: id } : s));
      const name = engines?.find((en) => en.id === id)?.displayName ?? id;
      toast(true, `已切换到引擎：${name}`);
    } catch (e) {
      toast(false, `切换引擎失败：${errText(e)}`);
    }
  }

  async function onDownload(id: string) {
    if (downloading) return;
    downloading = id;
    try {
      await downloadModel(id);
      models = models.map((m) => (m.id === id ? { ...m, downloaded: true } : m));
      toast(true, `模型下载完成：${id}`);
    } catch (e) {
      toast(false, `下载失败：${errText(e)}`);
    } finally {
      downloading = null;
    }
  }

  /** 引擎能力标签 */
  function engineTags(en: EngineInfo): { text: string; cls: string }[] {
    const tags: { text: string; cls: string }[] = [];
    if (en.capabilities.streaming) tags.push({ text: "流式", cls: "bg-kotone-cyan/15 text-kotone-cyan" });
    if (en.capabilities.offline) tags.push({ text: "离线", cls: "bg-kotone-violet/15 text-kotone-violet" });
    if (en.capabilities.hotwords) tags.push({ text: "热词", cls: "bg-white/10 text-white/70" });
    if (en.capabilities.gpu) tags.push({ text: "GPU", cls: "bg-white/10 text-white/70" });
    if (!en.isReady) tags.push({ text: "未就绪", cls: "bg-kotone-pink/15 text-kotone-pink" });
    return tags;
  }

  function formatSize(bytes: number): string {
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.round(bytes / 1_000_000)} MB`;
  }

  function modelsOf(engineId: string): ModelInfo[] {
    return models.filter((m) => m.engineId === engineId);
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">引擎与模型</h1>
  <p class="mt-0.5 text-[11px] text-white/45">识别引擎随时可换，模型本地运行</p>

  {#if engines === null}
    <div class="mt-10 flex flex-col items-center gap-3">
      <img src={stickerCurious} alt="加载中" class="h-24 w-24 object-contain" />
      <p class="text-sm text-white/50">Kotone 正在清点引擎…</p>
    </div>
  {:else if engines.length === 0}
    <div class="mt-10 flex flex-col items-center gap-3">
      <img src={stickerCurious} alt="空空如也" class="h-24 w-24 object-contain" />
      <p class="text-sm text-white/50">还没有可用引擎</p>
    </div>
  {:else}
    <div class="mt-4 flex flex-col gap-3">
      {#each engines as en}
        {@const active = $settingsStore?.sttEngine === en.id}
        <div
          class="kotone-card p-4 {active ? 'border-kotone-cyan/50 shadow-glow-cyan' : ''}"
        >
          <div class="flex items-center gap-3">
            <div class="min-w-0 flex-1">
              <p class="flex items-center gap-2 text-sm font-medium">
                <span class="truncate">{en.displayName}</span>
                {#if active}
                  <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">
                    使用中
                  </span>
                {/if}
              </p>
              <p class="mt-1.5 flex flex-wrap gap-1">
                {#each engineTags(en) as tag}
                  <span class="rounded px-1.5 py-0.5 text-[10px] {tag.cls}">{tag.text}</span>
                {/each}
                <span class="rounded bg-white/5 px-1.5 py-0.5 text-[10px] text-white/40">
                  {en.capabilities.languages.join(" / ")}
                </span>
              </p>
            </div>
            {#if !active}
              <button
                class="shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold transition active:scale-95 {en.isReady
                  ? 'bg-kotone-cyan text-kotone-deep hover:brightness-110'
                  : 'bg-white/10 text-white/60 hover:bg-white/20'}"
                onclick={() => void onEngineSwitch(en.id)}
              >
                {en.isReady ? "切换" : "仍切换"}
              </button>
            {/if}
          </div>

          <!-- 该引擎的模型 -->
          {#each modelsOf(en.id) as m}
            <div class="mt-3 flex items-center justify-between rounded-lg bg-white/5 px-3 py-2 ring-1 ring-white/8">
              <span class="text-[12px] text-white/70">
                {m.id} <span class="text-white/35">（{formatSize(m.sizeBytes)}）</span>
              </span>
              {#if m.downloaded}
                <span class="text-[11px] text-kotone-cyan">✓ 已下载</span>
              {:else}
                <button
                  class="rounded-lg bg-kotone-violet/25 px-2.5 py-1 text-[11px] font-semibold text-kotone-violet ring-1 ring-kotone-violet/40 transition hover:bg-kotone-violet/35 active:scale-95 disabled:opacity-50"
                  disabled={downloading !== null}
                  onclick={() => void onDownload(m.id)}
                >
                  {downloading === m.id ? "下载中…" : "下载"}
                </button>
              {/if}
            </div>
          {/each}
        </div>
      {/each}
    </div>
    <p class="mt-3 text-[11px] text-white/40">
      未就绪的引擎（模型未下载）切换后按热键会提示错误；联调建议使用 mock-stream。
    </p>
  {/if}
</div>
