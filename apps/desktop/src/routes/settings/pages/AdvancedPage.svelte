<script lang="ts">
  /*
   * 「设置」（ADR 028）：陪玩组 + 通用组两排分页。陪玩组是原「琴音 → 设置」整页（D11）：
   * 这些设置换角色时共用，不属于某个角色，所以放在共享的「设置」里；角色页只留 角色 · 记忆 · 技能。
   * 判断标准：“如果删掉角色，这项设置还需要吗？”需要的留在通用组。
   */
  import { onMount, tick } from "svelte";
  import { companionName } from "$lib/stores/companion";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import AppearanceSettingsPanel from "$lib/appearance/AppearanceSettingsPanel.svelte";
  import ConnectionsSection from "./advanced/ConnectionsSection.svelte";
  import ModelsSection from "./advanced/ModelsSection.svelte";
  import SoundSection from "./advanced/SoundSection.svelte";
  import SystemSection from "./advanced/SystemSection.svelte";
  import TuningSection from "./advanced/TuningSection.svelte";
  import PluginsSection from "./advanced/PluginsSection.svelte";
  import AssistantSettings from "./assistant/AssistantSettings.svelte";
  import CompanionInputSettings from "./assistant/CompanionInputSettings.svelte";
  import WakeSettings from "./assistant/WakeSettings.svelte";
  import BubbleSettings from "./assistant/BubbleSettings.svelte";
  import OutputSettings from "./assistant/OutputSettings.svelte";
  import VoiceChatSettings from "./assistant/VoiceChatSettings.svelte";
  import AgentSettings from "./assistant/AgentSettings.svelte";
  import UsageSettings from "./assistant/UsageSettings.svelte";
  import BrowserDataSettings from "./assistant/BrowserDataSettings.svelte";
  import { companionBindings, listConnections, type ConnectionInfo } from "../../../lib/ipc";
  import { usesLegacyVoiceInput } from "../../../lib/ipc/contracts";
  import { settingsStore } from "../../../lib/stores/ui";
  import { COMPANION_SECTIONS, GENERAL_SECTIONS, type AdvancedSection } from "./advanced/types";
  import { m } from "$lib/paraglide/messages.js";

  let {
    section = "system",
    anchor,
    onSectionChange,
    onOpenOnboarding,
  }: {
    section?: AdvancedSection;
    /** 打开后滚到某个分组（聊天里的“去看看”）。 */
    anchor?: { id: string; n: number } | null;
    onSectionChange?: (next: AdvancedSection) => void;
    onOpenOnboarding: () => void;
  } = $props();

  const legacyVoiceOnly = $derived(usesLegacyVoiceInput($settingsStore?.ui));
  const groups = $derived([
    // i18n-ignore-start：只用于 data-testid（测试按它找分组）
    ...(legacyVoiceOnly ? [] : [{ id: "陪玩", label: m.system_advanced_group_companion(), tabs: COMPANION_SECTIONS }]),
    { id: "通用", label: m.system_advanced_group_general(), tabs: GENERAL_SECTIONS },
    // i18n-ignore-end
  ]);
  const name = $derived($companionName);

  let connections = $state<ConnectionInfo[]>([]);
  const ready = $derived(!!connections.find((item) => item.id === companionBindings($settingsStore?.assistant)?.connectionId)?.hasApiKey);
  onMount(() => { void listConnections().then((items) => (connections = items)).catch(() => {}); });
  $effect(() => {
    if (section !== "reply") return;
    $settingsStore?.connections;
    void listConnections().then((items) => (connections = items)).catch(() => {});
  });

  $effect(() => {
    const target = anchor;
    if (!target) return;
    void tick().then(() => requestAnimationFrame(() =>
      document.getElementById(target.id.startsWith("models-") ? "models-category" : target.id)?.scrollIntoView({ block: "start" })));
  });
</script>

{#snippet group(id: string, title: string, description: string)}
  <div id={id} class="scroll-mt-16 border-t border-hairline pt-6">
    <h2 class="m-0 text-[15px] font-semibold text-foreground">{title}</h2>
    <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{description}</p>
  </div>
{/snippet}

<SettingsPage title={m.system_advanced_title()} description={legacyVoiceOnly ? m.system_advanced_description_legacy() : m.system_advanced_description({ name })} testid="advanced-page">
  <nav class="-mt-2 flex flex-col gap-2" aria-label={m.system_advanced_nav_label()}>
    {#each groups as item (item.id)}
      <div class="flex flex-wrap items-center gap-2">
        <span class="min-w-8 shrink-0 whitespace-nowrap text-[11px] font-medium text-muted-foreground" data-testid={`advanced-group-${item.id}`}>{item.label}</span>
        <div class="flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1">
          {#each item.tabs as tab (tab.id)}
            {@const active = section === tab.id}
            <button
              type="button"
              data-testid={`advanced-nav-${tab.id}`}
              aria-current={active ? "page" : undefined}
              class="h-7 rounded-lg px-3 text-xs font-medium transition-colors
                {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
              onclick={() => onSectionChange?.(tab.id)}
            >
              {tab.label}
            </button>
          {/each}
        </div>
      </div>
    {/each}
  </nav>

  <div>
    <!-- 模型清单会校验已下载 ONNX，保持挂载避免切 tab 重复哈希；其它页按需创建 -->
    <div class:hidden={section !== "models"}>
      <ModelsSection focus={anchor?.id.startsWith("models-") ? { category: anchor.id.slice(7), n: anchor.n } : null} />
    </div>
    {#if section === "call"}
      <div class="space-y-6" data-testid="settings-call">
        <div id="settings-call-hotkey" class="scroll-mt-16"><CompanionInputSettings /></div>
        <div id="settings-call-wake" class="scroll-mt-16"><WakeSettings /></div>
        <SettingsSection title={m.system_advanced_call_button_title()} description={m.system_advanced_call_button_description({ name })}>
          <p class="m-0 py-3 text-xs text-muted-foreground">{m.system_advanced_call_button_hint()}</p>
        </SettingsSection>
      </div>
    {:else if section === "reply"}
      <div class="space-y-6" data-testid="settings-reply">
        <div id="settings-reply-ai" class="scroll-mt-16"><AssistantSettings {connections} {ready} onOpenConnections={() => onSectionChange?.("connections")} /></div>
        <div id="settings-reply-bubble" class="scroll-mt-16"><BubbleSettings /></div>
        <div id="settings-reply-output" class="scroll-mt-16"><OutputSettings /></div>
        {@render group("settings-reply-teammates", m.system_advanced_teammates_title(), m.system_advanced_teammates_description())}
        <VoiceChatSettings />
      </div>
    {:else if section === "permissions"}
      <div class="space-y-6" data-testid="settings-permissions"><AgentSettings /><BrowserDataSettings /></div>
    {:else if section === "usage"}
      <div data-testid="settings-usage"><UsageSettings /></div>
    {:else if section === "connections"}
      <ConnectionsSection />
    {:else if section === "sound"}
      <SoundSection />
    {:else if section === "tuning"}
      <TuningSection />
    {:else if section === "system"}
      <SystemSection {onOpenOnboarding} />
    {:else if section === "plugins"}
      <PluginsSection />
    {:else if section === "appearance"}
      <!-- 外观：配色、明暗、缩放、字体（只影响设置窗口，偏好存在本机；ADR 023 从「通用」搬来） -->
      <SettingsSection title={m.system_section_appearance()} description={m.system_advanced_appearance_description()} id="appearance-title">
        <AppearanceSettingsPanel />
      </SettingsSection>
    {/if}
  </div>
</SettingsPage>
