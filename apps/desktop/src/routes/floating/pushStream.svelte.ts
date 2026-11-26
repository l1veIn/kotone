// 悬浮窗小窗里的推送（ADR 043 D6）：这段对话里插件推送的那些条（卡片、一行、语音的记录），和聊天页同一种样子
// （`pushOf`：新的推送和 rc.9 的自动卡片都认得）。读对话时一次读进来（加上还排着、没记进对话的），之后跟着
// kotone://push 原地换掉同一个标识（`插件:key`）的那条、没有的放在最后面——和聊天页 chatController 同一个规矩。
import { agentConversation, agentHeldCards, pushId, pushOf, type AgentMessage, type PushEvent, type PushRecord } from "../../lib/agent";

export interface StreamItem {
  /** `插件:key`。 */
  id: string;
  atMs: number;
  push: PushRecord;
}

/** 最多留多少条（往回翻够用；更早的在聊天里）。 */
const LIMIT = 30;

function upsert(items: StreamItem[], message: AgentMessage): void {
  const push = pushOf(message);
  const id = pushId(message);
  if (!push || !id) return;
  const item = { id, atMs: message.atMs, push };
  const at = items.findIndex((i) => i.id === id);
  if (at >= 0) items[at] = item;
  else items.push(item);
}

export class PushStream {
  conversationId = $state<string | null>(null);
  items = $state<StreamItem[]>([]);
  loading = $state(false);
  #token = 0;
  /** 读对话时来的推送：读完再放上去。 */
  #pending: AgentMessage[] = [];

  /** 小窗要显示这段对话：不是现在这段就重新读。 */
  async show(conversationId: string): Promise<void> {
    if (this.conversationId === conversationId) return;
    this.conversationId = conversationId;
    this.items = [];
    this.loading = true;
    const token = ++this.#token;
    const [conversation, held] = await Promise.all([
      agentConversation(conversationId).catch(() => null),
      agentHeldCards(conversationId),
    ]);
    if (token !== this.#token) return;
    const items: StreamItem[] = [];
    for (const message of [...(conversation?.messages ?? []), ...held, ...this.#pending]) upsert(items, message);
    this.#pending = [];
    this.items = items.slice(-LIMIT);
    this.loading = false;
  }

  /** kotone://push：这段对话里的新的一条、原地更新（`done` 只收起气泡里那一行，记录留着）。 */
  apply(event: PushEvent): void {
    if (event.kind === "done" || event.conversationId !== this.conversationId) return;
    if (this.loading) {
      this.#pending.push(event.message);
      return;
    }
    const items = [...this.items];
    upsert(items, event.message);
    this.items = items.slice(-LIMIT);
  }
}
