// 把一段对话（存盘的消息 + 正在流式输出的部分）整理成按“轮”显示的时间线。Kotone 自己的实现。
// 一轮 = 一条用户消息（或自动化送回的系统消息）及其后的回答、思考、工具调用与状态说明。
import { PUSH_NOTE_SOURCE, pushOf, type AgentMessage, type PushRecord, type CategoryTotals, type ConversationCosts, type TokenUsage, type ToolCall, type ToolDisplay, type ToolStatus, type UsageTotals } from "../agent";

export type UserMessage = Extract<AgentMessage, { role: "user" }>;

export interface ToolView {
  call: ToolCall;
  status: ToolStatus;
  /** 结果（存盘的工具消息，或执行中事件里带的内容）。 */
  content: string | null;
  untrusted: boolean;
  /** 插件工具的页面（ADR 038）：聊天里显示成卡片。 */
  display?: ToolDisplay;
  /** 超过 3 秒转到了后台（存盘的工具消息上的标记，ADR 029 §一）：显示成后台任务卡片。 */
  background?: boolean;
}

export type Block =
  | { kind: "thinking"; key: string; text: string; streaming: boolean }
  | { kind: "text"; key: string; text: string; streaming: boolean }
  | { kind: "tool"; key: string; tool: ToolView }
  | { kind: "notice"; key: string; text: string }
  /** 上下文压缩的检查点：它之前的对话已整理成这份摘要。 */
  | { kind: "checkpoint"; key: string; text: string; summary: string }
  /** 对局复盘：标题 + 几条要点。 */
  | { kind: "recap"; key: string; text: string; body: string };

export interface Turn {
  /** 稳定的标识（第一条消息的时间，同一毫秒的按先后编号），用于滚动锚点和列表的 key。 */
  id: string;
  user: UserMessage | null;
  blocks: Block[];
  /** 这一轮所有模型调用的用量之和；服务商没报就是 null（不估算）。 */
  usage: TokenUsage | null;
  /** 回答的纯文字（复制用）。 */
  answerText: string;
  active: boolean;
  endedAtMs: number | null;
  /** 这一轮模型以外的费用：语音合成 / 识别、读屏、联网搜索（来自本机账本，没有就不显示）。 */
  extraCosts?: CategoryTotals[];
  /** 推送（ADR 043；旧的自动卡片也画成它）：插件自己发进聊天的一条，单独占一轮（没有玩家的话、没有她的回答）。 */
  push?: PushRecord;
  /** 这一轮第一条消息的时间（正在回答、还没有消息的那一轮是现在）：悬浮窗拿它分出“这一次来的”（ADR 043 D6）。 */
  startedAtMs?: number;
}

export interface LiveState {
  /** 当前这次模型调用流式输出的文字 / 思考（存盘后清空）。 */
  text: string;
  thought: string;
  /** 工具调用事件里的状态（callId → 状态与内容）。 */
  tools: Record<string, { status: ToolStatus; content: string | null }>;
}

function addUsage(total: TokenUsage | null, usage: TokenUsage | undefined): TokenUsage | null {
  if (!usage) return total;
  const sum = (a?: number, b?: number) => (a === undefined && b === undefined ? undefined : (a ?? 0) + (b ?? 0));
  return {
    inputTokens: sum(total?.inputTokens, usage.inputTokens),
    outputTokens: sum(total?.outputTokens, usage.outputTokens),
    cachedInputTokens: sum(total?.cachedInputTokens, usage.cachedInputTokens),
    reasoningTokens: sum(total?.reasoningTokens, usage.reasoningTokens),
    costUsd: sum(total?.costUsd, usage.costUsd),
    // 币种不同（换过连接）就不加在一起。
    ...(total?.currency && usage.currency && total.currency !== usage.currency
      ? { cost: undefined, currency: undefined }
      : { cost: sum(total?.cost, usage.cost), currency: usage.currency ?? total?.currency }),
  };
}

