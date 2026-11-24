<!-- 聊天页右侧栏（ADR 031，照 DeepSeek Harness 桌面版的右侧栏与 Codex 的内置浏览器）： -->
<!-- 顶部两个标签——“卡片”（网页卡片 + 后台在做的事，按宽度 1–3 列）和“网页”（浏览器占满）；左边缘拖动调宽、 -->
<!-- 双击恢复；网页可以全屏（铺满整个 Kotone 窗口）。网页由后端摆在窗口里（盖在界面上），这里只报那块区域的位置。 -->
<!-- 第三个标签“页面”（ADR 038）：聊天卡片点开的插件页面（或她用 ui.open 打开的），能往下点、返回、关掉。 -->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { tick } from "svelte";
  import {
    ArrowLeft, ArrowRight, CircleAlert, Expand, Globe, Info, LayoutGrid, LoaderCircle, Lock, Maximize2, Minimize2,
    PanelsTopLeft, Plus, RotateCw, Shrink, Square, Volume2, X,
  } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import {
    activeTabOf, browserControl, browserHandoverDone, browserNavigate, browserSetBounds, browserStore, browserTabClose,
    browserTabNew, browserTabSelect, mediaLine, siteName, tabLabel,
    type BrowserAction, type BrowserBounds,
  } from "../browser";
  import type { SubagentInfo } from "../agent";
  import { isTauri } from "../ipc";
  import {
    browserFullscreen, clampSidebarWidth, rightSidebarOpen, rightSidebarTab, rightSidebarWidth, saveRightSidebarWidth,
    setRightSidebarTab, showBrowserTab, SIDEBAR_DEFAULT_WIDTH, toggleRightSidebar,
  } from "../stores/rightSidebar";
  import { tryAgentChat } from "./context";
  import SidebarTask from "./SidebarTask.svelte";
  import PluginStack from "../components/plugins/PluginStack.svelte";
  import { viewOf } from "../pluginUi";
  import { backPluginPage, closePluginPages, pluginPageStack, pushPluginPage, replacePluginPage } from "../stores/pluginPages";

  let { active = true, narrow = false, chatWidth = 0 }: {
    /** 聊天页正显示着（不在角色 / 记忆分页上）。 */
    active?: boolean;
    /** 聊天区窄：右侧栏盖在聊天上。 */
    narrow?: boolean;
    /** 聊天页总宽（右侧栏最宽能拖到多宽）。 */
    chatWidth?: number;
  } = $props();

  const chat = tryAgentChat();
  const browser = $derived($browserStore);
  const open = $derived($rightSidebarOpen);
  const fullscreen = $derived($browserFullscreen);

  /** “网页”标签能不能用：打开过网页，或者要玩家来登录、出了错要说明。 */
  const webAvailable = $derived(browser.open || browser.tabs.length > 0 || !!browser.handover || !!browser.error);
  /** 当前标签页。 */
  const current = $derived(activeTabOf(browser));
  /** 最多开几个标签页（和后端一样）。 */
  const MAX_TABS = 8;
  /** 插件页面（ADR 038）：开着几页（聊天卡片点开、ui.open）。 */
  const pages = $derived($pluginPageStack);
  const pluginPageTitle = $derived(pages.length ? (viewOf(pages[pages.length - 1].uri)?.view.title() ?? "") : "");
  /** 选中的标签。“网页”一直能点：没有网页时也有地址栏，玩家可以自己输入网址或搜索。“页面”没开着页面时按“卡片”。 */
  const tab = $derived($rightSidebarTab === "page" && !pages.length ? "cards" : $rightSidebarTab);

  // ---- 后台在做的事：在跑的在前，再列最近做完的几件 ----
  const RECENT = 6;
  const tasks = $derived.by<SubagentInfo[]>(() => {
    const list = [...(chat?.conversation?.subagents ?? [])].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
    const live = list.filter((s) => s.status === "running" || s.status === "waiting" || s.status === "interrupted");
    const finished = list.filter((s) => !live.includes(s)).slice(0, RECENT);
    return [...live, ...finished];
  });
  const liveCount = $derived(tasks.filter((t) => t.status === "running" || t.status === "waiting").length);
  const conversationId = $derived(chat?.conversation?.id ?? "");
  let openTasks = $state<Record<string, boolean>>({});

  // ---- 要玩家来（登录、验证码）：打开右侧栏、切到“网页” ----
  let lastHandover: string | null = null;
  $effect(() => {
    const reason = browser.handover?.reason ?? null;
    if (reason && reason !== lastHandover) showBrowserTab();
    lastHandover = reason;
  });

  let acting = $state<BrowserAction | "done" | null>(null);
  let actionError = $state("");
  async function control(action: BrowserAction) {
    if (acting) return;
    acting = action;
    actionError = "";
    try { await browserControl(action); }
    catch (e) { actionError = e instanceof Error ? e.message : String(e); }
    finally { acting = null; }
  }
  async function handoverDone() {
    if (acting) return;
    acting = "done";
    actionError = "";
    try { await browserHandoverDone(); }
    catch (e) { actionError = e instanceof Error ? e.message : String(e); }
    finally { acting = null; }
  }

  /** 标签页和地址栏的操作出错时说一句（例如内网地址打不开）。 */
  async function run(action: () => Promise<void>) {
    actionError = "";
    try { await action(); }
    catch (e) { actionError = e instanceof Error ? e.message : String(e); }
  }

  // ---- 地址栏（照 Codex / DSH 的内置浏览器）：平时显示当前网址，点进去可以改，回车打开或搜索 ----
  let address = $state<HTMLInputElement>();
  let editing = $state(false);
  let draft = $state("");
  const addressValue = $derived(editing ? draft : (current?.url ?? ""));
  const secure = $derived(!!current?.url?.startsWith("https://"));
  function startEditing() {
    if (!editing) { draft = current?.url ?? ""; editing = true; }
    queueMicrotask(() => address?.select());
  }
  function submitAddress() {
    const value = draft.trim();
    editing = false;
    address?.blur();
    if (value) void run(() => browserNavigate(value));
  }
  function addressKey(event: KeyboardEvent) {
    if (event.key === "Escape") { editing = false; address?.blur(); }
  }
  // 换了标签页：地址栏回到显示网址
  $effect(() => { void browser.activeTab; editing = false; });
  async function newTab() {
    await run(() => browserTabNew());
    await tick();
    startEditing();
    address?.focus();
  }

  // ---- 后端的提示（例如下载被拒绝）：玩家可以关掉，新的提示再出现 ----
  let dismissedNotice = $state<string | null>(null);
  const notice = $derived(browser.notice && browser.notice !== dismissedNotice ? browser.notice : null);
  // 提示消失后再来同样的一句，也要重新显示
  $effect(() => { if (!browser.notice) dismissedNotice = null; });

  // ---- 宽度：左边缘拖动调宽（照 DSH 桌面版），双击恢复默认，键盘左右键也能调 ----
  const width = $derived(clampSidebarWidth($rightSidebarWidth, chatWidth));
  let dragging = $state(false);
  let dragStartX = 0;
  let dragStartWidth = 0;
  function startDrag(event: PointerEvent) {
    if (narrow || event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragging = true;
    dragStartX = event.clientX;
    dragStartWidth = width;
    document.body.style.setProperty("user-select", "none");
    document.body.style.setProperty("cursor", "col-resize");
  }
  function moveDrag(event: PointerEvent) {
    if (!dragging) return;
    // 分隔线在右侧栏左边：往左拖变宽
    rightSidebarWidth.set(clampSidebarWidth(dragStartWidth + (dragStartX - event.clientX), chatWidth));
    schedule();
  }
  function endDrag() {
    if (!dragging) return;
    dragging = false;
    document.body.style.removeProperty("user-select");
    document.body.style.removeProperty("cursor");
    saveRightSidebarWidth(width);
    schedule();
  }
  function resetWidth() {
    saveRightSidebarWidth(clampSidebarWidth(SIDEBAR_DEFAULT_WIDTH, chatWidth));
    schedule();
  }
  function resizeByKey(event: KeyboardEvent) {
    const step = event.shiftKey ? 64 : 16;
    if (event.key === "ArrowLeft") saveRightSidebarWidth(clampSidebarWidth(width + step, chatWidth));
    else if (event.key === "ArrowRight") saveRightSidebarWidth(clampSidebarWidth(width - step, chatWidth));
    else if (event.key === "Home") resetWidth();
    else return;
    event.preventDefault();
    schedule();
  }

  // ---- 卡片：按卡片区宽度排 1–3 列（照 DSH 卡片面板） ----
  const CARD_MIN = 240;
  const CARD_GAP = 12;
  let cardsWidth = $state(0);
  const columns = $derived(Math.max(1, Math.min(3, Math.floor((cardsWidth + CARD_GAP) / (CARD_MIN + CARD_GAP)))));

  // ---- 网页全屏：铺满整个 Kotone 窗口（窗口能全屏时也让窗口全屏） ----
  let osFullscreen = false;
  async function setWindowFullscreen(on: boolean) {
    if (!isTauri) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      if (on) {
        await getCurrentWindow().setFullscreen(true);
        osFullscreen = true;
      } else if (osFullscreen) {
        osFullscreen = false;
        await getCurrentWindow().setFullscreen(false);
      }
    } catch {
      // 窗口不让全屏（权限没开）：只在窗口里铺满
      osFullscreen = false;
    }
  }
  function enterFullscreen() {
    browserFullscreen.set(true);
    void setWindowFullscreen(true);
  }
  /** `byPlayer`：玩家点了退出 / 按 Esc——页面还要全屏也先不再自动进去，等页面自己退出全屏。 */
  function exitFullscreen(byPlayer = false) {
    if (byPlayer && current?.fullscreenElement) suppressElement = true;
    byElement = false;
    browserFullscreen.set(false);
    void setWindowFullscreen(false);
  }
  // 页面里的元素要全屏（例如视频的全屏按钮）：自动进入浏览器全屏；页面退出全屏时跟着退出
  let byElement = false;
  let suppressElement = false;
  $effect(() => {
    const want = !!current?.fullscreenElement && active;
    if (!want) {
      suppressElement = false;
      if (byElement && $browserFullscreen) exitFullscreen();
      byElement = false;
      return;
    }
    if (!$browserFullscreen && !suppressElement) {
      byElement = true;
      enterFullscreen();
    }
  });
  // 离开聊天页、网页没了：退出全屏
  $effect(() => {
    if (fullscreen && (!active || !webAvailable)) exitFullscreen();
  });
  $effect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); exitFullscreen(true); } };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  /** 全屏层挂到 body 上，免得被祖先的布局（裁剪、层叠）困住。 */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  // ---- 把网页区域的位置报给后端（网页摆在那里） ----
  let slot = $state<HTMLDivElement>();
  let fullSlot = $state<HTMLDivElement>();
  /** 有对话框、引导、抽屉盖在界面上：网页先藏起来，不然会盖住它们。 */
  let blocked = $state(false);
  const wantVisible = $derived(active && webAvailable && (fullscreen || (open && tab === "web")));
  let last = "";
  let lastRect: Omit<BrowserBounds, "visible"> = { x: 0, y: 0, width: 0, height: 0 };
  let frame = 0;

  function measure(): BrowserBounds {
    const target = fullscreen ? fullSlot : slot;
    const show = wantVisible && !blocked && !!target && document.visibilityState !== "hidden";
    if (show && target) {
      const r = target.getBoundingClientRect();
      lastRect = { x: Math.round(r.left), y: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) };
      return { ...lastRect, visible: lastRect.width >= 40 && lastRect.height >= 40 };
    }
    return { ...lastRect, visible: false };
  }
  function report(force = false) {
    frame = 0;
    const bounds = measure();
    const key = JSON.stringify(bounds);
    if (!force && key === last) return;
    last = key;
    void browserSetBounds(bounds).catch(() => {});
  }
  /** 一帧最多报一次（拖动调宽时也是这样节流）。 */
  function schedule() {
    if (!frame) frame = requestAnimationFrame(() => report());
  }

  // 依赖变化（换标签、全屏、离开聊天页、对话框、宽度）时重新报
  $effect(() => {
    void wantVisible;
    void blocked;
    void slot;
    void fullSlot;
    void fullscreen;
    void width;
    schedule();
  });

  $effect(() => {
    const targets = [slot, fullSlot].filter((t): t is HTMLDivElement => !!t);
    if (!targets.length) return;
    const observer = new ResizeObserver(schedule);
    for (const t of targets) observer.observe(t);
    observer.observe(document.documentElement);
    return () => observer.disconnect();
  });

  $effect(() => {
    const check = () => {
      const covered = !!document.querySelector('[role="dialog"], [role="alertdialog"], [data-testid="sidebar-drawer"], [data-testid="onboarding"]');
      if (covered !== blocked) blocked = covered;
    };
    check();
    let pending = 0;
    const observer = new MutationObserver(() => {
      if (!pending) pending = requestAnimationFrame(() => { pending = 0; check(); });
    });
    observer.observe(document.body, { childList: true, subtree: true });
    const onResize = () => schedule();
    const onVisibility = () => schedule();
    window.addEventListener("resize", onResize);
    document.addEventListener("visibilitychange", onVisibility);
    // 兜底：位置可能在没有尺寸变化时挪动（左侧栏收起等），显示网页时每秒核对一次
    const timer = setInterval(() => { if (wantVisible) schedule(); }, 1_000);
    return () => {
      observer.disconnect();
      if (pending) cancelAnimationFrame(pending);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", onVisibility);
      clearInterval(timer);
    };
  });

  onDestroy(() => {
    if (frame) cancelAnimationFrame(frame);
    if (dragging) endDrag();
    if ($browserFullscreen) exitFullscreen();
    last = "";
    void browserSetBounds({ ...lastRect, visible: false }).catch(() => {});
  });
