<script lang="ts">
  /*
   * 高级页壳：二级导航 + 子页。
   * 模型 / API 连接 / 热键 / 调参 / 系统拆在 ./advanced/。
   */
  import ConnectionsSection from "./advanced/ConnectionsSection.svelte";
  import HotkeysSection from "./advanced/HotkeysSection.svelte";
  import ModelsSection from "./advanced/ModelsSection.svelte";
  import SystemSection from "./advanced/SystemSection.svelte";
  import TuningSection from "./advanced/TuningSection.svelte";
  import type { AdvancedSection } from "./advanced/types";

  let {
    section = "models",
    onSectionChange,
    onOpenOnboarding,
  }: {
    section?: AdvancedSection;
    onSectionChange?: (next: AdvancedSection) => void;
    onOpenOnboarding: () => void;
  } = $props();

  const tabs: { id: AdvancedSection; label: string }[] = [
    { id: "models", label: "模型" },
    { id: "connections", label: "API 连接" },
    { id: "hotkeys", label: "热键" },
    { id: "tuning", label: "调参" },
    { id: "system", label: "系统" },
  ];
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">高级</h1>
  <p class="mt-0.5 text-[11px] text-white/45">模型、连接与系统级设置</p>

  <nav class="mt-4 flex flex-wrap gap-1.5" aria-label="高级设置分类">
    {#each tabs as tab}
      {@const active = section === tab.id}
      <button
        data-testid={`advanced-nav-${tab.id}`}
        class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition
          {active
            ? 'bg-kotone-cyan/15 text-kotone-cyan ring-kotone-cyan/40'
            : 'bg-white/5 text-white/55 ring-white/10 hover:bg-white/10 hover:text-white/80'}"
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
    {:else if section === "tuning"}
      <TuningSection />
    {:else if section === "system"}
      <SystemSection {onOpenOnboarding} />
    {/if}
  </div>
</div>
