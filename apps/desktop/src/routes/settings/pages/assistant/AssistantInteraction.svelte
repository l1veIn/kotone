<script lang="ts">
  // 陪玩热键的输入方式（与常规语音输入的交互模式相互独立）。
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

<div class="py-3.5" data-testid="assistant-interaction">
  <p class="m-0 text-[13px] font-medium text-foreground">输入方式</p>
  <p class="mt-0.5 mb-0 text-xs text-muted-foreground">只影响陪玩热键，不改变常规语音输入。</p>
  <div class="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3" aria-label="助手输入方式">
    {#each modes as item}
      {@const active = mode === item.id}
      <button type="button" data-testid={`assistant-mode-${item.id}`} aria-pressed={active} disabled={saving}
        class="rounded-xl border px-3 py-2.5 text-left transition-colors hover:bg-muted/60 disabled:opacity-60 {active ? 'border-primary bg-primary/8' : 'border-hairline'}"
        onclick={() => void select(item.id)}>
        <p class="m-0 text-[13px] font-semibold {active ? 'text-primary' : 'text-foreground'}">{item.name}</p>
        <p class="mt-1 mb-0 text-[11px] leading-relaxed text-muted-foreground">{item.description}</p>
      </button>
    {/each}
  </div>
</div>
