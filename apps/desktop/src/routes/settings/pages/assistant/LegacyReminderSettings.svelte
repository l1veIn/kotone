<script lang="ts">
  import { updateSettings, type AssistantConfig, type ConnectionInfo } from "../../../../lib/ipc";
  import { previewReminder, stopSpeech, type DeliveryKind } from "../../../../lib/assistant";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import AssistantInteraction from "./AssistantInteraction.svelte";
  import SpeechSettings from "./SpeechSettings.svelte";
  import DanmakuSettings from "./DanmakuSettings.svelte";

  let { connections, ready, onOpenConnections }: {
    connections: ConnectionInfo[]; ready: boolean; onOpenConnections: () => void;
  } = $props();
  let saving = $state(false);
  let previewing = $state<DeliveryKind | null>(null);
  const config = $derived($settingsStore?.assistant);

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: patch }, m.companion_legacy_saved()); }
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
    try { await previewReminder(kind, true); }
    catch (error) { toast(false, m.companion_output_preview_failed({ error: errText(error) })); }
    finally { previewing = null; }
  }
</script>

{#if config}
  <div data-testid="assistant-settings">
    <section class="panel" aria-labelledby="assistant-input-settings-title">
      <h2 id="assistant-input-settings-title">{m.companion_legacy_input_title()}</h2>
      <div class="setting-row">
        <div><h3>{m.companion_ai_connection_title()}</h3><p>{m.companion_legacy_connection_desc()}</p></div>
        <div class="w-60 max-w-full">
          <SelectField ariaLabel={m.companion_legacy_connection_aria()} value={config.connectionId} disabled={saving}
            options={[{ value: "", label: m.companion_ai_connection_placeholder() }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : m.companion_ai_connection_no_key()}` }))]}
            onchange={(connectionId) => { void save({ connectionId }); }} />
        </div>
      </div>
      {#if !ready}
        <div class="notice">{connections.length ? m.companion_ai_choose_connection_hint() : m.companion_ai_add_connection_hint()}<button type="button" class="text-primary hover:underline" onclick={onOpenConnections}>{m.companion_ai_manage_connections()}</button></div>
      {/if}
      <div class="mt-4">
        <h3>{m.companion_legacy_hotkey_title()}</h3>
        <HotkeyField id="legacy-reminder" title={m.companion_legacy_hotkey_title()} value={config.hotkey} placeholder={m.companion_legacy_hotkey_placeholder()} disabled={saving} compact
          onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: { hotkey } })); }} />
      </div>
      <AssistantInteraction />
    </section>

    <section class="panel" aria-labelledby="assistant-delivery-title">
      <h2 id="assistant-delivery-title">{m.companion_legacy_delivery_title()}</h2>
      <div class="setting-row">
        <h3>{m.companion_output_default_delivery()}</h3>
        <div class="flex gap-4 text-sm text-foreground">
          {#each [{ id: "voice", label: m.companion_output_delivery_voice() }, { id: "danmaku", label: m.companion_output_delivery_danmaku() }] as item}
            <label class="flex items-center gap-2"><input type="checkbox" class="accent-cyan-300" disabled={saving}
              checked={config.defaultDelivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
          {/each}
        </div>
      </div>
      <div class="mt-5"><SpeechSettings legacyReminder /></div>
      <div class="mt-5"><DanmakuSettings /></div>
      <p id="danmaku-fullscreen-hint" class="mt-3 text-[11px] leading-relaxed text-amber-200/85">{m.companion_output_fullscreen_hint()}</p>
      <div class="mt-4 flex flex-wrap gap-2">
        <button type="button" class="secondary" disabled={!!previewing || saving} onclick={() => preview("voice")}>{previewing === "voice" ? m.companion_output_previewing_voice() : m.companion_output_preview_voice()}</button>
        <button type="button" class="secondary" aria-describedby="danmaku-fullscreen-hint" disabled={!!previewing || saving} onclick={() => preview("danmaku")}>{m.companion_output_preview_danmaku()}</button>
        <button type="button" class="secondary" onclick={() => stopSpeech().catch((error) => toast(false, errText(error)))}>{m.companion_output_stop()}</button>
      </div>
    </section>
  </div>
{/if}
