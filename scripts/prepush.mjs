// 推送到 main 之前，在本机把 CI 要跑的检查跑一遍（docs/development.md「推送前」）。
//
//   pnpm prepush             跑
//   pnpm prepush --dry-run   只列出会跑哪些步骤、为什么，不跑
//   pnpm prepush --all       不看改了什么，Playwright 和审计也跑
//
// 每一步失败了也接着跑后面的，最后一起报告；最后一行是“可以推送”或没过的步骤。
// 这台机器上有人打游戏：开头查游戏进程，有就把自己降到低优先级（子进程跟着降）、cargo 用 -j 2、
// Playwright 用 2 个 worker。不发任何系统输入。

import { spawn, spawnSync } from "node:child_process";
import { createWriteStream, mkdirSync, rmdirSync } from "node:fs";
import os from "node:os";
import { join, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const flags = process.argv.slice(2);
const known = new Set(["--dry-run", "--all"]);
const unknown = flags.filter((f) => !known.has(f));
if (unknown.length) {
  console.error(`不认识的参数：${unknown.join(" ")}\n用法：pnpm prepush [--dry-run] [--all]`);
  process.exit(2);
}
const dryRun = flags.includes("--dry-run");
const all = flags.includes("--all");

// 和 scripts/manual-e2e 各脚本 guard() 的名单一样（tasklist 的输出里出现就算），加上星铁、绝区零
const GAMES = [
  "league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe", "dota2.exe",
  "yuanshen", "genshinimpact", "starrail", "zenlesszonezero",
];

// Playwright 的输入：页面、e2e、文案，以及 spec 直接读的源文件（terminology.spec 读术语表、助手手册、
// 官网文案、几个 Rust 文件里玩家看得到的字；copilot-flow.spec 读 settings_guide.rs）。
// 新 spec 读了别的文件，要加到这里。
const PLAYWRIGHT_INPUTS = [
  "apps/desktop/",
  "plugins/",
  "crates/kotone-core/src/",
  "crates/kotone-agent/src/",
  "docs/glossary.md",
  "docs/assistant.md",
  "website/src/i18n/",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
];
// 审计只看锁文件（和 .github/workflows/audit.yml 一样）
const LOCKFILES = ["Cargo.lock", "pnpm-lock.yaml", ".cargo/audit.toml"];

function sh(cmd, args) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: "utf8", windowsHide: true });
  return { ok: r.status === 0, out: (r.stdout ?? "").trim() };
}

function runningGames() {
  const r = spawnSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "latin1", windowsHide: true });
  const out = (r.stdout ?? "").toLowerCase();
  return GAMES.filter((g) => out.includes(g));
}

