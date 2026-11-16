<script lang="ts">
  // 「琴音 → 设置」：AI 连接、陪玩热键与输入方式；下面接陪玩（能力自检、快捷指令）。
  import { updateSettings, type AssistantConfig, type ConnectionInfo } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import AssistantInteraction from "./AssistantInteraction.svelte";
  import CompanionSettings from "./CompanionSettings.svelte";

  let { connections, ready, onOpenConnections }: {
    connections: ConnectionInfo[]; ready: boolean; onOpenConnections: () => void;
  } = $props();
  let saving = $state(false);
  let companion = $state<CompanionSettings | null>(null);
  let checking = $state(false);
  const config = $derived($settingsStore?.assistant);

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: patch }, "陪玩设置已保存"); }
    finally { saving = false; }
  }
</script>

{#if config}
  <div class="space-y-6" data-testid="assistant-settings">
    <SettingsSection title="输入设置" id="assistant-input-settings-title" description="琴音用的 AI 连接，以及在游戏里叫琴音的热键。">
      <SettingsRow label="AI 连接" description="给琴音用的主流大模型：需能稳定调用工具，水准不低于 DeepSeek V4.1 Flash。">
        {#snippet control()}
          <div class="w-64 max-w-full">
            <SelectField ariaLabel="陪玩 AI 连接" value={config.connectionId} disabled={saving || checking}
              options={[{ value: "", label: "请选择连接" }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : "（未保存密钥）"}` }))]}
              onchange={(connectionId) => {
                // 选定连接时跑一次能力自检；没通过时陪玩热键不工作。
                void save({ connectionId }).then(() => {
                  if (connectionId && connections.find((item) => item.id === connectionId)?.hasApiKey) void companion?.runCheck();
                });
              }} />
          </div>
        {/snippet}
        {#if !ready}
          <p class="m-0 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-warning/30 bg-warning/8 px-3 py-2 text-xs text-foreground/85">
            {connections.length ? "请选择连接并配置密钥。" : "请先添加 AI 连接。"}
            <button type="button" class="font-medium text-primary hover:underline" onclick={onOpenConnections}>管理连接 →</button>
          </p>
        {/if}
      </SettingsRow>
      <SettingsRow label="陪玩热键" description="在游戏里按下后说一句话，交给琴音。" stacked>
        {#snippet control()}
          <HotkeyField id="assistant" title="陪玩热键" value={config.hotkey} placeholder="如 Ctrl+F9" disabled={saving} compact
            onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: { hotkey } })); }} />
        {/snippet}
      </SettingsRow>
      <AssistantInteraction />
    </SettingsSection>

    <CompanionSettings bind:this={companion} bind:checking />
  </div>
{/if}
