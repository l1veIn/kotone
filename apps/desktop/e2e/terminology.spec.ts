// 界面用语与 i18n 的守门测试（docs/glossary.md “界面用语（v2）”）。不开浏览器，只读源码。
//
// 1. 前端（src/**/*.svelte、*.ts）里不再写中文：文字都在 messages/zh-CN/*.json，用 m.xxx() 取。
//    注释不算；故意留下的（和后端文字比对的、存进配置的默认值、角色内容）在那一行写 `i18n-ignore` 并说明原因。
// 2. 术语表“不说”一栏的词，不出现在玩家看得到、模型会照着说的地方：界面文案、气泡和她念的固定的话、
//    设置说明、系统提示和工具说明、docs/assistant.md、官网。注释、日志、测试不查。
// 3. inlang 插件从 node_modules 读，不从网上拉（编译时不联网、不跑网上下载的代码）。
import { expect, test } from "@playwright/test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = join(here, "..");
const repo = join(app, "..", "..");
const cjk = /[一-鿿]/;
/** 插件（ADR 038）：plugins/<id>/ 下有自己的页面（ui/）和文案（messages/<语言>.json）。 */
const pluginsDir = join(repo, "plugins");
const pluginIds = readdirSync(pluginsDir).filter((id) => statSync(join(pluginsDir, id)).isDirectory());
const pluginCatalogs = pluginIds
  .filter((id) => { try { return statSync(join(pluginsDir, id, "messages", "zh-CN.json")).isFile(); } catch { return false; } })
  .map((id) => ({ id, zh: join(pluginsDir, id, "messages", "zh-CN.json"), en: join(pluginsDir, id, "messages", "en.json") }));
/** 插件在宿主里的页面（ui/），和插件自己的 MCP App（app/src/，ADR 039 D5；文案也用 `m.<键>(…)` 取）。 */
const pluginUiFiles = () => pluginIds.flatMap((id) => ["ui", join("app", "src")].flatMap((dir) => {
  try { return walk(join(pluginsDir, id, dir), [".svelte", ".ts"]); } catch { return []; }
}));
/** 插件的名字、说明、数据开关的名字 / 说明 / 风险提示、登记的游戏事件的名字（`event_<名字>`，lib/gameEvents.ts）由宿主按约定的键取
 * （lib/plugins.ts 的 pluginText），代码里不直接写 m.xxx。 */
const pluginConventionKey = /^plugin_[a-z0-9_]+?_(name|description|source_[a-z0-9_]+_(name|desc|risk)|event_[a-z0-9_]+)$/;

function walk(dir: string, exts: string[]): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "paraglide" || name === "node_modules" ? [] : walk(path, exts);
    return exts.some((ext) => name.endsWith(ext)) ? [path] : [];
  });
}

