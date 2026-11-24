<script lang="ts">
  /*
   * 「设置」（ADR 028）：一排分页，模型 · API 连接 · 声音 · 调参 · 外观 · 系统 · 浏览器（原来的通用组）。
   * 原来的陪玩组（怎么叫她 · 她怎么回答 · 权限 · 用量）搬进了角色页，叫 唤醒 · 交互 · 权限 · 用量（交接第十一节第 8 条，2026-11-25）；
   * 只剩一组，就不再写组名。说明下面一行链接指过去，找老位置的人一眼看得到。
   * 判断标准照旧：“如果删掉角色，这项设置还需要吗？”需要的留在设置里。
   */
  import { tick } from "svelte";
  import { companionName } from "$lib/stores/companion";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import AppearanceSettingsPanel from "$lib/appearance/AppearanceSettingsPanel.svelte";
  import ConnectionsSection from "./advanced/ConnectionsSection.svelte";
  import ModelsSection from "./advanced/ModelsSection.svelte";
  import SoundSection from "./advanced/SoundSection.svelte";
  import SystemSection from "./advanced/SystemSection.svelte";
  import TuningSection from "./advanced/TuningSection.svelte";
  import BrowserDataSettings from "./assistant/BrowserDataSettings.svelte";
  import { usesLegacyVoiceInput } from "../../../lib/ipc/contracts";
  import { settingsStore } from "../../../lib/stores/ui";
  import { GENERAL_SECTIONS, type AdvancedSection } from "./advanced/types";
  import { m } from "$lib/paraglide/messages.js";

  let {
    section = "models",
    anchor,
    onSectionChange,
    onOpenOnboarding,
    onOpenCompanion,
  }: {
    section?: AdvancedSection;
    /** 打开后滚到某个分组（聊天里的“去看看”）。 */
    anchor?: { id: string; n: number } | null;
    onSectionChange?: (next: AdvancedSection) => void;
    onOpenOnboarding: () => void;
    /** 角色页的唤醒（原「设置 → 怎么叫她」）。 */
    onOpenCompanion?: () => void;
  } = $props();

  const legacyVoiceOnly = $derived(usesLegacyVoiceInput($settingsStore?.ui));
  const name = $derived($companionName);

  $effect(() => {
    const target = anchor;
    if (!target) return;
    void tick().then(() => requestAnimationFrame(() =>
      document.getElementById(target.id.startsWith("models-") ? "models-category" : target.id)?.scrollIntoView({ block: "start" })));
  });
</script>

<SettingsPage title={m.system_advanced_title()} description={legacyVoiceOnly ? m.system_advanced_description_legacy() : m.system_advanced_description({ name })} testid="advanced-page">
  <div class="-mt-3 flex flex-col gap-3">
    {#if !legacyVoiceOnly && onOpenCompanion}
      <!-- 原来陪玩那一排搬走了：指一下新位置 -->
      <p class="m-0 text-xs text-muted-foreground" data-testid="advanced-companion-moved">
        {m.system_advanced_companion_moved({ name })}<button type="button" class="ml-1 font-medium text-primary hover:underline" data-testid="advanced-open-companion"
          onclick={onOpenCompanion}>{m.system_advanced_companion_open({ name })}</button>
      </p>
    {/if}
    <nav class="flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1" aria-label={m.system_advanced_nav_label()} data-testid="advanced-nav">
      {#each GENERAL_SECTIONS as tab (tab.id)}
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
    </nav>
  </div>

  <div>
    <!-- 模型清单会校验已下载 ONNX，保持挂载避免切 tab 重复哈希；其它页按需创建 -->
    <div class:hidden={section !== "models"}>
      <ModelsSection focus={anchor?.id.startsWith("models-") ? { category: anchor.id.slice(7), n: anchor.n } : null} />
    </div>
    {#if section === "connections"}
      <ConnectionsSection />
    {:else if section === "sound"}
      <SoundSection />
    {:else if section === "tuning"}
      <TuningSection />
    {:else if section === "system"}
      <SystemSection {onOpenOnboarding} />
    {:else if section === "browser"}
      <!-- 内置浏览器（ADR 031；开关放在设置里，ADR 038）：开关、清除登录、扩展 -->
      <div class="space-y-6" data-testid="settings-browser"><BrowserDataSettings /></div>
    {:else if section === "appearance"}
      <!-- 外观：配色、明暗、缩放、字体（只影响设置窗口，偏好存在本机；ADR 023 从「通用」搬来） -->
      <SettingsSection title={m.system_section_appearance()} description={m.system_advanced_appearance_description()} id="appearance-title">
        <AppearanceSettingsPanel />
      </SettingsSection>
    {/if}
  </div>
</SettingsPage>
