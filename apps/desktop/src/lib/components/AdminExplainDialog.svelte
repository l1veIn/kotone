<script lang="ts">
  /*
   * 管理员身份的说明（ADR 044 D1）：还没用现在的说法问过（设置里的 adminPromptRevision 旧）时，
   * Settings.svelte 在 Kotone 窗口里弹一次——启动时先不以管理员身份重开，先用一句话说明原因再弹 UAC。
   * 「好」：记下问过、打开“启动时以管理员身份运行”、以管理员身份重开（UAC；已经是管理员就不重开）。
   * 「先不用」：记下问过、关掉那个开关（「设置 → 系统」里随时能改）。
   * 代替以前的两段提示（ElevationPrompt 的“不再提示”、AutoAdminPrompt）。
   */
  import { updateSettings, restartAsAdmin } from "../ipc";
  import { m } from "$lib/paraglide/messages.js";
  import { settingsStore, toast, toastInfo, errText } from "../stores/ui";
  import stickerPointing from "../../assets/brand/stickers/pointing.webp";

  let { elevated, revision, onClose }: {
    /** Kotone 现在已经是管理员身份（点「好」不用重开） */
    elevated: boolean;
    /** 现在的说法是第几版（ElevationStatus.adminPromptRevision），点了以后记进设置 */
    revision: number;
    onClose: () => void;
  } = $props();

  let busy = $state(false);

  async function onOk() {
    if (busy) return;
    busy = true;
    try {
      settingsStore.set(await updateSettings({ adminPromptRevision: revision, runAsAdminOnStart: true }));
      // 成功时这个进程马上退出，以管理员身份的那份接着；UAC 点了“否”会抛错，照常以普通权限运行
      if (!elevated) await restartAsAdmin();
    } catch (e) {
      toast(false, errText(e));
    }
    onClose();
  }

  async function onLater() {
    if (busy) return;
    busy = true;
    try {
      settingsStore.set(await updateSettings({ adminPromptRevision: revision, runAsAdminOnStart: false }));
      toastInfo(m.system_admin_explain_off_toast());
    } catch (e) {
      toast(false, m.system_save_settings_failed({ error: errText(e) }));
    }
    onClose();
  }
</script>

<div class="absolute inset-0 z-[60] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm" data-testid="admin-explain-dialog">
  <div class="w-full max-w-md rounded-3xl border border-hairline bg-popover p-6 text-popover-foreground shadow-2xl" role="dialog" aria-modal="true" aria-labelledby="admin-explain-title">
    <div class="flex items-start gap-4">
      <img src={stickerPointing} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 id="admin-explain-title" class="text-base font-bold">{m.system_admin_explain_title()}</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">{m.system_admin_explain_body()}</p>
        <p class="mt-1.5 text-[11px] leading-relaxed text-muted-foreground/80">{m.system_admin_explain_hint()}</p>
      </div>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={() => void onLater()}
      >
        {m.system_admin_explain_later()}
      </button>
      <button
        class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onOk()}
      >
        {busy ? m.system_admin_prompt_busy() : m.system_admin_explain_ok()}
      </button>
    </div>
  </div>
</div>
