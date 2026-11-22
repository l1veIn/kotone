<script lang="ts">
  import { onMount } from "svelte";
  import {
    companionClearChatRegion, companionStatus, updateSettings,
    type CompanionStatus, type QuickMessage,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import { listAutomations, subscribeAutomations, type Automation } from "../../../../lib/automation";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import { CircleAlert, CircleCheck, Plus, Trash2 } from "@lucide/svelte";
  import ArgsForm from "../automation/ArgsForm.svelte";

  let status = $state<CompanionStatus | null>(null);
  let automations = $state<Automation[]>([]);
  let saving = $state(false);
  const config = $derived($settingsStore?.assistant);
  const quick = $derived(config?.companion?.quickMessages ?? []);
  const MAX = 8;

  export async function refresh() {
    try { status = await companionStatus(); } catch (error) { toast(false, m.companion_quick_status_load_failed({ error: errText(error) })); }
  }

  async function loadAutomations() {
    try { automations = await listAutomations(); } catch { /* 列表读不到时只是没有可选项 */ }
  }

  onMount(() => {
    void refresh();
    void loadAutomations();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void subscribeAutomations(() => { void loadAutomations(); }).then((fn) => { if (disposed) fn(); else unlisten = fn; });
    return () => { disposed = true; unlisten?.(); };
  });
  $effect(() => { config?.connectionId; config?.companionBindings?.connectionId; config?.companion; $settingsStore?.ocr?.activeModelId; void refresh(); });

  async function saveQuick(next: QuickMessage[], message = m.companion_quick_saved()) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: { companion: { quickMessages: next } } }, message); }
    finally { saving = false; }
  }

  function addQuick() {
    if (quick.length >= MAX) return;
    const id = `qm-${Date.now().toString(36)}`;
    // i18n-ignore-start：新快捷指令的默认话，存进配置
    void saveQuick([...quick, { id, label: "", text: "帮我看看队友说了什么", hotkey: "" }], m.companion_quick_added());
    // i18n-ignore-end
  }

  function patchQuick(index: number, patch: Partial<QuickMessage>) {
    void saveQuick(quick.map((item, i) => (i === index ? { ...item, ...patch } : item)));
  }

  function removeQuick(index: number) {
    void saveQuick(quick.filter((_, i) => i !== index), m.companion_quick_removed());
  }

  /** 切换“交给 AI 的一句话” / “运行自动化”。 */
  function setKind(index: number, kind: string) {
    const item = quick[index];
    if (kind === "automation") {
      if (!automations.length) { toast(false, m.companion_quick_no_automations()); return; }
      patchQuick(index, { automationId: automations[0].id, args: {} });
    } else {
      // i18n-ignore-start：新快捷指令的默认话，存进配置
      patchQuick(index, { automationId: "", args: {}, text: item.text.trim() || "帮我看看队友说了什么" });
      // i18n-ignore-end
    }
  }

</script>

