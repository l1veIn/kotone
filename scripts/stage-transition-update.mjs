// Creates a local rollout manifest from an existing signed COS manifest. No upload.
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const transitionNotes = [
  "语音输入法照常使用，原来的录音热键和识别设置会保留。",
  "琴音由你选择开启，不配置 API Key 就不会产生琴音费用。",
  "升级前会备份本机数据，也可以手动重装旧版。",
  "完整更新日志：https://github.com/l1veIn/kotone/releases/tag/v0.2.0",
].join("\n");

export function transitionManifest(source, percent) {
  if (!Number.isInteger(percent) || percent < 0 || percent > 100) {
    throw new Error("rolloutPercent 必须是 0–100 的整数");
  }
  const platform = source?.platforms?.["windows-x86_64-nsis"] ?? source?.platforms?.["windows-x86_64"];
  if (source?.version !== "0.2.0" || !platform?.signature || !/^https:\/\//.test(platform?.url ?? "")) {
    throw new Error("需要 0.2.0 稳定版的 Windows 安装包清单和签名");
  }
  return { ...source, rolloutPercent: percent, notes: transitionNotes };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [source, outDir, input] = process.argv.slice(2);
  if (!source || !outDir || !/^\d{1,3}$/.test(input ?? "")) {
    throw new Error("用法：node scripts/stage-transition-update.mjs <签名清单> <输出目录> <0-100>");
  }
  const target = resolve(outDir, "latest-0.2.json");
  if (resolve(source) === target) throw new Error("请指定原始签名清单作为输入");
  const payload = transitionManifest(JSON.parse(await readFile(source, "utf8")), Number(input));
  await writeFile(target, JSON.stringify(payload, null, 2) + "\n");
  console.log(target);
}
