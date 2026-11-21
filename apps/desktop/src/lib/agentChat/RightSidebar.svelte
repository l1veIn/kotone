<!-- 聊天页右侧栏（ADR 031，照 DeepSeek Harness 桌面版的右侧栏）：内置浏览器卡片 + 后台在做的事。 -->
<!-- 浏览器展开时右侧栏变宽，网页摆在占位区域上（网页由后端盖在界面上面，这里只报位置）。 -->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { ArrowLeft, CircleAlert, Globe, LoaderCircle, Maximize2, Minimize2, Pause, Play, RotateCw, SkipForward, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import {
    browserControl, browserHandoverDone, browserSetBounds, browserStore, mediaLine, siteName,
    type BrowserAction, type BrowserBounds,
  } from "../browser";
  import type { SubagentInfo } from "../agent";
  import { browserExpanded, openRightSidebar, rightSidebarOpen, toggleRightSidebar } from "../stores/rightSidebar";
  import { tryAgentChat } from "./context";
  import SidebarTask from "./SidebarTask.svelte";

  let { active = true, narrow = false, chatWidth = 0 }: {
    /** 聊天页正显示着（不在角色 / 记忆分页上）。 */
    active?: boolean;
    /** 聊天区窄：右侧栏盖在聊天上。 */
    narrow?: boolean;
    /** 聊天页总宽（展开浏览器时算右侧栏多宽）。 */
    chatWidth?: number;
  } = $props();

  const chat = tryAgentChat();
  const browser = $derived($browserStore);
  const open = $derived($rightSidebarOpen);
  const expanded = $derived($browserExpanded);

  // ---- 后台在做的事：在跑的在前，再列最近做完的几件 ----
  const RECENT = 4;
  const tasks = $derived.by<SubagentInfo[]>(() => {
    const list = [...(chat?.conversation?.subagents ?? [])].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
    const live = list.filter((s) => s.status === "running" || s.status === "waiting" || s.status === "interrupted");
    const finished = list.filter((s) => !live.includes(s)).slice(0, RECENT);
    return [...live, ...finished];
  });
  const conversationId = $derived(chat?.conversation?.id ?? "");
  /** 哪些任务卡在侧栏里展开着（展开网页时给任务区多留地方）。 */
  let openTasks = $state<Record<string, boolean>>({});
  const anyTaskOpen = $derived(tasks.some((t) => openTasks[t.name]));

  // ---- 要玩家来（登录、验证码）：打开右侧栏、展开浏览器 ----
  let lastHandover: string | null = null;
  $effect(() => {
    const reason = browser.handover?.reason ?? null;
    if (reason && reason !== lastHandover) {
      openRightSidebar();
      browserExpanded.set(true);
    }
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

  /** 展开时右侧栏的宽度：宽窗口约一半，至少 380、最多 760。 */
  const expandedWidth = $derived(Math.round(Math.min(760, Math.max(380, chatWidth * 0.5))));

  // ---- 把占位区域的位置报给后端（网页摆在那里） ----
  let slot = $state<HTMLDivElement>();
  /** 有对话框、引导、抽屉盖在界面上：网页先藏起来，不然会盖住它们。 */
  let blocked = $state(false);
  const wantVisible = $derived(active && open && expanded);
  let last = "";
  let lastRect: Omit<BrowserBounds, "visible"> = { x: 0, y: 0, width: 0, height: 0 };
  let frame = 0;

  function measure(): BrowserBounds {
    const show = wantVisible && !blocked && !!slot && document.visibilityState !== "hidden";
    if (show && slot) {
      const r = slot.getBoundingClientRect();
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
  function schedule() {
    if (!frame) frame = requestAnimationFrame(() => report());
  }

  // 依赖变化（展开 / 收起、离开聊天页、对话框）时重新报
  $effect(() => {
    void wantVisible;
    void blocked;
    void slot;
    void expandedWidth;
    schedule();
  });

  $effect(() => {
    const target = slot;
    if (!target) return;
    const observer = new ResizeObserver(schedule);
    observer.observe(target);
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
    // 兜底：位置可能在没有尺寸变化时挪动（左侧栏收起等），展开时每秒核对一次
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
    last = "";
    void browserSetBounds({ ...lastRect, visible: false }).catch(() => {});
  });
</script>

{#snippet mediaControls()}
  {#if browser.media}
    <div class="flex shrink-0 items-center gap-0.5">
      {#if browser.media.playing}
        <Button size="icon" variant="ghost" class="size-7" aria-label="暂停" title="暂停"
          data-testid="sidebar-browser-pause" disabled={!!acting} onclick={() => void control("pause")}><Pause class="size-3.5" /></Button>
      {:else}
        <Button size="icon" variant="ghost" class="size-7" aria-label="播放" title="播放"
          data-testid="sidebar-browser-play" disabled={!!acting} onclick={() => void control("play")}><Play class="size-3.5" /></Button>
      {/if}
      <Button size="icon" variant="ghost" class="size-7" aria-label="下一首" title="下一首"
        data-testid="sidebar-browser-next" disabled={!!acting} onclick={() => void control("next")}><SkipForward class="size-3.5" /></Button>
    </div>
  {/if}
{/snippet}

{#snippet browserError()}
  {#if browser.error}
    <div class="flex min-w-0 items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2" role="alert" data-testid="browser-error">
      <CircleAlert class="mt-0.5 size-3.5 shrink-0 text-destructive" />
      <p class="m-0 min-w-0 flex-1 text-xs leading-5 break-words text-destructive" data-testid="browser-error-text">{browser.error}</p>
      <Button size="sm" variant="outline" class="h-6 shrink-0 px-2 text-[11px]" data-testid="browser-error-retry"
        disabled={!!acting} onclick={() => void control("reload")}>重试</Button>
    </div>
  {/if}
{/snippet}

{#snippet handoverBanner()}
  {#if browser.handover}
    <div class="shrink-0 border-b border-warning/40 bg-warning/8 px-3 py-2.5" role="status" data-testid="browser-handover">
      <p class="m-0 text-[13px] leading-5 text-foreground"><span class="font-medium">{browser.handover.reason}</span>：在下面的页面里登录（网易云可以用手机扫码），好了点「好了」</p>
      <Button size="sm" class="mt-2 h-7" data-testid="browser-handover-done" disabled={!!acting} onclick={() => void handoverDone()}>好了</Button>
    </div>
  {/if}
{/snippet}

{#if open}
  {#if narrow}
    <!-- 窄窗口：盖在聊天上，点空白处收起 -->
    <button type="button" class="absolute inset-0 z-20 bg-black/30" aria-label="收起右侧栏" data-testid="right-sidebar-backdrop" onclick={toggleRightSidebar}></button>
  {/if}
  <aside
    class="flex min-h-0 flex-col border-l border-hairline bg-background {narrow ? 'absolute inset-y-0 right-0 z-30 shadow-2xl' : 'relative shrink-0'}"
    style:width={expanded ? (narrow ? "100%" : `${expandedWidth}px`) : narrow ? "min(100%, 340px)" : "300px"}
    aria-label="右侧栏" data-testid="right-sidebar" data-expanded={expanded ? "true" : "false"}
  >
    {#if expanded}
      <!-- 展开的网页：顶部一条（网站、后退、刷新、收起），下面是网页 -->
      <div class="flex min-h-0 flex-1 flex-col" data-testid="browser-panel">
        <div class="flex h-10 shrink-0 items-center gap-1 border-b border-hairline px-2">
          {#if browser.loading}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-muted-foreground" />{:else}<Globe class="size-3.5 shrink-0 text-muted-foreground" />{/if}
          <div class="min-w-0 flex-1 px-1">
            <p class="m-0 truncate text-[13px] font-medium text-foreground" data-testid="browser-panel-site">{siteName(browser)}</p>
          </div>
          {#if browser.busy}<span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" data-testid="browser-panel-busy">她在操作</span>{/if}
          {@render mediaControls()}
          <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label="后退" title="后退" data-testid="browser-panel-back"
            disabled={!browser.open || !!acting} onclick={() => void control("back")}><ArrowLeft class="size-3.5" /></Button>
          <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label="刷新" title="刷新" data-testid="browser-panel-reload"
            disabled={!browser.open || !!acting} onclick={() => void control("reload")}><RotateCw class="size-3.5" /></Button>
          <Button size="icon" variant="ghost" class="size-7 shrink-0" aria-label="收起网页" title="收起网页" data-testid="browser-panel-collapse"
            onclick={() => browserExpanded.set(false)}><Minimize2 class="size-3.5" /></Button>
        </div>
        {@render handoverBanner()}
        {#if browser.error}<div class="shrink-0 border-b border-hairline px-3 py-2">{@render browserError()}</div>{/if}
        {#if actionError}<p role="alert" class="m-0 shrink-0 px-3 py-1.5 text-xs text-destructive">{actionError}</p>{/if}
        <!-- 网页摆在这里（盖在上面）；没打开网页时显示说明 -->
        <div class="relative min-h-0 flex-1 bg-muted/30" bind:this={slot} data-testid="browser-panel-slot">
          {#if !browser.open}
            <p class="absolute inset-x-6 top-1/3 m-0 text-center text-xs leading-5 text-muted-foreground" data-testid="browser-panel-empty">
              还没有打开网页。让她放首歌，或者让她上网查东西，页面会在这里出现。
            </p>
          {/if}
        </div>
        {#if tasks.length}
          <!-- 任务区在网页下面（网页只盖占位区域）；有卡片展开时多给些高度，网页区跟着缩小、重新报位置 -->
          <div class="{anyTaskOpen ? 'max-h-[60%]' : 'max-h-[35%]'} shrink-0 overflow-y-auto border-t border-hairline px-3 py-2" data-testid="sidebar-tasks">
            <h2 class="mt-0 mb-1.5 text-[11px] font-medium tracking-wider text-muted-foreground">后台在做的事</h2>
            <ul class="m-0 list-none space-y-1.5 p-0">
              {#each tasks as info (info.name)}<SidebarTask {info} {conversationId} bind:open={openTasks[info.name]} />{/each}
            </ul>
          </div>
        {/if}
      </div>
    {:else}
      <div class="flex h-10 shrink-0 items-center justify-between border-b border-hairline pr-1.5 pl-3.5">
        <h2 class="m-0 text-[12px] font-medium text-muted-foreground">网页与后台</h2>
        <Button size="icon" variant="ghost" class="size-7" aria-label="收起右侧栏" title="收起右侧栏" data-testid="right-sidebar-close"
          onclick={toggleRightSidebar}><X class="size-3.5" /></Button>
      </div>
      <div class="min-h-0 flex-1 space-y-4 overflow-y-auto px-3 py-3">
        <!-- 浏览器卡片 -->
        <section aria-label="网页" class="min-w-0 overflow-hidden rounded-xl border border-hairline bg-card/60" data-testid="sidebar-browser-card"
          data-browser-state={browser.handover ? "handover" : browser.error ? "error" : browser.open ? "open" : "closed"}>
          {#if browser.open}
            <div class="flex min-w-0 items-center gap-2 px-3 pt-2.5">
              {#if browser.loading}<LoaderCircle class="size-3.5 shrink-0 animate-spin text-muted-foreground" />{:else}<Globe class="size-3.5 shrink-0 text-muted-foreground" />{/if}
              <span class="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground" data-testid="sidebar-browser-site">{siteName(browser)}</span>
              {#if browser.busy}<span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[11px] text-primary" data-testid="sidebar-browser-busy">她在操作</span>{/if}
            </div>
            {#if browser.title}<p class="m-0 truncate px-3 pt-0.5 pl-8.5 text-xs text-muted-foreground" title={browser.title} data-testid="sidebar-browser-title">{browser.title}</p>{/if}
            {#if browser.media}
              <div class="mx-3 mt-2 flex min-w-0 items-center gap-2 rounded-lg bg-muted/50 py-1 pr-1 pl-2.5">
                <p class="m-0 min-w-0 flex-1 truncate text-xs text-foreground/90" data-testid="sidebar-browser-media">{mediaLine(browser.media)}</p>
                {@render mediaControls()}
              </div>
            {/if}
            {#if browser.handover}
              <p class="m-0 mx-3 mt-2 rounded-lg bg-warning/10 px-2.5 py-1.5 text-xs text-warning" data-testid="sidebar-browser-handover">{browser.handover.reason}，展开登录一下</p>
            {/if}
            {#if browser.error}<div class="mx-3 mt-2">{@render browserError()}</div>{/if}
            <div class="flex justify-end px-1.5 py-1.5">
              <Button size="sm" variant="ghost" class="h-7 gap-1 px-2 text-xs" data-testid="sidebar-browser-expand"
                onclick={() => browserExpanded.set(true)}><Maximize2 class="size-3" />展开</Button>
            </div>
          {:else if browser.error}
            <!-- 浏览器没能打开（例如 WebView2 建不出来）：说清原因，给一个重试 -->
            <div class="p-3">{@render browserError()}</div>
          {:else}
            <div class="flex items-start gap-2 px-3 py-3">
              <Globe class="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
              <p class="m-0 text-xs leading-5 text-muted-foreground" data-testid="sidebar-browser-hint">让她放首歌，或者让她上网查东西，页面会在这里出现。</p>
            </div>
          {/if}
        </section>
        {#if actionError}<p role="alert" class="m-0 text-xs text-destructive">{actionError}</p>{/if}

        <!-- 后台在做的事 -->
        <section aria-label="后台在做的事" data-testid="sidebar-tasks">
          <h2 class="mt-0 mb-1.5 px-0.5 text-[11px] font-medium tracking-wider text-muted-foreground">后台在做的事</h2>
          {#if tasks.length}
            <ul class="m-0 list-none space-y-1.5 p-0">
              {#each tasks as info (info.name)}<SidebarTask {info} {conversationId} bind:open={openTasks[info.name]} />{/each}
            </ul>
          {:else}
            <p class="m-0 px-0.5 text-xs leading-5 text-muted-foreground" data-testid="sidebar-tasks-empty">复杂的事她会放到后台做，做的时候在这里看进度。</p>
          {/if}
        </section>
      </div>
    {/if}
  </aside>
{/if}
