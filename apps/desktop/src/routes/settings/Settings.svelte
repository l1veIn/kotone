<script lang="ts">
  /*
   * 设置窗口视图（index.html#/settings）——方向 B「中继站」：
   * 左侧导航 + 右侧页面。页面组件在 ./pages/ 下，
   * 共享 settingsStore / toast（lib/stores/ui.ts）；IPC 全部走 lib/ipc.ts。
   */
  import { onDestroy, onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    getSettings,
    updateSettings,
    getOverlayHealth,
    getSettingsLoadWarning,
    getStartupOptions,
    getElevationStatus,
    isTauri,
    type OverlayHealth,
  } from "../../lib/ipc";
  import {
    settingsStore,
    fullscreenWarningStore,
    toast,
    toastWarn,
    errText,
    AUTO_ADMIN_PROMPT_FLAG,
  } from "../../lib/stores/ui";
  import { initRuntime } from "../../lib/stores/runtime";
  import { companionBindings, startPageOf, usesLegacyVoiceInput } from "../../lib/ipc/contracts";
  import LegacyRemindersPage from "./pages/LegacyRemindersPage.svelte";
  import { checkForUpdates } from "../../lib/updater";
  import { Sonner } from "$lib/components/ui/sonner";
  import ElevationPrompt from "../../lib/components/ElevationPrompt.svelte";
  import AutoAdminPrompt from "../../lib/components/AutoAdminPrompt.svelte";
  import InjectionBlockedDialog from "../../lib/components/InjectionBlockedDialog.svelte";
  import UpdateDialog from "../../lib/components/UpdateDialog.svelte";
  import { appState } from "../../lib/stores/state";
  import Onboarding, { type OnboardingNext } from "./Onboarding.svelte";
  import type { DeepSeekSetupResult } from "../../lib/deepseekSetup";
  import { pendingPrompt } from "../../lib/agentChat/pendingPrompt";
  import Titlebar from "./Titlebar.svelte";
  import GeneralPage from "./pages/GeneralPage.svelte";
  import TextProcessingPage from "./pages/TextProcessingPage.svelte";
  import { navigationRequest, sidebarPagesOpen, type AssistantSection, type VoiceInputPage } from "../../lib/stores/navigation";
  import VoiceInputShell from "./pages/voice/VoiceInputShell.svelte";
  import AssistantPage from "./pages/AssistantPage.svelte";
  import AutomationPage from "./pages/AutomationPage.svelte";
  import AdvancedPage from "./pages/AdvancedPage.svelte";
  import { isAdvancedSection, type AdvancedSection } from "./pages/advanced/types";
  import GamePage from "./pages/GamePage.svelte";
  import MusicPage from "./pages/MusicPage.svelte";
  import SidebarPagesDialog from "./SidebarPagesDialog.svelte";
  import HistoryPage from "./pages/HistoryPage.svelte";
  import AboutPage from "./pages/AboutPage.svelte";
  import CharacterPage from "./pages/CharacterPage.svelte";
  import { initializeAppearance, resolvedTheme } from "../../lib/appearance/appearanceRuntime";

  import AppSidebar, { type PageId } from "./AppSidebar.svelte";
  import { AgentChat } from "../../lib/agentChat/chatController.svelte";
  import { provideAgentChat } from "../../lib/agentChat/context";
  import { m } from "$lib/paraglide/messages.js";

  /** 侧边栏：宽窗口展开 / 图标栏（记在本机）；窄窗口（< 760px）改成抽屉。 */
  const SIDEBAR_KEY = "kotone.sidebar.v1";
  let rail = $state(readRail());
  let drawerOpen = $state(false);
  let viewportWidth = $state(typeof window === "undefined" ? 1280 : window.innerWidth);
  const narrow = $derived(viewportWidth < 760);
  function readRail(): boolean {
    try { return localStorage.getItem(SIDEBAR_KEY) === "rail"; } catch { return false; }
  }
  function toggleSidebar() {
    if (narrow) { drawerOpen = !drawerOpen; return; }
    rail = !rail;
    try { localStorage.setItem(SIDEBAR_KEY, rail ? "rail" : "expanded"); } catch { /* 只是偏好 */ }
  }
  function navigate(id: PageId) {
    assistantOpen = null;
    if (id === "advanced" && page !== "advanced") advancedAnchor = null;
    page = id;
    drawerOpen = false;
  }
  /** 侧边栏点了对话或“新对话”：切到聊天。 */
  let chatRequest = $state(0);
  /** 打开角色页的某个分页（侧栏的角色入口、聊天里“去看看”）。普通导航清掉它，回到聊天。 */
  let assistantOpen = $state<{ section: AssistantSection; n: number } | null>(null);
  let openCount = 0;
  function openCompanion(section: AssistantSection) {
    page = "assistant";
    drawerOpen = false;
    assistantOpen = { section, n: ++openCount };
    if (section === "chat") chatRequest++;
  }
  /** 「设置」的陪玩组（原「琴音 → 设置」，ADR 028 D11）。 */
  function openCompanionSettings(section: AdvancedSection = "call", anchor?: string) {
    openAdvanced(section, anchor);
  }
  // 聊天里的“去看看”等：跳到设置说明里写的位置。
  const unsubscribeNavigation = navigationRequest.subscribe((request) => {
    if (!request) return;
    navigationRequest.set(null);
    const target = request.target;
    if (target.page === "assistant") openCompanion(target.section);
    else if (target.page === "sidebar-pages") sidebarPagesOpen.set(true);
    else if (target.page === "advanced") openAdvanced(isAdvancedSection(target.tab) ? target.tab : "system", target.anchor);
    else {
      navigate(target.page);
      if (target.anchor) scrollToAnchor(target.anchor);
    }
  });
  onDestroy(unsubscribeNavigation);
  function scrollToAnchor(id: string) {
    void tick().then(() => requestAnimationFrame(() => document.getElementById(id)?.scrollIntoView({ block: "start" })));
  }
  /** 打开聊天；prompt 只预填进输入框（D2）。 */
  function openChat(prompt?: string) {
    if (prompt) pendingPrompt.set(prompt);
    openCompanion("chat");
  }
  function openRole() {
    openCompanion("identity");
  }

  // ---------- 引导（ADR 028） ----------
  /** 正在显示的引导：新用户 / 0.1.x 升级用户；从聊天页 Key 卡片或“让 Kotone 帮我配置”进来时带起始步骤。 */
  let onboarding = $state<{ mode: "new" | "upgrade"; initialStep?: "key" | "connected"; result?: DeepSeekSetupResult | null } | null>(null);
  /** 语音输入子板块顶部的四步引导（V 路径）。 */
  let voiceGuideOpen = $state(false);

  function openOnboarding() {
    onboarding = { mode: legacyVoiceOnly ? "upgrade" : "new" };
  }
  function onboardingDone(next: OnboardingNext, prompt?: string) {
    onboarding = null;
    if (next === "chat") openChat(prompt);
    else {
      navigate("general");
      if (next === "voiceGuide") voiceGuideOpen = true;
    }
  }
  /** 聊天页的 Key 卡片连好了：接着设呼叫键（引导的“已连接”页）。 */
  function keyConnected(result: DeepSeekSetupResult) {
    onboarding = { mode: "new", initialStep: "connected", result };
  }
  /** 语音输入的「让 Kotone 帮我配置」：有可用的角色就请她帮忙（只预填，不经过语音输入）；没有就先填 Key。 */
  function configureVoiceInput() {
    const s = $settingsStore;
    const connectionId = companionBindings(s?.assistant)?.connectionId?.trim();
    if (s && !legacyVoiceOnly && connectionId && s.connections.some((c) => c.id === connectionId)) {
      openChat(m.shell_settings_voice_setup_prompt());
    } else {
      onboarding = { mode: legacyVoiceOnly ? "upgrade" : "new", initialStep: "key" };
    }
  }

  // 侧边栏的对话列表与聊天页共用一份状态。
  const chat = provideAgentChat(new AgentChat());
  $effect(() => {
    if (loading || legacyVoiceOnly) return;
    void chat.init();
    return () => chat.dispose();
  });

  // 外观（配色、明暗、缩放、字体）只作用于设置窗口：在这里挂上，卸载时还原。
  // 在首次绘制前应用，避免先闪一下默认配色。
  onDestroy(initializeAppearance());

  /** 打开时先显示聊天；逃生通道和“只想用语音输入？”的用户、或设置里选了语音输入的，落在语音输入（ADR 028）。 */
  let page = $state<PageId>("assistant");
  let advancedSection = $state<AdvancedSection>("call");
  const chatActive = $derived(page === "assistant" && (assistantOpen?.section ?? "chat") === "chat");
  let advancedAnchor = $state<{ id: string; n: number } | null>(null);
  let anchorCount = 0;

  function openAdvanced(section: AdvancedSection = "system", anchor?: string) {
    // 逃生通道用户没有陪玩组：落到模型页。
    advancedSection = legacyVoiceOnly && ["call", "reply", "permissions", "usage"].includes(section) ? "models" : section;
    advancedAnchor = anchor ? { id: anchor, n: ++anchorCount } : null;
    page = "advanced";
    drawerOpen = false;
  }
  /** 右侧滚动容器：切页时复位到顶部（角色详情页是长滚动页） */
  let mainEl = $state<HTMLElement>();
  $effect(() => {
    page;
    mainEl?.scrollTo({ top: 0 });
  });
  let loading = $state(true);
  const legacyVoiceOnly = $derived(usesLegacyVoiceInput($settingsStore?.ui));
  /** 管理员提权提示 · 第一段（仅 Windows 未提权且用户未表态时弹一次） */
  let showElevationPrompt = $state(false);
  /** 弹窗是因为刚才真的发不出去（不是启动时的建议）。 */
  let elevationBlocked = $state(false);
  /** 管理员提权提示 · 第二段（第一段「以管理员权限重启」成功后弹一次） */
  let showAutoAdminPrompt = $state(false);
  /** 模拟输入被系统拦截的排查引导弹窗（SendInput 0/N 时弹出；同一错误只弹一次） */
  let showBlockedDialog = $state(false);
  let blockedDismissedFor = $state<string | null>(null);
  let overlayHealth = $state<OverlayHealth>({ degraded: false, reason: null });
  let settingsReadRevision = 0;

  async function refreshSettings() {
    const revision = ++settingsReadRevision;
    const settings = await getSettings();
    if (revision === settingsReadRevision) settingsStore.set(settings);
    return settings;
  }

  $effect(() => {
    if ($appState.state === "error" && $appState.inputBlocked) {
      if (blockedDismissedFor !== ($appState.errorMessage ?? "")) {
        showBlockedDialog = true;
      }
    }
  });

  function closeBlockedDialog() {
    blockedDismissedFor = $appState.errorMessage ?? "";
    showBlockedDialog = false;
  }

  onMount(async () => {
    // runtime store 独立初始化（kotone://runtime 事件订阅 + 初始拉取）
    void initRuntime();
    try {
      const [s, startup, settingsWarning] = await Promise.all([
        refreshSettings(),
        getStartupOptions(),
        getSettingsLoadWarning(),
      ]);
      if (settingsWarning) toastWarn(settingsWarning);
      const legacy = usesLegacyVoiceInput(s.ui);
      page = legacy || startPageOf(s.ui) === "voiceInput" ? "general" : "assistant";
      if (legacy) advancedSection = "models";
      if (legacy) {
        // 0.1.x 升级用户：新引导显示一次（即使 firstRunCompleted 为真），看过 = legacyUpgrade 不再是 pending。
        if (startup.onboarding === "always" || (startup.onboarding === "auto" && s.ui.legacyUpgrade === "pending")) onboarding = { mode: "upgrade" };
      } else if (startup.onboarding === "always" || (startup.onboarding === "auto" && !s.ui.firstRunCompleted)) {
        onboarding = { mode: "new" };
      }
      // 两段式提权提示：
      // 第一段（未提权）：未勾选「不再提示」/「默认提权启动」时，询问是否以管理员身份重启；
      // 第二段（提权）：第一段的重启成功后（localStorage 接力标记），询问是否启动时自动请求。
      if (isTauri) {
        const st = await getElevationStatus().catch(() => null);
        if (st?.activeGameFullscreen === true) fullscreenWarningStore.set(true);
        showElevationPrompt =
          st !== null &&
          st.supported &&
          !st.elevated &&
          !s.adminPromptDismissed &&
          !s.runAsAdminOnStart;
        if (st?.supported && st.elevated && localStorage.getItem(AUTO_ADMIN_PROMPT_FLAG)) {
          localStorage.removeItem(AUTO_ADMIN_PROMPT_FLAG);
          showAutoAdminPrompt = !s.runAsAdminOnStart && !s.autoAdminPromptDismissed;
        }
      }
    } catch (e) {
      toast(false, m.shell_settings_load_failed({ error: errText(e) }));
    } finally {
      loading = false;
    }
  });

  onMount(() => {
    if (!isTauri) return;
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const applyHealth = (health: OverlayHealth) => {
      const wasDegraded = overlayHealth.degraded;
      overlayHealth = health;
      if (health.degraded) {
        void refreshSettings().catch(() => { /* 提示不依赖配置刷新成功。 */ });
      } else if (wasDegraded) {
        // 用户重新启用显示时，旧降级读取不能覆盖刚保存的设置。
        settingsReadRevision++;
      }
    };
    void (async () => {
      const cleanup = await listen<OverlayHealth>("kotone://overlay-degraded", ({ payload }) => {
        if (disposed) return;
        revision++;
        applyHealth(payload);
      });
      if (disposed) { cleanup(); return; }
      unlisten = cleanup;
      const snapshotRevision = revision;
      const health = await getOverlayHealth();
      if (!disposed && revision === snapshotRevision) applyHealth(health);
    })().catch(() => { /* 健康状态不可用不影响设置和语音输入。 */ });
    return () => { disposed = true; unlisten?.(); };
  });

  // 单实例转发：应用已在托盘运行时，再执行
  // `kotone.exe --onboarding=always` 会唤起现有窗口并重新打开向导。
  onMount(() => {
    if (!isTauri) return;
    // 启动后静默检测一次更新（离线/无 release 不打扰用户）
    const updateTimer = setTimeout(() => void checkForUpdates({ auto: true }), 3000);
    let unlisten: (() => void) | undefined;
    void listen("kotone://open-onboarding", openOnboarding).then(
      (un) => (unlisten = un),
    );
    return () => {
      clearTimeout(updateTimer);
      unlisten?.();
    };
  });

  // Agent 经工具服务改了设置（settings.update / 连接 / 模型 / profile …）：
  // 重新读取，打开着的设置页不显示过期值，也不会用旧值覆盖。
  onMount(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    void listen("kotone://settings-changed", () => {
      void refreshSettings().catch(() => { /* 下次打开设置页会重新读取 */ });
    }).then((un) => (unlisten = un));
    return () => unlisten?.();
  });

  // 打字 / 按键被 Windows 拦下（游戏是管理员权限、Kotone 不是）：弹“以管理员身份重启”。
  // 启动时那次提示玩家可能点过“不再提示”，这里是真的失败了，照样弹（她也会用语音说）。
  onMount(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    void listen("kotone://elevation-needed", () => {
      elevationBlocked = true;
      showElevationPrompt = true;
    }).then((un) => (unlisten = un));
    return () => unlisten?.();
  });

  // 独占全屏下后端会拦截浮窗显示，并发送持久到共享 store 的提示事件。
  // 监听放在设置根组件，不依赖用户当时是否停留在「通用」分页。
  onMount(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    void listen("kotone://fullscreen-warning", () => fullscreenWarningStore.set(true)).then(
      (un) => (unlisten = un),
    );
    return () => unlisten?.();
  });
