<script lang="ts">
  // 「游戏陪玩 → 角色 → 交互 → 自动卡片」（ADR 039 D3、D4）：插件自己往聊天里发卡片、发卡片时右下角的消息提示。
  // 设置在 config.json `agent.autoCards`（新字段，旧配置没有时两个都按开）。
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  const config = $derived({ enabled: true, popup: true, ...($settingsStore?.agent?.autoCards ?? {}) });

  async function save(patch: Partial<{ enabled: boolean; popup: boolean }>, message: string) {
    await patchSettings({ agent: { autoCards: { ...config, ...patch } } }, message);
  }
</script>

<SettingsSection title={m.companion_auto_cards_title()} description={m.companion_auto_cards_desc()} id="auto-card-settings">
  <div data-testid="auto-card-settings" class="contents">
    <SettingsRow label={m.companion_auto_cards_enabled_label()} description={m.companion_auto_cards_enabled_desc()}>
      {#snippet control()}
        <Switch data-testid="auto-cards-enabled" aria-label={m.companion_auto_cards_enabled_label()}
          bind:checked={() => config.enabled, (checked) => void save({ enabled: checked }, checked ? m.companion_auto_cards_enabled_on() : m.companion_auto_cards_enabled_off())} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_auto_cards_popup_label()} description={m.companion_auto_cards_popup_desc()}>
      {#snippet control()}
        <Switch data-testid="auto-cards-popup" aria-label={m.companion_auto_cards_popup_label()} disabled={!config.enabled}
          bind:checked={() => config.popup, (checked) => void save({ popup: checked }, checked ? m.companion_auto_cards_popup_on() : m.companion_auto_cards_popup_off())} />
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
