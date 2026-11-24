// 文案：和插件在宿主里的页面共用 plugins/lol/messages/<语言>.json（plugin_lol_*；terminology.spec 管着）。
// 语言跟宿主上下文的 locale（BCP 47），没给就是中文。
import zh from "../../messages/zh-CN.json";
import en from "../../messages/en.json";

export type MessageKey = keyof typeof zh;

const catalogs: Record<string, Record<string, string>> = { zh, en };
const state = $state({ lang: "zh" });

export function setLocale(locale: string) {
  state.lang = locale.toLowerCase().startsWith("en") ? "en" : "zh";
}

/** 一句文案，`{name}` 换成参数。 */
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const text = catalogs[state.lang]?.[key] ?? zh[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}