/** 去掉注释后的每一行（块注释、行注释、HTML 注释）。 */
function codeLines(text: string): [number, string][] {
  const out: [number, string][] = [];
  let block = false;
  text.split(/\r?\n/).forEach((raw, index) => {
    let line = raw;
    if (block) {
      const end = line.search(/\*\/|-->/);
      if (end < 0) return;
      line = line.slice(end + 3);
      block = false;
    }
    line = line.replace(/<!--.*?-->/g, "").replace(/\/\*.*?\*\//g, "");
    const open = line.search(/\/\*|<!--/);
    if (open >= 0) {
      block = true;
      line = line.slice(0, open);
    }
    line = line.replace(/(^|[^:"'`])\/\/.*$/, "$1");
    if (/^\s*\*/.test(line)) return;
    out.push([index + 1, line]);
  });
  return out;
}

/** 术语表“界面用语”一节“不说”一栏里的词。 */
function bannedWords(): string[] {
  const glossary = readFileSync(join(repo, "docs", "glossary.md"), "utf8");
  const section = glossary.split("## 界面用语（v2")[1].split("\n## ")[0];
  const words = new Set<string>();
  for (const line of section.split(/\r?\n/)) {
    if (!line.startsWith("| ") || line.startsWith("| 说法") || line.startsWith("| ---")) continue;
    const cells = line.trim().replace(/^\||\|$/g, "").split("|");
    for (const match of (cells[2] ?? "").matchAll(/`([^`]+)`/g)) words.add(match[1]);
  }
  return [...words];
}

/** Rust 文件里玩家看得到的字符串：去掉注释、日志、测试模块。 */
function rustStrings(path: string): [number, string][] {
  const text = readFileSync(path, "utf8").split("#[cfg(test)]")[0];
  const out: [number, string][] = [];
  for (const [line, code] of codeLines(text)) {
    if (/log::log\(|eprintln!|println!|^\s*#\[/.test(code)) continue;
    for (const match of code.matchAll(/"((?:[^"\\]|\\.)*)"/g)) out.push([line, match[1]]);
  }
  return out;
}

/** 写了 `i18n-ignore` 的行，以及 `i18n-ignore-start` 到 `i18n-ignore-end` 之间的行（行号从 1 起）。 */
function ignoredLines(text: string): Set<number> {
  const ignored = new Set<number>();
  let block = false;
  text.split(/\r?\n/).forEach((raw, index) => {
    if (raw.includes("i18n-ignore-start")) block = true;
    if (block || raw.includes("i18n-ignore")) ignored.add(index + 1);
    if (raw.includes("i18n-ignore-end")) block = false;
  });
  return ignored;
}

test("the frontend keeps its words in messages/zh-CN, not in code", () => {
  const hits: string[] = [];
  for (const file of [...walk(join(app, "src"), [".svelte", ".ts"]), ...pluginUiFiles()]) {
    // 预览用的模拟数据（假歌单、假歌名）不是界面文字
    if (/browserMock\.ts$|musicMock\.ts$|persona\.ts$|vite-env\.d\.ts$/.test(file)) continue;
    const text = readFileSync(file, "utf8");
    const ignored = ignoredLines(text);
    for (const [line, code] of codeLines(text)) {
      if (cjk.test(code) && !ignored.has(line) && !/console\.(log|warn|error|info)\(/.test(code)) {
        hits.push(`${relative(app, file)}:${line}: ${code.trim()}`);
      }
    }
  }
  expect(hits).toEqual([]);
});

test("every message in messages/zh-CN is used, and the catalogs agree with project.inlang", () => {
  const settings = JSON.parse(readFileSync(join(app, "project.inlang", "settings.json"), "utf8"));
  expect(settings.modules.every((m: string) => !/^https?:/.test(m)), "插件不从网上拉").toBe(true);
  const patterns: string[] = settings["plugin.inlang.messageFormat"].pathPattern;
  const catalogs = readdirSync(join(app, "messages", "zh-CN")).filter((n) => n.endsWith(".json"));
  expect([
    ...catalogs.map((n) => `./messages/{locale}/${n}`),
    ...pluginCatalogs.map((p) => `../../plugins/${p.id}/messages/{locale}.json`),
  ].sort()).toEqual([...patterns].sort());
  const sources = [...walk(join(app, "src"), [".svelte", ".ts"]), ...pluginUiFiles()].map((f) => readFileSync(f, "utf8")).join("\n");
  const unused: string[] = [];
  const seen = new Map<string, string>();
  const files = [
    ...catalogs.map((name) => ({ name, path: join(app, "messages", "zh-CN", name), plugin: null as string | null })),
    ...pluginCatalogs.map((p) => ({ name: `plugins/${p.id}`, path: p.zh, plugin: p.id })),
  ];
  for (const { name, path, plugin } of files) {
    const messages = JSON.parse(readFileSync(path, "utf8"));
    for (const key of Object.keys(messages)) {
      if (key === "$schema") continue;
      expect(seen.has(key), `${key} 在 ${seen.get(key)} 和 ${name} 里重复`).toBe(false);
      seen.set(key, name);
      // 插件的文案以 plugin_<id>_ 开头（不和本体的撞）
      if (plugin) expect(key.startsWith(`plugin_${plugin.replace(/-/g, "_")}_`), `${name}: ${key}`).toBe(true);
      if (plugin && pluginConventionKey.test(key)) continue;
      if (!new RegExp(`\\bm\\.${key}\\b`).test(sources)) unused.push(`${name}: ${key}`);
    }
  }
  expect(unused).toEqual([]);
});

test("every message has an English translation with the same parameters", () => {
  // 英文里可以留的中文：玩家嘴里说的语音命令、唤醒词、角色名（英文界面也得照着说中文）
  const spoken = ["琴音琴音", "你好琴音", "没事了", "退下", "安静", "挂了", "停下", "可以", "不要", "允许", "拜拜", "再见",
    "琴音", "大肥鱼", "Claude大小姐", "ことね"];
  const bilingual = new Set(["locale_title"]); // “语言 / Language”：两种语言都写，选错了也找得到
  const cjkOrFullWidth = /[぀-ヿ一-鿿　-〿！-～]/;
  const params = (text: string) =>
    [...text.replace(/\\[{}]/g, "").matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map((m) => m[1]).sort().join(",");
  const zhDir = join(app, "messages", "zh-CN");
  const enDir = join(app, "messages", "en");
  expect(readdirSync(enDir).sort()).toEqual(readdirSync(zhDir).sort());
  const problems: string[] = [];
  const pairs = [
    ...readdirSync(zhDir).map((name) => ({ name, zh: join(zhDir, name), en: join(enDir, name) })),
    ...pluginCatalogs.map((p) => ({ name: `plugins/${p.id}`, zh: p.zh, en: p.en })),
  ];
  for (const { name, zh: zhPath, en: enPath } of pairs) {
    const zh = JSON.parse(readFileSync(zhPath, "utf8"));
    const en = JSON.parse(readFileSync(enPath, "utf8"));
    for (const [key, source] of Object.entries<string>(zh)) {
      if (key === "$schema") continue;
      const value = en[key];
      if (typeof value !== "string" || !value.trim()) { problems.push(`${name} ${key}: 没有英文`); continue; }
      if (params(source) !== params(value)) problems.push(`${name} ${key}: 参数 ${params(value)} ≠ ${params(source)}`);
      const rest = spoken.reduce((text, word) => text.split(word).join(""), value);
      if (!bilingual.has(key) && cjkOrFullWidth.test(rest)) problems.push(`${name} ${key}: 英文里还有中文 ${value}`);
    }
    for (const key of Object.keys(en)) if (!(key in zh)) problems.push(`${name} ${key}: 中文里没有这条`);
  }
  expect(problems).toEqual([]);
});

test("words the glossary says not to use stay out of everything players see or she says", () => {
  const banned = bannedWords();
  expect(banned.length).toBeGreaterThan(20);
  const hits: string[] = [];
  // 纯英文的词按整词找（“ASR”不算“X-ASR”这种产品名里的一截，“Skill”不算 SKILL.md）
  const patterns = banned.map((word) => /^[A-Za-z ]+$/.test(word)
    ? new RegExp(`(?<![A-Za-z-])${word}(?![A-Za-z])`)
    : new RegExp(word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  const check = (where: string, text: string) => {
    banned.forEach((word, index) => {
      if (patterns[index].test(text)) hits.push(`${where}: 「${word}」 ${text.slice(0, 80)}`);
    });
  };
  for (const name of readdirSync(join(app, "messages", "zh-CN"))) {
    const messages = JSON.parse(readFileSync(join(app, "messages", "zh-CN", name), "utf8"));
    for (const [key, value] of Object.entries(messages)) if (key !== "$schema") check(`messages/zh-CN/${name} ${key}`, String(value));
  }
  // 插件的文案，以及 plugin.toml 里给她的话（系统提示）、名字和风险提示
  for (const p of pluginCatalogs) {
    const messages = JSON.parse(readFileSync(p.zh, "utf8"));
    for (const [key, value] of Object.entries(messages)) check(`plugins/${p.id}/messages ${key}`, String(value));
  }
  for (const id of pluginIds) {
    let toml = "";
    try { toml = readFileSync(join(pluginsDir, id, "plugin.toml"), "utf8"); } catch { continue; }
    toml.split(/\r?\n/).forEach((line, index) => { if (!line.trimStart().startsWith("#")) check(`plugins/${id}/plugin.toml:${index + 1}`, line); });
  }
  const rust = [
    "crates/kotone-core/src/settings_guide.rs",
    // 热键表里每个热键的名字和在哪改（hotkeys.status / hotkeys.conflicts 原样给她和外部 Agent，ADR 040）
    "crates/kotone-core/src/hotkey_table.rs",
    "crates/kotone-agent/src/prompt.rs",
    "crates/kotone-agent/src/catalog.rs",
    "apps/desktop/src-tauri/src/bubble.rs",
    "apps/desktop/src-tauri/src/fillers.rs",
    "apps/desktop/src-tauri/src/voice_ask.rs",
    "apps/desktop/src-tauri/src/call.rs",
    "apps/desktop/src-tauri/src/tray.rs",
    "apps/desktop/src-tauri/src/companion.rs",
  ];
  for (const file of rust) {
    for (const [line, value] of rustStrings(join(repo, file))) check(`${file}:${line}`, value);
  }
  for (const file of ["docs/assistant.md", "website/src/i18n/zh-CN.json"]) {
    readFileSync(join(repo, file), "utf8").split(/\r?\n/).forEach((line, index) => check(`${file}:${index + 1}`, line));
  }
  expect(hits).toEqual([]);
});
