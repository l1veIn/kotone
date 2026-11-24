// 文案：和插件在宿主里的页面共用 plugins/lol/messages/<语言>.json（plugin_lol_*；terminology.spec 管着）。
// 语言跟宿主上下文的 locale（BCP 47），没给就是中文。
// 写法和宿主的 Paraglide 一样：`m.plugin_lol_xxx({ 参数 })`，terminology.spec 按 `m.<键>` 认哪些文案用到了。
import zh from "../../messages/zh-CN.json";
import en from "../../messages/en.json";

export type MessageKey = keyof typeof zh;
type Params = Record<string, string | number>;

const catalogs: Record<string, Record<string, string>> = { zh, en };
const state = $state({ lang: "zh" });

export function setLocale(locale: string) {
  state.lang = locale.toLowerCase().startsWith("en") ? "en" : "zh";
}

/** 现在的语言（`zh` / `en`），数字和日期的格式跟着它。 */
export function lang(): string {
  return state.lang === "en" ? "en" : "zh-CN";
}

/** 一句文案，`{name}` 换成参数。 */
export function t(key: MessageKey, params: Params = {}): string {
  const text = catalogs[state.lang]?.[key] ?? zh[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

/** `m.plugin_lol_xxx(参数)`：读的是 `state.lang`，放在模板里换语言时跟着变。 */
export const m = new Proxy({} as { [K in MessageKey]: (params?: Params) => string }, {
  get: (_, key) => (params?: Params) => t(key as MessageKey, params),
});
