<script lang="ts">
  // 输出（声音与弹幕）：提醒、陪玩回答、自动化的 say / toast 共用。ADR 013 §8：从陪玩移到「通用」。
  import type { AssistantConfig } from "../../../../lib/ipc";
  import { previewReminder, stopSpeech, type DeliveryKind } from "../../../../lib/assistant";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import SpeechSettings from "./SpeechSettings.svelte";
  import DanmakuSettings from "./DanmakuSettings.svelte";
  import SettingsSection from "../../../../lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "../../../../lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";

  let saving = $state(false);
  let previewing = $state<DeliveryKind | null>(null);
  const config = $derived($settingsStore?.assistant);

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: patch }, m.companion_saved()); }
    finally { saving = false; }
  }

  function toggleDelivery(kind: DeliveryKind, checked: boolean) {
    if (!config) return;
    const next = checked ? [...config.defaultDelivery, kind] : config.defaultDelivery.filter((item) => item !== kind);
    if (!next.length) { toast(false, m.companion_output_delivery_required()); return; }
    void save({ defaultDelivery: [...new Set(next)] });
  }

  async function preview(kind: DeliveryKind) {
    if (previewing || saving) return;
    previewing = kind;
    try { await previewReminder(kind); }
    catch (error) { toast(false, m.companion_output_preview_failed({ error: errText(error) })); }
    finally { previewing = null; }
  }
</script>

{#if config}
  <div data-testid="output-settings">
    <SettingsSection title={m.companion_output_title()} description={m.companion_output_desc()} id="assistant-delivery-title">
      <SettingsRow label={m.companion_output_default_delivery()} description={m.companion_output_default_delivery_desc()}>
        {#snippet control()}
          <div class="flex gap-4 text-[13px] text-foreground">
            {#each [{ id: "voice", label: m.companion_output_delivery_voice() }, { id: "danmaku", label: m.companion_output_delivery_danmaku() }] as item}
              <label class="flex items-center gap-2"><input type="checkbox" class="size-4 accent-[var(--color-primary)]" disabled={saving}
                checked={config.defaultDelivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
      <div class="py-3.5"><SpeechSettings /></div>
      <div class="py-3.5"><DanmakuSettings /></div>
      <div class="py-3.5">
        <p id="danmaku-fullscreen-hint" class="m-0 text-xs leading-relaxed text-warning">{m.companion_output_fullscreen_hint()}</p>
        <div class="mt-3 flex flex-wrap gap-2">
          <Button size="sm" variant="outline" disabled={!!previewing || saving} onclick={() => preview("voice")}>{previewing === "voice" ? m.companion_output_previewing_voice() : m.companion_output_preview_voice()}</Button>
          <Button size="sm" variant="outline" aria-describedby="danmaku-fullscreen-hint" disabled={!!previewing || saving} onclick={() => preview("danmaku")}>{m.companion_output_preview_danmaku()}</Button>
          <Button size="sm" variant="ghost" onclick={() => stopSpeech().catch((error) => toast(false, errText(error)))}>{m.companion_output_stop()}</Button>
        </div>
      </div>
    </SettingsSection>
  </div>
{/if}
