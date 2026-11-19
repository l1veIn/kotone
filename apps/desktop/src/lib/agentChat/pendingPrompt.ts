// 别处（引导结束、语音输入的“让 Kotone 帮我配置”）请聊天页在输入框里放好一句话：
// 只预填，由玩家自己点发送（ADR 028 D2）。聊天页准备好之后取走并清空。
import { writable } from "svelte/store";

export const pendingPrompt = writable<string | null>(null);
