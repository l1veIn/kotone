/*
 * 更新服务：设置窗口打开后静默触发一次，关于页也可随时手动触发。
 * 两个入口复用同一流程：Rust 检查并按通道筛选 → 用户确认 → 备份 → 下载、验签、安装。
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import type { DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { writable } from "svelte/store";
import { isTauri, logFrontendError } from "./ipc";
import { toast, toastInfo, toastWarn } from "./stores/ui";

export type UpdateCheckResult =
  | { status: "up-to-date" }
  | { status: "available"; version: string; notes?: string }
  | { status: "downloaded"; version: string; notes?: string }
  | { status: "error"; message: string };

interface UpdateDialogBase {
  version: string;
  notes?: string;
  changelogUrl?: string;
}

export type UpdateDialogState =
  | (UpdateDialogBase & { phase: "available" | "backing-up" })
  | (UpdateDialogBase & {
      phase: "downloading";
      downloadedBytes: number;
      totalBytes?: number;
    })
  | (UpdateDialogBase & { phase: "ready" })
  | (UpdateDialogBase & { phase: "restarting" });

type UpdateDialogChoice = "primary" | "later";

/** 设置窗口根组件渲染的全局更新弹窗；检查入口只负责推动状态。 */
export const updateDialogStore = writable<UpdateDialogState | null>(null);

let pendingDialogChoice: ((choice: UpdateDialogChoice) => void) | null = null;

function requestDialogChoice(state: UpdateDialogState): Promise<UpdateDialogChoice> {
  if (pendingDialogChoice) {
    return Promise.reject(new Error("已有更新确认弹窗正在等待用户操作"));
  }
  return new Promise((resolve) => {
    pendingDialogChoice = resolve;
    updateDialogStore.set(state);
  });
}

/** UpdateDialog 的按钮回传；先收起旧阶段，再恢复更新流程。 */
export function answerUpdateDialog(choice: UpdateDialogChoice): void {
  const resolve = pendingDialogChoice;
  pendingDialogChoice = null;
  updateDialogStore.set(null);
  resolve?.(choice);
}

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

interface UpdateOffer {
  version: string;
  notes?: string | null;
}

export function describeUpdate(offer: UpdateOffer): UpdateDialogBase {
  const { version } = offer;
  if (!version.startsWith("0.2.")) {
    return { version, notes: readableNotes(offer.notes) };
  }
  return {
    version,
    notes: [
      "语音输入法的用法没有变，可以继续按原来的方式使用。",
      "琴音默认不打扰。不配置 API Key，就不会产生模型服务费用。",
      "安装前会保存备份，可以手动重装 0.1.x 并恢复备份。",
    ].join("\n"),
    changelogUrl: `https://github.com/l1veIn/kotone/blob/v${encodeURIComponent(version)}/CHANGELOG.md`,
  };
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
  let updateStarted = false;
  try {
    const update = await invoke<UpdateOffer | null>("check_kotone_update");
    if (!update) return { status: "up-to-date" };
    const summary = describeUpdate(update);
    const { notes } = summary;
    const choice = await requestDialogChoice({ ...summary, phase: "available" });
    if (choice === "later") return { status: "available", version: update.version, notes };

    updateStarted = true;
    let downloadedBytes = 0;
    let totalBytes: number | undefined;
    const showDownloadProgress = () => {
      updateDialogStore.set({
        phase: "downloading",
        version: update.version,
        notes,
        downloadedBytes,
        totalBytes,
      });
    };
    updateDialogStore.set({ ...summary, phase: "backing-up" });
    const onDownload = new Channel<DownloadEvent>();
    onDownload.onmessage = (event) => {
      if (event.event === "Started") {
        downloadedBytes = 0;
        totalBytes = event.data.contentLength ?? undefined;
        toastInfo(`正在下载 v${update.version} 更新包…`);
      } else if (event.event === "Progress") {
        downloadedBytes += event.data.chunkLength;
      } else if (event.event === "Finished" && totalBytes !== undefined) {
        downloadedBytes = totalBytes;
      }
      showDownloadProgress();
    };
    await invoke("install_kotone_update", { version: update.version, onDownload });

    const restartChoice = await requestDialogChoice({ ...summary, phase: "ready" });
    if (restartChoice === "later") {
      toastWarn("更新已就绪，将在下次启动时生效");
      return { status: "downloaded", version: update.version, notes };
    }
    updateDialogStore.set({ phase: "restarting", version: update.version, notes });
    await relaunch();
    return { status: "downloaded", version: update.version, notes };
  } catch (e) {
    pendingDialogChoice = null;
    updateDialogStore.set(null);
    const message = e instanceof Error ? e.message : String(e);
    if (updateStarted) toast(false, `更新失败：${message}`);
    // 启动时的调用方不展示结果；关于页手动调用会把 error 明确显示给用户。
    console.warn("[updater] 检查更新失败：", e);
    void logFrontendError("updater", message).catch(() => {});
    return { status: "error", message };
  }
}
