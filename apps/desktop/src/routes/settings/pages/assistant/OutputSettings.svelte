<script lang="ts">
  // 输出（声音与弹幕）：提醒、陪玩回答、自动化的 say / toast 共用。ADR 013 §8：从陪玩移到「通用」。
  import type { AssistantConfig } from "../../../../lib/ipc";
  import { previewReminder, stopSpeech, type DeliveryKind } from "../../../../lib/assistant";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import SpeechSettings from "./SpeechSettings.svelte";
  import DanmakuSettings from "./DanmakuSettings.svelte";
  import SettingsSection from "../../../../lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "../../../../lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
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
  <div data-testid="output-settings">
    <SettingsSection title="声音与弹幕" description="琴音提醒你、回答你、播报自动化结果时用的声音和弹幕。" id="assistant-delivery-title">
      <SettingsRow label="默认方式" description="提醒没指定方式时用哪种。">
        {#snippet control()}
          <div class="flex gap-4 text-[13px] text-foreground">
            {#each [{ id: "voice", label: "语音播报" }, { id: "danmaku", label: "弹幕" }] as item}
              <label class="flex items-center gap-2"><input type="checkbox" class="size-4 accent-[var(--color-primary)]" disabled={saving}
                checked={config.defaultDelivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
      <!-- 语音与弹幕的细项（仍用旧版 panel 样式，随琴音设置一起迁移） -->
      <div class="panel-page py-3.5"><SpeechSettings /></div>
      <div class="panel-page py-3.5"><DanmakuSettings /></div>
      <div class="py-3.5">
        <p id="danmaku-fullscreen-hint" class="m-0 text-xs leading-relaxed text-warning">弹幕不支持独占全屏，请使用无边框全屏或窗口化。</p>
        <div class="mt-3 flex flex-wrap gap-2">
          <Button size="sm" variant="outline" disabled={!!previewing || saving} onclick={() => preview("voice")}>{previewing === "voice" ? "播报中…" : "试听语音"}</Button>
          <Button size="sm" variant="outline" aria-describedby="danmaku-fullscreen-hint" disabled={!!previewing || saving} onclick={() => preview("danmaku")}>测试弹幕</Button>
          <Button size="sm" variant="ghost" onclick={() => stopSpeech().catch((error) => toast(false, errText(error)))}>停止播报 / 弹幕</Button>
        </div>
      </div>
    </SettingsSection>
  </div>
{/if}
