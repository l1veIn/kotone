<script lang="ts">
  /*
   * “用不了什么”（ADR 044 D1）：Kotone 不是管理员身份（UAC 点了“否”、或者关着“启动时以管理员身份运行”），
   * 而管理员身份的游戏在前面（后端每 2 秒看一次）、或者往游戏里打字真的被拦了时，经 kotone://elevation-needed
   * 在 Kotone 窗口里说清楚哪些用不了，给“以管理员身份重开”。每次启动最多说一次（后端和 Settings.svelte 都只放一次）。
   */
  import { restartAsAdmin, type ElevationNoticeReason } from "../ipc";
  import { m } from "$lib/paraglide/messages.js";
  import { toast, errText } from "../stores/ui";
  import stickerPointing from "../../assets/brand/stickers/pointing.webp";

  let { reason, onClose }: { reason: ElevationNoticeReason; onClose: () => void } = $props();

  let busy = $state(false);

  async function onRestart() {
    if (busy) return;
    busy = true;
    try {
      // 成功时这个进程马上退出；UAC 点了“否”会抛错
      await restartAsAdmin();
    } catch (e) {
      toast(false, errText(e));
    }
    busy = false;
  }
</script>

<div class="absolute inset-0 z-[60] flex items-center justify-center bg-black/50 p-6 backdrop-blur-sm" data-testid="elevation-notice">
  <div class="w-full max-w-md rounded-3xl border border-hairline bg-popover p-6 text-popover-foreground shadow-2xl" role="dialog" aria-modal="true" aria-labelledby="elevation-notice-title">
    <div class="flex items-start gap-4">
      <img src={stickerPointing} alt="" class="h-14 w-14 shrink-0 object-contain" />
      <div class="min-w-0">
        <h2 id="elevation-notice-title" class="text-base font-bold">{m.system_elevation_notice_title()}</h2>
        <p class="mt-1.5 text-[13px] leading-relaxed text-muted-foreground" data-testid="elevation-notice-text">
          {reason === "blocked" ? m.system_elevation_notice_blocked() : m.system_elevation_notice_foreground()}
          {m.system_elevation_notice_body()}
        </p>
      </div>
    </div>

    <div class="mt-5 flex items-center justify-end gap-3">
      <button
        class="rounded-lg bg-muted/70 px-4 py-2 text-xs font-semibold text-foreground/90 ring-1 ring-hairline transition hover:bg-muted disabled:opacity-50"
        disabled={busy}
        onclick={onClose}
      >
        {m.system_elevation_notice_dismiss()}
      </button>
      <button
        class="rounded-lg bg-primary px-4 py-2 text-xs font-semibold text-primary-foreground transition hover:brightness-110 active:scale-95 disabled:opacity-50"
        disabled={busy}
        onclick={() => void onRestart()}
      >
        {busy ? m.system_admin_prompt_busy() : m.system_elevation_notice_restart()}
      </button>
    </div>
  </div>
</div>
