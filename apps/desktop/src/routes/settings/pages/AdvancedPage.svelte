<script lang="ts">
  /*
   * 高级页壳：分段导航 + 子页（DeepSeek Harness 风格的设置页骨架）。
   * 模型 / API 连接 / 热键 / 调参 / 系统拆在 ./advanced/。
   */
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import ConnectionsSection from "./advanced/ConnectionsSection.svelte";
  import HotkeysSection from "./advanced/HotkeysSection.svelte";
  import ModelsSection from "./advanced/ModelsSection.svelte";
  import SoundSection from "./advanced/SoundSection.svelte";
  import SystemSection from "./advanced/SystemSection.svelte";
  import TuningSection from "./advanced/TuningSection.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import AppearanceSettingsPanel from "$lib/appearance/AppearanceSettingsPanel.svelte";
  import type { AdvancedSection } from "./advanced/types";

  let {
    section = "system",
    onSectionChange,
    onOpenOnboarding,
  }: {
    section?: AdvancedSection;
    onSectionChange?: (next: AdvancedSection) => void;
    onOpenOnboarding: () => void;
  } = $props();

  const tabs: { id: AdvancedSection; label: string }[] = [
    { id: "system", label: "系统" },
    { id: "models", label: "模型" },
    { id: "connections", label: "API 连接" },
    { id: "hotkeys", label: "其他热键" },
    { id: "sound", label: "音效" },
    { id: "tuning", label: "调参" },
    { id: "appearance", label: "外观" },
  ];
</script>

<SettingsPage title="设置" description="模型、AI 连接、外观与系统级设置" testid="advanced-page">
  <nav class="-mt-2 flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1" aria-label="设置分类">
    {#each tabs as tab}
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

  <div>
    <!-- 模型清单会校验已下载 ONNX，保持挂载避免切 tab 重复哈希；其它页按需创建 -->
    <div class:hidden={section !== "models"}>
      <ModelsSection />
    </div>
    {#if section === "connections"}
      <ConnectionsSection />
    {:else if section === "hotkeys"}
      <HotkeysSection />
    {:else if section === "sound"}
      <SoundSection />
    {:else if section === "tuning"}
      <TuningSection />
    {:else if section === "system"}
      <SystemSection {onOpenOnboarding} />
    {:else if section === "appearance"}
      <!-- 外观：配色、明暗、缩放、字体（只影响设置窗口，偏好存在本机；ADR 023 从「通用」搬来） -->
      <SettingsSection title="外观" description="只影响设置窗口；游戏里的悬浮窗与弹幕保持原样。" id="appearance-title">
        <AppearanceSettingsPanel />
      </SettingsSection>
    {/if}
  </div>
</SettingsPage>
