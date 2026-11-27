<script lang="ts">
  // 悬浮窗（ADR 043 D6，2026-11-26 晚修订）。玩家：“右下角肯定有角色头像，但是有的时候是聊天气泡，因为是一条简单消息，
  // 有的时候是chat小窗，因为是复杂的消息……可以在右下角头像上放一圈进度条，走完了就自动消失，鼠标移动上去就保持”。
  //
  // - 角上一定是她的头像（Avatar：和聊天页同一个头像，外面一圈进度条）。
  // - 旁边是气泡（她那句话 HerLine，或推送的一行）或聊天小窗（MiniChat）。选哪个只看 shape.ts 的 floatingShape；这一次出现里
  //   换成了小窗就一直是小窗，收起以后再从头算。
  // - 聊天小窗就是聊天页：同一个对话状态（AgentChat，同一套订阅：kotone://agent-event、kotone://push……）、同一个时间线
  //   （buildTurns）、同一套消息组件（ChatTurn 的紧凑样子）。原来只放推送的“推送流”去掉了。
  // - 显示不显示、进度条、摆在哪由后端 floating/mod.rs 定（kotone://floating）；这里画，量好大小报上去（floatingLayout，带上
  //   放的是不是小窗：后端按它定一圈多长；窗口跟着内容走，透明的地方不挡游戏）。
  // - 通知、卡片和拖动不抢焦点。只有主动点快捷输入才把键盘交给小窗，离开输入时归还；编辑和录音时停住倒计时。
  import { onMount, tick, untrack } from "svelte";
  import { X } from "@lucide/svelte";
  import { listen } from "@tauri-apps/api/event";
  import { m } from "$lib/paraglide/messages.js";
  import PluginLogo from "$lib/components/plugins/PluginLogo.svelte";
  import { AgentChat } from "../../lib/agentChat/chatController.svelte";
  import { provideAgentChat } from "../../lib/agentChat/context";
  import { buildTurns, type Turn } from "../../lib/agentChat/timeline";
  import { bubbleSnapshot, subscribeBubble, type BubbleSnapshot } from "../../lib/bubble";
  import type { PushRecord, PushTone } from "../../lib/agent";
  import { getSettings, isTauri } from "../../lib/ipc";
  import { pluginText } from "../../lib/plugins";
  import { settingsStore } from "../../lib/stores/ui";
  import { companionName } from "../../lib/stores/companion";
  import { initRuntime } from "../../lib/stores/runtime";
  import {
    floatingClose, floatingCurrent, floatingDrag, floatingHover, floatingInputFocus, floatingLayout, floatingOpen, subscribeFloating,
    type FloatingBox, type FloatingLine, type FloatingView,
  } from "../../lib/floating";
  import { appearancePreferences, initializeAppearancePreferences } from "../../lib/appearance/appearancePreferences";
  import { resolveTheme, resolvedTheme } from "../../lib/appearance/appearanceRuntime";
  import { resolveFontFamily, type AppearanceSettings } from "../../lib/appearance/appearanceSettings";
  import Avatar from "./Avatar.svelte";
  import HerLine from "./HerLine.svelte";
  import MiniChat from "./MiniChat.svelte";
  import { floatingShape, freshTurns, turnPushId, type FloatingShape } from "./shape";

  /** 小窗上面那一行的高度、边框（px）：列表最高 = maxHeight 减掉它们。 */
  const CHROME = 32 + 2;
  /** 列表最矮多高。 */
  const MIN_LIST = 72;
  /** 挡鼠标的几块四周多留多少（px）：阴影画在这一圈里（整块四周也是留 8），窗口只留这几块，再外面就裁掉了。 */
  const HIT_PAD = 8;
  /** 往回翻最多带上“这一次来的”前面几轮（更早的在 Kotone 里）。 */
  const OLDER = 10;
  /** 她在忙的几个状态（头像外面的圈满着、跟着动）。 */
  const BUSY = new Set(["listening", "recognizing", "thinking", "speaking", "asking"]);
  /** 聊天小窗最下面那一行显示的状态（对话里还没有的）。 */
  const STATUS = new Set(["listening", "recognizing", "asking", "note", "error"]);
  const TONES: Record<PushTone, string> = { good: "tone-good", ok: "tone-ok", bad: "tone-bad" };

  // 和聊天页同一个对话状态（同一套订阅）；小窗里的卡片、后台任务卡片照样从它拿
  const chat = provideAgentChat(new AgentChat());

  let view = $state<FloatingView | null>(null);
  let lastVisible = $state<FloatingView | null>(null);
  /** 她最新的快照。 */
  let her = $state<BubbleSnapshot | null>(null);
  /** 最后一个显示着的快照：她说完、bubble.rs 收起以后，后端还留着她那一行时接着画它。 */
  let last = $state<BubbleSnapshot | null>(null);
  let hovering = $state(false);
  let composerActive = $state(false);
  /** 这一次出现里换成过聊天小窗。 */
  let stuck = $state(false);
  let root = $state<HTMLDivElement>();
  let surface = $state<HTMLDivElement>();
  let mounted = $state<FloatingShape | null>(null);
  let unfolded = $state(false);
  let bubble = $state<{ her: BubbleSnapshot | null; line: FloatingLine | null }>({ her: null, line: null });
  let requested: FloatingShape | null = null;
  let motionRevision = 0;

  const shown = $derived(!!view?.shown);
  const displayView = $derived(view?.shown ? view : lastVisible);
  const corner = $derived(displayView?.corner ?? view?.corner ?? "bottom-right");
  const right = $derived(corner.endsWith("right"));
  const bottom = $derived(corner.startsWith("bottom"));
  const herShown = $derived(displayView?.her ? last : null);
  const busy = $derived(!!herShown && !!her && BUSY.has(her.phase) && !view?.ring?.running);
  const paused = $derived(shown && (hovering || composerActive));
  const focus = $derived(displayView?.mini?.focus ?? null);
  const turns = $derived<Turn[]>(shown || mounted === "mini"
    ? buildTurns(chat.conversation?.messages ?? [], chat.runningHere ? chat.live : null, chat.runningHere)
    : []);
  const fresh = $derived(freshTurns(turns, displayView?.since ?? 0, focus));
  const computed = $derived<FloatingShape>(shown
    ? floatingShape({ popped: !!view?.mini, her: herShown, lines: view?.lines.length ?? 0, fresh })
    : "bubble");
  const shape = $derived<FloatingShape>(stuck ? "mini" : computed);
  $effect(() => { if (shown && shape === "mini" && !stuck) stuck = true; });

  /** 小窗里画哪几轮：这一次来的，加上前面几轮（往回翻）。 */
  const anchor = $derived(fresh[0]?.id ?? turns[turns.length - 1]?.id ?? null);
  const listed = $derived.by(() => {
    const at = anchor ? turns.findIndex((t) => t.id === anchor) : turns.length;
    return turns.slice(Math.max(0, (at < 0 ? turns.length : at) - OLDER));
  });
  /** 跟着时放进来的那一轮：弹出的那条推送，之后又有新的就是最新的那一轮。 */
  const target = $derived.by(() => {
    const newest = turns[turns.length - 1] ?? null;
    const popped = focus ? turns.find((t) => turnPushId(t) === focus) ?? null : null;
    if (!popped || !newest) return newest?.id ?? null;
    const at = (t: Turn) => (t.active ? Infinity : Math.max(t.startedAtMs ?? 0, t.push?.updatedAtMs ?? 0));
    return at(popped) >= at(newest) ? popped.id : newest.id;
  });
  const status = $derived(herShown && STATUS.has(herShown.phase) ? herShown : null);
  const line = $derived<FloatingLine | null>(view?.lines[view.lines.length - 1] ?? null);
  const maxList = $derived(Math.max(MIN_LIST, (displayView?.maxHeight ?? 400) - CHROME));

  function accept(next: FloatingView) {
    if (view && next.revision < view.revision) return;
    if (!next.shown || next.since !== view?.since) stuck = false;
    // 收起了：鼠标停着的那一下不带到下一次（后端也清掉了）
    if (next.shown) lastVisible = next;
    else { hovering = false; composerActive = false; }
    view = next;
    void relayout();
  }

  function acceptHer(next: BubbleSnapshot) {
    if (her && next.revision < her.revision) return;
    her = next;
    if (next.phase !== "hidden") last = next;
    void relayout();
  }

  let relayoutQueued = false;
  /** 画完量一下，把整块的大小报上去（窗口藏着时也量得出来：强制排版不等画）。 */
  async function relayout() {
    if (relayoutQueued) return;
    relayoutQueued = true;
    await tick();
    relayoutQueued = false;
    await report();
  }

  async function report() {
    if (!view || !root || view.shown !== (mounted !== null) || mounted !== requested) return;
    const box = root.getBoundingClientRect();
    await floatingLayout(view.revision, Math.ceil(box.width), Math.ceil(box.height), mounted === "mini", hitBoxes(root, box)).catch(() => {});
  }

  /** 挡鼠标的几块：头像、旁边的气泡 / 聊天小窗（整块的直接子元素），相对整块的左上角，四周带上 HIT_PAD，裁到整块里。
   *  用排版的位置（offset*），不看出场动画的位移。窗口只留这几块（后端 SetWindowRgn），聊天小窗旁边、头像上面那一条
   *  透明的地方点得到下面的游戏。 */
  function hitBoxes(el: HTMLElement, box: DOMRect): FloatingBox[] {
    const out: FloatingBox[] = [];
    for (const child of el.children) {
      if (!(child instanceof HTMLElement) || !child.offsetWidth || !child.offsetHeight) continue;
      const left = Math.max(0, child.offsetLeft - HIT_PAD);
      const top = Math.max(0, child.offsetTop - HIT_PAD);
      const right = Math.min(Math.ceil(box.width), child.offsetLeft + child.offsetWidth + HIT_PAD);
      const bottom = Math.min(Math.ceil(box.height), child.offsetTop + child.offsetHeight + HIT_PAD);
      if (right > left && bottom > top) out.push([left, top, right - left, bottom - top]);
    }
    return out;
  }

  // 两种内容共用头像的圆心。气泡完全折回才替换 DOM，量好新一块再展开，原来的内容在退场期间留着。
  async function present(next: FloatingShape | null, revision: number) {
    if (mounted && mounted !== next) {
      unfolded = false;
      await tick();
      const folding = surface?.getAnimations().filter((animation) => "transitionProperty" in animation) ?? [];
      await Promise.allSettled(folding.map((animation) => animation.finished));
      if (revision !== motionRevision) return;
    }
    mounted = next;
    await tick();
    if (revision !== motionRevision) return;
    await report();
    if (next && revision === motionRevision) unfolded = true;
  }

  $effect(() => {
    const next = shown ? shape : null;
    const incoming = next === "bubble" ? { her: herShown, line } : null;
    untrack(() => {
      if (incoming) bubble = incoming;
      if (next === requested) return;
      requested = next;
      void present(next, ++motionRevision);
    });
  });

  // ---- 拖动和点 ----
  let pressAt: { x: number; y: number } | null = null;
  /** 在头像、气泡、小窗上面那一行按下左键：后端跟着光标挪（只是点一下不动它）。按钮、卡片里不算。 */
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
  const openHere = (key: string) => open(chat.conversation?.id ?? view?.mini?.conversationId, key || view?.mini?.key || "");
  const openPush = (push: PushRecord) => open(chat.conversation?.id ?? view?.mini?.conversationId, push.key);

  function openLine(event: MouseEvent, l: FloatingLine) {
    if (clicked(event)) open(l.conversationId, l.key);
  }

  function close() {
    void floatingClose().catch(() => {});
  }

  $effect(() => { void floatingHover(paused).catch(() => {}); });

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

  /** 她的名字和头像（换了头像、名字下一次就生效）。 */
  async function loadSettings() {
    try { settingsStore.set(await getSettings()); } catch { /* 读不到就用内置表情和默认名字 */ }
  }

  // 内容变大变小（卡片里的 App 画出来、她的话变长）：再量一次
  $effect(() => {
    if (!root || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => void relayout());
    observer.observe(root);
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
    keep(initRuntime().catch(() => () => {}));
    void loadSettings();
    if (isTauri) keep(listen("kotone://settings-changed", () => void loadSettings()));
    keep(subscribeBubble(acceptHer));
    keep(subscribeFloating(accept));
    void chat.init();
    void bubbleSnapshot().then((s) => { if (s && !disposed) acceptHer(s); }).catch(() => {});
    void floatingCurrent().then((v) => { if (v && !disposed) accept(v); }).catch(() => {});
    return () => {
      disposed = true;
      motionRevision++;
      for (const u of unlisten) u();
      chat.dispose();
      unsubscribe();
      media?.removeEventListener("change", apply);
      releasePreferences();
    };
  });
