<script lang="ts">
  /*
   * 高级页壳：二级导航 + 子页。
   * 模型 / API 连接 / 热键 / 调参 / 系统拆在 ./advanced/。
   */
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

<div class="mx-auto max-w-6xl px-4 py-5 sm:px-6">
  <h1 class="text-lg font-semibold">设置</h1>

  <nav class="mt-4 flex flex-wrap gap-1.5 border-b border-white/8 pb-2" aria-label="设置分组">
    {#each tabs as tab (tab.id)}
      {@const active = section === tab.id}
      <button
        type="button"
        aria-current={active ? "page" : undefined}
        data-testid={`advanced-nav-${tab.id}`}
        class="rounded-lg px-3 py-1.5 text-xs font-medium transition-colors focus-visible:ring-2 focus-visible:ring-kotone-cyan/60
          {active
            ? 'bg-kotone-cyan/10 text-kotone-cyan'
            : 'text-white/65 hover:bg-white/8 hover:text-white/90'}"
        onclick={() => onSectionChange?.(tab.id)}
      >
        {tab.label}
      </button>
    {/each}
  </nav>

  <div class="mt-4">
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
</div>
