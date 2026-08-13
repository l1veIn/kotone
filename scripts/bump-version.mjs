#!/usr/bin/env node
/**
 * 版本提升脚本：一次调用同步全部版本来源 + 预置 CHANGELOG 小节 + 兜底校验。
 *
 * 版本单一来源原则：以 apps/desktop/package.json 的 version 为当前版本，
 * 目标版本由显式入参或提升模式计算，随后写入全部四处来源：
 *   - apps/desktop/package.json
 *   - apps/desktop/src-tauri/tauri.conf.json
 *   - apps/desktop/src-tauri/Cargo.toml（[package] 块）
 *   - Cargo.lock（kotone-tauri 包版本）
 * 最后调用 scripts/verify-release-version.mjs 兜底校验一致性（含 tag 命名）。
 * 只替换版本行本身，不触碰文件其余格式，保证 diff 最小。
 *
 * 用法：
 *   node scripts/bump-version.mjs 0.1.7-rc.3            显式指定新版本
 *   node scripts/bump-version.mjs --rc                  0.1.7-rc.2 → 0.1.7-rc.3；0.1.7 → 0.1.7-rc.1
 *   node scripts/bump-version.mjs --patch               0.1.7-rc.2 → 0.1.7（定稿）；0.1.7 → 0.1.8
 *   node scripts/bump-version.mjs --minor               0.1.7 → 0.2.0（清空预发布）
 *   node scripts/bump-version.mjs --major               0.1.7 → 1.0.0（清空预发布）
 *   node scripts/bump-version.mjs 0.1.7-rc.3 --dry-run      只打印计划，不写盘
 *   node scripts/bump-version.mjs 0.1.7-rc.3 --no-changelog 不动 CHANGELOG.md
 */
import { readFile, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { prepareChangelogVersion } from "./release-notes.mjs";

const root = resolve(import.meta.dirname, "..");
const FILES = {
  "apps/desktop/package.json": resolve(root, "apps/desktop/package.json"),
  "apps/desktop/src-tauri/tauri.conf.json": resolve(root, "apps/desktop/src-tauri/tauri.conf.json"),
  "apps/desktop/src-tauri/Cargo.toml": resolve(root, "apps/desktop/src-tauri/Cargo.toml"),
  "Cargo.lock": resolve(root, "Cargo.lock"),
};

/** 从各版本源提取当前版本（JSON：version 字段；Cargo.toml：[package] 块；Cargo.lock：kotone-tauri 包） */
function extractVersion(key, text) {
  if (key.endsWith("Cargo.toml"))
    return text.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
  if (key.endsWith("Cargo.lock"))
    return text.match(/^\[\[package\]\]\r?\nname = "kotone-tauri"\r?\nversion\s*=\s*"([^"]+)"/m)?.[1];
  return JSON.parse(text).version;
}
/** 只替换版本行本身，保持其余字节不变（\r?\n 容忍 git autocrlf 的 CRLF 工作区） */
function replaceVersion(key, text, old, target) {
  if (key.endsWith("Cargo.toml"))
    return text.replace(/^(\[package\][\s\S]*?^version\s*=\s*)"[^"]*"/m, `$1"${target}"`);
  if (key.endsWith("Cargo.lock"))
    return text.replace(/^(\[\[package\]\]\r?\nname = "kotone-tauri"\r?\nversion\s*=\s*)"[^"]*"/m, `$1"${target}"`);
  return text.replace(`"version": "${old}"`, `"version": "${target}"`);
}
const CHANGELOG = resolve(root, "CHANGELOG.md");
const VERIFY_SCRIPT = resolve(root, "scripts/verify-release-version.mjs");

const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
function parseVersion(v) {
  const m = SEMVER_RE.exec(v);
  return m ? { major: +m[1], minor: +m[2], patch: +m[3], pre: m[4] ?? null } : null;
}
function formatVersion(v) {
  return `${v.major}.${v.minor}.${v.patch}${v.pre ? `-${v.pre}` : ""}`;
}
/** 本地日期（CHANGELOG 标题用的就是本地日期，不能用 UTC 以免跨日） */
function todayLocal() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

// ---------- 参数 ----------
const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const noChangelog = args.includes("--no-changelog");
const MODE_FLAGS = ["--rc", "--patch", "--minor", "--major"];
const modeFlags = MODE_FLAGS.filter((f) => args.includes(f));
const explicit = args.find((a) => !a.startsWith("--"));

if (explicit && modeFlags.length > 0) {
  console.error("显式版本号与提升模式（--rc/--patch/--minor/--major）不能同时使用。");
  process.exit(1);
}
if (modeFlags.length > 1) {
  console.error(`提升模式只能选一个，当前给了：${modeFlags.join(" ")}`);
  process.exit(1);
}

