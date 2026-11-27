// dev:web（浏览器预览、Playwright）里的原神插件：清单照 plugin.toml（文字从插件的文案读，Rust 测试对过两边一致），
// 工具照后端回，用一份假的 BetterGI（假路径、假配置名，不碰任何真东西）。apps/desktop/src/lib/ipc/pluginsMock.ts 引它。
// 测试切状态：`window.__kotoneGenshin = { running?: boolean, automation?: boolean, found?: boolean }`。
import zh from "../messages/zh-CN.json";
import type { PluginManifest, PluginToolResult, PluginVisibility } from "$lib/plugins";
import { getSettings } from "$lib/ipc/browserMock";
import type { GenshinRun, GenshinStatus } from "./genshin";

const text = zh as Record<string, string>;
const BOTH: PluginVisibility[] = ["model", "app"];

export const genshinManifest: PluginManifest = {
  id: "genshin",
  name: text.plugin_genshin_name,
  description: text.plugin_genshin_description,
  games: ["genshin"],
  adapter: null,
  enabledByDefault: false,
  sources: [{
    id: "bettergi",
    name: text.plugin_genshin_source_bettergi_name,
    description: text.plugin_genshin_source_bettergi_desc,
    enabledByDefault: true,
    risk: text.plugin_genshin_source_bettergi_risk,
  }],
  pages: [
    { uri: "ui://kotone/genshin/home", title: text.plugin_genshin_name, needs: ["bettergi"] },
    { uri: "ui://kotone/genshin/run", title: text.plugin_genshin_push_runs_title, needs: ["bettergi"] },
  ],
  agent: { overview: "", rule: "" },
  notices: [{
    id: "bettergi",
    title: text.plugin_genshin_notice_bettergi_title,
    body: text.plugin_genshin_notice_bettergi_body,
    links: ["https://github.com/babalae/better-genshin-impact", "https://www.gnu.org/licenses/gpl-3.0.html"],
  }],
  pushes: [{
    id: "runs",
    title: text.plugin_genshin_push_runs_title,
    when: text.plugin_genshin_push_runs_when,
    what: text.plugin_genshin_push_runs_what,
    forms: ["card"],
    priority: "normal",
    enabledByDefault: true,
    needs: ["bettergi"],
  }],
  events: [
    { type: "genshin.run_finished", label: text.plugin_genshin_event_run_finished, needs: ["bettergi"], data: { kind: "kind", name: "name", result: "result", message: "message" } },
    { type: "genshin.run_failed", label: text.plugin_genshin_event_run_failed, needs: ["bettergi"], data: { kind: "kind", name: "name", message: "message" } },
  ],
};

export const genshinTools: { name: string; needs: string[]; visibility: PluginVisibility[]; page: string | null }[] = [
  { name: "genshin.list", needs: ["bettergi"], visibility: BOTH, page: null },
  { name: "genshin.status", needs: ["bettergi"], visibility: BOTH, page: null },
  { name: "genshin.run", needs: ["bettergi"], visibility: BOTH, page: null },
  { name: "genshin.stop", needs: ["bettergi"], visibility: BOTH, page: null },
  { name: "genshin.webhook", needs: ["bettergi"], visibility: ["app"], page: null },
];

interface Knobs { running?: boolean; automation?: boolean; found?: boolean }
const knobs = (): Knobs => (globalThis as { __kotoneGenshin?: Knobs }).__kotoneGenshin ?? {};

