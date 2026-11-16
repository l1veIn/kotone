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
  ];
</script>

<SettingsPage title="高级" description="模型、连接与系统级设置" wide={section === "models" || section === "connections"} testid="advanced-page">
  <nav class="-mt-2 flex w-fit max-w-full flex-wrap gap-0.5 rounded-xl bg-muted/70 p-1" aria-label="高级设置分类">
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
    {/if}
  </div>
</SettingsPage>
