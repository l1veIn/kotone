<script lang="ts">
  import type { AssistantInteractionMode } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";

  let saving = $state(false);
  const mode = $derived($settingsStore?.assistant.interactionMode ?? "push-to-talk");
  const modes: { id: AssistantInteractionMode; name: string; description: string }[] = [
    { id: "push-to-talk", name: "对讲机", description: "按住说话，松开提交" },
    { id: "dictation", name: "录音笔", description: "点按录音，再点停止并确认" },
    { id: "one-shot", name: "说一句就走", description: "点按录音，停顿后提交" },
  ];

  async function select(interactionMode: AssistantInteractionMode) {
    if (saving || interactionMode === mode) return;
    saving = true;
    try { await patchSettings({ assistant: { interactionMode } }, "助手交互模式已保存"); }
    finally { saving = false; }
  }
</script>

<div class="mt-5 border-t border-white/8 pt-4" data-testid="assistant-interaction">
  <h3>输入方式</h3>
  <div class="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3" aria-label="助手输入方式">
    {#each modes as item}
      {@const active = mode === item.id}
      <button type="button" data-testid={`assistant-mode-${item.id}`} aria-pressed={active} disabled={saving}
        class="rounded-[var(--radius-kotone-card)] px-3 py-3 text-left ring-1 transition
          {active ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan' : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
        onclick={() => void select(item.id)}>
        <p class="text-sm font-semibold {active ? 'text-kotone-cyan' : ''}">{item.name}</p>
        <p class="mt-1 text-[11px] leading-relaxed text-white/50">{item.description}</p>
      </button>
    {/each}
  </div>
</div>
