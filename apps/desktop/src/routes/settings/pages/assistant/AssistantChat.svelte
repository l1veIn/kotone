<script lang="ts">
  // 「琴音 → 聊天」：和内置琴音 Agent 对话（ADR 014）。游戏内语音、快捷指令与这里是同一段“当前对话”。
  // 对话列表在侧边栏（AppSidebar）；这里是消息流与输入卡。空白对话时输入卡居中（参照 DeepSeek
  // Harness 的 EmptyHero，MIT），配上琴音形象与起手提示。
  import { CircleAlert, Eye } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { isTauri } from "../../../../lib/ipc";
  import { starterPrompts } from "../../../../lib/agent";
  import { useAgentChat } from "../../../../lib/agentChat/context";
  import { buildTurns } from "../../../../lib/agentChat/timeline";
  import ChatTranscript from "../../../../lib/agentChat/ChatTranscript.svelte";
  import ChatComposer from "../../../../lib/agentChat/ChatComposer.svelte";
  import { appState } from "../../../../lib/stores/state";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import CallKotoneButton from "../../../../lib/components/CallKotoneButton.svelte";
  import KeyRequestCard from "../../../../lib/agentChat/KeyRequestCard.svelte";
  import ComposerOptions from "../../../../lib/agentChat/ComposerOptions.svelte";
  import KotoneAvatar from "../../../../lib/components/KotoneAvatar.svelte";
  import { avatarLetter, customAvatar } from "../../../../lib/stores/avatar";

  let { ready, active, onOpenSettings, onOpenTalk }: {
    /** 陪玩 AI 连接已选好（语音按钮用）。 */
    ready: boolean;
    active: boolean;
    onOpenSettings: () => void;
    /** 「琴音 → 设置 → 呼叫」：呼叫热键、唤醒词。 */
    onOpenTalk: () => void;
  } = $props();

  const chat = useAgentChat();
  let draft = $state("");
  let textarea = $state<HTMLTextAreaElement>();
  let sendError = $state("");
  let sendNotice = $state("");
  let stopping = $state(false);
  const browserPreview = !isTauri;
  const inputBusy = $derived(!["idle", "success", "error"].includes($appState.state));
  const notReady = $derived(chat.state?.notReadyReason ?? null);
  const turns = $derived(buildTurns(chat.conversation?.messages ?? [], chat.runningHere ? chat.live : null, chat.runningHere));
  const empty = $derived(!chat.loading && turns.length === 0);
  /** 没挂到可见工具行上的询问（例如在另一段对话里）：单独列出来，不让它没人回答。 */
  const strayAsks = $derived(chat.asks.filter((ask) =>
    ask.conversationId !== chat.conversation?.id
    || !turns.some((turn) => turn.blocks.some((b) => b.kind === "tool" && b.tool.call.id === ask.request.callId))));

  async function submit(text: string): Promise<boolean> {
    sendError = "";
    sendNotice = "";
    try {
      const status = await chat.send(text);
      if (status === "steered") sendNotice = "已插话：琴音做完当前这一步就会看到。";
      return true;
    } catch (error) {
      sendError = errText(error);
      return false;
    }
  }

  async function stop() {
    stopping = true;
    try { await chat.stop(); }
    catch (error) { toast(false, `停止失败：${errText(error)}`); }
    finally { stopping = false; }
  }

  function starter(text: string) {
    // 以冒号结尾的提示要接着补充，先放进输入框；其余直接发出。
    if (text.endsWith("：") || text.endsWith(":")) {
      draft = text;
      textarea?.focus();
      return;
    }
    if (!inputBusy) void submit(text);
  }
</script>

