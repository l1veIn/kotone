<script lang="ts">
  // 悬浮窗（ADR 043 D6）：两种状态——气泡（一行字：她在听、在想、在说，或插件推送的一行）和小窗（这段对话里的推送，
  // 用聊天页的 PushItem，新的在下面，往回翻看更早的）。原来的说话气泡（routes/companion）和右下角的消息提示
  // （routes/messagePopup）并进了这里。
  //
  // 显示什么、什么时候收、摆在哪由后端 floating/mod.rs 定（kotone://floating）；这里只画，量好内容的大小报上去
  // （floatingLayout：窗口跟着内容走，透明的地方不挡游戏）。小窗的高度跟着定位的那一条（新推来的卡片、带提醒的更新）走，
  // 最高是游戏所在屏幕高度的 40%（maxHeight）；跟着的时候它在最上面，玩家自己往回翻了就不跟（照 dsh-orb-cordis 的
  // followBottom，MIT）。往回翻：滚轮；滚轮在不可聚焦的窗口里还没在真机上验证，先放上 / 下翻的按钮（能翻时才出来）。
  //
  // 不抢焦点（窗口不可聚焦，后端管）：点 ×、点卡片里的按钮都不激活窗口；按住气泡、小窗上面那一行拖动，由后端跟着光标挪
  // （floatingDrag）。点卡片上面那一行、“打开 Kotone”才把主窗口拿到前面（玩家自己的动作）。
  import { onMount, tick } from "svelte";
  import { ChevronDown, ChevronUp, SquareArrowOutUpRight, X } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import PluginLogo from "$lib/components/plugins/PluginLogo.svelte";
  import PushItem from "$lib/agentChat/PushItem.svelte";
  import { bubbleSnapshot, subscribeBubble, type BubbleSnapshot } from "../../lib/bubble";
  import { subscribePushes, type PushTone } from "../../lib/agent";
  import { pluginText } from "../../lib/plugins";
  import {
    floatingClose, floatingCurrent, floatingDrag, floatingHover, floatingLayout, floatingOpen, subscribeFloating,
    type FloatingLine, type FloatingView,
  } from "../../lib/floating";
  import { appearancePreferences, initializeAppearancePreferences } from "../../lib/appearance/appearancePreferences";
  import { resolveTheme, resolvedTheme } from "../../lib/appearance/appearanceRuntime";
  import { resolveFontFamily, type AppearanceSettings } from "../../lib/appearance/appearanceSettings";
  import HerLine from "./HerLine.svelte";
  import { PushStream } from "./pushStream.svelte";

  /** 小窗上面那一行的高度、列表上下的留白（px）：列表最高 = maxHeight 减掉它们。 */
  const CHROME = 32 + 12;
  /** 列表最矮多高（卡片还没画出来时）。 */
  const MIN_LIST = 72;

  let view = $state<FloatingView | null>(null);
  let her = $state<BubbleSnapshot | null>(null);
  const stream = new PushStream();
  let root = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let inner = $state<HTMLDivElement>();
  let listHeight = $state(MIN_LIST);
  /** 跟着定位的那一条（它在最上面）；玩家自己往回翻了就不跟。 */
  let follow = true;
  /** 自己滚的那一下引起的 scroll 不算玩家在翻。 */
  let ownScrollUntil = 0;
  let canUp = $state(false);
  let canDown = $state(false);

  const mode = $derived(view?.mode ?? "hidden");
  const right = $derived((view?.corner ?? "bottom-right").endsWith("right"));
  const bottom = $derived((view?.corner ?? "bottom-right").startsWith("bottom"));
  const herShown = $derived(!!view?.her && !!her && her.phase !== "hidden");
  const lines = $derived<FloatingLine[]>(mode === "bubble" ? (view?.lines ?? []) : []);
  const focus = $derived(view?.mini?.focus ?? null);
  const maxList = $derived(Math.max(MIN_LIST, (view?.maxHeight ?? 400) - CHROME));
  const TONES: Record<PushTone, string> = { good: "tone-good", ok: "tone-ok", bad: "tone-bad" };

  function accept(next: FloatingView) {
    if (view && next.revision < view.revision) return;
    if (next.mini && next.mini.focus !== view?.mini?.focus) follow = true;
    view = next;
    if (next.mini) void stream.show(next.mini.conversationId).then(() => void relayout());
    void relayout();
  }

  function acceptHer(next: BubbleSnapshot) {
    if (her && next.revision < her.revision) return;
    her = next;
    void relayout();
  }

  let relayoutQueued = false;
  /** 画完量一下：小窗列表的高度跟着定位的那一条和它下面的走（最高 maxList），跟着时把它滚到最上面；再把整块的大小报上去。 */
  async function relayout() {
    if (relayoutQueued) return;
    relayoutQueued = true;
    await tick();
    relayoutQueued = false;
    if (!view) return;
    if (view.mini && list && inner) {
      const el = focus ? inner.querySelector<HTMLElement>(`[data-floating-id="${CSS.escape(focus)}"]`) : null;
      const below = el ? inner.scrollHeight - el.offsetTop : inner.scrollHeight;
      listHeight = Math.max(MIN_LIST, Math.min(below, maxList));
      await tick();
      if (follow && list) {
        ownScrollUntil = performance.now() + 200;
        list.scrollTop = el ? el.offsetTop : list.scrollHeight;
      }
      arrows();
    }
    report();
  }

  /** 把内容的大小（逻辑像素）报给后端：窗口照它摆、该显示就显示。窗口藏着时也量得出来（强制排版不等画）。 */
  function report() {
    if (!view || !root) return;
    const box = root.getBoundingClientRect();
    void floatingLayout(view.revision, Math.ceil(box.width), Math.ceil(box.height)).catch(() => {});
  }

  function arrows() {
    if (!list) return;
    canUp = list.scrollTop > 2;
    canDown = list.scrollTop + list.clientHeight < list.scrollHeight - 2;
  }

  function onScroll() {
    arrows();
    if (!list || performance.now() < ownScrollUntil) return;
    // 玩家往回翻了就不跟；翻回最下面又跟上（新来的接着出现在下面）
    follow = list.scrollTop + list.clientHeight >= list.scrollHeight - 24;
  }

  function page(direction: 1 | -1) {
    if (!list) return;
    list.scrollBy({ top: direction * Math.max(48, list.clientHeight * 0.8), behavior: "smooth" });
  }

  // ---- 拖动和点 ----
  let pressAt: { x: number; y: number } | null = null;
  /** 在气泡、小窗上面那一行按下左键：后端跟着光标挪（只是点一下不动它）。按钮、卡片里不算。 */
  function drag(event: PointerEvent) {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button, a, input, iframe, [data-no-drag]")) return;
    pressAt = { x: event.screenX, y: event.screenY };
    void floatingDrag().catch(() => {});
  }
  /** 这一下是点（不是拖完松手）：窗口跟着光标走，页面里的坐标不变，看屏幕坐标。 */
  function clicked(event: MouseEvent): boolean {
    const moved = pressAt ? Math.hypot(event.screenX - pressAt.x, event.screenY - pressAt.y) > 4 : false;
    pressAt = null;
    return !moved;
  }

  function open(conversationId: string | null | undefined, key: string) {
    if (conversationId) void floatingOpen(conversationId, key).catch(() => {});
  }

  function openLine(event: MouseEvent, line: FloatingLine) {
    if (clicked(event)) open(line.conversationId, line.key);
  }

  /** 和 Kotone 主窗口一样的配色、亮暗、字体（外观偏好存在本机，窗口之间共用）；不跟缩放（窗口大小跟着内容）。 */
  function applyAppearance(settings: AppearanceSettings, prefersDark: boolean) {
    const html = document.documentElement;
    const theme = resolveTheme(settings.mode, prefersDark);
    html.dataset.appearance = "enabled";
    html.dataset.palette = settings.palette;
    html.dataset.theme = theme;
    html.style.colorScheme = theme;
    html.style.setProperty("--appearance-ui-font", resolveFontFamily(settings));
    resolvedTheme.set(theme);
  }

  // 内容变大变小（卡片里的 App 画出来、她的话变长）：再量一次
  $effect(() => {
    const targets = [root, inner].filter((el): el is HTMLDivElement => !!el);
    if (!targets.length || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => void relayout());
    for (const target of targets) observer.observe(target);
    return () => observer.disconnect();
  });

  onMount(() => {
    let disposed = false;
    const unlisten: (() => void)[] = [];
    const keep = (promise: Promise<() => void>) => void promise.then((u) => { if (disposed) u(); else unlisten.push(u); });
    const releasePreferences = initializeAppearancePreferences();
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    let current: AppearanceSettings | null = null;
    const apply = () => { if (current) applyAppearance(current, media?.matches ?? true); };
    const unsubscribe = appearancePreferences.subscribe((s) => { current = s; apply(); });
    media?.addEventListener("change", apply);
    keep(subscribeBubble(acceptHer));
    keep(subscribeFloating(accept));
    keep(subscribePushes((event) => { stream.apply(event); void relayout(); }));
    void bubbleSnapshot().then((s) => { if (s && !disposed) acceptHer(s); }).catch(() => {});
    void floatingCurrent().then((v) => { if (v && !disposed) accept(v); }).catch(() => {});
    return () => {
      disposed = true;
      for (const u of unlisten) u();
      unsubscribe();
      media?.removeEventListener("change", apply);
      releasePreferences();
    };
  });
