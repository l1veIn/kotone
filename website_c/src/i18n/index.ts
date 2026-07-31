import zh from "./zh-CN.json";
import en from "./en.json";

/**
 * 集中化 i18n：所有页面文案都从这两个 locale 文件路由。
 * 不使用框架级 API，按当前请求的 locale 返回整棵文案树。
 */
export const dicts = {
  "zh-CN": zh,
  en,
} as const;

export type Locale = keyof typeof dicts;

/** 依据路径判断 locale（默认中文）。 */
export function resolveLocale(pathname: string): Locale {
  return pathname.startsWith("/en") ? "en" : "zh-CN";
}

/** 返回当前请求的文案树。 */
export function t(pathname: string) {
  return dicts[resolveLocale(pathname)];
}
