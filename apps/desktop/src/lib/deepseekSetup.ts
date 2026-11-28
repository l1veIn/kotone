/*
 * 一个 DeepSeek Key 打开全部功能（ADR 028）。引导里的「填 Key」、聊天页的 Key 卡片、
 * 0.1.x 用户的「添加 DeepSeek Key，认识琴音」都走 `connectDeepSeek`；「设置 → API 连接」里
 * 存了 DeepSeek 的 Key、聊天页“准备好了吗”里的「换成 DeepSeek」走 `switchToDeepSeek`（ADR 051：
 * 不管从哪存的 DeepSeek Key，她都换成 DeepSeek）。之后玩家在“她用的模型”里自己换，就照他的。
 *
 * Key 的本地格式检查照搬 DeepSeek Harness 桌面端（MIT，ca33fd68，
 * apps/desktop/src/client/WelcomePage.tsx `saveKey`）：只能是可打印 ASCII、不能是
 * `NAME=value` 形式、不能整个被引号包着。DSH 不测试连接；我们多读一次模型列表（不花钱），
 * 错 Key 当场就能说清楚，而不是等到第一次发消息才报错。
 */
import {
  activateCatalogModel,
  connectionSelfCheck,
  deleteConnection,
  getSettings,
  listModels,
  setActiveModel,
  updateSettings,
  upsertConnection,
  type Settings,
} from "./ipc";
import { usesLegacyVoiceInput } from "./ipc/contracts";
import { m } from "./paraglide/messages.js";

export const DEEPSEEK_KEY_PAGE = "https://platform.deepseek.com/api_keys";
export const DEEPSEEK_BASE_URL = "https://api.deepseek.com/v1";
export const DEEPSEEK_DEFAULT_MODEL = "deepseek-flash";

/** 本地格式检查；通过返回 null，否则返回要显示给玩家的话。 */
export function deepseekKeyProblem(raw: string): string | null {
  const value = raw.trim();
  if (!value) return m.shell_deepseek_key_empty();
  if (
    !/^[\x21-\x7e]+$/.test(value) ||
    /^[A-Z][A-Z0-9_]*=[^=]/.test(value) ||
    ((value.startsWith('"') || value.startsWith("'") || value.charCodeAt(0) === 96) && value[value.length - 1] === value[0])
  ) {
    return m.shell_deepseek_key_malformed();
  }
  return null;
}

/** 换成 DeepSeek 的各项能力（引导的“已连接”页逐项列出）。 */
export type SwitchedItem = { id: "chat" | "asr" | "ocr" | "tts" | "search"; label: string; ok: boolean; note?: string };

export interface DeepSeekSetupResult {
  connectionId: string;
  items: SwitchedItem[];
  settings: Settings;
}

/** DeepSeek 的连接（和后端 `Settings::deepseek_connection_id` 同一条规则）。 */
export function isDeepSeekConnection(connection: { provider: string; baseUrl: string }): boolean {
  return connection.provider === "deepseek" || connection.baseUrl.includes("api.deepseek.com");
}

/**
 * 保存 Key → 测试连接 → 她和其余能力都换成 DeepSeek（见 `switchToDeepSeek`）。
 * 测试没通过时删掉刚建的连接并抛出可读的原因。
 */
export async function connectDeepSeek(rawKey: string, current: Settings | null): Promise<DeepSeekSetupResult> {
  const problem = deepseekKeyProblem(rawKey);
  if (problem) throw new Error(problem);
  const connectionId = `deepseek-${crypto.randomUUID()}`;
  await upsertConnection(
    { id: connectionId, displayName: "DeepSeek", kind: "remote", provider: "deepseek", baseUrl: DEEPSEEK_BASE_URL, model: DEEPSEEK_DEFAULT_MODEL },
    rawKey.trim(),
  );
  let check;
  try {
    check = await connectionSelfCheck(connectionId);
  } catch (error) {
    await deleteConnection(connectionId).catch(() => {});
    throw error;
  }
  if (!check.passed) {
    await deleteConnection(connectionId).catch(() => {});
    throw new Error(check.message || m.shell_deepseek_key_unusable());
  }
  return switchToDeepSeek(connectionId, current);
}

/**
 * 她换成这条（已存好 Key 的）DeepSeek 连接，语音识别、文字识别、语音合成、联网搜索跟着换。
 *
 * - 新用户：五项都换。只有一个例外：玩家已经自己下好并选了本地识别模型（走过“只想用语音输入？”），
 *   语音识别保持本地——不在玩家没选择的情况下换掉他选的识别方式。
 * - 0.1.x 升级后还没启用她的（`pending` / `dismissed`）：这就启用（`legacyUpgrade` 写成 `notNeeded`），落地页改成聊天
 *   （D5）；语音识别不动（ADR 028 / 027）。呼叫键另设（升级用户默认空）。
 * 她的连接只在 `assistant.connectionId`（ADR 052）；启用了她的 0.1.x 用户和新用户一样。
 */
export async function switchToDeepSeek(connectionId: string, current: Settings | null): Promise<DeepSeekSetupResult> {
  const items: SwitchedItem[] = [{ id: "chat", label: m.shell_deepseek_item_chat(), ok: true }];
  if (usesLegacyVoiceInput(current?.ui)) {
    await updateSettings({
      assistant: { connectionId },
      ui: { legacyUpgrade: "notNeeded", firstRunCompleted: true, startPage: "chat" },
    });
    items.push({ id: "asr", label: m.shell_deepseek_item_asr(), ok: true, note: m.shell_deepseek_asr_keep_settings() });
  } else {
    await updateSettings({ assistant: { connectionId }, ui: { firstRunCompleted: true } });
    const keepLocal = await keepsChosenLocalAsr(current);
    if (keepLocal) {
      items.push({ id: "asr", label: m.shell_deepseek_item_asr(), ok: true, note: m.shell_deepseek_asr_keep_local({ model: keepLocal }) });
    } else {
      items.push(await attempt("asr", m.shell_deepseek_item_asr(), () => setActiveModel("deepseek-asr", "deepseek-asr")));
    }
  }
  items.push(await attempt("ocr", m.shell_deepseek_item_ocr(), () => activateCatalogModel("ocr", "deepseek-ocr")));
  items.push(await attempt("tts", m.shell_deepseek_item_tts(), () => activateCatalogModel("tts", "deepseek-tts")));
  items.push(await attempt("search", m.shell_deepseek_item_search(), () => activateCatalogModel("search", "deepseek-search")));
  return { connectionId, items, settings: await getSettings() };
}

async function attempt(id: SwitchedItem["id"], label: string, run: () => Promise<unknown>): Promise<SwitchedItem> {
  try {
    await run();
    return { id, label, ok: true };
  } catch (error) {
    return { id, label, ok: false, note: error instanceof Error ? error.message : String(error) };
  }
}

/** 玩家自己选好并下载了本地识别模型时返回它的名字（这时不换成在线识别）。 */
async function keepsChosenLocalAsr(current: Settings | null): Promise<string | null> {
  const chosen = current?.activeModelId?.trim();
  if (!chosen) return null;
  try {
    const model = (await listModels()).find((item) => item.id === chosen);
    if (!model || model.backend === "remote" || !model.downloaded) return null;
    return model.displayName.replace(/（.*$/, "");
  } catch {
    return null;
  }
}
