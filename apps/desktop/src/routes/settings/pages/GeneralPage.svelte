<script lang="ts">
  /*
   * 语音输入 · 输入（ADR 028 W8 / W9，页面 ID 仍是 general）：语音识别卡片（在线 / 本地与启动）、
   * 语音输入热键、交互模式、悬浮窗、其他热键。说话、原话打进游戏聊天框，不经过角色（Agent）。
   * 麦克风搬到「设置 → 声音」，游戏在「游戏列表」——这里只显示一行共享设置。
   */
  import { onDestroy, onMount } from "svelte";
  import {
    updateSettings,
    listAudioDevices,
    listProfiles,
    getElevationStatus,
    restartAsAdmin,
    getHotkeyStatus,
    type AudioDevice,
    type ElevationStatus,
    type GameProfile,
    type HotkeyStatus,
    type InteractionMode,
  } from "../../../lib/ipc";
  import { captureHotkey } from "../../../lib/hotkeyCapture";
  import ToggleField from "../../../lib/components/settings/ToggleField.svelte";
  import SettingsPage from "../../../lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "../../../lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "../../../lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
  import { findHotkeyConflict } from "../../../lib/hotkeyCombo";
  import {
    settingsStore,
    fullscreenWarningStore,
    patchSettings,
    toast,
    errText,
  } from "../../../lib/stores/ui";
  import { asrLocalStore } from "../../../lib/stores/runtime";
  import AsrCard from "./voice/AsrCard.svelte";
  import VoiceHotkeysSection from "./voice/VoiceHotkeysSection.svelte";
  import VoiceSetupGuide from "./voice/VoiceSetupGuide.svelte";

  let { guideOpen = false, onOpenGuide, onCloseGuide, onConfigure, onOpenModels, onOpenSound, onOpenGames }: {
    /** 语音输入的四步引导（V 路径）展开着。 */
    guideOpen?: boolean;
    onOpenGuide: () => void;
    onCloseGuide: () => void;
    /** 让 Kotone 帮我配置（填 DeepSeek Key / 让她帮你设）。 */
    onConfigure: () => void;
    onOpenModels: () => void;
    onOpenSound: () => void;
    onOpenGames: () => void;
  } = $props();

  let devices = $state<AudioDevice[]>([]);
  let profiles = $state<GameProfile[]>([]);
  const micName = $derived(devices.find((d) => d.id === $settingsStore?.audioDeviceId)?.name ?? "系统默认");
  const game = $derived(profiles.find((p) => p.id === $settingsStore?.activeProfileId) ?? null);
  let elevation = $state<ElevationStatus | null>(null);
  let restarting = $state(false);

  function applyGameCompatibilityStatus(status: ElevationStatus) {
    elevation = status;
    if (status.activeGameFullscreen === true) fullscreenWarningStore.set(true);
  }

  /** 提权重启接力标记：旧进程退出前落 localStorage，新进程检测到已提权后 toast 并清除 */
  const ADMIN_RESTART_FLAG = "kotone:admin-restart-pending";

  onMount(async () => {
    try {
      const [nextDevices, nextElevation, nextProfiles] = await Promise.all([
        listAudioDevices(),
        getElevationStatus(),
        listProfiles(),
      ]);
      devices = nextDevices;
      profiles = nextProfiles;
      applyGameCompatibilityStatus(nextElevation);
      if (localStorage.getItem(ADMIN_RESTART_FLAG)) {
        localStorage.removeItem(ADMIN_RESTART_FLAG);
        if (elevation?.elevated) toast(true, "已通过管理员权限运行 ✨");
      }
    } catch (e) {
      toast(false, `加载设备信息失败：${errText(e)}`);
    }
  });

  /** 权限状态轮询：每 3s；页面隐藏时暂停，回到前台立即补一次 */
  onMount(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const refresh = async () => {
      try {
        applyGameCompatibilityStatus(await getElevationStatus());
      } catch {
        /* 轮询失败不打扰，下轮再试 */
      }
    };
    const start = () => {
      if (timer === null) timer = setInterval(() => void refresh(), 3000);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        void refresh();
        start();
      }
    };
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  /** UIPI：目标游戏提权且自身未提权 → 显示提权横幅 */
  const needsElevation = $derived(
    elevation !== null && !elevation.elevated && elevation.activeGameElevated === true,
  );

  async function onRestartAsAdmin() {
    restarting = true;
    try {
      // 成功后当前进程直接退出；走到 catch 说明用户取消 UAC 或失败
      localStorage.setItem(ADMIN_RESTART_FLAG, "1");
      await restartAsAdmin();
    } catch (e) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      restarting = false;
      toast(false, errText(e));
    }
  }

  // ---------- 热键 & 交互模式（自快捷键页并入） ----------

  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  /** 热键编辑草稿（保存前不动后端） */
  let hotkeyDraft = $state("");
  /** 热键录入捕获中（「点击录入」按钮态） */
  let capturing = $state(false);
  /** 进行中捕获的 cleanup（组件销毁兜底取消） */
  let captureCleanup: (() => void) | null = null;

  onMount(async () => {
    try {
      hotkeyStatus = await getHotkeyStatus();
      hotkeyDraft = $settingsStore?.hotkey.key ?? "CapsLock";
    } catch (e) {
      toast(false, `读取热键状态失败：${errText(e)}`);
    }
  });

  const modes: { id: InteractionMode; name: string; desc: string }[] = [
    { id: "push-to-talk", name: "对讲机", desc: "按住说话，松开发送" },
    { id: "dictation", name: "录音笔", desc: "点按开始，再点停止" },
    { id: "one-shot", name: "说一句就走", desc: "说完自动停、自动发" },
    { id: "solo", name: "独奏模式", desc: "持续收音，说一句发一句，再点停止" },
  ];

  const currentMode = $derived($settingsStore?.interactionMode ?? null);
  const vadMs = $derived($settingsStore?.vadSilenceMs ?? 700);

  async function onModeSelect(id: InteractionMode) {
    try {
      // 预设与 hotkey.mode 同步落盘：壳侧注册用 effective_hotkey_mode 推导，
      // 这里保持旧字段一致，避免设置页其它处读到脱节的 mode。
      // 对讲机 = 按住（hold），录音笔 / 说一句就走 = 点按（toggle）。
      settingsStore.set(
        await updateSettings({
          interactionMode: id,
          hotkey: {
            key: $settingsStore?.hotkey.key ?? "CapsLock",
            mode: id === "push-to-talk" ? "hold" : "toggle",
          },
        }),
      );
      const m = modes.find((x) => x.id === id);
      toast(true, `交互模式已切换：${m?.name ?? id}`);
    } catch (e) {
      toast(false, `切换模式失败：${errText(e)}`);
    }
  }

  async function onVadChange(e: Event) {
    const vadSilenceMs = Number((e.target as HTMLInputElement).value);
    try {
      settingsStore.set(await updateSettings({ vadSilenceMs }));
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function saveHotkey() {
    const key = hotkeyDraft.trim();
    if (!key) {
      toast(false, "热键不能为空");
      return;
    }
    const conflict = findHotkeyConflict($settingsStore, "record", key);
    if (conflict) {
      toast(false, `语音输入热键和${conflict}相同，请换一个组合`);
      return;
    }
    try {
      // 热键变更后端自动重注册（lib.rs update_settings）；模式沿用当前值
      settingsStore.set(
        await updateSettings({ hotkey: { key, mode: $settingsStore?.hotkey.mode ?? "toggle" } }),
      );
      hotkeyDraft = $settingsStore?.hotkey.key ?? key;
      hotkeyStatus = await getHotkeyStatus().catch(() => null);
      if (hotkeyStatus?.error) toast(false, `热键已保存，但未生效：${hotkeyStatus.error}`);
      else toast(true, hotkeyStatus?.registered ? `热键已保存并生效：${key}` : `热键已保存，${$asrLocalStore ? "启动本地语音识别后生效" : "语音识别准备好后生效"}：${key}`);
    } catch (e) {
      toast(false, `保存热键失败：${errText(e)}`);
    } finally {
      try {
        hotkeyStatus = await getHotkeyStatus();
      } catch {
        /* 状态刷新失败不覆盖保存反馈 */
      }
    }
  }

  /**
   * 「点击录入」：LL 钩子捕获下一个按键组合（共享 helper 见 lib/hotkeyCapture.ts）。
   */
  async function startCapture() {
    if (capturing) return;
    capturing = true;
    captureCleanup = await captureHotkey((r) => {
      capturing = false;
      captureCleanup = null;
      if (r.kind === "combo") {
        hotkeyDraft = r.combo;
        void saveHotkey();
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  // 组件销毁时兜底取消进行中的捕获（避免钩子停在捕获模式）
  onDestroy(() => {
    captureCleanup?.();
  });

</script>

<SettingsPage embedded title="输入" testid="general-page">
  {#if guideOpen}
    <VoiceSetupGuide onClose={onCloseGuide} {onConfigure} />
  {/if}

  {#if $settingsStore}
    {#if needsElevation}
      <section class="flex flex-wrap items-center gap-3 rounded-2xl border border-brand-pink/45 bg-brand-pink/8 px-4 py-3.5">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] font-semibold text-brand-pink">当前游戏需要更高权限才能接收文字</p>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">重新启动 Kotone 后即可继续，其他设置不会丢失。</p>
        </div>
        <Button size="sm" class="bg-brand-pink text-[#fff] hover:bg-brand-pink/90" disabled={restarting} onclick={() => void onRestartAsAdmin()}>
          {restarting ? "正在重启…" : "重新启动"}
        </Button>
      </section>
    {/if}

    {#if $fullscreenWarningStore}
      <section class="flex flex-wrap items-center gap-3 rounded-2xl border border-warning/45 bg-warning/8 px-4 py-3.5">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] font-semibold text-warning">检测到游戏正在使用全屏模式</p>
          <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">
            全屏模式下唤起悬浮窗可能导致游戏最小化。请在游戏的视频设置中切换为无边框或窗口模式。
          </p>
        </div>
        <Button size="sm" variant="outline" onclick={() => fullscreenWarningStore.set(false)}>知道了</Button>
      </section>
    {/if}

    <AsrCard {onOpenModels} {onConfigure} />

    <div id="voice-hotkey" class="scroll-mt-16">
    <SettingsSection title="语音输入热键">
      <SettingsRow label="语音输入热键" description="按下它说话，原话打进游戏聊天框。支持键盘组合以及鼠标侧键 Mouse4 / Mouse5。" stacked for="hotkey-input">
        {#snippet control()}
          <div class="flex flex-wrap items-center gap-2">
            <input
              id="hotkey-input"
              bind:value={hotkeyDraft}
              disabled={capturing}
              class="h-8 w-36 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30 disabled:opacity-50"
              placeholder="如 CapsLock / Alt+V"
              spellcheck="false"
            />
            <Button size="sm" variant="outline" class={capturing ? "animate-pulse border-primary/60 text-primary" : ""} disabled={capturing} onclick={() => void startCapture()}>
              {capturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
            </Button>
            <Button size="sm" disabled={capturing} onclick={() => void saveHotkey()}>保存并生效</Button>
            <span class="text-xs text-muted-foreground">当前：{$settingsStore?.hotkey.key ?? "…"}</span>
          </div>
          {#if hotkeyStatus?.error}
            <div class="mt-3 rounded-xl border border-destructive/40 bg-destructive/8 p-3">
              <p class="m-0 text-xs font-medium text-destructive">热键注册失败，可能被其他程序或其他 Kotone 实例占用</p>
              <p class="mt-0.5 mb-0 text-[11px] break-all text-muted-foreground">{hotkeyStatus.error}</p>
              <Button size="xs" variant="destructive" class="mt-2" onclick={() => void saveHotkey()}>重新注册</Button>
            </div>
          {/if}
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    </div>

    <!-- 交互模式（ADR-006 四预设） -->
    <div id="voice-mode" class="scroll-mt-16">
    <SettingsSection title="交互模式" description="按热键之后怎么录、什么时候发。">
      <div class="py-3.5">
        <div class="grid grid-cols-2 gap-2 lg:grid-cols-4" role="group" aria-label="交互模式">
          {#each modes as m}
            {@const active = currentMode === m.id}
            <button
              aria-pressed={active}
              class="rounded-xl border px-3 py-3 text-left transition-colors {active
                ? 'border-primary/60 bg-primary/10'
                : 'border-hairline bg-background/40 hover:bg-muted/60'}"
              onclick={() => void onModeSelect(m.id)}
            >
              <span class="inline-flex size-5 items-center justify-center {active ? 'text-primary' : 'text-muted-foreground'}" aria-hidden="true">
                {#if m.id === "push-to-talk"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>
                {:else if m.id === "dictation"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3" fill="currentColor" stroke="none"/></svg>
                {:else if m.id === "one-shot"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>
                {:else}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>
                {/if}
              </span>
              <p class="mt-1 mb-0 text-[13px] font-semibold {active ? 'text-primary' : 'text-foreground'}">{m.name}</p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">{m.desc}</p>
            </button>
          {/each}
        </div>
        {#if currentMode === null}
          <p class="mt-2 mb-0 text-[11px] text-muted-foreground">
            当前为兼容模式（由热键触发方式 + 发送行为旧字段推导）；点选上方卡片即切换为预设模式。
          </p>
        {/if}
      </div>
      {#if currentMode === "one-shot" || currentMode === "solo"}
        <!-- one-shot / solo 专属：VAD 静音判停时长（solo 按该阈值切分每一句） -->
        <SettingsRow label="静音判停时长" for="vad-slider" stacked
          description={currentMode === "solo"
            ? "每句之间静音超过该时长即切分并发送，随后继续监听下一句（200–5000ms，需要 VAD 模型就绪）。"
            : "说完话后静音超过该时长即自动结束并发送（200–5000ms，需要 VAD 模型就绪）。"}>
          {#snippet control()}
            <div class="flex items-center gap-3">
              <input id="vad-slider" type="range" min="200" max="5000" step="100" value={vadMs}
                onchange={(e) => void onVadChange(e)} class="w-full accent-[var(--color-primary)]" />
              <span class="shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-xs font-semibold text-primary tabular-nums">{vadMs} ms</span>
            </div>
          {/snippet}
        </SettingsRow>
      {/if}
    </SettingsSection>
    </div>

    <!-- 悬浮窗：只属于语音输入（角色用气泡，ADR 028）。样式从「设置 → 系统」搬来。 -->
    <div id="voice-overlay" class="scroll-mt-16">
    <SettingsSection title="悬浮窗" description="语音输入时在游戏里显示识别进度的小窗。角色的回答在气泡里，不用它。">
      <SettingsRow label="什么时候出现" stacked>
        {#snippet control()}
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
            {#each [
              { id: "on_demand", name: "用时浮现", desc: "平时隐藏，说话时出现，发完自动隐藏" },
              { id: "always", name: "常驻", desc: "一直显示在游戏画面上" },
              { id: "never", name: "不显示", desc: "关闭悬浮窗，热键和语音输入照常" },
            ] as const as opt}
              {@const selected = ($settingsStore.overlay?.visibility ?? "on_demand") === opt.id}
              <button
                aria-pressed={selected}
                class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected
                  ? 'border-primary/60 bg-primary/10'
                  : 'border-hairline bg-background/40 hover:bg-muted/60'}"
                onclick={() => void patchSettings({ overlay: { visibility: opt.id } }, `悬浮窗已切换：${opt.name}`)}
              >
                <p class="m-0 text-[13px] font-semibold {selected ? 'text-primary' : 'text-foreground'}">{opt.name}</p>
                <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">{opt.desc}</p>
              </button>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label="固定位置" stacked>
        {#snippet control()}
          <div class="grid grid-cols-4 gap-2" role="group" aria-label="悬浮窗位置">
            {#each [
              { id: "auto", name: "智能" },
              { id: "top_left", name: "左上" },
              { id: "top_center", name: "顶部" },
              { id: "top_right", name: "右上" },
              { id: "center", name: "中央" },
              { id: "bottom_left", name: "左下" },
              { id: "bottom_center", name: "底部" },
              { id: "bottom_right", name: "右下" },
            ] as const as opt}
              {@const selected = ($settingsStore.overlay.position ?? "auto") === opt.id}
              <button
                aria-pressed={selected}
                class="h-8 rounded-lg border text-center text-xs font-medium transition-colors {selected
                  ? 'border-primary/60 bg-primary/10 text-primary'
                  : 'border-hairline bg-background/40 text-muted-foreground hover:bg-muted/60 hover:text-foreground'}"
                onclick={() => void patchSettings({ overlay: { position: opt.id } }, `悬浮窗位置已切换：${opt.name}`)}
              >
                {opt.name}
              </button>
            {/each}
          </div>
          {#if $settingsStore.overlay.position === "custom"}
            <p class="mt-2 mb-0 text-[11px] text-primary">当前使用你上次拖动后的位置。</p>
          {/if}
        {/snippet}
      </SettingsRow>
      <div class="py-3.5">
        <ToggleField
          checked={!$settingsStore.overlay.clickThrough}
          label="允许鼠标操作和拖动悬浮窗"
          desc="开启时可拖动并操作按钮；关闭后自动锁定位置，鼠标会穿透到游戏"
          onchange={(v) =>
            void patchSettings(
              { overlay: { draggable: v, clickThrough: !v } },
              v ? "悬浮窗已允许操作和拖动" : "悬浮窗已锁定并开启鼠标穿透",
            )}
        />
      </div>
      <SettingsRow label="样式" stacked>
        {#snippet control()}
          <div class="grid grid-cols-2 gap-2">
            {#each [
              { id: "capsule", name: "胶囊", desc: "更轻巧，减少遮挡" },
              { id: "card", name: "卡片", desc: "信息完整，适合首次使用" },
            ] as const as option}
              {@const selected = $settingsStore.overlay.style === option.id}
              <button type="button" aria-pressed={selected}
                class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected ? 'border-primary/60 bg-primary/10' : 'border-hairline hover:bg-muted/60'}"
                onclick={() => void patchSettings({ overlay: { style: option.id } }, `悬浮窗样式已切换：${option.name}`)}>
                <p class="m-0 text-[13px] font-medium {selected ? 'text-primary' : 'text-foreground'}">{option.name}</p>
                <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{option.desc}</p>
              </button>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    </div>

    <VoiceHotkeysSection />

    <section class="rounded-2xl border border-hairline bg-card/40 px-4 py-3 text-xs leading-relaxed text-muted-foreground" data-testid="voice-shared">
      共享设置：麦克风 <b class="font-medium text-foreground">{micName}</b>
      <button class="text-primary hover:underline" onclick={onOpenSound}>设置 → 声音</button>
      <span class="mx-1">·</span>
      游戏 <b class="font-medium text-foreground">{game?.displayName ?? "通用"}</b>{#if game}（{game.openChatKey} 开聊天，热词 {game.hotwords.length}）{/if}
      <button class="text-primary hover:underline" onclick={onOpenGames}>游戏列表</button>
    </section>

    {#if !guideOpen}
      <div class="flex justify-center">
        <Button size="sm" variant="ghost" data-testid="voice-guide-open" onclick={onOpenGuide}>重新走一遍语音输入引导</Button>
      </div>
    {/if}
  {/if}
</SettingsPage>