</script>

{#snippet browserError()}
  {#if browser.error}
    <div class="flex min-w-0 items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2" role="alert" data-testid="browser-error">
      <CircleAlert class="mt-0.5 size-3.5 shrink-0 text-destructive" />
      <p class="m-0 min-w-0 flex-1 text-xs leading-5 break-words text-destructive" data-testid="browser-error-text">{browser.error}</p>
      <Button size="sm" variant="outline" class="h-6 shrink-0 px-2 text-[11px]" data-testid="browser-error-retry"
        disabled={!!acting} onclick={() => void control("reload")}>{m.chat_browser_retry()}</Button>
    </div>
  {/if}
{/snippet}

{#snippet handoverBanner()}
  {#if browser.handover}
    <div class="shrink-0 border-b border-warning/40 bg-warning/8 px-3 py-2.5" role="status" data-testid="browser-handover">
      <p class="m-0 text-[13px] leading-5 text-foreground"><span class="font-medium">{browser.handover.reason}</span>{m.chat_browser_handover_hint()}</p>
      <Button size="sm" class="mt-2 h-7" data-testid="browser-handover-done" disabled={!!acting} onclick={() => void handoverDone()}>{m.chat_browser_handover_done()}</Button>
    </div>
  {/if}
{/snippet}

{#snippet pageTitle(testid: string)}
  {#if browser.loading}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-muted-foreground" />{:else}<Globe class="size-3.5 shrink-0 text-muted-foreground" />{/if}
  <div class="flex min-w-0 flex-1 items-baseline gap-2 px-1">
    <p class="m-0 shrink-0 truncate text-[13px] font-medium text-foreground" data-testid={testid}>{siteName(browser)}</p>
    {#if browser.title}<p class="m-0 min-w-0 truncate text-xs text-muted-foreground" title={browser.title}>{browser.title}</p>{/if}
  </div>
  {#if browser.busy}<span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" data-testid="browser-panel-busy">{m.chat_browser_busy()}</span>{/if}
{/snippet}

{#if open}
  {#if narrow}
    <!-- 窄窗口：盖在聊天上，点空白处收起 -->
    <button type="button" class="absolute inset-0 z-20 bg-black/30" aria-label={m.chat_sidebar_collapse()} data-testid="right-sidebar-backdrop" onclick={toggleRightSidebar}></button>
  {/if}
  <aside
    class="flex min-h-0 flex-col border-l border-hairline bg-background {narrow ? 'absolute inset-y-0 right-0 z-30 shadow-2xl' : 'relative shrink-0'}"
    style:width={narrow ? (tab === "web" || tab === "page" ? "100%" : "min(100%, 360px)") : `${width}px`}
    aria-label={m.chat_sidebar_label()} data-testid="right-sidebar" data-tab={tab} data-width={narrow ? undefined : width}
  >
    {#if !narrow}
      <!-- 分隔线：拖动调宽、双击恢复默认（照 DSH 桌面版）；网页只盖它右边的区域，不会挡住这里。
           可聚焦的 separator 是 ARIA 的“窗口分隔条”（键盘左右键调宽），Svelte 的检查不认识这种用法。 -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div role="separator" aria-orientation="vertical" aria-label={m.chat_sidebar_resize_label()} tabindex="0"
        aria-valuenow={width} aria-valuemin={300} title={m.chat_sidebar_resize_title()}
        class="group absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none focus-visible:outline-none"
        data-testid="right-sidebar-resize" data-dragging={dragging ? "true" : "false"}
        onpointerdown={startDrag} onpointermove={moveDrag} onpointerup={endDrag} onpointercancel={endDrag} onlostpointercapture={endDrag}
        ondblclick={resetWidth} onkeydown={resizeByKey}>
        <div class="mx-auto h-full w-px transition-colors {dragging ? 'bg-primary' : 'bg-transparent group-hover:bg-primary/60 group-focus-visible:bg-primary'}"></div>
      </div>
    {/if}

    <!-- 标签：卡片 / 网页 -->
    <div class="flex h-10 shrink-0 items-center gap-1 border-b border-hairline pr-1.5 pl-2" role="tablist" aria-label={m.chat_sidebar_label()}>
      <button type="button" role="tab" aria-selected={tab === "cards"} data-testid="sidebar-tab-cards"
        class="flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors {tab === 'cards' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => setRightSidebarTab("cards")}>
        <LayoutGrid class="size-3.5" />{m.chat_sidebar_tab_cards()}
        {#if liveCount}<span class="rounded-full bg-primary/15 px-1.5 text-[10px] leading-4 text-primary" data-testid="sidebar-tab-cards-count">{liveCount}</span>{/if}
      </button>
      <button type="button" role="tab" aria-selected={tab === "web"} data-testid="sidebar-tab-web"
        title={browser.open ? siteName(browser) : m.chat_sidebar_tab_web_title()}
        class="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors {tab === 'web' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => setRightSidebarTab("web")}>
        <Globe class="size-3.5 shrink-0" /><span class="truncate">{m.chat_sidebar_tab_web()}</span>
        {#if browser.tabs.length > 1}<span class="rounded-full bg-muted px-1.5 text-[10px] leading-4 text-muted-foreground" data-testid="sidebar-tab-web-count">{browser.tabs.length}</span>{/if}
        {#if browser.handover}<span class="size-1.5 shrink-0 rounded-full bg-warning" aria-label={m.chat_sidebar_tab_web_handover()}></span>{/if}
      </button>
      {#if pages.length}
        <button type="button" role="tab" aria-selected={tab === "page"} data-testid="sidebar-tab-page" title={pluginPageTitle}
          class="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors {tab === 'page' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => setRightSidebarTab("page")}>
          <PanelsTopLeft class="size-3.5 shrink-0" /><span class="max-w-[9rem] truncate">{pluginPageTitle || m.chat_sidebar_tab_page()}</span>
        </button>
      {/if}
      <div class="flex-1"></div>
      <Button size="icon" variant="ghost" class="size-7" aria-label={m.chat_sidebar_collapse()} title={m.chat_sidebar_collapse()} data-testid="right-sidebar-close"
        onclick={toggleRightSidebar}><X class="size-3.5" /></Button>
    </div>

    {#if tab === "page"}
      <!-- 插件页面（ADR 038）：最上面那一页，能返回、关掉 -->
      <PluginStack {pages} onPush={pushPluginPage} onBack={backPluginPage} onReplace={replacePluginPage} onClose={closePluginPages} testid="sidebar-plugin-page" />
    {:else if tab === "web"}
      <!-- 网页：浏览器占满这个标签（照 Codex 内置浏览器）：一排标签页，一行地址栏（后退、前进、刷新、网址、播放控制、全屏、回到卡片） -->
      <div class="flex min-h-0 flex-1 flex-col" data-testid="browser-panel">
        <div class="flex h-9 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-hairline bg-muted/40 px-1.5 pt-1 [scrollbar-width:none]"
          role="tablist" aria-label={m.chat_browser_tabs_label()} data-testid="browser-tabs">
          {#each browser.tabs as item (item.id)}
            {@const selected = item.id === browser.activeTab}
            <div class="group flex h-8 max-w-[200px] min-w-[84px] flex-1 items-center gap-1 rounded-t-lg pr-1 pl-2 text-xs transition-colors
              {selected ? 'border border-b-0 border-hairline bg-background text-foreground' : 'text-muted-foreground hover:bg-background/60 hover:text-foreground'}"
              data-testid="browser-tab" data-tab-id={item.id} data-active={selected ? "true" : "false"}>
              <button type="button" role="tab" aria-selected={selected} title={item.url ?? tabLabel(item)}
                class="flex min-w-0 flex-1 items-center gap-1.5 text-left focus-visible:outline-none"
                data-testid="browser-tab-select" onclick={() => { if (!selected) void run(() => browserTabSelect(item.id)); }}>
                {#if item.loading}<LoaderCircle class="size-3 shrink-0 animate-spin" />
                {:else if item.site}<span class="flex size-3.5 shrink-0 items-center justify-center rounded-sm bg-muted text-[9px] font-semibold uppercase text-muted-foreground" aria-hidden="true">{siteName(item).slice(0, 1)}</span>
                {:else}<Globe class="size-3 shrink-0" />{/if}
                <span class="min-w-0 flex-1 truncate" data-testid="browser-tab-title">{tabLabel(item)}</span>
              </button>
              {#if item.agent}<span class="shrink-0 rounded bg-primary/15 px-1 text-[10px] leading-4 text-primary" title={m.chat_browser_tab_agent_title()} data-testid="browser-tab-agent">{m.chat_browser_tab_agent()}</span>{/if}
              {#if item.audible}<Volume2 class="size-3 shrink-0 text-primary" aria-label={m.chat_browser_tab_audible()} data-testid="browser-tab-audible" />{/if}
              <button type="button" aria-label={m.chat_browser_tab_close_label({ title: tabLabel(item) })} title={m.chat_browser_tab_close()} data-testid="browser-tab-close"
                class="flex size-5 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground
                  {selected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}"
                onclick={() => void run(() => browserTabClose(item.id))}><X class="size-3" /></button>
            </div>
          {/each}
          <Button size="icon" variant="ghost" class="mb-0.5 size-7 shrink-0" aria-label={m.chat_browser_tab_new()} title={browser.tabs.length >= MAX_TABS ? m.chat_browser_tab_max({ count: MAX_TABS }) : m.chat_browser_tab_new()}
            data-testid="browser-tab-new" disabled={browser.tabs.length >= MAX_TABS} onclick={() => void newTab()}><Plus class="size-3.5" /></Button>
        </div>
        <form class="flex h-10 shrink-0 items-center gap-0.5 border-b border-hairline px-1.5" data-testid="browser-address-bar"
          onsubmit={(event) => { event.preventDefault(); submitAddress(); }}>
          <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_back()} title={m.chat_browser_back()} data-testid="browser-panel-back"
            disabled={!current?.canGoBack || !!acting} onclick={() => void control("back")}><ArrowLeft class="size-3.5" /></Button>
          <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_forward()} title={m.chat_browser_forward()} data-testid="browser-panel-forward"
            disabled={!current?.canGoForward || !!acting} onclick={() => void control("forward")}><ArrowRight class="size-3.5" /></Button>
          {#if current?.loading}
            <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_stop()} title={m.chat_browser_stop()} data-testid="browser-panel-stop"
              onclick={() => void control("stop")}><Square class="size-3" /></Button>
          {:else}
            <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_reload()} title={m.chat_browser_reload()} data-testid="browser-panel-reload"
              disabled={!current || !!acting} onclick={() => void control("reload")}><RotateCw class="size-3.5" /></Button>
          {/if}
          <div class="mx-1 flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-full bg-muted/60 px-2.5 focus-within:ring-1 focus-within:ring-primary/50">
            {#if secure && !editing}<Lock class="size-3 shrink-0 text-muted-foreground" aria-label={m.chat_browser_secure()} />{:else}<Globe class="size-3 shrink-0 text-muted-foreground" />{/if}
            <input bind:this={address} value={addressValue} placeholder={m.chat_browser_address_placeholder()} spellcheck="false" autocomplete="off"
              aria-label={m.chat_browser_address_label()} data-testid="browser-address"
              class="min-w-0 flex-1 bg-transparent text-xs text-foreground outline-none placeholder:text-muted-foreground"
              onfocus={startEditing} oninput={(e) => { draft = (e.currentTarget as HTMLInputElement).value; editing = true; }}
              onkeydown={addressKey} onblur={() => setTimeout(() => { editing = false; }, 0)} />
          </div>
          {#if browser.busy}<span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" data-testid="browser-panel-busy">{m.chat_browser_busy()}</span>{/if}
          <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_fullscreen()} title={m.chat_browser_fullscreen_title()} data-testid="browser-panel-fullscreen"
            disabled={!current} onclick={enterFullscreen}><Expand class="size-3.5" /></Button>
          <Button type="button" size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_back_to_cards()} title={m.chat_browser_back_to_cards()} data-testid="browser-panel-collapse"
            onclick={() => setRightSidebarTab("cards")}><Minimize2 class="size-3.5" /></Button>
        </form>
        {@render handoverBanner()}
        {#if browser.error}<div class="shrink-0 border-b border-hairline px-3 py-2">{@render browserError()}</div>{/if}
        {#if notice}
          <div class="flex shrink-0 items-center gap-2 border-b border-hairline bg-muted/40 px-3 py-1.5" role="status" data-testid="browser-notice">
            <Info class="size-3.5 shrink-0 text-muted-foreground" />
            <p class="m-0 min-w-0 flex-1 text-xs leading-5 text-foreground/85" data-testid="browser-notice-text">{notice}</p>
            <Button size="icon" variant="ghost" class="size-6 shrink-0" aria-label={m.chat_browser_notice_dismiss()} title={m.chat_browser_notice_dismiss()} data-testid="browser-notice-dismiss"
              onclick={() => { dismissedNotice = browser.notice; }}><X class="size-3" /></Button>
          </div>
        {/if}
        {#if actionError}<p role="alert" class="m-0 shrink-0 px-3 py-1.5 text-xs text-destructive" data-testid="browser-action-error">{actionError}</p>{/if}
        <!-- 网页摆在这里（盖在上面）；没打开网页时显示说明 -->
        <div class="relative min-h-0 flex-1 bg-muted/30" bind:this={slot} data-testid="browser-panel-slot">
          {#if !browser.open}
            <p class="absolute inset-x-6 top-1/3 m-0 text-center text-xs leading-5 text-muted-foreground" data-testid="browser-panel-empty">
              {m.chat_browser_empty()}
            </p>
          {/if}
        </div>
      </div>
    {:else}
      <!-- 卡片：网页卡片 + 后台在做的事，按宽度 1–3 列；点开的任务占满一行 -->
      <div class="min-h-0 flex-1 overflow-y-auto px-3 py-3" bind:clientWidth={cardsWidth}>
        <ul class="m-0 grid list-none gap-3 p-0" style:grid-template-columns={`repeat(${columns}, minmax(0, 1fr))`}
          aria-label={m.chat_sidebar_cards_label()} data-testid="sidebar-cards" data-columns={columns}>
          <li class="min-w-0">
            <section aria-label={m.chat_sidebar_tab_web()} class="h-full min-w-0 overflow-hidden rounded-xl border border-hairline bg-card/60" data-testid="sidebar-browser-card"
              data-browser-state={browser.handover ? "handover" : browser.error ? "error" : browser.open ? "open" : "closed"}>
              {#if browser.open}
                <div class="flex min-w-0 items-center gap-2 px-3 pt-2.5">
                  {#if browser.loading}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-muted-foreground" />{:else}<Globe class="size-3.5 shrink-0 text-muted-foreground" />{/if}
                  <span class="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground" data-testid="sidebar-browser-site">{siteName(browser)}</span>
                  {#if browser.busy}<span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" data-testid="sidebar-browser-busy">{m.chat_browser_busy()}</span>{/if}
                </div>
                {#if browser.title}<p class="m-0 truncate px-3 pt-0.5 pl-8.5 text-xs text-muted-foreground" title={browser.title} data-testid="sidebar-browser-title">{browser.title}</p>{/if}
                {#if browser.tabs.length > 1}<p class="m-0 px-3 pt-0.5 pl-8.5 text-[11px] text-muted-foreground" data-testid="sidebar-browser-tabs">{m.chat_sidebar_browser_tab_count({ count: browser.tabs.length })}</p>{/if}
                {#if browser.media}
                  <!-- 网页里在放什么（只读；放歌在音乐页，ADR 034） -->
                  <div class="mx-3 mt-2 flex min-w-0 items-center gap-2 rounded-lg bg-muted/50 px-2.5 py-1.5">
                    <p class="m-0 min-w-0 flex-1 truncate text-xs text-foreground/90" data-testid="sidebar-browser-media">{mediaLine(browser.media)}</p>
                  </div>
                {/if}
                {#if browser.handover}
                  <p class="m-0 mx-3 mt-2 rounded-lg bg-warning/10 px-2.5 py-1.5 text-xs text-warning" data-testid="sidebar-browser-handover">{m.chat_sidebar_browser_handover({ reason: browser.handover.reason })}</p>
                {/if}
                {#if browser.error}<div class="mx-3 mt-2">{@render browserError()}</div>{/if}
                <div class="flex justify-end px-1.5 py-1.5">
                  <Button size="sm" variant="ghost" class="h-7 gap-1 px-2 text-xs" data-testid="sidebar-browser-expand"
                    onclick={showBrowserTab}><Maximize2 class="size-3" />{m.chat_sidebar_browser_expand()}</Button>
                </div>
              {:else if browser.error}
                <!-- 浏览器没能打开（例如 WebView2 建不出来）：说清原因，给一个重试 -->
                <div class="p-3">{@render browserError()}</div>
              {:else}
                <div class="flex items-start gap-2 px-3 py-3">
                  <Globe class="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                  <p class="m-0 text-xs leading-5 text-muted-foreground" data-testid="sidebar-browser-hint">{m.chat_sidebar_browser_hint()}</p>
                </div>
              {/if}
            </section>
          </li>
          {#each tasks as info (info.name)}<SidebarTask {info} {conversationId} bind:open={openTasks[info.name]} />{/each}
          {#if !tasks.length}
            <li class="min-w-0 rounded-xl border border-dashed border-hairline px-3 py-3" data-testid="sidebar-tasks">
              <p class="m-0 text-xs leading-5 text-muted-foreground" data-testid="sidebar-tasks-empty">{m.chat_sidebar_tasks_empty()}</p>
            </li>
          {/if}
        </ul>
        {#if actionError}<p role="alert" class="mt-2 mb-0 text-xs text-destructive">{actionError}</p>{/if}
      </div>
    {/if}
  </aside>
{/if}

{#if fullscreen && active && webAvailable}
  <!-- 网页全屏：铺满整个 Kotone 窗口，只留一条很窄的顶栏（照 Codex 内置浏览器的全屏） -->
  <div use:portal class="ui-scope fixed inset-0 z-[100] flex flex-col bg-background text-foreground" role="region" aria-label={m.chat_browser_fullscreen_region()} data-testid="browser-fullscreen">
    <div class="flex h-9 shrink-0 items-center gap-1 border-b border-hairline px-2">
      {@render pageTitle("browser-fullscreen-site")}
      <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_back()} title={m.chat_browser_back()}
        disabled={!current?.canGoBack || !!acting} onclick={() => void control("back")}><ArrowLeft class="size-3.5" /></Button>
      <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_forward()} title={m.chat_browser_forward()}
        disabled={!current?.canGoForward || !!acting} onclick={() => void control("forward")}><ArrowRight class="size-3.5" /></Button>
      <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label={m.chat_browser_reload()} title={m.chat_browser_reload()}
        disabled={!current || !!acting} onclick={() => void control("reload")}><RotateCw class="size-3.5" /></Button>
      <Button size="sm" variant="ghost" class="h-7 shrink-0 gap-1 px-2 text-xs" title={m.chat_browser_exit_fullscreen_title()} data-testid="browser-fullscreen-exit"
        onclick={() => exitFullscreen(true)}><Shrink class="size-3.5" />{m.chat_browser_exit_fullscreen()}</Button>
    </div>
    {@render handoverBanner()}
    <div class="relative min-h-0 flex-1 bg-muted/30" bind:this={fullSlot} data-testid="browser-fullscreen-slot"></div>
  </div>
{/if}
