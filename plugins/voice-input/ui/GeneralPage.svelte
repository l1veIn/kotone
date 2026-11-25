<script lang="ts">
  /*
   * 语音输入 · 输入（ADR 028 W8 / W9，页面 ID 仍是 general）：语音识别卡片（只显示现在用的，换和启动在「设置 → 模型」）、
   * 语音输入热键、交互模式、悬浮窗、其他热键。说话、原话打进游戏聊天框，不经过角色（Agent）。
   * 麦克风搬到「设置 → 声音」，游戏在「游戏列表」——这里只显示一行共享设置。
   */
  import { onMount } from "svelte";
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
  } from "$lib/ipc";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
  import HotkeyLink from "$lib/components/HotkeyLink.svelte";
  import {
    settingsStore,
    fullscreenWarningStore,
    patchSettings,
    toast,
    errText,
  } from "$lib/stores/ui";
  import { asrLocalStore } from "$lib/stores/runtime";
  import AsrCard from "./AsrCard.svelte";
  import VoiceHotkeysSection from "./VoiceHotkeysSection.svelte";
  import VoiceSetupGuide from "./VoiceSetupGuide.svelte";
  import { m } from "$lib/paraglide/messages.js";

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
  const micName = $derived(devices.find((d) => d.id === $settingsStore?.audioDeviceId)?.name ?? m.voice_general_mic_default());
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
        if (elevation?.elevated) toast(true, m.voice_general_admin_ok());
      }
    } catch (e) {
      toast(false, m.voice_general_load_devices_failed({ error: errText(e) }));
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

  // 语音输入热键只在「设置 → 热键」改（ADR 040 D3）；这里显示现在的键、登记失败的原因
  let hotkeyStatus = $state<HotkeyStatus | null>(null);

  onMount(async () => {
    try {
      hotkeyStatus = await getHotkeyStatus();
    } catch (e) {
      toast(false, m.voice_hotkey_status_read_failed({ error: errText(e) }));
    }
  });

  const modes: { id: InteractionMode; name: string; desc: string }[] = [
    { id: "push-to-talk", name: m.voice_mode_ptt_name(), desc: m.voice_mode_ptt_desc() },
    { id: "dictation", name: m.voice_mode_dictation_name(), desc: m.voice_mode_dictation_desc() },
    { id: "one-shot", name: m.voice_mode_one_shot_name(), desc: m.voice_mode_one_shot_desc() },
    { id: "solo", name: m.voice_mode_solo_name(), desc: m.voice_mode_solo_desc() },
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
      const mode = modes.find((x) => x.id === id);
      toast(true, m.voice_general_mode_switched({ name: mode?.name ?? id }));
    } catch (e) {
      toast(false, m.voice_general_mode_switch_failed({ error: errText(e) }));
    }
  }

  async function onVadChange(e: Event) {
    const vadSilenceMs = Number((e.target as HTMLInputElement).value);
    try {
      settingsStore.set(await updateSettings({ vadSilenceMs }));
    } catch (err) {
      toast(false, m.voice_save_failed({ error: errText(err) }));
    }
  }

</script>

