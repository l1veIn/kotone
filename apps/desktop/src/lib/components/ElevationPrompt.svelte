<script lang="ts">
  /*
   * 管理员提权提示弹窗（仅 Windows 未提权时由 Settings.svelte 弹出）：
   * 部分游戏（如 League of Legends）以管理员运行时，普通权限进程无法向其注入文字。
   * 「暂不」与「以管理员权限重启」都会先持久化勾选状态（合并成一次 patch）。
   */
  import { updateSettings, restartAsAdmin } from "../ipc";
  import { settingsStore, toast, errText } from "../stores/ui";
  import stickerPointing from "../../assets/brand/stickers/pointing.png";

  let { onClose }: { onClose: () => void } = $props();

  /** 勾选：不再提示 */
  let neverAsk = $state(false);
  /** 勾选：以后默认以管理员权限启动 */
  let alwaysAdmin = $state(false);
  let busy = $state(false);

  /** 勾选状态合并成一次 patch 写盘（两个都勾也只写一次） */
  async function persistChoices() {
    const patch: Record<string, unknown> = {};
    if (neverAsk) patch.adminPromptDismissed = true;
    if (alwaysAdmin) patch.runAsAdminOnStart = true;
    if (Object.keys(patch).length === 0) return;
    settingsStore.set(await updateSettings(patch));
  }

  /** 主按钮：持久化勾选后弹 UAC 重启（成功后当前进程退出，无需处理返回值） */
  async function onRestart() {
    if (busy) return;
    busy = true;
    try {
      await persistChoices();
      await restartAsAdmin();
    } catch (e) {
      busy = false;
      toast(false, errText(e));
    }
  }

  /** 次按钮：持久化勾选后仅关闭弹窗 */
  async function onLater() {
    if (busy) return;
    busy = true;
    try {
      await persistChoices();
    } catch (e) {
      toast(false, `保存设置失败：${errText(e)}`);
    }
    onClose();
  }
</script>

<div class="absolute inset-0 z-[60] flex items-center justify-center bg-kotone-deep/80 p-6 backdrop-blur-sm">
  <div class="kotone-panel w-full max-w-md p-6 shadow-glow-cyan-lg">
    <div class="flex items-start gap-4">
      <img src={stickerPointing} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">需要管理员权限</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-white/65">
          当前未以管理员权限运行，部分游戏（如 League of Legends）内可能无法注入文字。
          是否以管理员权限重启？
        </p>
      </div>
    </div>

    <div class="mt-4 flex flex-col gap-2">
      <label class="flex cursor-pointer items-center gap-2 text-xs text-white/70">
        <input type="checkbox" bind:checked={neverAsk} class="h-3.5 w-3.5 accent-kotone-cyan" />
        不再提示
      </label>
      <label class="flex cursor-pointer items-center gap-2 text-xs text-white/70">
        <input type="checkbox" bind:checked={alwaysAdmin} class="h-3.5 w-3.5 accent-kotone-cyan" />
        以后默认以管理员权限启动
      </label>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-white/10 px-4 py-2 text-xs font-semibold text-white/75 ring-1 ring-white/15 transition hover:bg-white/20 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onLater()}
      >
        暂不
      </button>
      <button
        class="rounded-lg bg-kotone-cyan px-4 py-2 text-xs font-semibold text-kotone-deep transition hover:brightness-110 hover:shadow-glow-cyan active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onRestart()}
      >
        {busy ? "处理中…" : "以管理员权限重启"}
      </button>
    </div>
  </div>
</div>
