// 生成更新器清单（GitHub、旧客户端镜像兼容、腾讯云 COS），本地与 CI release 共用。
//
// 0.1.7 及更早客户端内置了第三方镜像回退链，所以暂时继续生成对应 JSON；
// 新客户端优先读取 COS 清单并下载 COS 安装包，GitHub 官方地址作为兜底。
// tauri-action 自己也会上传一份 latest.json（asset URL 是 api.github.com 临时地址），
// CI 会在其后用本脚本产物 --clobber 覆盖为稳定的 releases/download 直链版本。
//
// 用法：
//   node scripts/make-updater-jsons.mjs --exe <setup.exe 路径> --sig <.sig 路径> --tag v0.1.5
//     [--notes 文本] [--changelog CHANGELOG.md] [--cos-base-url URL] [--out-dir 目录]
import { readFile, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { requireReleaseNotes } from "./release-notes.mjs";

const args = process.argv.slice(2);
function opt(name, fallback) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
}

const exe = opt("--exe");
const sig = opt("--sig");
const tag = opt("--tag");
const notesArg = opt("--notes");
const scriptDir = dirname(fileURLToPath(import.meta.url));
const changelogPath = opt("--changelog", resolve(scriptDir, "../CHANGELOG.md"));
const cosBaseUrl = opt("--cos-base-url")?.replace(/\/$/, "");
const outDir = opt("--out-dir", exe ? dirname(exe) : undefined);

if (!exe || !sig || !tag) {
  console.error(
    "用法: node scripts/make-updater-jsons.mjs --exe <setup.exe> --sig <setup.exe.sig> --tag vX.Y.Z [--notes 文本] [--changelog CHANGELOG.md] [--cos-base-url URL] [--out-dir 目录]",
  );
  process.exit(1);
}

const version = tag.replace(/^v/, "");

/** 从 CHANGELOG 取当前 tag 的小节；显式 --notes 始终优先。 */
async function resolveNotes() {
  if (notesArg?.trim()) return notesArg.trim();
  const changelog = await readFile(changelogPath, "utf8");
  return requireReleaseNotes(changelog, version);
}

/** 兼容 0.1.7 及更早客户端内置的 updater endpoints。 */
const MIRRORS = [
  { file: "latest-cn-ghfast.json", prefix: "https://ghfast.top/" },
  { file: "latest-cn-ghproxy.json", prefix: "https://gh-proxy.com/" },
  { file: "latest-cn-ghproxynet.json", prefix: "https://ghproxy.net/" },
  { file: "latest-cn-llkk.json", prefix: "https://gh.llkk.cc/" },
];

const notes = await resolveNotes();
const signature = (await readFile(sig, "utf8")).trim();
const url = `https://github.com/l1veIn/kotone/releases/download/${tag}/${basename(exe)}`;
const pubDate = new Date().toISOString().replace(/\.\d+Z$/, "Z");

function manifest(installerUrl) {
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: {
      "windows-x86_64": { signature, url: installerUrl },
      "windows-x86_64-nsis": { signature, url: installerUrl },
    },
  };
}

const doc = manifest(url);

const latestPath = join(outDir, "latest.json");
await writeFile(latestPath, `${JSON.stringify(doc, null, 2)}\n`);
console.log(`已生成 ${latestPath}`);

for (const mirror of MIRRORS) {
  const variant = structuredClone(doc);
  for (const platform of Object.values(variant.platforms)) {
    platform.url = mirror.prefix + platform.url;
  }
  const out = join(outDir, mirror.file);
  await writeFile(out, `${JSON.stringify(variant, null, 2)}\n`);
  console.log(`已生成 ${out}`);
}

if (cosBaseUrl) {
  const cosUrl = `${cosBaseUrl}/${tag}/${basename(exe)}`;
  const cosPath = join(outDir, "latest-cos.json");
  await writeFile(cosPath, `${JSON.stringify(manifest(cosUrl), null, 2)}\n`);
  console.log(`已生成 ${cosPath}`);
}
