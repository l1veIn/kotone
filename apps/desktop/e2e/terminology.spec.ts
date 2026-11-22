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
  for (const file of walk(join(app, "src"), [".svelte", ".ts"])) {
    if (/browserMock\.ts$|persona\.ts$|vite-env\.d\.ts$/.test(file)) continue;
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
  expect(catalogs.map((n) => `./messages/{locale}/${n}`).sort()).toEqual([...patterns].sort());
  const sources = walk(join(app, "src"), [".svelte", ".ts"]).map((f) => readFileSync(f, "utf8")).join("\n");
  const unused: string[] = [];
  const seen = new Map<string, string>();
  for (const name of catalogs) {
    const messages = JSON.parse(readFileSync(join(app, "messages", "zh-CN", name), "utf8"));
    for (const key of Object.keys(messages)) {
      if (key === "$schema") continue;
      expect(seen.has(key), `${key} 在 ${seen.get(key)} 和 ${name} 里重复`).toBe(false);
      seen.set(key, name);
      if (!new RegExp(`\\bm\\.${key}\\b`).test(sources)) unused.push(`${name}: ${key}`);
    }
  }
  expect(unused).toEqual([]);
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
  const rust = [
    "crates/kotone-core/src/settings_guide.rs",
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
