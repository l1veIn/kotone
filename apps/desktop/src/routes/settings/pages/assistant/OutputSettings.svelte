<script lang="ts">
  // 输出（声音与弹幕）：提醒、陪玩回答、自动化的 say / toast 共用。ADR 013 §8：从陪玩移到「通用」。
  import type { AssistantConfig } from "../../../../lib/ipc";
  import { previewReminder, stopSpeech, type DeliveryKind } from "../../../../lib/assistant";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import SpeechSettings from "./SpeechSettings.svelte";
  import DanmakuSettings from "./DanmakuSettings.svelte";
  import "../panel.css";

  let saving = $state(false);
  let previewing = $state<DeliveryKind | null>(null);
  const config = $derived($settingsStore?.assistant);

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: patch }, "已保存"); }
    finally { saving = false; }
  }

  function toggleDelivery(kind: DeliveryKind, checked: boolean) {
    if (!config) return;
    const next = checked ? [...config.defaultDelivery, kind] : config.defaultDelivery.filter((item) => item !== kind);
    if (!next.length) { toast(false, "请至少保留一种默认提醒方式"); return; }
    void save({ defaultDelivery: [...new Set(next)] });
  }

  async function preview(kind: DeliveryKind) {
    if (previewing || saving) return;
    previewing = kind;
    try { await previewReminder(kind); }
    catch (error) { toast(false, `测试失败：${errText(error)}`); }
    finally { previewing = null; }
  }
</script>

{#if config}
  <div class="panel-page" data-testid="output-settings">
    <section class="panel" aria-labelledby="assistant-delivery-title">
      <h2 id="assistant-delivery-title">声音与弹幕</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">琴音提醒你、回答你、播报自动化结果时用的声音和弹幕。</p>
      <div class="setting-row">
        <div><h3>默认方式</h3><p>提醒没指定方式时用哪种。</p></div>
        <div class="flex gap-4 text-sm text-white/80">
          {#each [{ id: "voice", label: "语音播报" }, { id: "danmaku", label: "弹幕" }] as item}
            <label class="flex items-center gap-2"><input type="checkbox" class="accent-cyan-300" disabled={saving}
              checked={config.defaultDelivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
          {/each}
        </div>
      </div>
      <div class="mt-5"><SpeechSettings /></div>
      <div class="mt-5"><DanmakuSettings /></div>
      <p id="danmaku-fullscreen-hint" class="mt-3 text-[11px] leading-relaxed text-amber-200/85">弹幕不支持独占全屏，请使用无边框全屏或窗口化。</p>
      <div class="mt-4 flex flex-wrap gap-2">
        <button type="button" class="secondary" disabled={!!previewing || saving} onclick={() => preview("voice")}>{previewing === "voice" ? "播报中…" : "试听语音"}</button>
        <button type="button" class="secondary" aria-describedby="danmaku-fullscreen-hint" disabled={!!previewing || saving} onclick={() => preview("danmaku")}>测试弹幕</button>
        <button type="button" class="secondary" onclick={() => stopSpeech().catch((error) => toast(false, errText(error)))}>停止播报 / 弹幕</button>
      </div>
    </section>
  </div>
{/if}
