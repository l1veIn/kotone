// 设置窗口里共用的一份琴音对话状态：侧边栏的对话列表与「琴音」聊天页看的是同一个。
import { getContext, setContext } from "svelte";
import type { AgentChat } from "./chatController.svelte";

const KEY = Symbol("kotone.agentChat");

export function provideAgentChat(chat: AgentChat): AgentChat {
  return setContext(KEY, chat);
}

export function useAgentChat(): AgentChat {
  const chat = getContext<AgentChat | undefined>(KEY);
  if (!chat) throw new Error("useAgentChat() 需要在设置窗口内使用");
  return chat;
}

/** 不在设置窗口里（没有提供对话状态）时为 undefined。 */
export function tryAgentChat(): AgentChat | undefined {
  return getContext<AgentChat | undefined>(KEY);
}
