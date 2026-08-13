/*
 * 更新服务：设置窗口打开后静默触发一次，关于页也可随时手动触发。
 * 两个入口复用同一流程：check → ask → downloadAndInstall → ask → relaunch。
 */
import { check } from "@tauri-apps/plugin-updater";
import { ask } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri, logFrontendError } from "./ipc";
import { toastInfo, toastWarn } from "./stores/ui";

export type UpdateCheckResult =
  | { status: "up-to-date" }
  | { status: "available"; version: string; notes?: string }
  | { status: "downloaded"; version: string; notes?: string }
  | { status: "error"; message: string };

let updateCheckInFlight: Promise<UpdateCheckResult> | null = null;

/** updater 清单中的 notes 是纯文本展示；去掉常见 Markdown 标记并限制弹窗长度。 */
function readableNotes(body: string | undefined | null): string | undefined {
  const notes = body
    ?.trim()
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\[([^\]]+)]\((https?:\/\/[^)]+)\)/g, "$1（$2）")
    .replace(/\*\*([^*\n]+)\*\*/g, "$1")
    .replace(/`([^`\n]+)`/g, "$1");
  if (!notes) return undefined;
  const characters = Array.from(notes);
  return characters.length <= 3000 ? notes : `${characters.slice(0, 2999).join("")}…`;
}

/** 检查一次更新；自动检查和手动点击重叠时复用同一个请求。 */
export function checkForUpdates(): Promise<UpdateCheckResult> {
  if (updateCheckInFlight) return updateCheckInFlight;
  const task = runUpdateCheck().finally(() => {
    if (updateCheckInFlight === task) updateCheckInFlight = null;
  });
  updateCheckInFlight = task;
  return task;
}

/** 有更新时询问用户是否立即下载安装并重启。 */
async function runUpdateCheck(): Promise<UpdateCheckResult> {
  // dev:web 使用稳定结果，方便关于页和 E2E 验证完整交互。
  if (!isTauri) return { status: "up-to-date" };
  try {
    const update = await check();
    if (!update) return { status: "up-to-date" };
    const notes = readableNotes(update.body);
    const details = notes ? `\n\n本次更新：\n${notes}` : "";
    const yes = await ask(`发现新版本 v${update.version}${details}\n\n是否立即更新？`, {
      title: "Kotone 更新",
      kind: "info",
      okLabel: "立即更新",
      cancelLabel: "稍后",
    });
    if (!yes) return { status: "available", version: update.version, notes };
    toastInfo(`正在下载 v${update.version} 更新包…`);
    await update.downloadAndInstall();
    const restart = await ask("更新完成，立即重启生效？", {
      title: "Kotone 更新",
      kind: "info",
      okLabel: "立即重启",
      cancelLabel: "稍后",
    });
    if (!restart) {
      toastWarn("更新已就绪，将在下次启动时生效");
      return { status: "downloaded", version: update.version, notes };
    }
    await relaunch();
    return { status: "downloaded", version: update.version, notes };
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    // 启动时的调用方不展示结果；关于页手动调用会把 error 明确显示给用户。
    console.warn("[updater] 检查更新失败：", e);
    void logFrontendError("updater", message).catch(() => {});
    return { status: "error", message };
  }
}