// i18n-ignore-start：示例数据（假的 BetterGI：路径、配置名、计划名、日志、和后端一样的固定的话），一直到文件末尾
const DIR = "D:\\BetterGI";
const URL = "http://127.0.0.1:38648/genshin/0123456789abcdef0123456789abcdef";
let webhookToKotone = false;
let runs: GenshinRun[] = [{
  key: "run:1", kind: "dragon", name: "默认配置", status: "succeeded", message: "一条龙和配置组任务结束", step: "", error: "",
  byKotone: true, startedAtMs: Date.now() - 3_600_000, finishedAtMs: Date.now() - 1_800_000,
}];
const LISTS = {
  status: "ok",
  oneDragon: [{ name: "默认配置", selected: true }, { name: "周末", selected: false }],
  groups: [{ name: "锄地", projects: 6 }, { name: "钓鱼", projects: 2 }],
  plans: [{ id: "pln_morning", name: "早上的日常", description: "每天 8 点" }],
  plansSupported: true,
};
const STOP = "BetterGI 没有给外部停止的接口，Kotone 不能替你停，也不会结束它的进程。要停请在 BetterGI 的窗口里点停止（它的“停止任意独立任务”热键默认没设，可以在 BetterGI 的热键设置里设一个）。";
const NOT_ALLOWED = "「原神」的游戏配置设成了不允许自动化输入。BetterGI 会替你操作游戏的键盘鼠标，要先在「游戏适配」里打开“允许自动化输入”";
const BUSY = "BetterGI 已经开着：它开着时 Kotone 只能交给它计划。";

function status(): GenshinStatus {
  const k = knobs();
  if (k.found === false) return { status: "not_found", text: "没找到 BetterGI", searched: ["C:\\BetterGI", "D:\\BetterGI"] };
  const current = runs.filter((r) => r.status === "starting" || r.status === "running");
  return {
    status: "ok",
    install: { dir: DIR, exe: `${DIR}\\BetterGI.exe`, version: "0.67.0", found: "common" },
    running: !!k.running,
    webhook: { url: URL, listening: true, error: "", enabled: webhookToKotone, configured: webhookToKotone, toKotone: webhookToKotone,
      lastNoticeAtMs: webhookToKotone ? Date.now() - 60_000 : null, lastTestAtMs: null },
    external: null,
    health: null,
    plans: { supported: true, queue: { active: [], pending: 0 }, recent: [] },
    current,
    recent: runs.filter((r) => !current.includes(r)),
    log: [{ time: "09:00:00", level: "INF", text: "一条龙启动" }, { time: "09:30:00", level: "INF", text: "一条龙和配置组任务结束" }],
    stop: STOP,
    automation: k.automation === false ? { allowed: false, text: NOT_ALLOWED } : { allowed: true },
  };
}

export async function genshinCall(tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  const config = (await getSettings()).plugins?.genshin ?? {};
  if (config.sources?.bettergi === false) return { content: { status: "disabled", text: genshinManifest.sources[0].name } };
  const k = knobs();
  if (k.found === false && tool !== "genshin.stop") return { content: status() };
  switch (tool) {
    case "genshin.status": {
      const full = status();
      return { content: { status: "ok", running: full.running, webhookToKotone: webhookToKotone }, structuredContent: full };
    }
    case "genshin.list":
      return { content: structuredClone(LISTS) };
    case "genshin.stop":
      return { content: { status: "no_stop_api", text: STOP, hotkey: "" } };
    case "genshin.webhook":
      if (k.running) return { content: { status: "busy", text: "BetterGI 开着时不能改它的配置" } };
      webhookToKotone = true;
      return { content: { status: "filled", url: URL, backup: "%KOTONE_HOME%\\plugins\\genshin\\backups\\config-1.json" } };
    case "genshin.run": {
      if (k.automation === false) return { content: { status: "not_allowed", text: NOT_ALLOWED } };
      const kind = String(args.kind ?? "");
      if (k.running && kind !== "plan") return { content: { status: "busy", text: BUSY } };
      const name = typeof args.name === "string" ? args.name : "";
      const plan = LISTS.plans.find((p) => p.id === name);
      const shown = plan?.name ?? name;
      runs = [{ key: `run:${Date.now()}`, kind: kind as GenshinRun["kind"], name: shown, status: "starting", message: "", step: "", error: "",
        byKotone: true, startedAtMs: Date.now(), finishedAtMs: null }, ...runs];
      return { content: { status: "started", kind, name: shown, pid: 4242, forwarded: !!k.running, notes: [] } };
    }
    default:
      return { content: { status: "error", text: tool } };
  }
}
// i18n-ignore-end
