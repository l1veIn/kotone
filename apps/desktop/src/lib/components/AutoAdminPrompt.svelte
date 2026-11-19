<script lang="ts">
  /*
   * 管理员提权提示 · 第二段：用户通过第一段弹窗「以管理员权限重启」且
   * 提权成功后（Settings.svelte 凭 AUTO_ADMIN_PROMPT_FLAG 判定）弹出，
   * 单独询问「是否启动时自动请求管理员权限」。
   * 逃生通道：勾选「不再提醒」并跳过（autoAdminPromptDismissed），
   * 并提示可随时在「设置 → 系统」里更改同一开关。
   */
  import { updateSettings } from "../ipc";
  import { settingsStore, toast, toastInfo, errText } from "../stores/ui";
  import stickerPointing from "../../assets/brand/stickers/pointing.webp";

  let { onClose }: { onClose: () => void } = $props();

  /** 勾选：不再提醒 */
  let neverAsk = $state(false);
  let busy = $state(false);

  /** 主按钮：开启「启动时自动请求管理员权限」（与高级页开关同源），此后不再询问 */
  async function onEnable() {
    if (busy) return;
    busy = true;
    try {
      settingsStore.set(
        await updateSettings({
          runAsAdminOnStart: true,
          autoAdminPromptDismissed: true,
        }),
      );
      toast(true, "已开启：以后每次启动都会出现 Windows UAC 确认");
    } catch (e) {
      toast(false, `保存设置失败：${errText(e)}`);
    }
    onClose();
  }

  /** 次按钮：跳过；勾选「不再提醒」时写盘并告知高级页逃生通道 */
  async function onLater() {
    if (busy) return;
    busy = true;
    try {
      if (neverAsk) {
        settingsStore.set(
          await updateSettings({ autoAdminPromptDismissed: true }),
        );
        toastInfo("好的，以后可以随时在「设置 → 系统」里开启或关闭此选项");
      }
    } catch (e) {
      toast(false, `保存设置失败：${errText(e)}`);
    }
    onClose();
  }
</script>

<div class="absolute inset-0 z-[60] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm">
  <div class="rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl w-full max-w-md p-6">
    <div class="flex items-start gap-4">
      <img src={stickerPointing} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 class="text-base font-bold">启动时自动请求管理员权限？</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
          已以管理员权限运行。要让 Kotone 以后每次启动都自动发起 Windows
          UAC 请求吗？省去每次手动重启的麻烦。
        </p>
        <p class="mt-1.5 text-[11px] leading-relaxed text-muted-foreground/80">
          无论是否开启，以后都可以在「设置 → 系统」里随时更改。
        </p>
      </div>
    </div>

    <div class="mt-4 flex flex-col gap-2">
      <label class="flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
        <input
          type="checkbox"
          checked={neverAsk}
          class="h-3.5 w-3.5 accent-primary"
          onchange={(event) => {
            neverAsk = (event.target as HTMLInputElement).checked;
          }}
        />
        不再提醒
      </label>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={() => void onLater()}
      >
        暂不
      </button>
      <button
        class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onEnable()}
      >
        {busy ? "处理中…" : "每次启动自动请求"}
      </button>
    </div>
  </div>
</div>
