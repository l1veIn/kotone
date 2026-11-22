<script lang="ts" module>
  // 输入框草稿：她的设置搬进了「设置」（ADR 028），去改个设置再回来，草稿还在。
  let keptDraft = "";
</script>

<script lang="ts">
  // 聊天页（ADR 014 / 028，主界面）：和角色 Agent 对话。游戏内语音、快捷指令与这里是同一段“当前对话”。
  // 对话列表在侧边栏（AppSidebar）；这里是消息流与输入卡。空白对话时输入卡居中（参照 DeepSeek
  // Harness 的 EmptyHero，MIT），配上角色形象、呼叫按钮与起手提示。三种状态（ADR 028 W5–W7）：
  // 没有 AI 连接 → Key 卡片（和引导里填 Key 的是同一个组件，照 DSH“回到空白会话就检查有没有 Key”）；
  // 已配置 → 呼叫按钮下面写当前的触发方式；用本地识别且没启动 → 呼叫按钮上方一行状态。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
  import { CircleAlert, Eye } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { isTauri } from "../../../../lib/ipc";
  import { starterPrompts, usageConversation, type ConversationCosts } from "../../../../lib/agent";
  import { useAgentChat } from "../../../../lib/agentChat/context";
  import { buildTurns, withCosts } from "../../../../lib/agentChat/timeline";
  import ChatTranscript from "../../../../lib/agentChat/ChatTranscript.svelte";
  import ChatComposer from "../../../../lib/agentChat/ChatComposer.svelte";
  import { appState } from "../../../../lib/stores/state";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import CallKotoneButton from "../../../../lib/components/CallKotoneButton.svelte";
  import KeyRequestCard from "../../../../lib/agentChat/KeyRequestCard.svelte";
  import PermissionMenu from "../../../../lib/agentChat/PermissionMenu.svelte";
  import UsagePanel from "../../../../lib/agentChat/UsagePanel.svelte";
  import { pendingPrompt } from "../../../../lib/agentChat/pendingPrompt";
  import DeepSeekKeyForm from "../../../../lib/components/DeepSeekKeyForm.svelte";
  import type { DeepSeekSetupResult } from "../../../../lib/deepseekSetup";
  import { startRuntime } from "../../../../lib/ipc";
  import { LOCAL_ASR_LABELS, companionBindings } from "../../../../lib/ipc/contracts";
  import { untrack } from "svelte";
  import { asrStore } from "../../../../lib/stores/runtime";
  import KotoneAvatar from "../../../../lib/components/KotoneAvatar.svelte";
  import { avatarLetter, customAvatar, roleFaces } from "../../../../lib/stores/avatar";
  import RightSidebar from "../../../../lib/agentChat/RightSidebar.svelte";
  import { chatNarrow, RIGHT_SIDEBAR_MIN_CHAT } from "../../../../lib/stores/rightSidebar";

  let { ready, active, legacy = false, onOpenSettings, onOpenTalk, onOpenConnections, onKeyConnected, onBackToVoice }: {
    /** 角色的 AI 连接已选好、有密钥。 */
    ready: boolean;
    active: boolean;
    /** 0.1.x 升级后还没启用角色的用户（逃生通道）：Key 卡片下多一行“回到语音输入”。 */
    legacy?: boolean;
    onOpenSettings: () => void;
    /** 「设置 → 怎么叫她」：呼叫键、唤醒词。 */
    onOpenTalk: () => void;
    onOpenConnections: () => void;
    /** Key 卡片连好了：接着设呼叫键（引导的“已连接”页）。 */
    onKeyConnected: (result: DeepSeekSetupResult) => void;
    onBackToVoice: () => void;
  } = $props();

  const chat = useAgentChat();
  let draft = $state(keptDraft);
  $effect(() => { keptDraft = draft; });
  let textarea = $state<HTMLTextAreaElement>();
  let sendError = $state("");
  let sendNotice = $state("");
  let stopping = $state(false);
  const browserPreview = !isTauri;
  const inputBusy = $derived(!["idle", "success", "error"].includes($appState.state));
  /** 有 Key 但 Agent 仍报没准备好（例如选的模型不对）：顶部一条说明。没有 Key 时显示 Key 卡片。 */
  const notReady = $derived(ready ? (chat.state?.notReadyReason ?? null) : null);
  // 连接或角色的 AI 设置变了（填 Key、换连接、0.1.x 用户启用角色）：重新读一次 Agent 的状态，
  // 否则“还没准备好”会停在打开软件时读到的那一刻（2026-11-20 真机：填完 Key 进聊天发不出去）。
  const companionConnection = $derived(companionBindings($settingsStore?.assistant)?.connectionId ?? "");
  $effect(() => {
    companionConnection;
    $settingsStore?.connections;
    ready;
    untrack(() => { if (chat.state) void chat.refresh().catch(() => {}); });
  });
  const asr = $derived($asrStore);
  const localPending = $derived(asr === "local-stopped" || asr === "local-starting" || asr === "local-error");
  let startingAsr = $state(false);
  async function startLocal() {
    if (startingAsr) return;
    startingAsr = true;
    try { await startRuntime(); }
    catch (error) { toast(false, m.chat_local_asr_start_failed({ error: errText(error) })); }
    finally { startingAsr = false; }
  }
  // 这段对话里模型以外的花费（语音、读屏、搜索；本机账本）：答完读一次。回答是答完才念的，
  // 念的费用晚几秒才记下，所以过一会儿再读两次。
  let costs = $state<ConversationCosts | null>(null);
  $effect(() => {
    const id = chat.conversation?.id;
    const since = chat.conversation?.createdAtMs ?? 0;
    const running = chat.runningHere;
    void chat.conversation?.messages.length;
    untrack(() => { if (costs && costs.conversationId !== id) costs = null; });
    if (!id || running) return;
    let alive = true;
    const load = () => usageConversation(id, since)
      .then((c) => { if (alive) costs = c?.conversationId === id ? c : null; })
      .catch(() => {});
    void load();
    const timers = [setTimeout(load, 6_000), setTimeout(load, 20_000)];
    return () => { alive = false; timers.forEach(clearTimeout); };
  });
  const turns = $derived(withCosts(buildTurns(chat.conversation?.messages ?? [], chat.runningHere ? chat.live : null, chat.runningHere), costs));
  // 逃生通道用户还没启用角色，对话存储没打开（ADR 027 静默）：直接按空白对话显示 Key 卡片。
  const empty = $derived((!chat.loading || legacy) && turns.length === 0);
  /** 没挂到可见工具行上的询问（例如在另一段对话里）：单独列出来，不让它没人回答。 */
  const strayAsks = $derived(chat.asks.filter((ask) =>
    ask.conversationId !== chat.conversation?.id
    || !turns.some((turn) => turn.blocks.some((b) => b.kind === "tool" && b.tool.call.id === ask.request.callId))));

  async function submit(text: string): Promise<boolean> {
    sendError = "";
    sendNotice = "";
    try {
      const status = await chat.send(text);
      if (status === "steered") sendNotice = m.chat_send_steered();
      return true;
    } catch (error) {
      sendError = errText(error);
      return false;
    }
  }

  async function stop() {
    stopping = true;
    try { await chat.stop(); }
    catch (error) { toast(false, m.chat_stop_request_failed({ error: errText(error) })); }
    finally { stopping = false; }
  }

  // 引导结束后放好“你能做什么？”等：只预填进输入框，玩家自己点发送（D2）。
  $effect(() => {
    const text = $pendingPrompt;
    if (!text || !ready) return;
    pendingPrompt.set(null);
    draft = text;
    queueMicrotask(() => textarea?.focus());
  });

  // 右侧栏（ADR 031）：聊天页够宽时并排，窄了盖在聊天上、默认收起。宽度量的是整个聊天页。
  let chatWidth = $state(0);
  const narrow = $derived(chatWidth > 0 && chatWidth < RIGHT_SIDEBAR_MIN_CHAT);
  $effect(() => { chatNarrow.set(narrow); });

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
            <span class="min-w-0 flex-1">{m.chat_ask_confirm_prefix()}<strong class="font-medium">{ask.request.title}</strong>{#if ask.request.reason}<span class="text-muted-foreground"> · {ask.request.reason}</span>{/if}</span>
            <Button size="sm" data-testid="agent-ask-allow-once" onclick={() => void chat.answer(ask.askId, "allow-once")}>{m.chat_ask_allow_once()}</Button>
            <Button size="sm" variant="outline" data-testid="agent-ask-allow-conversation" onclick={() => void chat.answer(ask.askId, "allow-for-conversation")}>{m.chat_ask_allow_conversation()}</Button>
            <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10" data-testid="agent-ask-deny" onclick={() => void chat.answer(ask.askId, "deny")}>{m.chat_ask_deny()}</Button>
          </div>
        {/each}
      </div>
    {/if}
    {#if chat.conversation?.parent}<p role="note" class="m-0 px-2 text-xs text-muted-foreground" data-testid="agent-child-notice">{m.chat_child_notice({ label: chat.conversation.label ?? "" })}</p>{/if}
    {#if sendError}<p role="alert" class="m-0 px-2 text-xs text-destructive" data-testid="agent-send-error">{sendError}</p>{/if}
    {#if sendNotice}<p role="status" class="m-0 px-2 text-xs text-primary" data-testid="agent-send-notice">{sendNotice}</p>{/if}
    <ChatComposer
      bind:value={draft}
      bind:textarea
      disabled={inputBusy}
      blockedReason={notReady ? m.chat_composer_not_ready() : ""}
      running={chat.running}
      {stopping}
      onsubmit={submit}
      onstop={() => void stop()}
    >
      <!-- 输入框底栏（rc2 玩家反馈）：左边一行用量小字；右边 权限图标、呼叫图标（悬停看快捷键）、发送。
           选模型挪进「设置 → 她怎么回答 → AI 连接」：游戏陪玩不常换模型。 -->
      {#snippet tools()}
        <UsagePanel compact messages={chat.conversation?.messages ?? []} running={chat.runningHere} {costs} />
      {/snippet}
      {#snippet options()}
        <PermissionMenu onchanged={() => void chat.refresh()} />
        <CallKotoneButton size="icon" {ready} onSetCallKey={onOpenTalk} />
      {/snippet}
    </ChatComposer>
  </div>
{/snippet}

<div class="ui-scope relative flex h-full min-h-0 text-foreground" data-testid="assistant-chat" bind:clientWidth={chatWidth}>
<div class="flex min-h-0 min-w-0 flex-1 flex-col" data-testid="chat-column">
  {#if browserPreview || notReady || chat.loadError}
    <div class="mx-auto w-full max-w-[808px] space-y-2 px-6 pt-4">
      {#if browserPreview}
        <p role="status" class="m-0 flex items-center gap-2 rounded-xl border border-warning/30 bg-warning/8 px-3.5 py-2 text-xs text-warning" data-testid="agent-preview-banner">
          <Eye class="size-3.5 shrink-0" />{m.chat_preview_banner()}
        </p>
      {/if}
      {#if notReady}
        <div class="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-destructive/35 bg-destructive/8 px-3.5 py-2.5 text-[13px]" data-testid="agent-not-ready">
          <CircleAlert class="size-4 shrink-0 text-destructive" />
          <span class="font-medium text-destructive">{m.chat_not_ready_title({ name: $companionName })}</span>
          <span class="min-w-0 flex-1 text-foreground/80">{notReady}</span>
          <Button variant="link" size="sm" class="h-auto p-0" data-testid="agent-open-settings" onclick={onOpenSettings}>{m.chat_open_settings()}</Button>
        </div>
      {/if}
      {#if chat.loadError}<p role="alert" class="m-0 text-xs text-destructive">{chat.loadError}</p>{/if}
    </div>
  {/if}

  {#if !ready && empty}
    <!-- 没有 AI 连接（W5）：和引导里的“填 Key”是同一个组件 -->
    <div class="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto px-6 py-8" data-testid="agent-empty">
      <div class="flex w-full max-w-[680px] flex-col items-center text-center" data-testid="chat-unconfigured">
        <div class="mb-4 rounded-full shadow-[0_0_40px_-8px_var(--color-primary)]"><KotoneAvatar size={72} mood="asking" custom={$customAvatar} faces={$roleFaces} letter={$avatarLetter} /></div>
        <DeepSeekKeyForm variant="card" testid="chat-key" onConnected={onKeyConnected} />
        <p class="mt-4 mb-0 text-xs text-muted-foreground">{m.chat_other_provider()}<button class="text-primary hover:underline" data-testid="chat-open-connections" onclick={onOpenConnections}>{m.chat_open_connections()}</button></p>
        {#if legacy}
          <p class="mt-1.5 mb-0 text-xs text-muted-foreground">{m.chat_voice_only()}<button class="text-primary hover:underline" data-testid="chat-back-to-voice" onclick={onBackToVoice}>{m.chat_back_to_voice()}</button></p>
        {/if}
      </div>
    </div>
  {:else if empty}
    <!-- 空白对话（W6 / W7）：角色形象 + 呼叫按钮 + 居中的输入卡 + 起手提示 -->
    <div class="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto px-6 py-8" data-testid="agent-empty">
      <div class="w-full max-w-[680px]">
        <div class="mb-6 flex flex-col items-center text-center">
          <div class="mb-4 shadow-[0_0_40px_-8px_var(--color-primary)] rounded-full"><KotoneAvatar size={80} mood={localPending ? "note" : "asking"} custom={$customAvatar} faces={$roleFaces} letter={$avatarLetter} /></div>
          {#if localPending}
            <div class="mb-3 flex flex-wrap items-center justify-center gap-2 text-xs" data-testid="chat-local-asr">
              <span class="rounded-full border px-2.5 py-0.5 {asr === 'local-error' ? 'border-destructive/50 text-destructive' : 'border-warning/50 text-warning'}">
                {m.chat_local_asr_status({ status: LOCAL_ASR_LABELS[asr as keyof typeof LOCAL_ASR_LABELS] })}{asr === "local-starting" ? "…" : ""}
              </span>
              {#if asr !== "local-starting"}
                <Button size="sm" variant="outline" class="h-7" disabled={startingAsr} data-testid="chat-local-asr-start" onclick={() => void startLocal()}>{asr === "local-error" ? m.chat_local_asr_retry() : m.chat_local_asr_start()}</Button>
              {/if}
            </div>
          {/if}
          <CallKotoneButton size="lg" {ready} testid="chat-call" onSetCallKey={onOpenTalk} />
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
      {#if !ready}
        <div class="mb-3 flex justify-center" data-testid="chat-unconfigured"><DeepSeekKeyForm variant="card" testid="chat-key" onConnected={onKeyConnected} /></div>
      {/if}
      {@render composer()}
    </div>
  {/if}
</div>
  <RightSidebar {active} {narrow} {chatWidth} />
</div>