// ---------- 当前版本 & 目标版本 ----------
const current = JSON.parse(await readFile(FILES["apps/desktop/package.json"], "utf8")).version;
const cur = parseVersion(current);
if (!cur) {
  console.error(`当前版本无法解析：${current}`);
  process.exit(1);
}

let target;
if (explicit) {
  if (!parseVersion(explicit)) {
    console.error(`无效版本号：${explicit}（应为 SemVer，如 0.1.7 或 0.1.7-rc.2）`);
    process.exit(1);
  }
  target = explicit;
} else if (modeFlags[0] === "--rc") {
  target = cur.pre && /^rc\.\d+$/.test(cur.pre)
    ? formatVersion({ ...cur, pre: `rc.${Number(cur.pre.split(".")[1]) + 1}` })
    : formatVersion({ ...cur, pre: "rc.1" });
} else if (modeFlags[0] === "--patch") {
  // 有预发布 → 定稿（去预发布后缀）；无 → 补丁 +1
  target = cur.pre
    ? formatVersion({ ...cur, pre: null })
    : formatVersion({ ...cur, patch: cur.patch + 1 });
} else if (modeFlags[0] === "--minor") {
  target = formatVersion({ major: cur.major, minor: cur.minor + 1, patch: 0, pre: null });
} else if (modeFlags[0] === "--major") {
  target = formatVersion({ major: cur.major + 1, minor: 0, patch: 0, pre: null });
} else {
  console.error("缺少新版本号或提升模式（--rc / --patch / --minor / --major）。");
  console.error("用法见脚本头注释。");
  process.exit(1);
}

if (target === current) {
  console.error(`目标版本与当前一致（${current}），没有可提升的。`);
  process.exit(1);
}

console.log(`当前版本：${current}`);
console.log(`目标版本：${target}${dryRun ? "（dry-run，不写盘）" : ""}`);
console.log("将更新：");
for (const key of Object.keys(FILES)) console.log(`  · ${key}`);
if (!noChangelog) console.log("  · CHANGELOG.md（预置小节标题）");
if (dryRun) {
  console.log("dry-run 结束，未做任何修改。");
  process.exit(0);
}

// 先计算 CHANGELOG 变更；有冲突时必须在修改版本文件之前失败，避免半更新状态。
let nextChangelog = null;
let changelogMessage = null;
if (!noChangelog) {
  const text = await readFile(CHANGELOG, "utf8");
  try {
    const plan = prepareChangelogVersion(text, target, todayLocal());
    if (plan.kind !== "unchanged") nextChangelog = plan.text;
    changelogMessage = plan.kind === "archived"
      ? `  ✔ CHANGELOG.md：已将 Unreleased 归档为 "## ${target} —"`
      : plan.kind === "inserted"
        ? `  ✔ CHANGELOG.md：已插入 "## ${target} —" 小节标题（摘要留待补充）`
        : `ℹ CHANGELOG.md 已有 ${target} 小节，跳过插入`;
  } catch (error) {
    console.error(`${error.message} 已停止版本提升。`);
    process.exit(1);
  }
}

// ---------- 读取各文件当前版本（独立提取，不假设三处已一致） ----------
const entries = [];
for (const [key, path] of Object.entries(FILES)) {
  const text = await readFile(path, "utf8");
  const old = extractVersion(key, text);
  if (old === undefined) {
    console.error(`无法从 ${key} 提取当前版本，请检查文件。`);
    process.exit(1);
  }
  if (old === target) {
    console.log(`ℹ ${key} 已是 ${target}，跳过`);
    continue;
  }
  entries.push({ key, path, text, old });
}

// ---------- 写盘（仅替换版本行） ----------
for (const { key, path, text, old } of entries) {
  const next = replaceVersion(key, text, old, target);
  if (next === text) {
    console.error(`未能在 ${path} 替换版本 "${old}"，请检查文件。`);
    process.exit(1);
  }
  await writeFile(path, next);
  console.log(`  ✔ ${key}：${old} → ${target}`);
}

// ---------- CHANGELOG：应用预先校验过的变更 ----------
if (nextChangelog !== null) await writeFile(CHANGELOG, nextChangelog);
if (changelogMessage) console.log(changelogMessage);

// ---------- 兜底校验（复用 release:verify，含 tag 命名核对） ----------
console.log("运行 release:verify 兜底校验…");
const res = spawnSync(process.execPath, [VERIFY_SCRIPT, `v${target}`], {
  cwd: root,
  stdio: "inherit",
});
if (res.status !== 0) {
  console.error("release:verify 未通过，请检查版本一致性后重试。");
  process.exit(1);
}

console.log(`
版本已提升到 v${target}。接下来：
  1. 检查 CHANGELOG.md 的 "## ${target} —" 发布摘要
  2. git add apps/desktop Cargo.lock CHANGELOG.md
     git commit -m "release: prepare Kotone v${target}"
  3. git tag v${target}
     git push origin main --tags   # 触发 .github/workflows/release.yml 的 Release Windows
`);
