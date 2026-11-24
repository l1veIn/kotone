/**
 * dev:web 里的同传插件（ADR 021、038）：照桌面端 plugins/interpreter 回答 interpreter.* 工具。
 * 设置读 `plugins.interpreter.settings`；“开始”时她的耳朵就在听（真的要等她交给后台、开始听），“停止”时不听了。
 * 预览里不真的对她说那句话，只照真的回替你说了什么。
 */
import type { PluginToolResult } from "../plugins";
import { getSettings } from "./browserMock";

/** 她的耳朵现在在听的程序（预览开始时没在听）。 */
let listening: string[] = [];

// i18n-ignore-start：和后端 plugins/interpreter/src/lib.rs 一样的值与替你说的话（不是界面文字）
const TARGETS = ["中文", "英语", "日语", "韩语"];
const STOP_TEXT = "停止同传";
const CURRENT_GAME = "现在的游戏";
const startText = (app: string, target: string, bubble: boolean) =>
  `帮我盯着队友说的话，当同传：听 ${app || CURRENT_GAME}，每句外语翻成${target}，用${bubble ? "气泡" : "弹幕"}显示。`;
// i18n-ignore-end
/** 没说听哪个程序时，预览里假装现在的游戏是这个。 */
const SAMPLE_GAME = "TslGame.exe";

async function prefs(): Promise<{ app: string; target: string; bubble: boolean }> {
  const raw = ((await getSettings()).plugins?.interpreter?.settings ?? {}) as Record<string, unknown>;
  const text = (key: string) => (typeof raw[key] === "string" ? (raw[key] as string).trim() : "");
  const target = text("target");
  return {
    app: text("app").slice(0, 64),
    target: TARGETS.includes(target) ? target : TARGETS[0],
    bubble: raw.display === "bubble",
  };
}

/** 插件开着时的工具（插件开关由 pluginsMock 先看）。 */
export async function interpreterCall(tool: string): Promise<PluginToolResult> {
  const p = await prefs();
  switch (tool) {
    case "interpreter.status":
      return {
        content: { listening: [...listening], app: p.app, target: p.target, display: p.bubble ? "bubble" : "danmaku", targets: [...TARGETS] },
      };
    case "interpreter.start":
      listening = [p.app || SAMPLE_GAME];
      return { content: { said: startText(p.app, p.target, p.bubble) } };
    case "interpreter.stop":
      listening = [];
      return { content: { said: STOP_TEXT } };
    default:
      return { content: { status: "error", text: tool } };
  }
}
