// 生成更新器清单（latest.json + 各镜像回退变体），本地与 CI release 共用。
//
// 背景：updater endpoints 是多镜像回退链（见 tauri.conf.json plugins.updater.endpoints），
// 每个镜像一份独立 JSON，安装包 URL 用同一镜像前缀——某镜像整体挂掉时，
// 更新器落到下一个镜像端点，拿到的安装包地址也走同一个存活镜像。
// tauri-action 自己也会上传一份 latest.json（asset URL 是 api.github.com 临时地址），
// CI 会在其后用本脚本产物 --clobber 覆盖为稳定的 releases/download 直链版本。
//
// 用法：
//   node scripts/make-updater-jsons.mjs --exe <setup.exe 路径> --sig <.sig 路径> --tag v0.1.5 [--notes 文本] [--out-dir 目录]
import { readFile, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

const args = process.argv.slice(2);
function opt(name, fallback) {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
}

const exe = opt("--exe");
const sig = opt("--sig");
const tag = opt("--tag");
const notes = opt("--notes", "完整变更见仓库 CHANGELOG。");
const outDir = opt("--out-dir", exe ? dirname(exe) : undefined);

if (!exe || !sig || !tag) {
  console.error(
    "用法: node scripts/make-updater-jsons.mjs --exe <setup.exe> --sig <setup.exe.sig> --tag vX.Y.Z [--notes 文本] [--out-dir 目录]",
  );
  process.exit(1);
}

/** 与 tauri.conf.json updater endpoints 一一对应；改这里要同步改配置 */
const MIRRORS = [
  { file: "latest-cn-ghfast.json", prefix: "https://ghfast.top/" },
  { file: "latest-cn-ghproxy.json", prefix: "https://gh-proxy.com/" },
  { file: "latest-cn-ghproxynet.json", prefix: "https://ghproxy.net/" },
  { file: "latest-cn-llkk.json", prefix: "https://gh.llkk.cc/" },
];

const version = tag.replace(/^v/, "");
const signature = (await readFile(sig, "utf8")).trim();
const url = `https://github.com/l1veIn/kotone/releases/download/${tag}/${basename(exe)}`;

const doc = {
  version,
  notes,
  pub_date: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
  platforms: {
    "windows-x86_64": { signature, url },
    "windows-x86_64-nsis": { signature, url },
  },
};

const latestPath = join(outDir, "latest.json");
await writeFile(latestPath, JSON.stringify(doc));
console.log(`已生成 ${latestPath}`);

for (const mirror of MIRRORS) {
  const variant = structuredClone(doc);
  for (const platform of Object.values(variant.platforms)) {
    platform.url = mirror.prefix + platform.url;
  }
  const out = join(outDir, mirror.file);
  await writeFile(out, JSON.stringify(variant));
  console.log(`已生成 ${out}`);
}
