<script lang="ts">
  // 「设置 → 她怎么回答 → AI 连接」：角色用的 AI 连接和模型（ADR 028，原「琴音 → 设置」）。
  // 模型原来在聊天输入框旁边（照 DeepSeek Harness），游戏陪玩不常换模型，挪到这里（rc2 玩家反馈）。
  // 模型清单照 DeepSeek Harness：读服务商的模型列表（GET /models，不花 token），读不到时用预置的模型名。
  import {
    companionBindingPatch, companionBindings, connectionModels, listConnectionPresets, upsertConnection,
    type ConnectionInfo, type ConnectionPreset,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { connections, ready, onOpenConnections }: {
    connections: ConnectionInfo[]; ready: boolean; onOpenConnections: () => void;
  } = $props();
  let saving = $state(false);
  const config = $derived($settingsStore?.assistant);
  const bindings = $derived(companionBindings(config));
  const current = $derived(connections.find((c) => c.id === bindings?.connectionId) ?? null);

  async function save(connectionId: string) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: companionBindingPatch(config, { connectionId }) }, m.companion_ai_connection_saved()); }
    finally { saving = false; }
  }

  // ---- 模型：当前连接的服务商列出的模型 ----
  let presets = $state<ConnectionPreset[]>([]);
  /** 服务商实际列出的模型（每条连接读一次）。 */
  let listed = $state<Record<string, string[]>>({});
  /** 刚保存的模型（连接清单还没刷新时先显示这个）。 */
  let savedModel = $state<Record<string, string>>({});
  const currentModel = $derived(current ? (savedModel[current.id] ?? current.model) : "");
  const models = $derived.by(() => {
    if (!current) return [] as string[];
    const list = [...(listed[current.id]?.length ? listed[current.id] : presets.find((p) => p.id === current.provider)?.models ?? [])];
    if (currentModel && !list.includes(currentModel)) list.unshift(currentModel);
    return list;
  });

  $effect(() => {
    const c = current;
    if (!c || !c.hasApiKey || c.kind !== "remote") return;
    if (!presets.length) void listConnectionPresets().then((p) => { presets = p; }).catch(() => {});
    if (c.id in listed) return;
    void connectionModels(c.id)
      .then((reply) => { listed = { ...listed, [c.id]: reply?.supported ? reply.models : [] }; })
      .catch(() => { listed = { ...listed, [c.id]: [] }; });
  });

  async function setModel(model: string) {
    const c = current;
    if (!c || saving || !model || model === currentModel) return;
    saving = true;
    try {
      const { hasApiKey: _key, ...connection } = c;
      // 换模型不再自检（照原来输入框里的做法）；设置更新后「设置」页的连接清单会跟着刷新
      settingsStore.set(await upsertConnection({ ...connection, model }));
      savedModel = { ...savedModel, [c.id]: model };
      toast(true, m.companion_ai_model_switched({ model }));
    } catch (error) {
      toast(false, m.companion_ai_model_switch_failed({ error: errText(error) }));
    } finally {
      saving = false;
    }
  }
</script>

{#if bindings}
  <div class="space-y-6" data-testid="assistant-settings">
    <SettingsSection title={m.companion_ai_section_title()} id="assistant-input-settings-title" description={m.companion_ai_section_desc()}>
      <SettingsRow label={m.companion_ai_connection_title()} description={m.companion_ai_connection_desc()}>
        {#snippet control()}
          <div class="w-64 max-w-full">
            <SelectField ariaLabel={m.companion_ai_connection_aria()} value={bindings.connectionId} disabled={saving}
              options={[{ value: "", label: m.companion_ai_connection_placeholder() }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : m.companion_ai_connection_no_key()}` }))]}
              onchange={(connectionId) => { void save(connectionId); }} />
          </div>
        {/snippet}
        {#if !ready}
          <p class="m-0 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-warning/30 bg-warning/8 px-3 py-2 text-xs text-foreground/85">
            {connections.length ? m.companion_ai_choose_connection_hint() : m.companion_ai_add_connection_hint()}
            <button type="button" class="font-medium text-primary hover:underline" onclick={onOpenConnections}>{m.companion_ai_manage_connections()}</button>
          </p>
        {/if}
      </SettingsRow>
      {#if current && current.hasApiKey && current.kind === "remote"}
        <SettingsRow label={m.companion_ai_model_label()} description={m.companion_ai_model_desc()}>
          {#snippet control()}
            <div class="w-64 max-w-full">
              <SelectField ariaLabel={m.companion_ai_model_aria()} testid="assistant-model" value={currentModel} disabled={saving || models.length === 0}
                options={models.map((m) => ({ value: m, label: m }))}
                onchange={(model) => { void setModel(model); }} />
            </div>
          {/snippet}
        </SettingsRow>
      {/if}
    </SettingsSection>
  </div>
{/if}
