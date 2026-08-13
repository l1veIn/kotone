<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getHotkeyStatus, updateSettings, type HotkeyStatus } from "../../../../lib/ipc";
  import { captureHotkey } from "../../../../lib/hotkeyCapture";
  import { combosConflict } from "../../../../lib/hotkeyCombo";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";

  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  let cycleDraft = $state("Shift+CapsLock");
  let cycleCapturing = $state(false);
  let cycleCleanup: (() => void) | null = null;
  let resendDraft = $state("");
  let resendCapturing = $state(false);
  let resendCleanup: (() => void) | null = null;

  onMount(async () => {
    cycleDraft = $settingsStore?.channelCycleHotkey ?? "Shift+CapsLock";
    resendDraft = $settingsStore?.resendLastHotkey ?? "";
    hotkeyStatus = await getHotkeyStatus().catch(() => null);
  });

  async function saveCycleHotkey() {
    const key = cycleDraft.trim();
    if (!key) {
      toast(false, "频道切换热键不能为空");
      return;
    }
    const recordKey = $settingsStore?.hotkey.key ?? "";
    if (recordKey && combosConflict(key, recordKey)) {
      toast(false, `频道切换热键与录制热键（${recordKey}）冲突，请换一个`);
      return;
    }
    try {
      settingsStore.set(await updateSettings({ channelCycleHotkey: key }));
      cycleDraft = $settingsStore?.channelCycleHotkey ?? key;
      toast(true, `频道切换热键已保存并生效：${cycleDraft}`);
    } catch (e) {
      toast(false, `保存频道切换热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  async function startCycleCapture() {
    if (cycleCapturing) return;
    cycleCapturing = true;
    cycleCleanup = await captureHotkey((r) => {
      cycleCapturing = false;
      cycleCleanup = null;
      if (r.kind === "combo") {
        cycleDraft = r.combo;
        void saveCycleHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  async function saveResendHotkey() {
    const key = resendDraft.trim();
    if (!key) {
      toast(false, "重发热键不能为空（留空 = 关闭功能，可点清除）");
      return;
    }
    const recordKey = $settingsStore?.hotkey.key ?? "";
    const cycleKey = $settingsStore?.channelCycleHotkey ?? "";
    if (recordKey && combosConflict(key, recordKey)) {
      toast(false, `重发热键与录制热键（${recordKey}）冲突，请换一个`);
      return;
    }
    if (cycleKey && combosConflict(key, cycleKey)) {
      toast(false, `重发热键与频道切换热键（${cycleKey}）冲突，请换一个`);
      return;
    }
    try {
      settingsStore.set(await updateSettings({ resendLastHotkey: key }));
      resendDraft = $settingsStore?.resendLastHotkey ?? key;
      toast(true, `重发热键已保存并生效：${resendDraft}`);
    } catch (e) {
      toast(false, `保存重发热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  async function startResendCapture() {
    if (resendCapturing) return;
    resendCapturing = true;
    resendCleanup = await captureHotkey((r) => {
      resendCapturing = false;
      resendCleanup = null;
      if (r.kind === "combo") {
        resendDraft = r.combo;
        void saveResendHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  async function clearResendHotkey() {
    resendDraft = "";
    try {
      settingsStore.set(await updateSettings({ resendLastHotkey: "" }));
      toast(true, "已关闭重发热键");
    } catch (e) {
      toast(false, `清除重发热键失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  onDestroy(() => {
    cycleCleanup?.();
    resendCleanup?.();
  });
</script>

<section class="kotone-panel p-4">
  <h2 class="text-sm font-semibold text-kotone-cyan/90">频道切换热键</h2>
  <p class="mt-1 text-[11px] leading-relaxed text-white/45">
    支持多频道的游戏适配（如英雄联盟的「队伍 / 所有人」）按声明顺序循环切换，悬浮窗会显示当前频道。
  </p>
  <div class="mt-3 flex items-center gap-2">
    <input
      bind:value={cycleDraft}
      disabled={cycleCapturing}
      class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
      placeholder="如 Shift+CapsLock"
      spellcheck="false"
      onkeydown={(e) => {
        if (e.key === "Enter" && !cycleCapturing) void saveCycleHotkey();
      }}
    />
    <button
      class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {cycleCapturing
        ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
        : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
      disabled={cycleCapturing}
      onclick={() => void startCycleCapture()}
    >
      {cycleCapturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
    </button>
  </div>
  {#if hotkeyStatus?.cycleError}
    <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{hotkeyStatus.cycleError}</p>
  {:else if hotkeyStatus?.cycleKey}
    <p class="mt-2 text-[11px] text-white/40">当前生效：{hotkeyStatus.cycleKey}</p>
  {/if}
  <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">
    不能与「通用」页的录制热键相同；当前游戏适配只有一个频道时该键不生效。
  </p>
</section>

<section class="kotone-panel mt-3 p-4">
  <h2 class="text-sm font-semibold text-kotone-cyan/90">重发最近一条热键</h2>
  <p class="mt-1 text-[11px] leading-relaxed text-white/45">
    空闲时按下，把历史记录里最新一条发送成功的文本重新发送到当前前台窗口
    （用当前游戏适配与频道；正在说话/发送时按下不生效）。
  </p>
  <div class="mt-3 flex items-center gap-2">
    <input
      bind:value={resendDraft}
      disabled={resendCapturing}
      class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
      placeholder="如 Alt+F6"
      spellcheck="false"
      onkeydown={(e) => {
        if (e.key === "Enter" && !resendCapturing) void saveResendHotkey();
      }}
    />
    <button
      class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {resendCapturing
        ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
        : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
      disabled={resendCapturing}
      onclick={() => void startResendCapture()}
    >
      {resendCapturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
    </button>
    {#if resendDraft}
      <button
        class="rounded-lg px-2.5 py-1.5 text-xs text-white/60 ring-1 ring-white/15 transition hover:bg-white/10 active:scale-95"
        onclick={() => void clearResendHotkey()}
      >
        清除
      </button>
    {/if}
  </div>
  {#if hotkeyStatus?.resendError}
    <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{hotkeyStatus.resendError}</p>
  {:else if hotkeyStatus?.resendKey}
    <p class="mt-2 text-[11px] text-white/40">当前生效：{hotkeyStatus.resendKey}</p>
  {:else if !resendDraft}
    <p class="mt-2 text-[11px] text-white/40">未设置（默认关闭，不会误触发）</p>
  {/if}
  <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">
    不能与「通用」页的录制热键或频道切换热键相同；历史中没有发送成功的文本时按下无效果。
  </p>
</section>
