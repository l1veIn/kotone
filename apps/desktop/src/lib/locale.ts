// 界面语言（i18n）：中文 / English（Paraglide，messages/<locale>/*.json）。
//
// - 选择存在本机（Paraglide 的 localStorage 策略；Kotone 的各个窗口同源，共用一份），没选过就是中文。
// - 切换后所有窗口重新加载：设置窗口发 kotone://locale-changed，悬浮窗、气泡、弹幕窗收到也重新加载。
// - 只管前端的字。Rust 发出来的字（错误、气泡状态、她念的话）和她说话用的语言不在这里。
import { emit, listen } from "@tauri-apps/api/event";
import { getLocale, setLocale, type Locale } from "./paraglide/runtime.js";

export type { Locale };

export const LOCALE_EVENT = "kotone://locale-changed";

/** 每种语言用它自己的写法显示（选错了也认得出来）。 */
export const LANGUAGES: { value: Locale; label: string }[] = [
  // i18n-ignore-start：语言名用各自的语言写，不翻译
  { value: "zh-CN", label: "中文" },
  // i18n-ignore-end
  { value: "en", label: "English" },
];

const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function currentLocale(): Locale {
  return getLocale();
}

/** 换语言：记下来，所有窗口重新加载。 */
export async function changeLocale(locale: Locale): Promise<void> {
  if (locale === getLocale()) return;
  setLocale(locale, { reload: false });
  if (inTauri()) {
    try {
      await emit(LOCALE_EVENT, locale);
    } catch {
      // 发不出去就只刷新自己，其他窗口下次打开时生效
    }
  }
  location.reload();
}

/** 每个窗口启动时调一次：`<html lang>` 跟着语言；别的窗口换了语言就重新加载。 */
export function installLocaleSync(): void {
  document.documentElement.lang = getLocale();
  if (!inTauri()) return;
  // 存的语言各窗口共用，收到就重新加载（不和 getLocale() 比：它可能已经读到新值了）
  void listen<string>(LOCALE_EVENT, () => location.reload()).catch(() => {});
}

/** 列举用的分隔：中文“、”，英文“, ”。 */
export function listSeparator(): string {
  // i18n-ignore-start：标点本身
  return getLocale() === "en" ? ", " : "、";
  // i18n-ignore-end
}

/** 分句用的分隔：中文“；”，英文“; ”。 */
export function clauseSeparator(): string {
  // i18n-ignore-start：标点本身
  return getLocale() === "en" ? "; " : "；";
  // i18n-ignore-end
}
