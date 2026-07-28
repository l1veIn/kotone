/*
 * 自动检测更新（设置窗口打开后延迟触发一次，见 Settings.svelte）。
 * 仅桌面端生效；离线 / 无 release / 检查失败全程静默，只在 console 留痕，
 * 不打扰用户。确认更新流程：ask → downloadAndInstall → ask → relaunch。
 */
import { check } from "@tauri-apps/plugin-updater";
import { ask } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri } from "./ipc";
import { toastInfo, toastWarn } from "./stores/ui";

/** 检查一次更新；有更新时询问用户是否立即下载安装并重启 */
export async function checkForUpdates(): Promise<void> {
  if (!isTauri) return;
  try {
    const update = await check();
    if (!update) return; // 已是最新
    const yes = await ask(`发现新版本 v${update.version}，是否立即更新？`, {
      title: "Kotone 更新",
      kind: "info",
      okLabel: "立即更新",
      cancelLabel: "稍后",
    });
    if (!yes) return;
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
      return;
    }
    await relaunch();
  } catch (e) {
    // 离线 / 无 release / 权限问题等：静默跳过
    console.warn("[updater] 检查更新失败（已忽略）：", e);
  }
}
