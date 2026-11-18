// 别处（新手引导的“打字问她”）请聊天页替玩家发一句话：聊天页准备好之后发出并清空。
import { writable } from "svelte/store";

export const pendingPrompt = writable<string | null>(null);
