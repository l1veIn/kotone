<script lang="ts">
  // 「琴音 → 设置」：琴音用的 AI 连接。陪玩热键、快捷指令搬到了「交流」（ADR 023）。
  import type { AssistantConfig, ConnectionInfo } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";

  let { connections, ready, onOpenConnections }: {
    connections: ConnectionInfo[]; ready: boolean; onOpenConnections: () => void;
  } = $props();
  let saving = $state(false);
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
    <SettingsSection title="AI 连接" id="assistant-input-settings-title" description="用哪个大模型。在游戏里怎么叫她，在「交流」里设置。">
      <SettingsRow label="AI 连接" description="选好、有密钥就能用；想确认能不能连上，到「设置 → API 连接」点“测试连接”。">
        {#snippet control()}
          <div class="w-64 max-w-full">
            <SelectField ariaLabel="陪玩 AI 连接" value={config.connectionId} disabled={saving}
              options={[{ value: "", label: "请选择连接" }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : "（未保存密钥）"}` }))]}
              onchange={(connectionId) => { void save({ connectionId }); }} />
          </div>
        {/snippet}
        {#if !ready}
          <p class="m-0 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-warning/30 bg-warning/8 px-3 py-2 text-xs text-foreground/85">
            {connections.length ? "请选择连接并配置密钥。" : "请先添加 AI 连接。"}
            <button type="button" class="font-medium text-primary hover:underline" onclick={onOpenConnections}>管理连接 →</button>
          </p>
        {/if}
      </SettingsRow>
    </SettingsSection>
  </div>
{/if}
