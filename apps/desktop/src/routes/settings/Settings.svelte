<script lang="ts">
  /*
   * 设置窗口视图（index.html#/settings）——方向 B「中继站」：
   * 左侧导航（头像 + 六项）+ 右侧页面。页面组件在 ./pages/ 下，
   * 共享 settingsStore / toast（lib/stores/ui.ts）；IPC 全部走 lib/ipc.ts。
   */
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    getSettings,
    getSettingsLoadWarning,
    getStartupOptions,
    getElevationStatus,
    isTauri,
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
  import { checkForUpdates } from "../../lib/updater";
  import Toasts from "../../lib/components/Toasts.svelte";
  import ElevationPrompt from "../../lib/components/ElevationPrompt.svelte";
  import AutoAdminPrompt from "../../lib/components/AutoAdminPrompt.svelte";
  import InjectionBlockedDialog from "../../lib/components/InjectionBlockedDialog.svelte";
  import { appState } from "../../lib/stores/state";
  import Onboarding from "./Onboarding.svelte";
  import Titlebar from "./Titlebar.svelte";
  import GeneralPage from "./pages/GeneralPage.svelte";
  import AdvancedPage from "./pages/AdvancedPage.svelte";
  import GamePage from "./pages/GamePage.svelte";
  import HistoryPage from "./pages/HistoryPage.svelte";
  import AboutPage from "./pages/AboutPage.svelte";
  import CharacterPage from "./pages/CharacterPage.svelte";
  import patternSwitch from "../../assets/brand/patterns/switch.webp";

  type PageId = "general" | "game" | "history" | "advanced" | "about" | "character";

  const navItems: { id: PageId; label: string; icon: string }[] = [
    { id: "general", label: "通用", icon: "home" },
    { id: "game", label: "游戏适配", icon: "gamepad" },
    { id: "history", label: "历史记录", icon: "clock" },
    { id: "advanced", label: "高级", icon: "sliders" },
    { id: "about", label: "关于", icon: "info" },
  ];

  let page = $state<PageId>("general");
  /** 右侧滚动容器：切页时复位到顶部（角色详情页是长滚动页） */
  let mainEl = $state<HTMLElement>();
  $effect(() => {
    page;
    mainEl?.scrollTo({ top: 0 });
  });
  let loading = $state(true);
  /** 首启向导：ui.firstRunCompleted === false 时弹出（完成/跳过后置 true） */
  let showOnboarding = $state(false);
  /** 管理员提权提示 · 第一段（仅 Windows 未提权且用户未表态时弹一次） */
  let showElevationPrompt = $state(false);
  /** 管理员提权提示 · 第二段（第一段「以管理员权限重启」成功后弹一次） */
  let showAutoAdminPrompt = $state(false);
  /** 模拟输入被系统拦截的排查引导弹窗（SendInput 0/N 时弹出；同一错误只弹一次） */
  let showBlockedDialog = $state(false);
  let blockedDismissedFor = $state<string | null>(null);

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
        getSettings(),
        getStartupOptions(),
        getSettingsLoadWarning(),
      ]);
      settingsStore.set(s);
      if (settingsWarning) toastWarn(settingsWarning);
      showOnboarding =
        startup.onboarding === "always" ||
        (startup.onboarding === "auto" && !s.ui.firstRunCompleted);
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

  // 单实例转发：应用已在托盘运行时，再执行
  // `kotone.exe --onboarding=always` 会唤起现有窗口并重新打开向导。
  onMount(() => {
    if (!isTauri) return;
    // 启动后静默检测一次更新（离线/无 release 不打扰用户）
    const updateTimer = setTimeout(() => void checkForUpdates(), 3000);
    let unlisten: (() => void) | undefined;
    void listen("kotone://open-onboarding", () => (showOnboarding = true)).then(
      (un) => (unlisten = un),
    );
    return () => {
      clearTimeout(updateTimer);
      unlisten?.();
    };
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

<div class="flex h-full flex-col overflow-hidden bg-kotone-deep text-white">
  <!-- 自绘标题栏（decorations:false）：拖拽区 + 运行状态 + 启动/停止 + 窗口控制 -->
  <Titlebar onOpenAdvanced={() => (page = "advanced")} />
  <div class="relative flex min-h-0 flex-1 overflow-hidden">
  <!-- 窗口底纹理：switch 无缝 tile，极低透明度 -->
  <div
    class="pointer-events-none absolute inset-0 opacity-[0.04]"
    style:background-image="url({patternSwitch})"
    style:background-size="256px"
  ></div>

  <!-- 左侧导航（品牌位移交顶部操作面板，导航直接从「通用」开始） -->
  <nav class="relative z-10 flex w-50 shrink-0 flex-col border-r border-white/8 bg-kotone-deep/60 px-3 py-4">
    <div class="flex flex-col gap-1">
      {#each navItems as item}
        {@const active = page === item.id || (page === "character" && item.id === "about")}
        <button
          class="group relative flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-sm transition
            {active
            ? 'bg-kotone-cyan/12 text-kotone-cyan shadow-glow-cyan'
            : 'text-white/60 hover:bg-white/8 hover:text-white/90 hover:shadow-[0_0_14px_rgba(0,229,255,0.15)]'}"
          onclick={() => (page = item.id)}
        >
          <!-- 激活指示条 -->
          <span
            class="absolute top-1/2 left-0 h-5 w-[3px] -translate-y-1/2 rounded-full bg-kotone-cyan transition-opacity
              {active ? 'opacity-100 shadow-glow-cyan' : 'opacity-0'}"
          ></span>
          <span class="inline-flex h-4.5 w-4.5 items-center justify-center" aria-hidden="true">
            {#if item.icon === "home"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M3 10.5 12 3l9 7.5M5 9.5V21h14V9.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
            {:else if item.icon === "sliders"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M4 7h10M18 7h2M4 17h4M12 17h8" stroke-linecap="round"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/></svg>
            {:else if item.icon === "gamepad"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><path d="M6 9h4M8 7v4M15 8h.01M18 10h.01M17.3 5H6.7a4.7 4.7 0 0 0-4.6 5.6l1 5.4A3 3 0 0 0 6 18.6c.8 0 1.6-.3 2.1-.9L9.5 16h5l1.4 1.7c.5.6 1.3.9 2.1.9a3 3 0 0 0 2.9-2.6l1-5.4A4.7 4.7 0 0 0 17.3 5Z" stroke-linecap="round" stroke-linejoin="round"/></svg>
            {:else if item.icon === "clock"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            {:else}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01" stroke-linecap="round"/></svg>
            {/if}
          </span>
          {item.label}
        </button>
      {/each}
    </div>

    <p class="mt-auto px-2 pt-4 text-[10px] leading-relaxed text-white/30">
       反馈群：1092354484
    </p>
  </nav>

  <!-- 右侧内容 -->
  <main class="relative z-10 min-w-0 flex-1 overflow-y-auto" bind:this={mainEl}>
    {#if loading}
      <p class="px-8 py-10 text-sm text-white/50">加载配置中…</p>
    {:else if page === "general"}
      <GeneralPage onOpenAdvanced={() => (page = "advanced")} />
    {:else if page === "advanced"}
      <AdvancedPage onOpenOnboarding={() => (showOnboarding = true)} />
    {:else if page === "game"}
      <GamePage />
    {:else if page === "history"}
      <HistoryPage />
    {:else if page === "character"}
      <CharacterPage onBack={() => (page = "about")} />
    {:else}
      <AboutPage onOpenCharacter={() => (page = "character")} />
    {/if}

    <!-- 底部留白（反馈条已升级为右上角 toast，见 <Toasts />） -->
    <div class="h-4"></div>
  </main>

  <!-- 首启向导覆盖层（加载完成且未完成向导时弹出） -->
  {#if showOnboarding && !loading}
    <Onboarding onDone={() => (showOnboarding = false)} />
  {/if}

  <!-- 管理员提权提示 · 第一段（Windows 未提权且用户未表态时弹一次） -->
  {#if showElevationPrompt && !loading}
    <ElevationPrompt onClose={() => (showElevationPrompt = false)} />
  {/if}

  <!-- 管理员提权提示 · 第二段（第一段的重启成功后弹一次） -->
  {#if showAutoAdminPrompt && !loading}
    <AutoAdminPrompt onClose={() => (showAutoAdminPrompt = false)} />
  {/if}

  <!-- 模拟输入被系统拦截的排查引导（注入 0/N 落地时弹出，同一错误不重复打扰） -->
  {#if showBlockedDialog && !loading}
    <InjectionBlockedDialog onClose={closeBlockedDialog} />
  {/if}
  </div>

  <!-- 右上角 toast 堆叠（fixed 定位，z 高于向导） -->
  <Toasts />
</div>
