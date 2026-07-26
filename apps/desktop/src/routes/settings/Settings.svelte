<script lang="ts">
  /*
   * 设置窗口视图（index.html#/settings）——方向 B「中继站」：
   * 左侧导航（头像 + 六项）+ 右侧页面。页面组件在 ./pages/ 下，
   * 共享 settingsStore / toast（lib/stores/ui.ts）；IPC 全部走 lib/ipc.ts。
   */
  import { onMount } from "svelte";
  import { getSettings } from "../../lib/ipc";
  import { settingsStore, feedback, toast, errText } from "../../lib/stores/ui";
  import { initRuntime } from "../../lib/stores/runtime";
  import Onboarding from "./Onboarding.svelte";
  import GeneralPage from "./pages/GeneralPage.svelte";
  import HotkeyPage from "./pages/HotkeyPage.svelte";
  import EnginePage from "./pages/EnginePage.svelte";
  import GamePage from "./pages/GamePage.svelte";
  import HistoryPage from "./pages/HistoryPage.svelte";
  import AboutPage from "./pages/AboutPage.svelte";
  import iconSrc from "../../assets/brand/icon-src.png";
  import patternSwitch from "../../assets/brand/patterns/switch.png";

  type PageId = "general" | "hotkey" | "engine" | "game" | "history" | "about";

  const navItems: { id: PageId; label: string; icon: string }[] = [
    { id: "general", label: "通用", icon: "home" },
    { id: "hotkey", label: "快捷键", icon: "keyboard" },
    { id: "engine", label: "引擎与模型", icon: "chip" },
    { id: "game", label: "游戏适配", icon: "gamepad" },
    { id: "history", label: "历史记录", icon: "clock" },
    { id: "about", label: "关于", icon: "info" },
  ];

  let page = $state<PageId>("general");
  let loading = $state(true);
  /** 首启向导：ui.firstRunCompleted === false 时弹出（完成/跳过后置 true） */
  let showOnboarding = $state(false);

  onMount(async () => {
    // runtime store 独立初始化（kotone://runtime 事件订阅 + 初始拉取）
    void initRuntime();
    try {
      const s = await getSettings();
      settingsStore.set(s);
      showOnboarding = !s.ui.firstRunCompleted;
    } catch (e) {
      toast(false, `加载配置失败：${errText(e)}`);
    } finally {
      loading = false;
    }
  });
</script>

<div class="relative flex h-full overflow-hidden bg-kotone-deep text-white">
  <!-- 窗口底纹理：switch 无缝 tile，极低透明度 -->
  <div
    class="pointer-events-none absolute inset-0 opacity-[0.04]"
    style:background-image="url({patternSwitch})"
    style:background-size="256px"
  ></div>

  <!-- 左侧导航 -->
  <nav class="relative z-10 flex w-50 shrink-0 flex-col border-r border-white/8 bg-kotone-deep/60 px-3 py-5">
    <div class="flex flex-col items-center gap-2 pb-5">
      <img
        src={iconSrc}
        alt="Kotone 头像"
        class="h-16 w-16 rounded-full ring-2 ring-kotone-cyan/70 shadow-glow-cyan object-cover"
      />
      <p class="text-sm font-bold tracking-wide">
        Kotone <span class="text-white/55 font-medium">琴音</span>
      </p>
    </div>

    <div class="flex flex-col gap-1">
      {#each navItems as item}
        {@const active = page === item.id}
        <button
          class="group relative flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-sm transition
            {active
            ? 'bg-kotone-cyan/12 text-kotone-cyan shadow-glow-cyan'
            : 'text-white/60 hover:bg-white/5 hover:text-white/90'}"
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
            {:else if item.icon === "keyboard"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M18 14h.01M9 14h6" stroke-linecap="round"/></svg>
            {:else if item.icon === "chip"}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4"><rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" stroke-linecap="round"/></svg>
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
      打字比打游戏快的主播<br />深夜直播间 · 随时待命
    </p>
  </nav>

  <!-- 右侧内容 -->
  <main class="relative z-10 min-w-0 flex-1 overflow-y-auto">
    {#if loading}
      <p class="px-8 py-10 text-sm text-white/50">加载配置中…</p>
    {:else if page === "general"}
      <GeneralPage />
    {:else if page === "hotkey"}
      <HotkeyPage />
    {:else if page === "engine"}
      <EnginePage />
    {:else if page === "game"}
      <GamePage />
    {:else if page === "history"}
      <HistoryPage />
    {:else}
      <AboutPage />
    {/if}

    <!-- 底部反馈条 -->
    <div class="h-10 px-8 pt-3">
      {#if $feedback}
        <p class="text-xs {$feedback.ok ? 'text-kotone-cyan' : 'text-kotone-pink'}">
          {$feedback.text}
        </p>
      {/if}
    </div>
  </main>

  <!-- 首启向导覆盖层（加载完成且未完成向导时弹出） -->
  {#if showOnboarding && !loading}
    <Onboarding onDone={() => (showOnboarding = false)} />
  {/if}
</div>