</script>

<svelte:window bind:innerWidth={viewportWidth} />
<div class="flex h-full flex-col overflow-hidden bg-sidebar text-foreground" data-testid="app-frame">
  <!-- 自绘标题栏（decorations:false）：侧边栏开关 + 本地识别状态（只在用本地识别时）+ 窗口控制 -->
  <Titlebar onOpenVoiceInput={() => { navigate("general"); scrollToAnchor("voice-asr"); }} onToggleSidebar={toggleSidebar}
    brandWidth={narrow ? "auto" : rail ? "3.5rem" : "14.75rem"} />
  <div class="relative flex min-h-0 flex-1 overflow-hidden">
  {#if !narrow}
    <AppSidebar {page} {chatActive} {rail} {legacyVoiceOnly} onNavigate={navigate} onOpenChat={() => openChat()} onOpenRole={openRole} />
  {:else if drawerOpen}
    <!-- 窄窗口：侧边栏作为抽屉盖在内容上 -->
    <button type="button" class="absolute inset-0 z-40 bg-black/40" aria-label={m.shell_settings_close_sidebar()} onclick={() => (drawerOpen = false)}></button>
    <div class="absolute inset-y-0 left-0 z-50 shadow-2xl" data-testid="sidebar-drawer">
      <AppSidebar {page} {chatActive} {legacyVoiceOnly} onNavigate={navigate} onOpenChat={() => openChat()} onOpenRole={openRole} />
    </div>
  {/if}

  <!-- 内容区：比侧边栏亮一档的底色，左上圆角（参照 DeepSeek Harness 的 Windows 外框） -->
  <main class="relative z-10 min-w-0 flex-1 overflow-y-auto border-t border-hairline bg-background [scrollbar-gutter:stable]
    {narrow ? '' : 'rounded-tl-2xl border-l'}" bind:this={mainEl}>
    {#if overlayHealth.degraded}
      <p role="status" data-testid="overlay-degraded-notice" class="mx-8 mt-5 rounded-lg border border-amber-300/20 bg-amber-300/8 px-4 py-3 text-xs leading-relaxed text-amber-100/90">
        {m.shell_settings_overlay_degraded()}
      </p>
    {/if}
    {#if loading}
      <p class="px-8 py-10 text-sm text-muted-foreground">{m.shell_settings_loading()}</p>
    {:else if page === "general" || page === "processing" || page === "history"}
      <VoiceInputShell {page} onTab={(next: VoiceInputPage) => navigate(next)} onConfigure={configureVoiceInput}>
        {#if page === "general"}
          <GeneralPage guideOpen={voiceGuideOpen} onOpenGuide={() => (voiceGuideOpen = true)} onCloseGuide={() => (voiceGuideOpen = false)}
            onConfigure={configureVoiceInput} onOpenModels={() => openAdvanced("models", "models-asr")}
            onOpenSound={() => openAdvanced("sound")} onOpenGames={() => navigate("game")} />
        {:else if page === "processing"}
          <TextProcessingPage onOpenConnections={() => openAdvanced("connections")} />
        {:else}
          <HistoryPage />
        {/if}
      </VoiceInputShell>
    {:else if page === "advanced"}
      <AdvancedPage
        section={advancedSection}
        anchor={advancedAnchor}
        onSectionChange={(next) => { advancedSection = next; advancedAnchor = null; }}
        onOpenOnboarding={openOnboarding}
      />
    {:else if page === "assistant"}
      <AssistantPage {chatRequest} open={assistantOpen}
        onOpenConnections={() => openAdvanced("connections")} onOpenAutomation={() => navigate("automation")}
        onOpenSettings={() => openCompanionSettings("reply", "settings-reply-ai")} onOpenTalk={() => openCompanionSettings("call", "settings-call-hotkey")}
        onKeyConnected={keyConnected} onBackToVoice={() => navigate("general")} />
    {:else if page === "legacy-reminders"}
      <LegacyRemindersPage onOpenConnections={() => openAdvanced("connections")} />
    {:else if page === "automation"}
      <AutomationPage onOpenHotkeys={() => openAdvanced("system", "system-stop")} />
    {:else if page === "game"}
      <GamePage {legacyVoiceOnly} />
    {:else if page === "music"}
      <MusicPage />
    {:else if page === "character"}
      <CharacterPage onBack={() => (page = "about")} />
    {:else}
      <AboutPage onOpenCharacter={() => (page = "character")} />
    {/if}

    <!-- 底部留白（提示在右上角，见 Sonner） -->
    {#if page !== "assistant" && page !== "music"}<div class="h-4"></div>{/if}
  </main>

  <!-- 左侧栏的“全部页面”（ADR 034 第六节）：侧栏的“⋯”、右键菜单、聊天里“去看看：左侧栏 → ⋯” -->
  <SidebarPagesDialog onOpen={(id) => (id === "assistant" ? openRole() : navigate(id))} />

  <!-- 引导覆盖层（ADR 028）：新用户 / 0.1.x 升级用户；聊天页 Key 卡片连好后接着设呼叫键 -->
  {#if onboarding && !loading}
    {#key onboarding}
      <Onboarding mode={onboarding.mode} initialStep={onboarding.initialStep} initialResult={onboarding.result ?? null} onDone={onboardingDone} />
    {/key}
  {/if}

  <!-- 管理员提权提示 · 第一段（Windows 未提权且用户未表态时弹一次） -->
  {#if showElevationPrompt && !loading && !onboarding}
    <ElevationPrompt blocked={elevationBlocked} onClose={() => { showElevationPrompt = false; elevationBlocked = false; }} />
  {/if}

  <!-- 管理员提权提示 · 第二段（第一段的重启成功后弹一次） -->
  {#if showAutoAdminPrompt && !loading && !onboarding}
    <AutoAdminPrompt onClose={() => (showAutoAdminPrompt = false)} />
  {/if}

  <!-- 模拟输入被系统拦截的排查引导（注入 0/N 落地时弹出，同一错误不重复打扰） -->
  {#if showBlockedDialog && !loading}
    <InjectionBlockedDialog onClose={closeBlockedDialog} />
  {/if}
  </div>

  <!-- 右上角 toast 堆叠（fixed 定位，z 高于向导） -->
  <Sonner theme={$resolvedTheme} />
  <UpdateDialog />
</div>
