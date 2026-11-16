<script lang="ts">
  import { updateSettings, type AssistantConfig, type ConnectionInfo } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import Select from "../../../../lib/components/Select.svelte";
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
  <div data-testid="assistant-settings">
    <section class="panel" aria-labelledby="assistant-input-settings-title">
      <h2 id="assistant-input-settings-title">输入设置</h2>
      <div class="setting-row">
        <div><h3>AI 连接</h3><p>需支持工具调用，水准不低于 DeepSeek V4.1 Flash。</p></div>
        <div class="w-60 max-w-full">
          <Select ariaLabel="陪玩 AI 连接" value={config.connectionId} disabled={saving || checking}
            options={[{ value: "", label: "请选择连接" }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : "（未保存密钥）"}` }))]}
            onchange={(connectionId) => {
              // 选定连接时跑一次能力自检；没通过时陪玩热键不工作。
              void save({ connectionId }).then(() => {
                if (connectionId && connections.find((item) => item.id === connectionId)?.hasApiKey) void companion?.runCheck();
              });
            }} />
        </div>
      </div>
      {#if !ready}
        <div class="notice">{connections.length ? "请选择连接并配置密钥。" : "请先添加 AI 连接。"}<button type="button" class="text-kotone-cyan hover:underline" onclick={onOpenConnections}>管理连接 →</button></div>
      {/if}
      <div class="mt-4">
        <h3>陪玩热键</h3>
        <HotkeyField id="assistant" title="陪玩热键" value={config.hotkey} placeholder="如 Ctrl+F9" disabled={saving} compact
          onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: { hotkey } })); }} />
      </div>
      <AssistantInteraction />
    </section>

    <CompanionSettings bind:this={companion} bind:checking />

  </div>
{/if}