<SettingsPage embedded title={m.voice_tab_input()} testid="general-page">
  {#if guideOpen}
    <VoiceSetupGuide onClose={onCloseGuide} {onConfigure} />
  {/if}

  {#if $settingsStore}
    {#if needsElevation}
      <section class="flex flex-wrap items-center gap-3 rounded-2xl border border-brand-pink/45 bg-brand-pink/8 px-4 py-3.5">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] font-semibold text-brand-pink">{m.voice_general_elevation_title()}</p>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.voice_general_elevation_desc()}</p>
        </div>
        <Button size="sm" class="bg-brand-pink text-[#fff] hover:bg-brand-pink/90" disabled={restarting} onclick={() => void onRestartAsAdmin()}>
          {restarting ? m.voice_general_restarting() : m.voice_general_restart()}
        </Button>
      </section>
    {/if}

    {#if $fullscreenWarningStore}
      <section class="flex flex-wrap items-center gap-3 rounded-2xl border border-warning/45 bg-warning/8 px-4 py-3.5">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] font-semibold text-warning">{m.voice_general_fullscreen_title()}</p>
          <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">
            {m.voice_general_fullscreen_desc()}
          </p>
        </div>
        <Button size="sm" variant="outline" onclick={() => fullscreenWarningStore.set(false)}>{m.voice_action_got_it()}</Button>
      </section>
    {/if}

    <AsrCard {onOpenModels} />

    <div id="voice-hotkey" class="scroll-mt-16">
    <SettingsSection title={m.voice_hotkey_name_record()}>
      <SettingsRow label={m.voice_hotkey_name_record()} description={m.voice_general_hotkey_desc()} stacked>
        {#snippet control()}
          <HotkeyLink id="voice-input.talk" value={$settingsStore?.hotkey.key ?? ""} />
          {#if hotkeyStatus?.error}
            <div class="mt-3 rounded-xl border border-destructive/40 bg-destructive/8 p-3">
              <p class="m-0 text-xs font-medium text-destructive">{m.voice_general_hotkey_register_failed()}</p>
              <p class="mt-0.5 mb-0 text-[11px] break-all text-muted-foreground">{hotkeyStatus.error}</p>
            </div>
          {/if}
        {/snippet}
      </SettingsRow>
    </SettingsSection>
    </div>

    <!-- 交互模式（ADR-006 四预设） -->
    <div id="voice-mode" class="scroll-mt-16">
    <SettingsSection title={m.voice_general_mode_title()} description={m.voice_general_mode_desc()}>
      <div class="py-3.5">
        <div class="grid grid-cols-2 gap-2 lg:grid-cols-4" role="group" aria-label={m.voice_general_mode_title()}>
          {#each modes as mode}
            {@const active = currentMode === mode.id}
            <button
              aria-pressed={active}
              class="rounded-xl border px-3 py-3 text-left transition-colors {active
                ? 'border-primary/60 bg-primary/10'
                : 'border-hairline bg-background/40 hover:bg-muted/60'}"
              onclick={() => void onModeSelect(mode.id)}
            >
              <span class="inline-flex size-5 items-center justify-center {active ? 'text-primary' : 'text-muted-foreground'}" aria-hidden="true">
                {#if mode.id === "push-to-talk"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>
                {:else if mode.id === "dictation"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3" fill="currentColor" stroke="none"/></svg>
                {:else if mode.id === "one-shot"}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>
                {:else}
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>
                {/if}
              </span>
              <p class="mt-1 mb-0 text-[13px] font-semibold {active ? 'text-primary' : 'text-foreground'}">{mode.name}</p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">{mode.desc}</p>
            </button>
          {/each}
        </div>
        {#if currentMode === null}
          <p class="mt-2 mb-0 text-[11px] text-muted-foreground">
            {m.voice_general_mode_legacy()}
          </p>
        {/if}
      </div>
      {#if currentMode === "one-shot" || currentMode === "solo"}
        <!-- one-shot / solo 专属：VAD 静音判停时长（solo 按该阈值切分每一句） -->
        <SettingsRow label={m.voice_general_vad_label()} for="vad-slider" stacked
          description={currentMode === "solo"
            ? m.voice_general_vad_desc_solo()
            : m.voice_general_vad_desc()}>
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
    <SettingsSection title={m.voice_general_overlay_title()} description={m.voice_general_overlay_desc()}>
      <SettingsRow label={m.voice_general_overlay_visibility_label()} stacked>
        {#snippet control()}
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
            {#each [
              { id: "on_demand", name: m.voice_overlay_visibility_on_demand_name(), desc: m.voice_overlay_visibility_on_demand_desc() },
              { id: "always", name: m.voice_overlay_visibility_always_name(), desc: m.voice_overlay_visibility_always_desc() },
              { id: "never", name: m.voice_overlay_visibility_never_name(), desc: m.voice_overlay_visibility_never_desc() },
            ] as const as opt}
              {@const selected = ($settingsStore.overlay?.visibility ?? "on_demand") === opt.id}
              <button
                aria-pressed={selected}
                class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected
                  ? 'border-primary/60 bg-primary/10'
                  : 'border-hairline bg-background/40 hover:bg-muted/60'}"
                onclick={() => void patchSettings({ overlay: { visibility: opt.id } }, m.voice_general_overlay_visibility_switched({ name: opt.name }))}
              >
                <p class="m-0 text-[13px] font-semibold {selected ? 'text-primary' : 'text-foreground'}">{opt.name}</p>
                <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">{opt.desc}</p>
              </button>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.voice_general_overlay_position_label()} stacked>
        {#snippet control()}
          <div class="grid grid-cols-4 gap-2" role="group" aria-label={m.voice_general_overlay_position_aria()}>
            {#each [
              { id: "auto", name: m.voice_overlay_position_auto() },
              { id: "top_left", name: m.voice_overlay_position_top_left() },
              { id: "top_center", name: m.voice_overlay_position_top_center() },
              { id: "top_right", name: m.voice_overlay_position_top_right() },
              { id: "center", name: m.voice_overlay_position_center() },
              { id: "bottom_left", name: m.voice_overlay_position_bottom_left() },
              { id: "bottom_center", name: m.voice_overlay_position_bottom_center() },
              { id: "bottom_right", name: m.voice_overlay_position_bottom_right() },
            ] as const as opt}
              {@const selected = ($settingsStore.overlay.position ?? "auto") === opt.id}
              <button
                aria-pressed={selected}
                class="h-8 rounded-lg border text-center text-xs font-medium transition-colors {selected
                  ? 'border-primary/60 bg-primary/10 text-primary'
                  : 'border-hairline bg-background/40 text-muted-foreground hover:bg-muted/60 hover:text-foreground'}"
                onclick={() => void patchSettings({ overlay: { position: opt.id } }, m.voice_general_overlay_position_switched({ name: opt.name }))}
              >
                {opt.name}
              </button>
            {/each}
          </div>
          {#if $settingsStore.overlay.position === "custom"}
            <p class="mt-2 mb-0 text-[11px] text-primary">{m.voice_general_overlay_position_custom()}</p>
          {/if}
        {/snippet}
      </SettingsRow>
      <div class="py-3.5">
        <ToggleField
          checked={!$settingsStore.overlay.clickThrough}
          label={m.voice_general_overlay_mouse_label()}
          desc={m.voice_general_overlay_mouse_desc()}
          onchange={(v) =>
            void patchSettings(
              { overlay: { draggable: v, clickThrough: !v } },
              v ? m.voice_general_overlay_mouse_on() : m.voice_general_overlay_mouse_off(),
            )}
        />
      </div>
      <SettingsRow label={m.voice_general_overlay_style_label()} stacked>
        {#snippet control()}
          <div class="grid grid-cols-2 gap-2">
            {#each [
              { id: "capsule", name: m.voice_overlay_style_capsule_name(), desc: m.voice_overlay_style_capsule_desc() },
              { id: "card", name: m.voice_overlay_style_card_name(), desc: m.voice_overlay_style_card_desc() },
            ] as const as option}
              {@const selected = $settingsStore.overlay.style === option.id}
              <button type="button" aria-pressed={selected}
                class="rounded-xl border px-3 py-2.5 text-left transition-colors {selected ? 'border-primary/60 bg-primary/10' : 'border-hairline hover:bg-muted/60'}"
                onclick={() => void patchSettings({ overlay: { style: option.id } }, m.voice_general_overlay_style_switched({ name: option.name }))}>
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
      {m.voice_general_shared_mic()} <b class="font-medium text-foreground">{micName}</b>
      <button class="text-primary hover:underline" onclick={onOpenSound}>{m.voice_general_shared_sound_link()}</button>
      <span class="mx-1">·</span>
      {m.voice_general_shared_game()} <b class="font-medium text-foreground">{game?.displayName ?? m.voice_general_shared_game_generic()}</b>{#if game}{m.voice_general_shared_game_info({ chatKey: game.openChatKey, count: game.hotwords.length })}{/if}
      <button class="text-primary hover:underline" onclick={onOpenGames}>{m.voice_general_shared_games_link()}</button>
    </section>

    {#if !guideOpen}
      <div class="flex justify-center">
        <Button size="sm" variant="ghost" data-testid="voice-guide-open" onclick={onOpenGuide}>{m.voice_general_guide_reopen()}</Button>
      </div>
    {/if}
  {/if}
</SettingsPage>

