<script lang="ts">
  import type { AssistantInteractionMode } from "../../../../lib/ipc";
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let saving = $state(false);
  const mode = $derived($settingsStore?.assistant.interactionMode ?? "push-to-talk");
  const modes: { id: AssistantInteractionMode; name: string; description: string }[] = [
    { id: "push-to-talk", name: m.companion_interaction_ptt_name(), description: m.companion_interaction_ptt_desc() },
    { id: "dictation", name: m.companion_interaction_dictation_name(), description: m.companion_interaction_dictation_desc() },
    { id: "one-shot", name: m.companion_interaction_one_shot_name(), description: m.companion_interaction_one_shot_desc() },
  ];

  async function select(interactionMode: AssistantInteractionMode) {
    if (saving || interactionMode === mode) return;
    saving = true;
    try { await patchSettings({ assistant: { interactionMode } }, m.companion_interaction_saved()); }
    finally { saving = false; }
  }
</script>

<div class="mt-5 border-t border-white/8 pt-4" data-testid="assistant-interaction">
  <h3>{m.companion_interaction_title()}</h3>
  <div class="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3" aria-label={m.companion_interaction_aria()}>
    {#each modes as item}
      {@const active = mode === item.id}
      <button type="button" data-testid={`assistant-mode-${item.id}`} aria-pressed={active} disabled={saving}
        class="rounded-[var(--radius-kotone-card)] px-3 py-3 text-left ring-1 transition
          {active ? 'bg-kotone-cyan/12 ring-kotone-cyan/60 shadow-glow-cyan' : 'bg-kotone-card/60 ring-white/10 hover:bg-kotone-card'}"
        onclick={() => void select(item.id)}>
        <p class="text-sm font-semibold {active ? 'text-primary' : ''}">{item.name}</p>
        <p class="mt-1 text-[11px] leading-relaxed text-foreground/50">{item.description}</p>
      </button>
    {/each}
  </div>
</div>