/** 从仓库 target/ 里启动的 kotone.exe：玩家在用 `pnpm dev` 测，这时不该构建。 */
function devKotoneRunning() {
  const r = spawnSync(
    "powershell",
    ["-NoProfile", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Process kotone -ErrorAction SilentlyContinue | ForEach-Object Path"],
    { encoding: "utf8", windowsHide: true },
  );
  const target = join(root, "target").toLowerCase();
  return (r.stdout ?? "").split(/\r?\n/).map((p) => p.trim()).filter((p) => p.toLowerCase().startsWith(target));
}

/** 相对 origin/main 改了的文件（已提交 + 没提交 + 新文件）；null = 不知道（没有 origin/main）。 */
function changedFiles() {
  const base = sh("git", ["merge-base", "origin/main", "HEAD"]);
  if (!base.ok || !base.out) return null;
  const diff = sh("git", ["diff", "--name-only", base.out]);
  const untracked = sh("git", ["ls-files", "--others", "--exclude-standard"]);
  if (!diff.ok) return null;
  return [...diff.out.split("\n"), ...untracked.out.split("\n")].map((f) => f.trim()).filter(Boolean);
}

function touched(files, inputs) {
  return files.filter((f) => inputs.some((i) => (i.endsWith("/") ? f.startsWith(i) : f === i)));
}

function duration(ms) {
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

// ---------- 环境 ----------

const dev = process.platform === "win32" ? devKotoneRunning() : [];
if (dev.length) {
  console.error(`玩家在用 pnpm dev 测（${dev.join("，")} 在跑）：先别构建，等他测完再跑。`);
  if (!dryRun) process.exit(1);
}

const games = process.platform === "win32" ? runningGames() : [];
const lowPriority = games.length > 0;
const jobs = lowPriority ? { clippy: 2, test: 2, workers: 2 } : { clippy: 4, test: 2, workers: 4 };
if (lowPriority && !dryRun) {
  try {
    // Windows 上子进程（cargo、rustc、link.exe、Chrome）默认继承低于正常的优先级
    os.setPriority(os.constants.priority.PRIORITY_BELOW_NORMAL);
  } catch (e) {
    console.error(`降优先级失败（${e.message}），不跑了：有游戏在跑。`);
    process.exit(1);
  }
}

const tmp = join(root, "target", "tmp");
const logs = join(root, "target", "prepush");
const env = { ...process.env, TMP: tmp, TEMP: tmp };

const files = all ? null : changedFiles();
const why = (inputs, label) => {
  if (all) return { run: true, reason: "--all" };
  if (files === null) return { run: true, reason: "找不到 origin/main，不知道改了什么，全跑" };
  const hit = touched(files, inputs);
  return hit.length
    ? { run: true, reason: `改了 ${hit.slice(0, 3).join("、")}${hit.length > 3 ? ` 等 ${hit.length} 个` : ""}` }
    : { run: false, reason: `没改${label}` };
};
const always = { run: true, reason: "" };
const playwright = why(PLAYWRIGHT_INPUTS, "前端和 spec 读的文件");
const audit = why(LOCKFILES, "锁文件");

// 版本号校验和发版脚本测试加起来一两秒，不按“改没改版本号”挑，每次都跑（CI 的 Frontend job 也是每次跑）
const steps = [
  { name: "cargo fmt", cmd: "cargo fmt --all -- --check", ...always },
  { name: "release:verify", cmd: "pnpm release:verify", ...always },
  { name: "release:test", cmd: "pnpm release:test", ...always },
  { name: "pnpm audit", cmd: "pnpm audit --prod", ...audit },
  { name: "cargo audit", cmd: "cargo audit", ...audit },
  { name: "pnpm check", cmd: "pnpm check", ...always },
  {
    name: "cargo clippy",
    cmd: `cargo clippy --workspace --all-targets --locked -j ${jobs.clippy} -- -D warnings`,
    cargo: true,
    ...always,
  },
  { name: "cargo test", cmd: `cargo test --workspace --locked --no-fail-fast -j ${jobs.test}`, cargo: true, ...always },
  { name: "Playwright", cmd: `pnpm -C apps/desktop test:e2e --workers=${jobs.workers}`, lock: true, ...playwright },
];

console.log(
  lowPriority
    ? `有游戏在跑（${games.join("、")}）：低优先级，cargo clippy -j ${jobs.clippy}、cargo test -j ${jobs.test}，Playwright ${jobs.workers} 个 worker`
    : `没有游戏：cargo clippy -j ${jobs.clippy}、cargo test -j ${jobs.test}，Playwright ${jobs.workers} 个 worker`,
);
if (files !== null) console.log(`相对 origin/main 改了 ${files.length} 个文件`);
console.log(`TMP / TEMP = ${relative(root, tmp)}，日志在 ${relative(root, logs)}`);
for (const s of steps) console.log(`  ${s.run ? "跑  " : "跳过"} ${s.name.padEnd(15)} ${s.reason}`);
if (dryRun) process.exit(0);

// ---------- 跑 ----------

mkdirSync(tmp, { recursive: true });
mkdirSync(logs, { recursive: true });
console.log(sh("rustc", ["--version"]).out);

// 浏览器测试共用 vite 的端口：和真机脚本一样拿 target/tmp/pw.lock（一个目录），只删自己建的
const pwLock = join(tmp, "pw.lock");
let holdingLock = false;
async function takeLock() {
  const deadline = Date.now() + 10 * 60 * 1000;
  for (;;) {
    try {
      mkdirSync(pwLock);
      holdingLock = true;
      return true;
    } catch (e) {
      if (e.code !== "EEXIST") throw e;
      if (Date.now() > deadline) return false;
      console.log(`${relative(root, pwLock)} 被别的脚本拿着，等它放开……`);
      await new Promise((r) => setTimeout(r, 15_000));
    }
  }
}
function releaseLock() {
  if (!holdingLock) return;
  holdingLock = false;
  try {
    rmdirSync(pwLock);
  } catch {
    // 已经没了
  }
}
process.on("SIGINT", () => {
  releaseLock();
  process.exit(130);
});

function run(step, index) {
  const log = join(logs, `${String(index + 1).padStart(2, "0")}-${step.name.replace(/[^a-z0-9]+/gi, "-")}.log`);
  return new Promise((done) => {
    const file = createWriteStream(log);
    let text = "";
    const child = spawn(step.cmd, { cwd: root, env, shell: true, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    const tee = (out) => (chunk) => {
      out.write(chunk);
      file.write(chunk);
      if (step.cargo) text += chunk.toString();
    };
    child.stdout.on("data", tee(process.stdout));
    child.stderr.on("data", tee(process.stderr));
    child.on("error", (e) => {
      file.end();
      done({ ok: false, log, note: e.message });
    });
    child.on("close", (code) => {
      file.end();
      let ok = code === 0;
      let note = ok ? "" : `退出码 ${code}`;
      if (step.cargo) {
        // 退出码之外再看日志：要编完（Finished），而且没有一行以 error 开头
        const plain = text.replace(/\x1b\[[0-9;]*m/g, "");
        if (!/Finished/.test(plain)) [ok, note] = [false, note || "日志里没有 Finished"];
        if (/^error/m.test(plain)) [ok, note] = [false, note || "日志里有 error"];
      }
      if (!ok && step.name === "cargo audit" && /no such command/.test(text)) note = "没装 cargo-audit：cargo install cargo-audit --locked";
      done({ ok, log, note });
    });
  });
}

const results = [];
for (const [index, step] of steps.entries()) {
  if (!step.run) {
    results.push({ step, status: "跳过", note: step.reason });
    continue;
  }
  console.log(`\n==> ${step.name}: ${step.cmd}`);
  if (step.lock && !(await takeLock())) {
    results.push({ step, status: "失败", note: `${relative(root, pwLock)} 等了 10 分钟还被拿着（确认没人在跑浏览器测试再删）` });
    continue;
  }
  const started = Date.now();
  const r = await run(step, index);
  if (step.lock) releaseLock();
  results.push({ step, status: r.ok ? "通过" : "失败", took: duration(Date.now() - started), note: r.ok ? "" : `${r.note}，日志 ${relative(root, r.log)}` });
}

console.log("\n—— 结果 ——");
for (const r of results) console.log(`  ${r.status}  ${r.step.name.padEnd(15)} ${(r.took ?? "").padEnd(7)} ${r.note ?? ""}`);
const failed = results.filter((r) => r.status === "失败").map((r) => r.step.name);
console.log(failed.length ? `不能推送：${failed.join("、")} 没过` : "可以推送");
process.exit(failed.length ? 1 : 0);
