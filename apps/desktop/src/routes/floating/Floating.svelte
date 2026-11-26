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
  // - 不抢焦点（窗口不可聚焦，后端管）：点 ×、点卡片里的按钮都不激活窗口；按住头像、气泡、小窗上面那一行拖动，由后端跟着
  //   光标挪（floatingDrag）。点推送的一行、卡片上面那一行、“打开 Kotone”才把主窗口拿到前面（玩家自己的动作）。
  import { onMount, tick } from "svelte";
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
  import {
    floatingClose, floatingCurrent, floatingDrag, floatingHover, floatingLayout, floatingOpen, subscribeFloating,
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
  /** 她最新的快照。 */
  let her = $state<BubbleSnapshot | null>(null);
  /** 最后一个显示着的快照：她说完、bubble.rs 收起以后，后端还留着她那一行时接着画它。 */
  let last = $state<BubbleSnapshot | null>(null);
  let hovering = $state(false);
  /** 这一次出现里换成过聊天小窗。 */
  let stuck = $state(false);
  let root = $state<HTMLDivElement>();

  const shown = $derived(!!view?.shown);
  const corner = $derived(view?.corner ?? "bottom-right");
  const right = $derived(corner.endsWith("right"));
  const bottom = $derived(corner.startsWith("bottom"));
  const herShown = $derived(shown && view?.her ? last : null);
  const busy = $derived(!!herShown && !!her && BUSY.has(her.phase) && !view?.ring?.running);
  const focus = $derived(view?.mini?.focus ?? null);
  const turns = $derived<Turn[]>(shown
    ? buildTurns(chat.conversation?.messages ?? [], chat.runningHere ? chat.live : null, chat.runningHere)
    : []);
  const fresh = $derived(shown ? freshTurns(turns, view?.since ?? 0, focus) : []);
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
  const maxList = $derived(Math.max(MIN_LIST, (view?.maxHeight ?? 400) - CHROME));

  function accept(next: FloatingView) {
    if (view && next.revision < view.revision) return;
    if (!next.shown || next.since !== view?.since) stuck = false;
    // 收起了：鼠标停着的那一下不带到下一次（后端也清掉了）
    if (!next.shown) hovering = false;
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
    report();
  }

  function report() {
    if (!view || !root) return;
    const box = root.getBoundingClientRect();
    void floatingLayout(view.revision, Math.ceil(box.width), Math.ceil(box.height), shape === "mini", hitBoxes(root, box)).catch(() => {});
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

  // 换了样子（气泡 ↔ 小窗）：再量一次
  $effect(() => { void shape; void relayout(); });

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

  function hover(inside: boolean) {
    hovering = inside;
    void floatingHover(inside).catch(() => {});
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
    void loadSettings();
    if (isTauri) keep(listen("kotone://settings-changed", () => void loadSettings()));
    keep(subscribeBubble(acceptHer));
    keep(subscribeFloating(accept));
    void chat.init();
    void bubbleSnapshot().then((s) => { if (s && !disposed) acceptHer(s); }).catch(() => {});
    void floatingCurrent().then((v) => { if (v && !disposed) accept(v); }).catch(() => {});
    return () => {
      disposed = true;
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
  onpointerenter={() => hover(true)} onpointerleave={() => hover(false)}>
  {#if shown}
    <div class="corner" role="presentation" onpointerdown={drag}>
      <Avatar ring={view?.ring ?? null} phase={herShown?.phase ?? null} {busy} paused={hovering} />
    </div>
    {#if shape === "mini"}
      <MiniChat turns={listed} {anchor} {target} pop={focus} {maxList} title={$companionName} {chat} {status}
        onOpen={openHere} onOpenPush={openPush} onClose={close} onDrag={drag} onLayout={() => void relayout()} />
    {:else if herShown || line}
      <div class="side" role="presentation" onpointerdown={drag}>
        <button type="button" class="close" aria-label={m.chat_floating_close()} title={m.chat_floating_close()} data-testid="floating-close"
          onclick={close}><X class="size-3" /></button>
        {#if herShown}
          <div data-testid="floating-her"><HerLine snap={herShown} /></div>
        {:else if line}
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
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent !important; overflow: hidden; margin: 0; }
  /* 窗口跟着这一块的大小走（后端按报上去的大小摆），这一块贴着窗口贴着的那个角：头像在角上，旁边的往屏幕里长；
     四周 8 留给阴影。宽度是内容自己的宽度，不受窗口大小影响（窗口还是旧大小时也量得准）。 */
  .root {
    position: fixed; display: flex; flex-direction: row; gap: 8px; box-sizing: border-box;
    width: max-content; max-width: 480px; padding: 8px; user-select: none;
    left: 0; top: 0; align-items: flex-start;
    font-family: var(--appearance-ui-font, "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif);
  }
  .root.right { left: auto; right: 0; flex-direction: row-reverse; }
  .root.bottom { top: auto; bottom: 0; align-items: flex-end; }
  .corner { flex: none; }
  .side { position: relative; display: flex; flex-direction: column; margin-bottom: 4px; }
  .root:not(.bottom) .side { margin: 4px 0 0; }

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
  /* 只有一格的一行（同传的一句译文，最多 40 个字）：在 360 里折行；分了几格的（“对面寒冰 | 6 级了”）照旧一行 */
  .line.single { align-items: flex-start; }
  .line.single > :global(:first-child) { flex: none; margin-top: 3px; }
  .line.single .cell { min-width: 0; white-space: normal; overflow-wrap: anywhere; }
  .tone-good { background: rgba(52, 199, 123, .22); color: #7ff0b0; }
  .tone-ok { background: rgba(80, 160, 255, .22); color: #9fcbff; }
  .tone-bad { background: rgba(255, 176, 32, .22); color: #ffd27f; }

  @keyframes rise { from { transform: translateY(6px); opacity: 0; } to { transform: none; opacity: 1; } }
  @media (prefers-reduced-motion: reduce) {
    .line { animation: none; }
  }
</style>
