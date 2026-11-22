<script lang="ts">
  /*
   * 管理员提权提示 · 第一段（仅 Windows 未提权时由 Settings.svelte 弹出）：
   * 部分程序（如 League of Legends）以管理员权限运行时，普通权限进程无法
   * 向其注入文字。本弹窗只回答一个问题：「是否以管理员身份重启？」
   * 唯一的勾选项是「不再提示」；勾选并跳过后 toast 告知高级页逃生通道。
   * 「启动时自动请求管理员权限」是第二段，在提权重启成功后由
   * AutoAdminPrompt.svelte 单独询问（AUTO_ADMIN_PROMPT_FLAG 接力）。
   * `blocked`：刚才往游戏里打字 / 按键真的被拦下了（kotone://elevation-needed）——
   * 说清发生了什么，不给「不再提示」（这是一次失败，不是建议）。
   */
  import { updateSettings, restartAsAdmin } from "../ipc";
  import { m } from "$lib/paraglide/messages.js";
  import {
    settingsStore,
    toast,
    toastInfo,
    errText,
    AUTO_ADMIN_PROMPT_FLAG,
  } from "../stores/ui";
  import stickerPointing from "../../assets/brand/stickers/pointing.webp";

  let { onClose, blocked = false }: { onClose: () => void; blocked?: boolean } = $props();

  /** 勾选：不再提示 */
  let neverAsk = $state(false);
  let busy = $state(false);

  /** 「不再提示」勾选状态写盘（未勾选则不动设置） */
  async function persistDismissed() {
    if (!neverAsk) return;
    settingsStore.set(await updateSettings({ adminPromptDismissed: true }));
  }

  /** 主按钮：写盘 + 挂上第二段接力标记后弹 UAC 重启（成功后当前进程退出） */
  async function onRestart() {
    if (busy) return;
    busy = true;
    // 提权副本启动后凭此标记弹第二段「启动时自动请求管理员权限」；
    // UAC 被取消时 restartAsAdmin 抛错，catch 里撤掉标记。
    localStorage.setItem(AUTO_ADMIN_PROMPT_FLAG, "1");
    try {
      await persistDismissed();
      await restartAsAdmin();
    } catch (e) {
      localStorage.removeItem(AUTO_ADMIN_PROMPT_FLAG);
      busy = false;
      toast(false, errText(e));
    }
  }

  /** 次按钮：跳过；勾选「不再提示」时告知高级页逃生通道 */
  async function onLater() {
    if (busy) return;
    busy = true;
    try {
      await persistDismissed();
      if (neverAsk) {
        toastInfo(m.system_elevation_never_ask_toast());
      }
    } catch (e) {
      toast(false, m.system_save_settings_failed({ error: errText(e) }));
    }
    onClose();
  }
</script>

<div class="absolute inset-0 z-[60] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm">
  <div class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl w-full max-w-md p-6">
    <div class="flex items-start gap-4">
      <img src={stickerPointing} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">{blocked ? m.system_elevation_blocked_title() : m.system_elevation_title()}</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground" data-testid="elevation-prompt-text">
          {#if blocked}
            {m.system_elevation_blocked_body()}
          {:else}
            {m.system_elevation_body()}
          {/if}
        </p>
      </div>
    </div>

    <div class="mt-4 flex flex-col gap-2" class:hidden={blocked}>
      <label class="flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
        <input
          type="checkbox"
          checked={neverAsk}
          class="h-3.5 w-3.5 accent-primary"
          onchange={(event) => {
            neverAsk = (event.target as HTMLInputElement).checked;
          }}
        />
        {m.system_elevation_never_ask()}
      </label>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={() => void onLater()}
      >
        {m.system_admin_prompt_later()}
      </button>
      <button
        class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onRestart()}
      >
        {busy ? m.system_admin_prompt_busy() : m.system_elevation_restart_button()}
      </button>
    </div>
  </div>
</div>
