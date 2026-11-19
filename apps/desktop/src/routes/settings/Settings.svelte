<script lang="ts">
  /*
   * 设置窗口视图（index.html#/settings）——方向 B「中继站」：
   * 左侧导航 + 右侧页面。页面组件在 ./pages/ 下，
   * 共享 settingsStore / toast（lib/stores/ui.ts）；IPC 全部走 lib/ipc.ts。
   */
  import { onDestroy, onMount } from "svelte";
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
  import { usesLegacyVoiceInput } from "../../lib/ipc/contracts";
  import { Button } from "$lib/components/ui/button";
  import LegacyRemindersPage from "./pages/LegacyRemindersPage.svelte";
  import { checkForUpdates } from "../../lib/updater";
  import { Sonner } from "$lib/components/ui/sonner";
  import ElevationPrompt from "../../lib/components/ElevationPrompt.svelte";
  import AutoAdminPrompt from "../../lib/components/AutoAdminPrompt.svelte";
  import InjectionBlockedDialog from "../../lib/components/InjectionBlockedDialog.svelte";
  import UpdateDialog from "../../lib/components/UpdateDialog.svelte";
  import { appState } from "../../lib/stores/state";
  import Onboarding from "./Onboarding.svelte";
  import { pendingPrompt } from "../../lib/agentChat/pendingPrompt";
  import Titlebar from "./Titlebar.svelte";
  import GeneralPage from "./pages/GeneralPage.svelte";
  import TextProcessingPage from "./pages/TextProcessingPage.svelte";
  import { navigationRequest } from "../../lib/stores/navigation";
  import AssistantPage from "./pages/AssistantPage.svelte";
  import AutomationPage from "./pages/AutomationPage.svelte";
  import AdvancedPage from "./pages/AdvancedPage.svelte";
  import type { AdvancedSection } from "./pages/advanced/types";
  import GamePage from "./pages/GamePage.svelte";
  import HistoryPage from "./pages/HistoryPage.svelte";
  import AboutPage from "./pages/AboutPage.svelte";
  import CharacterPage from "./pages/CharacterPage.svelte";
  import { initializeAppearance, resolvedTheme } from "../../lib/appearance/appearanceRuntime";

  import AppSidebar, { type PageId } from "./AppSidebar.svelte";
  import { AgentChat } from "../../lib/agentChat/chatController.svelte";
  import { provideAgentChat } from "../../lib/agentChat/context";

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
    if (id === "assistant" && legacyVoiceOnly) {
      showUpgradeNotice = true;
      drawerOpen = false;
      return;
    }
    assistantOpen = null;
    page = id;
    drawerOpen = false;
  }
  /** 侧边栏点了对话或“新对话”：切到「琴音」的聊天分页。 */
  let chatRequest = $state(0);
  /** 打开「琴音」的某个分页 / 分组（首页入口、聊天里“去看看”）；普通导航清掉它，回到聊天。 */
  let assistantOpen = $state<{ section: "chat" | "identity" | "memory" | "skills" | "settings"; anchor?: string; n: number } | null>(null);
  let openCount = 0;
  function openCompanion(section: "chat" | "identity" | "memory" | "skills" | "settings", anchor?: string) {
    navigate("assistant");
    if (!legacyVoiceOnly) assistantOpen = { section, anchor, n: ++openCount };
  }
  /** 「琴音 → 设置」（原「交流」页的内容也在这里）。 */
  function openCompanionSettings() {
    openCompanion("settings", "companion-settings-call");
  }
  // 聊天里的“去看看”等：跳到设置说明里写的位置。
  const unsubscribeNavigation = navigationRequest.subscribe((request) => {
    if (!request) return;
    navigationRequest.set(null);
    const target = request.target;
    if (target.page === "assistant") openCompanion(target.section, target.anchor);
    else if (target.page === "advanced") openAdvanced(target.tab as AdvancedSection);
    else navigate(target.page);
  });
  onDestroy(unsubscribeNavigation);
  function openChat(prompt?: string) {
    if (legacyVoiceOnly) {
      showUpgradeNotice = true;
      return;
    }
    if (prompt) pendingPrompt.set(prompt);
    navigate("assistant");
    chatRequest++;
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

  let page = $state<PageId>("general");
  let advancedSection = $state<AdvancedSection>("system");

  function openAdvanced(section: AdvancedSection = "system") {
    advancedSection = section;
    page = "advanced";
  }
  /** 右侧滚动容器：切页时复位到顶部（角色详情页是长滚动页） */
  let mainEl = $state<HTMLElement>();
  $effect(() => {
    page;
    mainEl?.scrollTo({ top: 0 });
  });
  let loading = $state(true);
  /** 首启向导：ui.firstRunCompleted === false 时弹出（完成/跳过后置 true） */
  let showOnboarding = $state(false);
  let showUpgradeNotice = $state(false);
  let showLegacySetup = $state(false);
  let savingUpgradeChoice = $state(false);
  const legacyVoiceOnly = $derived(usesLegacyVoiceInput($settingsStore?.ui));

  function openOnboarding() {
    if (legacyVoiceOnly) showUpgradeNotice = true;
    else showOnboarding = true;
  }

  async function keepVoiceInput() {
    if (savingUpgradeChoice) return;
    savingUpgradeChoice = true;
    try {
      settingsStore.set(await updateSettings({ ui: { legacyUpgrade: "dismissed" } }));
      showUpgradeNotice = false;
      showLegacySetup = false;
    } catch (error) {
      toast(false, errText(error));
    } finally {
      savingUpgradeChoice = false;
    }
  }
  /** 管理员提权提示 · 第一段（仅 Windows 未提权且用户未表态时弹一次） */
  let showElevationPrompt = $state(false);
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
      if (usesLegacyVoiceInput(s.ui)) {
        showUpgradeNotice = startup.onboarding !== "never" && s.ui.legacyUpgrade === "pending";
      } else {
        showOnboarding =
          startup.onboarding === "always" ||
          (startup.onboarding === "auto" && !s.ui.firstRunCompleted);
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
      toast(false, `加载配置失败：${errText(e)}`);
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
  <!-- 自绘标题栏（decorations:false）：侧边栏开关 + 运行状态 + 启动/停止 + 窗口控制 -->
  <Titlebar onOpenAdvanced={() => openAdvanced("models")} onToggleSidebar={toggleSidebar}
    brandWidth={narrow ? "auto" : rail ? "3.5rem" : "14.75rem"} />
  <div class="relative flex min-h-0 flex-1 overflow-hidden">
  {#if !narrow}
    <AppSidebar {page} {rail} {legacyVoiceOnly} onNavigate={navigate} onOpenChat={openChat} />
  {:else if drawerOpen}
    <!-- 窄窗口：侧边栏作为抽屉盖在内容上 -->
    <button type="button" class="absolute inset-0 z-40 bg-black/40" aria-label="关闭侧边栏" onclick={() => (drawerOpen = false)}></button>
    <div class="absolute inset-y-0 left-0 z-50 shadow-2xl" data-testid="sidebar-drawer">
      <AppSidebar {page} {legacyVoiceOnly} onNavigate={navigate} onOpenChat={openChat} />
    </div>
  {/if}

  <!-- 内容区：比侧边栏亮一档的底色，左上圆角（参照 DeepSeek Harness 的 Windows 外框） -->
  <main class="relative z-10 min-w-0 flex-1 overflow-y-auto border-t border-hairline bg-background [scrollbar-gutter:stable]
    {narrow ? '' : 'rounded-tl-2xl border-l'}" bind:this={mainEl}>
    {#if overlayHealth.degraded}
      <p role="status" data-testid="overlay-degraded-notice" class="mx-8 mt-5 rounded-lg border border-amber-300/20 bg-amber-300/8 px-4 py-3 text-xs leading-relaxed text-amber-100/90">
        悬浮窗显示异常，已自动关闭。语音输入仍可使用。
      </p>
    {/if}
    {#if loading}
      <p class="px-8 py-10 text-sm text-muted-foreground">加载配置中…</p>
    {:else if page === "general"}
      <GeneralPage {legacyVoiceOnly} onOpenAdvanced={() => openAdvanced("models")} onOpenCompanion={openChat} onOpenTalk={openCompanionSettings} />
    {:else if page === "advanced"}
      <AdvancedPage
        section={advancedSection}
        onSectionChange={(next) => (advancedSection = next)}
        onOpenOnboarding={openOnboarding}
      />
    {:else if page === "processing"}
      <TextProcessingPage onOpenConnections={() => openAdvanced("connections")} />
    {:else if page === "assistant"}
      <AssistantPage {chatRequest} open={assistantOpen} onOpenConnections={() => openAdvanced("connections")} onOpenAutomation={() => (page = "automation")} />
    {:else if page === "legacy-reminders"}
      <LegacyRemindersPage onOpenConnections={() => openAdvanced("connections")} />
    {:else if page === "automation"}
      <AutomationPage onOpenHotkeys={() => openAdvanced("hotkeys")} />
    {:else if page === "game"}
      <GamePage {legacyVoiceOnly} />
    {:else if page === "history"}
      <HistoryPage />
    {:else if page === "character"}
      <CharacterPage onBack={() => (page = "about")} />
    {:else}
      <AboutPage onOpenCharacter={() => (page = "character")} />
    {/if}

    <!-- 底部留白（提示在右上角，见 Sonner） -->
    {#if page !== "assistant"}<div class="h-4"></div>{/if}
  </main>

  <!-- 首启向导覆盖层（加载完成且未完成向导时弹出） -->
  {#if showUpgradeNotice && !loading}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4">
      <div role="dialog" aria-modal="true" aria-labelledby="legacy-upgrade-title"
        class="w-full max-w-md rounded-xl border border-hairline bg-card p-6 shadow-xl"
        data-testid="legacy-upgrade-notice">
        <h2 id="legacy-upgrade-title" class="m-0 text-lg font-semibold">语音输入法的用法没有变</h2>
        <p class="mt-3 text-sm text-muted-foreground">你可以继续用原来的热键和识别设置。琴音是可选的游戏陪玩，可以听你说话、回答问题，也可以在你允许时帮你操作游戏。</p>
        <div class="mt-5 flex flex-wrap gap-2">
          <Button disabled={savingUpgradeChoice} onclick={() => void keepVoiceInput()}>继续只用语音输入法</Button>
          <Button variant="outline" disabled={savingUpgradeChoice} onclick={() => { showUpgradeNotice = false; showLegacySetup = true; }}>认识琴音</Button>
        </div>
      </div>
    </div>
  {:else if showLegacySetup && !loading}
    <Onboarding legacyUpgrade onCancel={() => { showLegacySetup = false; showUpgradeNotice = true; }}
      onDone={(next, prompt) => { showLegacySetup = false; if (next === "assistant") openChat(prompt); }} />
  {:else if showOnboarding && !loading}
    <Onboarding onDone={(next, prompt) => { showOnboarding = false; if (next === "assistant") openChat(prompt); }} />
  {/if}

  <!-- 管理员提权提示 · 第一段（Windows 未提权且用户未表态时弹一次） -->
  {#if showElevationPrompt && !loading && !showUpgradeNotice && !showLegacySetup}
    <ElevationPrompt onClose={() => (showElevationPrompt = false)} />
  {/if}

  <!-- 管理员提权提示 · 第二段（第一段的重启成功后弹一次） -->
  {#if showAutoAdminPrompt && !loading && !showUpgradeNotice && !showLegacySetup}
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