{#snippet composer()}
  <div class="space-y-2">
    {#each chat.keyRequests as request (request.requestId)}
      <KeyRequestCard {request} />
    {/each}
    {#if strayAsks.length}
      <div class="space-y-2" data-testid="agent-stray-asks">
        {#each strayAsks as ask (ask.askId)}
          <div class="flex flex-wrap items-center gap-2 rounded-2xl border border-warning/45 bg-composer px-4 py-2.5 text-[13px]" data-testid="agent-ask" data-ask-id={ask.askId}>
            <span class="min-w-0 flex-1">琴音需要你确认：<strong class="font-medium">{ask.request.title}</strong>{#if ask.request.reason}<span class="text-muted-foreground"> · {ask.request.reason}</span>{/if}</span>
            <Button size="sm" data-testid="agent-ask-allow-once" onclick={() => void chat.answer(ask.askId, "allow-once")}>允许</Button>
            <Button size="sm" variant="outline" data-testid="agent-ask-allow-conversation" onclick={() => void chat.answer(ask.askId, "allow-for-conversation")}>本段对话都允许</Button>
            <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10" data-testid="agent-ask-deny" onclick={() => void chat.answer(ask.askId, "deny")}>拒绝</Button>
          </div>
        {/each}
      </div>
    {/if}
    {#if chat.conversation?.parent}<p role="note" class="m-0 px-2 text-xs text-muted-foreground" data-testid="agent-child-notice">这是琴音派出的子会话「{chat.conversation.label ?? ""}」的记录，只能查看；在这里发消息会开始一段新对话。</p>{/if}
    {#if sendError}<p role="alert" class="m-0 px-2 text-xs text-destructive" data-testid="agent-send-error">{sendError}</p>{/if}
    {#if sendNotice}<p role="status" class="m-0 px-2 text-xs text-primary" data-testid="agent-send-notice">{sendNotice}</p>{/if}
    <ChatComposer
      bind:value={draft}
      bind:textarea
      disabled={inputBusy}
      blockedReason={notReady ? "琴音还没准备好" : ""}
      running={chat.running}
      {stopping}
      onsubmit={submit}
      onstop={() => void stop()}
    >
      {#snippet tools()}
        <CallKotoneButton {ready} />
      {/snippet}
      {#snippet options()}
        <ComposerOptions onchanged={() => void chat.refresh()} />
      {/snippet}
    </ChatComposer>
  </div>
{/snippet}

<div class="ui-scope flex h-full min-h-0 flex-col text-foreground" data-testid="assistant-chat">
  {#if browserPreview || notReady || chat.loadError}
    <div class="mx-auto w-full max-w-[808px] space-y-2 px-6 pt-4">
      {#if browserPreview}
        <p role="status" class="m-0 flex items-center gap-2 rounded-xl border border-warning/30 bg-warning/8 px-3.5 py-2 text-xs text-warning" data-testid="agent-preview-banner">
          <Eye class="size-3.5 shrink-0" />界面预览：显示的是示例对话，不会真的调用 AI。
        </p>
      {/if}
      {#if notReady}
        <div class="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-destructive/35 bg-destructive/8 px-3.5 py-2.5 text-[13px]" data-testid="agent-not-ready">
          <CircleAlert class="size-4 shrink-0 text-destructive" />
          <span class="font-medium text-destructive">琴音还没准备好</span>
          <span class="min-w-0 flex-1 text-foreground/80">{notReady}</span>
          <Button variant="link" size="sm" class="h-auto p-0" data-testid="agent-open-settings" onclick={onOpenSettings}>前往设置 →</Button>
        </div>
      {/if}
      {#if chat.loadError}<p role="alert" class="m-0 text-xs text-destructive">{chat.loadError}</p>{/if}
    </div>
  {/if}

  {#if empty}
    <!-- 空白对话：琴音形象 + 问候 + 居中的输入卡 + 起手提示 -->
    <div class="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto px-6 py-8" data-testid="agent-empty">
      <div class="w-full max-w-[680px]">
        <div class="mb-6 flex flex-col items-center text-center">
          <div class="mb-4 shadow-[0_0_40px_-8px_var(--color-primary)] rounded-full"><KotoneAvatar size={80} mood="asking" custom={$customAvatar} letter={$avatarLetter} /></div>
          <h2 class="m-0 text-2xl font-semibold tracking-tight">想让{$settingsStore?.agent?.identity?.name?.trim() || "琴音"}做点什么？</h2>
          <p class="mt-2 mb-0 text-[13px] text-muted-foreground">做自动化、看游戏聊天、设提醒……在游戏里呼叫她说的话也会出现在这里。</p>
        </div>
        {@render composer()}
        <div class="mt-4 flex flex-wrap justify-center gap-2" data-testid="agent-starters">
          {#each starterPrompts as prompt (prompt.label)}
            <button type="button" data-testid="agent-starter" disabled={inputBusy} onclick={() => starter(prompt.text)}
              class="h-8 rounded-full border border-hairline bg-card/50 px-3.5 text-[13px] text-foreground/85 transition-colors hover:border-primary/45 hover:text-primary disabled:opacity-50">{prompt.label}</button>
          {/each}
        </div>
      </div>
    </div>
  {:else}
    <ChatTranscript
      conversationKey={chat.conversation?.id ?? "new"}
      {turns}
      askFor={(callId) => chat.askFor(callId)}
      onAnswer={(askId, answer) => chat.answer(askId, answer)}
    />
    <div class="mx-auto w-full max-w-[840px] shrink-0 px-6 pb-4">
      {@render composer()}
    </div>
  {/if}
</div>
