<script lang="ts">
  /*
   * 语音输入子板块（ADR 028 W8）：一个侧栏入口，三个分页对应原来的页面 ID——
   * 输入 = general、文字处理 = processing、历史 = history。说话、原话打进游戏聊天框，不经过角色（Agent）。
   * 历史只有语音输入的记录，和角色的对话列表分开。
   */
  import type { Snippet } from "svelte";
  import { companionName } from "$lib/stores/companion";
  import { Button } from "$lib/components/ui/button";
  import { Sparkles } from "@lucide/svelte";
  import type { VoiceInputPage } from "../../../../lib/stores/navigation";
  import { m } from "$lib/paraglide/messages.js";

  let { page, onTab, onConfigure, children }: {
    page: VoiceInputPage;
    onTab: (page: VoiceInputPage) => void;
    /** 「让 Kotone 帮我配置」。 */
    onConfigure: () => void;
    children: Snippet;
  } = $props();

  const tabs: { id: VoiceInputPage; label: string }[] = [
    { id: "general", label: m.voice_tab_input() }, { id: "processing", label: m.voice_tab_processing() }, { id: "history", label: m.voice_tab_history() },
  ];
</script>

<div data-testid="voice-input">
  <div class="ui-scope mx-auto w-full max-w-3xl px-6 pt-7 text-foreground">
    <header class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
      <div class="min-w-0">
        <h1 class="m-0 text-xl font-semibold tracking-tight">{m.voice_shell_title()}</h1>
        <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted-foreground">{m.voice_shell_subtitle({ name: $companionName })}</p>
      </div>
      <Button size="sm" variant="outline" data-testid="voice-configure" onclick={onConfigure}><Sparkles class="size-3.5 text-primary" />{m.voice_shell_configure()}</Button>
    </header>
    <nav class="mt-4 flex w-fit gap-0.5 rounded-xl bg-muted/70 p-1" aria-label={m.voice_shell_tabs_label()}>
      {#each tabs as tab (tab.id)}
        {@const active = page === tab.id}
        <button type="button" data-testid={`voice-tab-${tab.id}`} aria-current={active ? "page" : undefined}
          class="h-7 rounded-lg px-3 text-xs font-medium transition-colors {active ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => onTab(tab.id)}>{tab.label}</button>
      {/each}
    </nav>
  </div>
  {@render children()}
</div>