</script>

<div bind:this={root} class="root" class:right class:bottom data-testid="floating" data-mode={mode}
  data-revision={view?.revision ?? 0} role="region" aria-label={m.chat_floating_label()}
  onpointerenter={() => void floatingHover(true).catch(() => {})}
  onpointerleave={() => void floatingHover(false).catch(() => {})}>
  {#if mode === "mini" && view?.mini}
    {@const mini = view.mini}
    <section class="mini" data-testid="floating-mini" data-focus={mini.focus}>
      <div class="head" role="presentation" onpointerdown={drag} data-testid="floating-mini-head">
        <span class="title">{m.chat_floating_title()}</span>
        {#if canUp || canDown}
          <!-- 往回翻（滚轮之外的办法） -->
          <button type="button" class="tool" disabled={!canUp} aria-label={m.chat_floating_older()} title={m.chat_floating_older()}
            data-testid="floating-older" onclick={() => page(-1)}><ChevronUp class="size-3.5" /></button>
          <button type="button" class="tool" disabled={!canDown} aria-label={m.chat_floating_newer()} title={m.chat_floating_newer()}
            data-testid="floating-newer" onclick={() => page(1)}><ChevronDown class="size-3.5" /></button>
        {/if}
        <button type="button" class="tool" aria-label={m.chat_floating_open()} title={m.chat_floating_open()} data-testid="floating-open"
          onclick={() => open(mini.conversationId, mini.key)}><SquareArrowOutUpRight class="size-3.5" /></button>
        <button type="button" class="tool" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
          onclick={() => void floatingClose().catch(() => {})}><X class="size-3.5" /></button>
      </div>
      <div bind:this={list} class="list" style:height="{listHeight}px" onscroll={onScroll} data-testid="floating-list">
        <div bind:this={inner} class="inner">
          {#each stream.items as item (item.id)}
            <div class="item" data-floating-id={item.id} data-testid="floating-item" data-key={item.push.key}>
              <PushItem push={item.push} atMs={item.atMs} onOpen={() => open(stream.conversationId, item.push.key)} />
            </div>
          {/each}
        </div>
      </div>
    </section>
  {/if}
  {#if lines.length || herShown}
    <div class="bubbles">
      {#if mode === "bubble"}
        <button type="button" class="close" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
          onclick={() => void floatingClose().catch(() => {})}><X class="size-3" /></button>
      {/if}
      {#each lines as line (line.id)}
        {@const plugin = pluginText(line.plugin, "name", line.pluginName)}
        <!-- 一行推送：点它打开 Kotone、定位到这一条；按住拖动 -->
        <div class="line" role="button" tabindex="-1" data-testid="floating-line" data-id={line.id}
          aria-label={m.chat_floating_line_label({ plugin })} title={m.chat_floating_open()}
          onpointerdown={drag} onclick={(e) => openLine(e, line)} onkeydown={() => {}}>
          <PluginLogo id={line.plugin} name={plugin} size={16} />
          {#each line.cells as cell, i (i)}
            <span class="cell {cell.tone ? TONES[cell.tone] : ''}" data-testid="floating-cell" data-tone={cell.tone ?? ""}>{cell.text}</span>
          {/each}
        </div>
      {/each}
      {#if herShown && her}
        <div class="her-wrap" role="presentation" onpointerdown={drag}><HerLine snap={her} left={!right} /></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent !important; overflow: hidden; margin: 0; }
  /* 窗口跟着这一块的大小走（后端按报上去的大小摆），这一块贴着窗口贴着的那个角；四周 8 留给阴影。
     宽度是内容自己的宽度，不受窗口大小影响（窗口还是旧大小时也量得准）。 */
  .root {
    position: fixed; display: flex; flex-direction: column; gap: 6px; box-sizing: border-box;
    width: max-content; max-width: 420px; padding: 8px; user-select: none;
    left: 0; top: 0; align-items: flex-start;
    font-family: var(--appearance-ui-font, "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif);
  }
  .root.right { left: auto; right: 0; align-items: flex-end; }
  .root.bottom { top: auto; bottom: 0; }
  /* 上面的角：气泡贴着角（在上面），小窗在下面 */
  .root:not(.bottom) { flex-direction: column-reverse; }

  .mini {
    display: flex; flex-direction: column; width: 380px; overflow: hidden; box-sizing: border-box;
    background: var(--card); color: var(--foreground); border: 1px solid var(--hairline, var(--border));
    border-radius: 12px; box-shadow: 0 8px 24px rgb(0 0 0 / 0.18);
    animation: enter 180ms cubic-bezier(.2, .8, .3, 1) both;
  }
  :global(:root[data-theme="dark"]) .mini { box-shadow: 0 8px 24px rgb(0 0 0 / 0.55); }
  .head {
    display: flex; height: 32px; flex: none; align-items: center; gap: 2px; padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--hairline, var(--border)); cursor: grab;
  }
  .title { min-width: 0; flex: 1; font-size: 12px; font-weight: 600; color: var(--muted-foreground); }
  .tool {
    display: flex; width: 24px; height: 24px; align-items: center; justify-content: center; border: 0; border-radius: 6px;
    background: transparent; color: var(--muted-foreground); cursor: pointer;
  }
  .tool:hover:not(:disabled) { background: var(--muted); color: var(--foreground); }
  .tool:disabled { opacity: .35; cursor: default; }
  .list { overflow-y: auto; overscroll-behavior: contain; padding: 4px 8px 8px; box-sizing: content-box; scrollbar-width: thin; }
  .inner { position: relative; display: flex; flex-direction: column; gap: 8px; }
  .item { min-width: 0; }

  .bubbles { position: relative; display: flex; flex-direction: column; gap: 6px; align-items: inherit; }
  .close {
    position: absolute; top: -6px; z-index: 1; display: flex; width: 18px; height: 18px; align-items: center; justify-content: center;
    border: 1px solid rgba(255, 255, 255, .16); border-radius: 50%; background: rgba(22, 20, 34, .95); color: #d9d4ef;
    cursor: pointer; opacity: 0; transition: opacity 120ms ease;
  }
  .root.right .close { left: -6px; }
  .root:not(.right) .close { right: -6px; }
  .root:hover .close, .close:focus-visible { opacity: 1; }
  .close:hover { color: #fff; }
  .line {
    display: flex; max-width: 360px; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 12px;
    background: rgba(22, 20, 34, .92); color: #f4f2fb; border: 1px solid rgba(255, 255, 255, .12);
    box-shadow: 0 6px 18px #0007; cursor: pointer; animation: rise .18s ease-out both;
  }
  .line:focus { outline: none; }
  .cell { border-radius: 6px; padding: 2px 8px; font-size: 13px; font-weight: 600; line-height: 18px; background: rgba(255, 255, 255, .1); white-space: nowrap; }
  .tone-good { background: rgba(52, 199, 123, .22); color: #7ff0b0; }
  .tone-ok { background: rgba(80, 160, 255, .22); color: #9fcbff; }
  .tone-bad { background: rgba(255, 176, 32, .22); color: #ffd27f; }
  .her-wrap { cursor: grab; }

  @keyframes enter { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
  @keyframes rise { from { transform: translateY(6px); opacity: 0; } to { transform: none; opacity: 1; } }
  @media (prefers-reduced-motion: reduce) {
    .mini, .line { animation: none; }
  }
</style>
