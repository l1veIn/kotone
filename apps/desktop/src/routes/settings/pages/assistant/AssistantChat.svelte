<script lang="ts">
  // 「琴音 → 聊天」：和内置琴音 Agent 对话（ADR 014）。游戏内语音、快捷指令与这里是同一段“当前对话”。
  import { onMount } from "svelte";
  import { ChevronDown, CircleAlert, MessageSquarePlus, Sparkles } from "@lucide/svelte";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import { isTauri } from "../../../../lib/ipc";
  import { starterPrompts } from "../../../../lib/agent";
  import { AgentChat } from "../../../../lib/agentChat/chatController.svelte";
  import { buildTurns } from "../../../../lib/agentChat/timeline";
  import ChatTranscript from "../../../../lib/agentChat/ChatTranscript.svelte";
  import ChatComposer from "../../../../lib/agentChat/ChatComposer.svelte";
  import ConversationList from "../../../../lib/agentChat/ConversationList.svelte";
  import { appState } from "../../../../lib/stores/state";
  import { errText, toast } from "../../../../lib/stores/ui";
  import AssistantVoiceInput from "./AssistantVoiceInput.svelte";
  import OpenRunsSummary from "./OpenRunsSummary.svelte";

  let { ready, active, onOpenSettings, onOpenAutomation }: {
    /** 陪玩 AI 连接已选好（语音按钮用）。 */
    ready: boolean;
    active: boolean;
    onOpenSettings: () => void;
    onOpenAutomation: () => void;
  } = $props();

  const chat = new AgentChat();
  let draft = $state("");
  let textarea = $state<HTMLTextAreaElement>();
  let sendError = $state("");
  let sendNotice = $state("");
  let stopping = $state(false);
  let voiceBusy = $state(false);
  const browserPreview = !isTauri;
  const inputBusy = $derived(voiceBusy || !["idle", "success", "error"].includes($appState.state));
  const notReady = $derived(chat.state?.notReadyReason ?? null);
  const turns = $derived(buildTurns(chat.conversation?.messages ?? [], chat.runningHere ? chat.live : null, chat.runningHere));
  /** 没挂到可见工具卡片上的询问（例如在另一段对话里）：单独列出来，不让它没人回答。 */
  const strayAsks = $derived(chat.asks.filter((ask) =>
    ask.conversationId !== chat.conversation?.id
    || !turns.some((turn) => turn.blocks.some((b) => b.kind === "tool" && b.tool.call.id === ask.request.callId))));

  onMount(() => {
    void chat.init();
    return () => chat.dispose();
  });

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

  async function guarded(action: () => Promise<void>, failure: string) {
    try { await action(); sendError = ""; sendNotice = ""; }
    catch (error) { toast(false, `${failure}：${errText(error)}`); }
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

<div class="ui-scope @container flex h-full min-h-[26rem] overflow-hidden rounded-xl border bg-background/60 text-foreground" data-testid="assistant-chat">
  <!-- 窄窗口（设置窗口最小 640px）时收起对话列表，改用标题旁的下拉菜单。 -->
  <div class="hidden h-full @xl:block">
  <ConversationList
    conversations={chat.state?.conversations ?? []}
    currentId={chat.currentId}
    running={chat.running}
    onNew={() => void guarded(() => chat.newConversation(), "新建对话失败")}
    onSelect={(id) => void guarded(() => chat.select(id), "切换对话失败")}
    onDelete={(id) => guarded(() => chat.remove(id), "删除对话失败")}
  />
  </div>

  <div class="flex min-w-0 flex-1 flex-col">
    <header class="flex items-center justify-between gap-3 border-b px-4 py-2">
      <div class="flex min-w-0 items-center gap-1">
        <p class="m-0 min-w-0 truncate text-sm font-medium" data-testid="agent-conversation-title">
          {chat.conversation?.title || "新对话"}
          {#if chat.runningHere}<span class="ml-2 text-xs font-normal text-primary" role="status">回答中…</span>{/if}
        </p>
        <div class="@xl:hidden">
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props })}
                <Button {...props} variant="ghost" size="icon-xs" aria-label="切换对话" data-testid="agent-conversation-menu"><ChevronDown /></Button>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="start" class="ui-scope max-h-80 w-60">
              <DropdownMenu.Item disabled={chat.running || chat.currentId === null} onSelect={() => void guarded(() => chat.newConversation(), "新建对话失败")}>
                <MessageSquarePlus />新对话
              </DropdownMenu.Item>
              <DropdownMenu.Separator />
              {#each chat.state?.conversations ?? [] as item (item.id)}
                <DropdownMenu.Item disabled={chat.running || item.id === chat.currentId} onSelect={() => void guarded(() => chat.select(item.id), "切换对话失败")}>
                  <span class="truncate">{item.title || "（无标题）"}</span>
                </DropdownMenu.Item>
              {/each}
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </div>
      </div>
      <OpenRunsSummary {onOpenAutomation} />
    </header>

    {#if browserPreview}
      <p role="status" class="m-0 border-b border-warning/30 bg-warning/10 px-4 py-1.5 text-xs text-warning" data-testid="agent-preview-banner">
        界面预览：显示的是示例对话，不会真的调用 AI。
      </p>
    {/if}
    {#if notReady}
      <div class="border-b px-4 py-2">
        <Alert.Root variant="destructive" data-testid="agent-not-ready">
          <CircleAlert />
          <Alert.Title>琴音还没准备好</Alert.Title>
          <Alert.Description>
            <span>{notReady}</span>
            <Button variant="link" size="sm" class="h-auto p-0" data-testid="agent-open-settings" onclick={onOpenSettings}>前往设置 →</Button>
          </Alert.Description>
        </Alert.Root>
      </div>
    {/if}
    {#if chat.loadError}<p role="alert" class="m-0 border-b px-4 py-1.5 text-xs text-destructive">{chat.loadError}</p>{/if}

    <ChatTranscript
      conversationKey={chat.conversation?.id ?? "new"}
      {turns}
      askFor={(callId) => chat.askFor(callId)}
      onAnswer={(askId, answer) => chat.answer(askId, answer)}
    >
      {#snippet empty()}
        {#if !chat.loading}
          <div class="mx-auto flex h-full max-w-md flex-col items-center justify-center py-8 text-center" data-testid="agent-empty">
            <Sparkles class="mb-3 size-6 text-primary" />
            <p class="m-0 text-sm font-medium">想让琴音做点什么？</p>
            <p class="mt-1.5 mb-4 text-xs text-muted-foreground">做自动化、看游戏聊天、设提醒……游戏里按陪玩热键说的话也会出现在这里。</p>
            <div class="flex flex-wrap justify-center gap-2" data-testid="agent-starters">
              {#each starterPrompts as prompt (prompt.label)}
                <Button variant="outline" size="sm" data-testid="agent-starter" disabled={inputBusy} onclick={() => starter(prompt.text)}>{prompt.label}</Button>
              {/each}
            </div>
          </div>
        {/if}
      {/snippet}
    </ChatTranscript>

    {#if strayAsks.length}
      <div class="space-y-2 border-t px-4 py-2" data-testid="agent-stray-asks">
        {#each strayAsks as ask (ask.askId)}
          <div class="flex flex-wrap items-center gap-2 text-xs" data-testid="agent-ask" data-ask-id={ask.askId}>
            <span class="min-w-0 flex-1">琴音需要你确认：<strong>{ask.request.title}</strong>{#if ask.request.reason}<span class="text-muted-foreground"> · {ask.request.reason}</span>{/if}</span>
            <Button size="sm" data-testid="agent-ask-allow-once" onclick={() => void chat.answer(ask.askId, "allow-once")}>允许</Button>
            <Button size="sm" variant="secondary" data-testid="agent-ask-allow-conversation" onclick={() => void chat.answer(ask.askId, "allow-for-conversation")}>本段对话都允许</Button>
            <Button size="sm" variant="destructive" data-testid="agent-ask-deny" onclick={() => void chat.answer(ask.askId, "deny")}>拒绝</Button>
          </div>
        {/each}
      </div>
    {/if}

    <div class="border-t px-4 pt-2 pb-3">
      {#if sendError}<p role="alert" class="mt-0 mb-1.5 text-xs text-destructive" data-testid="agent-send-error">{sendError}</p>{/if}
      {#if sendNotice}<p role="status" class="mt-0 mb-1.5 text-xs text-primary" data-testid="agent-send-notice">{sendNotice}</p>{/if}
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
          <AssistantVoiceInput {ready} {active} submitting={false} {onOpenSettings} onBusyChange={(busy) => { voiceBusy = busy; }} />
        {/snippet}
      </ChatComposer>
    </div>
  </div>
</div>