</script>

<div bind:this={root} class="root" class:right class:bottom data-testid="floating" data-mode={shown ? shape : "hidden"}
  data-revision={view?.revision ?? 0} role="region" aria-label={m.chat_floating_label()}
  data-surface={mounted ?? "none"} data-unfolded={unfolded}
  onpointerenter={() => { if (shown) hovering = true; }} onpointerleave={() => { hovering = false; }}>
  {#if mounted}
    <div class="corner" class:folding={!unfolded} role="presentation" onpointerdown={drag}>
      <Avatar ring={view?.ring ?? displayView?.ring ?? null} phase={herShown?.phase ?? null} {busy} {paused} />
    </div>
    {#key mounted}
      {#if mounted === "mini"}
        <div bind:this={surface} class="surface" class:unfolded inert={!unfolded}>
          <MiniChat turns={listed} {anchor} {target} pop={focus} {maxList} title={$companionName} {chat} {status}
            active={shown && unfolded} conversationId={displayView?.mini?.conversationId ?? chat.conversation?.id ?? null}
            onOpen={openHere} onOpenPush={openPush} onClose={close} onDrag={drag} onLayout={() => void relayout()}
            onInputFocus={floatingInputFocus} onComposerActive={(active) => { composerActive = active; }} />
        </div>
      {:else if bubble.her || bubble.line}
        <div bind:this={surface} class="surface side" class:unfolded inert={!unfolded} role="presentation" onpointerdown={drag}>
          <button type="button" class="close" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
            onclick={close}><X class="size-3" /></button>
          {#if bubble.her}
            <div data-testid="floating-her"><HerLine snap={bubble.her} /></div>
          {:else if bubble.line}
            {@const line = bubble.line}
            {@const plugin = pluginText(line.plugin, "name", line.pluginName)}
            <!-- 一行推送：点它打开 Kotone、定位到这一条；按住拖动 -->
            <div class="line" class:single={line.cells.length === 1} role="button" tabindex="-1" data-testid="floating-line" data-id={line.id}
              aria-label={m.chat_floating_line_label({ plugin })} title={m.chat_floating_open()}
              onclick={(e) => openLine(e, line)} onkeydown={() => {}}>
              <PluginLogo id={line.plugin} name={plugin} size={16} />
              {#each line.cells as cell, i (i)}
                <span class="cell {cell.tone ? TONES[cell.tone] : ''}" data-testid="floating-cell" data-tone={cell.tone ?? ""}>{cell.text}</span>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/key}
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent !important; overflow: hidden; margin: 0; }
  /* 窗口跟着这一块的大小走（后端按报上去的大小摆），这一块贴着窗口贴着的那个角：头像在角上，旁边的往屏幕里长；
     四周 8 留给阴影。宽度是内容自己的宽度，不受窗口大小影响（窗口还是旧大小时也量得准）。 */
  .root {
    --floating-orb-overlap: -8px;
    --hinge-gap: calc(28px - var(--floating-orb-overlap));
    --hinge-x: calc(0px - var(--hinge-gap));
    --fold-y: 78deg;
    --fold-x: -14deg;
    --floating-bubble: #1d201d;
    --floating-bubble-border: #ffffff22;
    --floating-bubble-ink: #eeefe6;
    position: fixed; display: flex; flex-direction: row; box-sizing: border-box;
    width: max-content; max-width: 480px; padding: 8px; user-select: none;
    left: 0; top: 0; align-items: flex-start;
    font-family: var(--appearance-ui-font, "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif);
  }
  .root.right { left: auto; right: 0; flex-direction: row-reverse; --hinge-x: calc(100% + var(--hinge-gap)); --fold-y: -78deg; }
  .root.bottom { top: auto; bottom: 0; align-items: flex-end; --fold-x: 14deg; --floating-orb-overlap: 18px; }
  .root.bottom.right { --floating-compose-end: 17px; }
  .root.bottom:not(.right) { --floating-compose-start: 17px; }
  .corner { position: relative; z-index: 2; flex: none; transition: scale 235ms cubic-bezier(.22, .72, .17, 1); }
  .corner.folding { scale: .955; }
  .surface {
    --surface-inset: 0px;
    --hinge-y: calc(28px - var(--surface-inset));
    position: relative; margin: var(--surface-inset) 0 0 calc(0px - var(--floating-orb-overlap)); flex: none;
    transform-origin: var(--hinge-x) var(--hinge-y);
    transform: perspective(720px) rotateY(var(--fold-y)) rotateX(var(--fold-x)) scale(.06);
    opacity: 0;
    transition: transform 260ms cubic-bezier(.32, .57, .28, 1), opacity 120ms ease 110ms;
  }
  .root.right .surface { margin-left: 0; margin-right: calc(0px - var(--floating-orb-overlap)); }
  .root.bottom .surface { margin-top: 0; margin-bottom: var(--surface-inset); --hinge-y: calc(100% - 28px + var(--surface-inset)); }
  .surface.unfolded { transform: none; opacity: 1; transition: transform 395ms cubic-bezier(.23, .65, .19, 1), opacity 145ms ease 18ms; }
  .side { --surface-inset: 5px; display: flex; flex-direction: column; }

  .close {
    position: absolute; top: -6px; z-index: 1; display: flex; width: 18px; height: 18px; align-items: center; justify-content: center;
    border: 1px solid var(--floating-bubble-border); border-radius: 50%; background: var(--floating-bubble); color: var(--floating-bubble-ink);
    cursor: pointer; opacity: 0; transition: opacity 120ms ease;
  }
  .root.right .close { left: -6px; }
  .root:not(.right) .close { right: -6px; }
  .root:hover .close, .close:focus-visible { opacity: 1; }
  .close:hover { color: #fff; }
  .line {
    display: flex; max-width: 360px; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 12px;
    background: var(--floating-bubble); color: var(--floating-bubble-ink); border: 1px solid var(--floating-bubble-border);
    box-shadow: 0 6px 18px #0007; cursor: pointer;
  }
  .line:focus { outline: none; }
  .cell { border-radius: 6px; padding: 2px 8px; font-size: 13px; font-weight: 600; line-height: 18px; background: rgba(255, 255, 255, .1); white-space: nowrap; }
  /* 只有一格的一行（同传的一句译文，最多 40 个字）：在 360 里折行；分了几格的（“对面寒冰 | 6 级了”）照旧一行 */
  .line.single { align-items: flex-start; }
  .line.single > :global(:first-child) { flex: none; margin-top: 3px; }
  .line.single .cell { min-width: 0; white-space: normal; overflow-wrap: anywhere; }
  .tone-good { background: rgba(52, 199, 123, .22); color: #7ff0b0; }
  .tone-ok { background: rgba(80, 160, 255, .22); color: #9fcbff; }
  .tone-bad { background: rgba(255, 176, 32, .22); color: #ffd27f; }

  @media (prefers-reduced-motion: reduce) {
    .surface, .surface.unfolded, .corner { transition: none; }
  }
</style>
