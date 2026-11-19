<script lang="ts" module>
  export type OnboardingNext = "chat" | "voiceInput" | "voiceGuide";
</script>

<script lang="ts">
  /*
   * 引导（ADR 028）。两种开头：
   * - 新用户（mode="new"）：欢迎（W1）→ 填 DeepSeek Key（W2）→ 已连接、设呼叫键（W3）→ 聊天页，
   *   输入框里放好“你能做什么？”。“只想用语音输入？”进语音输入子板块的引导（V 路径），
   *   “稍后配置”进聊天页的 Key 卡片（D14）。
   * - 0.1.x 升级用户（mode="upgrade"）：升级欢迎（W4），“继续使用语音输入法”最醒目，什么都不改；
   *   “添加 DeepSeek Key，认识琴音”走 W2 → W3，但不动语音识别（D6）。
   * 参照 DeepSeek Harness 的首启欢迎窗：两个入口、一个密码框、保存并继续 / 稍后配置 / 返回。
   */
  import { companionName } from "$lib/stores/companion";
  import { onDestroy, onMount, untrack } from "svelte";
  import {
    detectHotkeyConflicts,
    listModels,
    listProfiles,
    updateSettings,
    companionBindingPatch,
    companionBindings,
  } from "../../lib/ipc";
  import { hasLegacyReminders } from "../../lib/ipc/contracts";
  import { captureHotkey } from "../../lib/hotkeyCapture";
  import { findHotkeyConflict, hotkeyLabel } from "../../lib/hotkeyCombo";
  import { errText, settingsStore, toast } from "../../lib/stores/ui";
  import { runtimeStore } from "../../lib/stores/runtime";
  import type { DeepSeekSetupResult } from "../../lib/deepseekSetup";
  import DeepSeekKeyForm from "../../lib/components/DeepSeekKeyForm.svelte";
  import heroOnboarding from "../../assets/brand/hero-onboarding.webp";
  import stickerHello from "../../assets/brand/stickers/hello.webp";
  import stickerCheering from "../../assets/brand/stickers/cheering.webp";

  let { mode = "new", initialStep, initialResult = null, onDone }: {
    mode?: "new" | "upgrade";
    /** 从聊天页的 Key 卡片过来时直接停在“已连接”；语音输入的“让 Kotone 帮我配置”直接到“填 Key”。 */
    initialStep?: "key" | "connected";
    /** 聊天页的 Key 卡片已经连好时，带上换了哪几项。 */
    initialResult?: DeepSeekSetupResult | null;
    onDone: (next: OnboardingNext, prompt?: string) => void;
  } = $props();

  type Step = "welcome" | "upgrade" | "key" | "connected";
  let step = $state<Step>(untrack(() => initialStep ?? (mode === "upgrade" ? "upgrade" : "welcome")));
  let result = $state<DeepSeekSetupResult | null>(untrack(() => initialResult));
  let saving = $state(false);
  const name = $derived($companionName);
  const dots = $derived<Step[]>(mode === "upgrade" ? ["upgrade", "key", "connected"] : ["welcome", "key", "connected"]);
  /** 默认呼叫键（新用户本来就是 ~）。 */
  const RECOMMENDED_CALL_KEY = "Backquote";

  // ---------- 升级欢迎：读他现在的语音输入设置 ----------
  let summary = $state<{ hotkey: string; mode: string; asr: string; asrOnline: boolean; game: string; reminders: boolean } | null>(null);
  const modeNames: Record<string, string> = { "push-to-talk": "按住说话", dictation: "按一下开始、再按停止", "one-shot": "说一句就走", solo: "独奏模式" };
  onMount(async () => {
    if (mode !== "upgrade") return;
    const settings = $settingsStore;
    if (!settings) return;
    try {
      const [models, profiles] = await Promise.all([listModels(), listProfiles()]);
      const modelId = $runtimeStore?.modelId || settings.activeModelId;
      const model = models.find((item) => item.id === modelId);
      summary = {
        hotkey: hotkeyLabel(settings.hotkey.key),
        mode: modeNames[settings.interactionMode ?? ""] ?? (settings.hotkey.mode === "hold" ? "按住说话" : "按一下开始"),
        asr: model?.displayName.replace(/（.*$/, "") ?? "本地识别",
        asrOnline: model?.backend === "remote",
        game: profiles.find((item) => item.id === settings.activeProfileId)?.displayName ?? "通用",
        reminders: hasLegacyReminders(settings.ui) && !!settings.assistant.hotkey.trim(),
      };
    } catch {
      summary = null;
    }
  });

  async function keepVoiceInput() {
    if (saving) return;
    saving = true;
    try {
      // 只记“看过、选了继续”，其余一律不动（ADR 028 流程 B2）。
      settingsStore.set(await updateSettings({ ui: { legacyUpgrade: "dismissed", startPage: "voiceInput" } }));
      onDone("voiceInput");
    } catch (error) {
      toast(false, errText(error));
    } finally {
      saving = false;
    }
  }

  async function finishWithoutKey(next: "chat" | "voiceGuide") {
    if (saving) return;
    saving = true;
    try {
      // “稍后配置”记为引导完成（D14）；“只想用语音输入？”以后打开落在语音输入（D13）。
      settingsStore.set(await updateSettings({ ui: next === "voiceGuide"
        ? { firstRunCompleted: true, startPage: "voiceInput" }
        : { firstRunCompleted: true } }));
      onDone(next);
    } catch (error) {
      toast(false, errText(error));
    } finally {
      saving = false;
    }
  }

  // ---------- 已连接：设呼叫键 ----------
  const legacyOptedIn = $derived($settingsStore?.ui.legacyUpgrade === "optedIn");
  let callKey = $state("");
  let callKeyTouched = false;
  let capturing = $state(false);
  let captureCleanup: (() => void) | null = null;
  let staticWarnings = $state<string[]>([]);
  const conflict = $derived(callKey ? findHotkeyConflict($settingsStore, "assistant", callKey) : null);

  function onConnected(next: DeepSeekSetupResult) {
    result = next;
    const current = companionBindings(next.settings.assistant)?.hotkey?.trim() ?? "";
    // 新用户本来就有 ~；升级用户默认空，推荐 ~（玩家点一下才用）。
    callKey = current;
    callKeyTouched = false;
    step = "connected";
    if (callKey) void refreshWarnings(callKey);
  }

  // 从聊天页的 Key 卡片直接进“已连接”：呼叫键取当前值。
  onMount(() => {
    if (initialStep !== "connected") return;
    callKey = companionBindings($settingsStore?.assistant)?.hotkey?.trim() ?? "";
    if (callKey) void refreshWarnings(callKey);
  });

  async function refreshWarnings(key: string) {
    try { staticWarnings = await detectHotkeyConflicts(key); } catch { staticWarnings = []; }
  }

  function useKey(key: string) {
    callKey = key;
    callKeyTouched = true;
    staticWarnings = [];
    if (key) void refreshWarnings(key);
  }

  async function recordCallKey() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((capture) => {
      capturing = false;
      captureCleanup = null;
      if (capture.kind === "combo") useKey(capture.combo);
      else if (capture.kind === "error") toast(false, capture.message);
    });
  }

  async function enterChat() {
    if (saving) return;
    if (conflict) { toast(false, `呼叫键和${conflict}相同，请换一个`); return; }
    saving = true;
    try {
      if (callKeyTouched || (companionBindings($settingsStore?.assistant)?.hotkey ?? "") !== callKey) {
        settingsStore.set(await updateSettings({
          assistant: companionBindingPatch($settingsStore?.assistant, { hotkey: callKey }),
        }));
      }
      onDone("chat", "你能做什么？");
    } catch (error) {
      toast(false, `保存呼叫键失败：${errText(error)}`);
    } finally {
      saving = false;
    }
  }

  onDestroy(() => captureCleanup?.());