{#if config}
  <div class="space-y-6" data-testid="companion-settings">
    <SettingsSection title={m.companion_quick_screen_title()} id="companion-title" description={m.companion_quick_screen_desc()}>
      {#if status}
        <div class="py-3.5">
          <p class="m-0 flex items-start gap-2 rounded-lg border px-3 py-2 text-xs leading-relaxed {status.ready ? 'border-success/35 bg-success/8 text-success' : 'border-warning/35 bg-warning/8 text-foreground/85'}">
            {#if status.ready}<CircleCheck class="mt-0.5 size-3.5 shrink-0" />{:else}<CircleAlert class="mt-0.5 size-3.5 shrink-0 text-warning" />{/if}
            <span data-testid="companion-status">{status.ready ? m.companion_quick_status_ready() : status.reason}</span>
          </p>
          {#if status.ocr.state?.dataEgress === "selected-image"}
            <p class="mt-2 mb-0 text-xs leading-relaxed text-brand-pink" data-testid="companion-upload-warning">
              {m.companion_quick_upload_warning()}
            </p>
          {/if}
        </div>
      {/if}
      <SettingsRow label={m.companion_quick_auto_send_label()} description={m.companion_quick_auto_send_desc()}>
        {#snippet control()}
          <Switch aria-label={m.companion_quick_auto_send_label()} disabled={saving}
            bind:checked={() => config.companion.autoSend, (checked) => void patchSettings({ assistant: { companion: { autoSend: checked } } }, m.companion_saved())} />
        {/snippet}
      </SettingsRow>
      {#if status?.profile.chatRegion}
        <SettingsRow label={m.companion_quick_chat_region_label()} description={m.companion_quick_chat_region_desc()}>
          {#snippet control()}
            <Button variant="outline" size="sm" onclick={() => companionClearChatRegion().then(refresh).catch((error) => toast(false, errText(error)))}>{m.companion_quick_chat_region_reset()}</Button>
          {/snippet}
        </SettingsRow>
      {/if}
    </SettingsSection>

    <SettingsSection title={m.companion_quick_title()} id="companion-quick-title"
      description={m.companion_quick_desc({ max: MAX })}>
      {#snippet actions()}
        <Button variant="outline" size="sm" disabled={saving || quick.length >= MAX} onclick={addQuick}><Plus class="size-3.5" />{m.companion_quick_add()}</Button>
      {/snippet}
      {#if !quick.length}
        <p class="my-0 py-6 text-center text-xs text-muted-foreground">{m.companion_quick_empty()}</p>
      {/if}
      {#each quick as item, index (item.id)}
        {@const runsAutomation = !!item.automationId}
        {@const target = automations.find((a) => a.id === item.automationId)}
        <div class="space-y-3 py-3.5" data-testid="quick-message">
          <div class="flex flex-wrap items-center gap-2">
            <Input class="h-8 w-28 text-[13px]" placeholder={m.companion_quick_name_placeholder()} value={item.label} aria-label={m.companion_quick_name_aria({ index: index + 1 })}
              onchange={(event) => patchQuick(index, { label: event.currentTarget.value.trim() })} />
            <div class="w-40"><SelectField ariaLabel={m.companion_quick_kind_aria({ index: index + 1 })} value={runsAutomation ? "automation" : "ask"} disabled={saving}
              options={[{ value: "ask", label: m.companion_quick_kind_ask() }, { value: "automation", label: m.companion_quick_kind_automation() }]}
              onchange={(kind) => setKind(index, kind)} /></div>
            {#if runsAutomation}
              <div class="min-w-0 flex-1"><SelectField ariaLabel={m.companion_quick_automation_aria({ index: index + 1 })} value={item.automationId ?? ""} disabled={saving}
                options={[...(target ? [] : [{ value: item.automationId ?? "", label: m.companion_quick_automation_deleted() }]), ...automations.map((a) => ({ value: a.id, label: a.name }))]}
                onchange={(automationId) => patchQuick(index, { automationId, args: {} })} /></div>
            {:else}
              <Input class="h-8 min-w-0 flex-1 text-[13px]" value={item.text} aria-label={m.companion_quick_text_aria({ index: index + 1 })} maxlength={200}
                onchange={(event) => {
                  const text = event.currentTarget.value.trim();
                  if (!text) { toast(false, m.companion_quick_text_empty()); event.currentTarget.value = item.text; return; }
                  patchQuick(index, { text });
                }} />
            {/if}
            <Button variant="ghost" size="sm" class="text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={saving} onclick={() => removeQuick(index)}><Trash2 class="size-3.5" />{m.companion_quick_remove()}</Button>
          </div>
          {#if runsAutomation && target?.params.length}
            <div class="rounded-xl border border-hairline bg-muted/40 p-3">
              <ArgsForm params={target.params} values={item.args ?? {}} disabled={saving} prefix={`${m.companion_quick_args_prefix({ index: index + 1 })} `}
                onchange={(args) => patchQuick(index, { args })} />
            </div>
          {/if}
          <HotkeyField id={`quick-message-${index + 1}`} title={m.companion_quick_hotkey_title({ index: index + 1 })} value={item.hotkey} placeholder={m.companion_quick_hotkey_placeholder()} compact disabled={saving}
            onSave={async (hotkey) => {
              const next = quick.map((q, i) => (i === index ? { ...q, hotkey } : q));
              settingsStore.set(await updateSettings({ assistant: { companion: { quickMessages: next } } }));
            }} />
        </div>
      {/each}
    </SettingsSection>
  </div>
{/if}
