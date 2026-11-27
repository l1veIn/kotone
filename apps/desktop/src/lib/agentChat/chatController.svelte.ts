// 「琴音」聊天页的状态：对话列表、当前对话、流式输出、工具状态、权限询问。Kotone 自己的实现。
// 当前对话由后端决定（游戏内语音与聊天窗口共用“当前对话”）；这里显示的总是 agent_state.current。
import {
  agentAnswer, agentConversation, agentDeleteConversation, agentHeldCards, agentNewConversation, agentSelectConversation,
  agentSend, agentState, agentStop, pushId, subscribeAgent, subscribeAsks, subscribeAutoCardFocus, subscribeKeyRequests,
  subscribePushes, type AgentAsk, type KeyRequest, type AgentEvent, type AgentMessage, type AgentState, type Answer,
  type Conversation, type PushEvent, type SubagentInfo,
} from "../agent";
import type { LiveState } from "./timeline";
import { m } from "../paraglide/messages.js";
import { openTarget } from "../stores/navigation";

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
  /** 玩家点了消息提示：要定位到的那条推送（`n` 每次都变，同一条也能再闪一次）。 */
  focusCard = $state<{ id: string; n: number } | null>(null);

  #disposed = false;
  /**
   * 她这一轮里来的推送（ADR 043，按 `pushId`）：先显示在当前对话的最后面，这一轮结束后端才把它记进对话。
   * 这一轮新来的消息插在它们前面（记进对话时推送就在这一轮后面）。
   */
  #held = new Set<string>();
  #focusCount = 0;
  #revision = 0;
  /** 读取对话期间到达的事件：读完后补上（去掉已经在存盘里的）。 */
  #buffer: ({ id: string; event: AgentEvent } | { push: PushEvent })[] | null = null;
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
      retain(await subscribeAutoCardFocus((focus) => void this.#focus(focus.conversationId, focus.key)));
      retain(await subscribePushes((event) => this.#onPush(event)));
    } catch (error) {
      this.loadError = m.chat_load_subscribe_failed({ error: errorText(error) });
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
      const held = conversation ? await agentHeldCards(conversation.id) : [];
      if (this.#disposed || revision !== this.#revision) return;
      this.state = state;
      this.runningId = state.running;
      const switched = conversation?.id !== this.conversation?.id;
      this.conversation = conversation;
      // 排着的推送（还没记进对话的新推送、排着的原地更新）：对话里有同一条的原地换掉，没有的显示在最后
      this.#held.clear();
      for (const message of held) this.#showHeld(message);
      if (switched || !state.running) this.live = { text: "", thought: "", tools: {} };
      this.loadError = "";
      const buffered = this.#buffer ?? [];
      this.#buffer = null;
      for (const item of buffered) {
        if ("push" in item) this.#onPush(item.push);
        else this.#apply(item.id, item.event, true);
      }
    } catch (error) {
      if (!this.#disposed && revision === this.#revision) {
        this.#buffer = null;
        this.loadError = m.chat_load_failed({ error: errorText(error) });
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
    if (event.kind === "subagent") {
      // 派出去的事有了新状态（ADR 029）：只更新登记，不算这段对话在回答。
      if (id === this.conversation?.id) this.#upsertSubagent(event.info);
      return;
    }
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
        // 刚读出来的对话里可能已经有这条（后端先存盘再发事件）：同内容同时间的就是同一条。
        if (messages.some((m) => sameMessage(m, event.message))) {
          const id = pushId(event.message);
          if (id) this.#held.delete(id);
          break;
        }
        // 推送：排着的那条记进对话了（它已经显示着，可能已经原地更新过）——同一个标识的原地换成存盘的这条，不再加一条
        const id = pushId(event.message);
        const at = id ? messages.findIndex((m) => pushId(m) === id) : -1;
        if (id && at >= 0) {
          messages[at] = event.message;
          this.#held.delete(id);
          break;
        }
        // 这一轮新来的消息插在排着的推送前面（这一轮结束后推送才记进对话，排在这一轮后面）
        let end = messages.length;
        while (end > 0 && this.#isHeld(messages[end - 1])) end -= 1;
        messages.splice(end, 0, event.message);
        if (event.message.role === "assistant") { this.live.text = ""; this.live.thought = ""; }
        break;
      }
      case "discarded":
        this.live.text = "";
        this.live.thought = "";
        break;
      case "compacted":
        // 检查点插在对话中间：按存盘的重新读一遍。
        if (!replay) void this.refresh();
        break;
      case "finished":
        this.live.text = "";
        this.live.thought = "";
        void this.refreshList();
        break;
    }
  }

  #isHeld(message: AgentMessage): boolean {
    const id = pushId(message);
    return id !== null && this.#held.has(id);
  }

  /**
   * 推送的界面事件（ADR 043，`kotone://push`）：新的一条、原地更新都按标识（`plugin:key`）放——对话里已经有这一条就原地换掉
   * （位置不动、不多一条），没有就放在最后面（还排着的，这一轮结束后端才记进对话）。`done` 只给悬浮窗（聊天里的记录留着）。
   */
  #onPush(event: PushEvent) {
    if (this.#disposed || event.kind === "done") return;
    if (this.#buffer) { this.#buffer.push({ push: event }); return; }
    if (event.conversationId !== this.conversation?.id) return;
    this.#showHeld(event.message, event.held);
  }

  /** 一条推送放进当前对话：同一个标识的原地换掉；没有的放在最后面（`held`：还没记进对话，这一轮新来的消息插在它前面）。 */
  #showHeld(message: AgentMessage, held = true) {
    const id = pushId(message);
    if (!id || !this.conversation) return;
    const messages = this.conversation.messages;
    const at = messages.findIndex((m) => pushId(m) === id);
    if (at >= 0) {
      messages[at] = message;
      return;
    }
    if (held) this.#held.add(id);
    messages.push(message);
  }

  /** 玩家点了消息提示（ADR 039 D4）：切到聊天页、那段对话，定位到那张卡片（卡片组件看 `focusCard` 滚过去、闪一下）。 */
  async #focus(conversationId: string, key: string) {
    if (this.#disposed) return;
    openTarget({ page: "assistant", section: "chat" });
    if (this.conversation?.id !== conversationId) await this.refresh();
    this.focusCard = { id: key, n: ++this.#focusCount };
  }

  #upsertSubagent(info: SubagentInfo) {
    if (!this.conversation) return;
    const list = this.conversation.subagents ?? [];
    const index = list.findIndex((s) => s.name === info.name);
    // 读取对话之后才到的旧事件不覆盖更新的状态
    if (index >= 0 && list[index].updatedAtMs > info.updatedAtMs) return;
    this.conversation.subagents = index >= 0 ? list.map((s, i) => (i === index ? info : s)) : [...list, info];
  }

  /** 当前对话里某件派出去的事的登记。 */
  subagent(name: string): SubagentInfo | undefined {
    return this.conversation?.subagents?.find((s) => s.name === name);
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
  async send(text: string, conversationId?: string): Promise<"started" | "steered"> {
    const result = await agentSend(text, conversationId);
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
