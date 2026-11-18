// 「琴音」聊天页的状态：对话列表、当前对话、流式输出、工具状态、权限询问。Kotone 自己的实现。
// 当前对话由后端决定（游戏内语音与聊天窗口共用“当前对话”）；这里显示的总是 agent_state.current。
import {
  agentAnswer, agentConversation, agentDeleteConversation, agentNewConversation, agentSelectConversation,
  agentSend, agentState, agentStop, subscribeAgent, subscribeAsks, subscribeKeyRequests,
  type AgentAsk, type KeyRequest, type AgentEvent, type AgentMessage, type AgentState, type Answer, type Conversation,
} from "../agent";
import type { LiveState } from "./timeline";

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function sameMessage(a: AgentMessage, b: AgentMessage): boolean {
  return a.role === b.role && a.atMs === b.atMs && JSON.stringify(a) === JSON.stringify(b);
}

export class AgentChat {
  state = $state<AgentState | null>(null);
  conversation = $state<Conversation | null>(null);
  /** 正在回答的对话 id（事件比 agent_state 更及时）。 */
  runningId = $state<string | null>(null);
  live = $state<LiveState>({ text: "", thought: "", tools: {} });
  asks = $state<AgentAsk[]>([]);
  /** 琴音请玩家粘贴的 Key（输入框在聊天里）。 */
  keyRequests = $state<KeyRequest[]>([]);
  loading = $state(true);
  loadError = $state("");

  #disposed = false;
  #revision = 0;
  /** 读取对话期间到达的事件：读完后补上（去掉已经在存盘里的）。 */
  #buffer: { id: string; event: AgentEvent }[] | null = null;
  #cleanups: (() => void)[] = [];

  get currentId(): string | null { return this.state?.current ?? null; }
  get running(): boolean { return this.runningId !== null; }
  get runningHere(): boolean { return this.runningId !== null && this.runningId === this.conversation?.id; }

  async init(): Promise<void> {
    const retain = (cleanup: () => void) => (this.#disposed ? cleanup() : this.#cleanups.push(cleanup));
    try {
      retain(await subscribeAgent((id, event) => this.#onEvent(id, event)));
      retain(await subscribeAsks((ask) => this.#onAsk(ask), (askId) => this.#onResolved(askId)));
      retain(await subscribeKeyRequests(
        (request) => { if (!this.#disposed && !this.keyRequests.some((r) => r.requestId === request.requestId)) this.keyRequests.push(request); },
        (requestId) => { this.keyRequests = this.keyRequests.filter((r) => r.requestId !== requestId); },
      ));
    } catch (error) {
      this.loadError = `订阅琴音事件失败：${errorText(error)}`;
    }
    await this.refresh();
  }

  dispose(): void {
    this.#disposed = true;
    this.#cleanups.forEach((cleanup) => cleanup());
    this.#cleanups = [];
  }

  /** 重新读取状态与当前对话。 */
  async refresh(): Promise<void> {
    const revision = ++this.#revision;
    this.#buffer = [];
    try {
      const state = await agentState();
      const conversation = state.current ? await agentConversation(state.current) : null;
      if (this.#disposed || revision !== this.#revision) return;
      this.state = state;
      this.runningId = state.running;
      const switched = conversation?.id !== this.conversation?.id;
      this.conversation = conversation;
      if (switched || !state.running) this.live = { text: "", thought: "", tools: {} };
      this.loadError = "";
      const buffered = this.#buffer ?? [];
      this.#buffer = null;
      for (const { id, event } of buffered) this.#apply(id, event, true);
    } catch (error) {
      if (!this.#disposed && revision === this.#revision) {
        this.#buffer = null;
        this.loadError = `读取对话失败：${errorText(error)}`;
      }
    } finally {
      if (!this.#disposed && revision === this.#revision) this.loading = false;
    }
  }

  /** 只更新对话列表与就绪状态（不重新读取当前对话）。 */
  async refreshList(): Promise<void> {
    try {
      const state = await agentState();
      if (this.#disposed) return;
      if (state.current !== this.state?.current) { await this.refresh(); return; }
      this.state = state;
    } catch { /* 下次刷新再说 */ }
  }

  #onEvent(id: string, event: AgentEvent) {
    if (this.#disposed) return;
    if (this.#buffer) { this.#buffer.push({ id, event }); return; }
    this.#apply(id, event, false);
  }

  #apply(id: string, event: AgentEvent, replay: boolean) {
    if (event.kind === "idle") {
      if (this.runningId === id) this.runningId = null;
      this.live = { text: "", thought: "", tools: {} };
      void this.refreshList();
      return;
    }
    if (event.kind === "step") this.runningId = id;
    if (id !== this.conversation?.id) {
      // 新对话的第一条消息，或游戏内入口切到了别的对话：重新读取。
      if (event.kind === "appended" && !replay) void this.refresh();
      return;
    }
    switch (event.kind) {
      case "step":
        this.live.text = "";
        this.live.thought = "";
        break;
      case "chunk":
        this.live.text += event.text;
        break;
      case "thought":
        this.live.thought += event.text;
        break;
      case "toolCall":
        this.live.tools[event.call.id] = { status: "pending", content: null };
        break;
      case "toolCallUpdate":
        this.live.tools[event.callId] = { status: event.status, content: event.content ?? this.live.tools[event.callId]?.content ?? null };
        break;
      case "appended": {
        const messages = this.conversation.messages;
        if (replay && messages.some((m) => sameMessage(m, event.message))) break;
        messages.push(event.message);
        if (event.message.role === "assistant") { this.live.text = ""; this.live.thought = ""; }
        break;
      }
      case "finished":
        this.live.text = "";
        this.live.thought = "";
        void this.refreshList();
        break;
    }
  }

  #onAsk(ask: AgentAsk) {
    if (this.#disposed || this.asks.some((a) => a.askId === ask.askId)) return;
    this.asks.push(ask);
  }
  #onResolved(askId: string) {
    this.asks = this.asks.filter((a) => a.askId !== askId);
  }

  askFor(callId: string): AgentAsk | undefined {
    return this.asks.find((a) => a.request.callId === callId && a.conversationId === this.conversation?.id);
  }

  async answer(askId: string, answer: Answer): Promise<void> {
    const delivered = await agentAnswer(askId, answer);
    // 送达后后端会发 agent-ask-resolved；没送达（已超时 / 已停止）就直接收起。
    if (!delivered) this.#onResolved(askId);
  }

  /** 发送；正在回答时算插话。返回 steered = 插话。 */
  async send(text: string): Promise<"started" | "steered"> {
    const result = await agentSend(text);
    if (result.status === "started") this.runningId = result.conversationId;
    if (result.conversationId !== this.conversation?.id) await this.refresh();
    return result.status;
  }

  async stop(): Promise<boolean> {
    return agentStop();
  }

  async newConversation(): Promise<void> {
    await agentNewConversation();
    await this.refresh();
  }

  async select(id: string): Promise<void> {
    await agentSelectConversation(id);
    await this.refresh();
  }

  async remove(id: string): Promise<void> {
    await agentDeleteConversation(id);
    await this.refresh();
  }
}