/** running：这段对话是否正在回答（决定最后一轮是否“进行中”、未结束的工具算执行中还是未完成）。 */
export function buildTurns(messages: readonly AgentMessage[], live: LiveState | null, running: boolean): Turn[] {
  const results = new Map<string, Extract<AgentMessage, { role: "tool" }>>();
  for (const message of messages) if (message.role === "tool") results.set(message.callId, message);

  const turns: Turn[] = [];
  let turn: Turn | null = null;
  // 每一轮的标识：第一条消息的时间（同一毫秒开始的几轮按先后加 -1、-2……）；块的标识：这一轮的标识 + 在这一轮里第几条。
  // 不用在整段对话里的位置：带提醒的推送挪到最后时（ADR 043 D5 修订）它后面每一条的位置都变了，标识不变才不会整段重建
  // （卡片里的 App 不重新加载，往上翻着看时不跳）。
  const sameMs = new Map<number, number>();
  let base = 0;
  const start = (user: UserMessage | null, index: number, atMs: number) => {
    const n = sameMs.get(atMs) ?? 0;
    sameMs.set(atMs, n + 1);
    base = index;
    turn = { id: n ? `${atMs}-${n}` : `${atMs}`, user, blocks: [], usage: null, answerText: "", active: false, endedAtMs: null, startedAtMs: atMs };
    turns.push(turn);
    return turn;
  };

  messages.forEach((message, index) => {
    // 推送给她的那一句：界面不显示（显示的是前面那条推送）
    if (message.role === "user" && message.source === PUSH_NOTE_SOURCE) return;
    if (message.role === "user") { start(message, index, message.atMs); return; }
    const push = pushOf(message);
    if (push) {
      const own = start(null, index, message.atMs);
      own.push = push;
      own.endedAtMs = message.atMs;
      // 推送自成一轮：后面的东西不挂在它下面
      turn = null;
      return;
    }
    const current: Turn = turn ?? start(null, index, message.atMs);
    const key = (kind: string) => `${kind}${current.id}.${index - base}`;
    if (message.role === "assistant") {
      if (message.reasoning?.trim()) current.blocks.push({ kind: "thinking", key: key("r"), text: message.reasoning, streaming: false });
      const pushCall = (call: ToolCall) => {
        const result = results.get(call.id);
        const event = live?.tools[call.id];
        const status: ToolStatus = result ? (result.isError ? "failed" : "completed") : event?.status ?? "pending";
        current.blocks.push({
          kind: "tool", key: `t${call.id}`,
          tool: { call, status, content: result?.content ?? event?.content ?? null, untrusted: result?.untrusted ?? false, display: result?.display, background: result?.background },
        });
      };
      // 她说出口的话（speak，ADR 029 第八节）排在正文前面：先听到结论，细节在下面。
      const calls = message.toolCalls ?? [];
      for (const call of calls.filter((call) => call.name === "speak")) {
        pushCall(call);
        // 只写不念的细节（speak.written）也算这一轮的回答（复制时带上）
        let written = "";
        try { written = String(JSON.parse(call.arguments)?.written ?? ""); } catch { /* 参数还没写完 */ }
        if (written.trim()) current.answerText = current.answerText ? `${current.answerText}\n\n${written}` : written;
      }
      if (message.text.trim()) {
        current.blocks.push({ kind: "text", key: key("a"), text: message.text, streaming: false });
        current.answerText = current.answerText ? `${current.answerText}\n\n${message.text}` : message.text;
      }
      calls.filter((call) => call.name !== "speak").forEach(pushCall);
      current.usage = addUsage(current.usage, message.usage);
      current.endedAtMs = message.atMs;
    } else if (message.role === "notice" && message.recap) {
      current.blocks.push({ kind: "recap", key: key("p"), text: message.text, body: message.recap });
      current.endedAtMs = message.atMs;
    } else if (message.role === "notice" && message.compaction) {
      current.blocks.push({ kind: "checkpoint", key: key("k"), text: message.text, summary: message.compaction });
    } else if (message.role === "notice") {
      current.blocks.push({ kind: "notice", key: key("n"), text: message.text });
      current.endedAtMs = message.atMs;
    }
    // role === "tool"：结果已并入对应的工具卡片。
  });

  // 正在回答的是最后一轮；她这一轮里来的推送先显示在最后面（这一轮结束才记进对话），不算这一轮
  const last = [...turns].reverse().find((t) => !t.push);
  if (running) {
    const current = last ?? start(null, messages.length, Date.now());
    current.active = true;
    if (live?.thought) current.blocks.push({ kind: "thinking", key: "live-thought", text: live.thought, streaming: true });
    if (live?.text) current.blocks.push({ kind: "text", key: "live-text", text: live.text, streaming: true });
  }
  return turns;
}

/** 显示用：1234 → “1,234”。 */
export function formatTokens(value: number): string {
  return value.toLocaleString("en-US");
}

/** 一段对话所有模型调用的用量之和（含压缩）。 */
export function conversationUsage(messages: readonly AgentMessage[]): TokenUsage | null {
  let total: TokenUsage | null = null;
  for (const message of messages) {
    if (message.role === "assistant" || message.role === "notice") total = addUsage(total, message.usage);
  }
  return total;
}

/**
 * 命中率显示（照 DeepSeek Harness 的 token-format）：整数百分比；没全中时不显示成 100
 * （99.5、99.95……多保留到不是 100 为止）。没有输入时为空。
 */
export function formatHitRate(hit: number | undefined, input: number | undefined): string {
  if (!input) return "";
  const h = Math.min(hit ?? 0, input);
  if (h === input) return "100%";
  const percent = (h / input) * 100;
  for (let digits = 0; digits <= 4; digits += 1) {
    const text = percent.toFixed(digits);
    if (Number(text) < 100) return `${text}%`;
  }
  return "99.9999%";
}

/** 显示费用：人民币 ¥、美元 $；很小的数多保留几位。 */
export function formatCost(cost: number | undefined, currency: string | undefined): string {
  if (cost === undefined) return "";
  const symbol = currency === "CNY" ? "¥" : currency === "USD" || !currency ? "$" : `${currency} `;
  const digits = cost >= 1 ? 2 : cost >= 0.01 ? 3 : 4;
  return `${symbol}${cost.toFixed(digits)}`;
}

/** 一组用量的费用（按币种，多个币种用“+”连起来）；没有价格时为空。 */
export function totalsMoney(t: UsageTotals): string {
  return Object.entries(t.cost).map(([currency, cost]) => formatCost(cost, currency)).join(" + ");
}

/**
 * 把账本里记到这段对话的费用挂到对应的轮上（按玩家那句话的时间对上；这一轮自己的模型费用在消息里，不重复挂；
 * 交给后台的事的模型费用不在这段对话的消息里，按“后台”挂到派它的那一轮）。
 * 没有费用的轮原样返回。
 */
export function withCosts(turns: Turn[], costs: ConversationCosts | null): Turn[] {
  if (!costs?.turns.length) return turns;
  const byTurn = new Map(costs.turns.map((t) => [t.turn, t.byCategory.filter((c) => c.category !== "chat")]));
  return turns.map((turn) => {
    const extra = turn.user ? byTurn.get(turn.user.atMs) : undefined;
    return extra?.length ? { ...turn, extraCosts: extra } : turn;
  });
}