</script>

<div class="absolute inset-0 z-50 flex items-center justify-center bg-black/50 p-5 backdrop-blur-sm" data-testid="onboarding">
  <div class="relative max-h-full w-full max-w-xl overflow-y-auto rounded-3xl border border-hairline bg-popover text-popover-foreground shadow-2xl"
    role="dialog" aria-modal="true" aria-label="Kotone 引导">
    <div class="pointer-events-none absolute -top-24 left-1/2 h-48 w-96 -translate-x-1/2 rounded-full bg-kotone-violet/25 blur-3xl"></div>

    {#if step === "welcome"}
      <div class="relative overflow-hidden" data-testid="onboarding-welcome">
        <div class="relative">
          <img src={heroOnboarding} alt="Kotone 张开双臂欢迎你" class="h-48 w-full object-cover object-top" />
          <div class="absolute inset-0 bg-gradient-to-t from-popover via-popover/30 to-transparent"></div>
        </div>
        <div class="relative -mt-8 px-8 pb-6">
          <img src={stickerHello} alt="" class="mb-3 h-12 w-12" />
          <h1 class="m-0 text-2xl font-bold">欢迎来到 <span class="kotone-gradient-text">Kotone 琴音</span></h1>
          <p class="mt-2 mb-0 text-sm leading-relaxed text-muted-foreground">陪你打游戏的 AI 陪玩：叫她一声，她帮你查、帮你说、帮你按。</p>
          <button class="mt-6 w-full rounded-xl bg-primary px-5 py-2.5 text-sm font-semibold text-primary-foreground transition hover:brightness-110 active:scale-[0.99]"
            data-testid="onboarding-add-key" onclick={() => (step = "key")}>添加 DeepSeek Key</button>
          <p class="mt-2 mb-0 text-center text-[11px] text-muted-foreground">有这一个 Key 就能用全部功能：对话、语音识别、文字识别、语音合成、联网搜索。</p>
          <div class="mt-5 flex items-center justify-between">
            <button class="text-xs text-primary underline-offset-2 hover:underline disabled:opacity-50" data-testid="onboarding-voice-only"
              disabled={saving} onclick={() => void finishWithoutKey("voiceGuide")}>只想用语音输入？</button>
            <button class="text-xs text-muted-foreground hover:text-foreground/90 disabled:opacity-50" data-testid="onboarding-later"
              disabled={saving} onclick={() => void finishWithoutKey("chat")}>稍后配置</button>
          </div>
        </div>
      </div>
    {:else if step === "upgrade"}
      <div class="relative p-7" data-testid="onboarding-upgrade">
        <div class="flex items-start gap-4">
          <img src={stickerHello} alt="" class="h-11 w-11 shrink-0" />
          <div class="min-w-0">
            <h2 class="m-0 text-lg font-bold">Kotone 0.2：多了一位游戏陪玩</h2>
            <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">升级不会改你的任何设置。</p>
          </div>
        </div>
        <div class="mt-5 rounded-2xl border border-hairline bg-card/70 px-4 py-3" data-testid="onboarding-upgrade-summary">
          <p class="m-0 text-sm font-semibold">你的语音输入没有变</p>
          {#if summary}
            <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">
              热键 {summary.hotkey}（{summary.mode}）· {summary.asrOnline ? "在线识别" : "本地识别"} {summary.asr} · {summary.game}{summary.reminders ? " · 提醒助手照旧" : ""}
            </p>
            {#if summary.asrOnline}
              <p class="mt-1 mb-0 text-xs text-primary">在线识别不用再点「启动」，打开 Kotone 就能用。</p>
            {/if}
          {:else}
            <p class="mt-1 mb-0 text-xs text-muted-foreground">热键、识别模型、游戏配置都和以前一样。</p>
          {/if}
        </div>
        <button class="mt-5 w-full rounded-xl bg-primary px-5 py-2.5 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
          data-testid="onboarding-keep-voice" disabled={saving} onclick={() => void keepVoiceInput()}>继续使用语音输入法</button>
        <button class="mt-2 w-full rounded-xl border border-hairline px-5 py-2.5 text-sm text-foreground/90 transition hover:bg-muted disabled:opacity-50"
          data-testid="onboarding-meet-companion" disabled={saving} onclick={() => (step = "key")}>添加 DeepSeek Key，认识{name}</button>
        <p class="mt-4 mb-0 text-[11px] leading-relaxed text-muted-foreground">
          {name}是可选的：她能听你说话、回答问题，在你允许时帮你操作游戏。以后随时可以从侧栏开始。
        </p>
      </div>
    {:else if step === "key"}
      <div class="relative p-7" data-testid="onboarding-key-step">
        <DeepSeekKeyForm testid="onboarding-key" onConnected={onConnected}
          onBack={() => (step = mode === "upgrade" ? "upgrade" : "welcome")}
          onLater={mode === "upgrade" ? () => void keepVoiceInput() : () => void finishWithoutKey("chat")}
          laterLabel={mode === "upgrade" ? "继续使用语音输入法" : "稍后配置"} />
      </div>
    {:else if step === "connected"}
      <div class="relative p-7" data-testid="onboarding-connected">
        <div class="flex items-start gap-4">
          <img src={stickerCheering} alt="" class="h-11 w-11 shrink-0" />
          <div>
            <h2 class="m-0 text-lg font-bold">已连接 DeepSeek</h2>
            <p class="mt-1 mb-0 text-xs text-muted-foreground">下面这些现在都用 DeepSeek，以后可以在「设置 → 模型」里换。</p>
          </div>
        </div>
        {#if result}
          <ul class="mt-4 mb-0 flex list-none flex-wrap gap-x-4 gap-y-1 rounded-2xl border border-hairline bg-card/70 px-4 py-3 text-xs" data-testid="onboarding-switched">
            {#each result.items as item (item.id)}
              <li class={item.ok ? "text-success" : "text-warning"} title={item.note}>
                {item.ok ? "✓" : "!"} {item.label}{#if item.note}<span class="text-muted-foreground"> · {item.note}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="mt-3 rounded-2xl border border-hairline bg-card/70 px-4 py-3" data-testid="onboarding-call-key">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-sm font-semibold">呼叫键</span>
            <div class="flex items-center gap-2">
              {#if callKey}
                <kbd class="rounded-md border border-hairline bg-background px-2 py-0.5 font-mono text-xs" data-testid="onboarding-call-key-value">{hotkeyLabel(callKey)}</kbd>
              {:else}
                <span class="text-xs text-muted-foreground" data-testid="onboarding-call-key-value">未设置</span>
                <button class="rounded-lg border border-primary/50 px-2.5 py-1 text-xs text-primary hover:bg-primary/10" data-testid="onboarding-call-key-recommended"
                  onclick={() => useKey(RECOMMENDED_CALL_KEY)}>用 ~</button>
              {/if}
              <button class="rounded-lg border border-hairline px-2.5 py-1 text-xs hover:bg-muted disabled:opacity-50" disabled={capturing}
                data-testid="onboarding-call-key-record" onclick={() => void recordCallKey()}>{capturing ? "请按键…（Esc 取消）" : "换一个"}</button>
            </div>
          </div>
          <p class="mt-1.5 mb-0 text-xs text-muted-foreground">在游戏里按一下，{name}就在右下角的气泡里听你说，说完自动交给她。</p>
          {#if conflict}
            <p class="mt-1.5 mb-0 text-xs text-destructive" role="alert" data-testid="onboarding-call-key-conflict">和你的{conflict}相同，请换一个</p>
          {/if}
          {#each staticWarnings as warning}
            <p class="mt-1 mb-0 text-xs text-warning">⚠ {warning}</p>
          {/each}
          {#if legacyOptedIn && !callKey}
            <p class="mt-1.5 mb-0 text-[11px] text-muted-foreground">你原来的语音输入热键和提醒助手热键不受影响。</p>
          {/if}
        </div>
        <div class="mt-6 flex items-center justify-between">
          {#if callKey}
            <button class="text-xs text-muted-foreground hover:text-foreground/90" data-testid="onboarding-call-key-clear" onclick={() => useKey("")}>不要呼叫键</button>
          {:else}<span></span>{/if}
          <button class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50"
            data-testid="onboarding-enter-chat" disabled={saving || capturing || !!conflict} onclick={() => void enterChat()}>进入聊天</button>
        </div>
      </div>
    {/if}

    <div class="relative flex justify-center gap-1.5 pb-4">
      {#each dots as dot (dot)}
        <span class="h-1.5 rounded-full transition-all {dot === step ? 'w-5 bg-primary' : 'w-1.5 bg-muted'}"></span>
      {/each}
    </div>
  </div>
</div>
