<script lang="ts">
  // 消息提示（ADR 039 D4）：发自动卡片时屏幕右下角的小窗——她的头像（右下角压插件的 logo）、“{名字} · {插件}”、
  // 插件给的一句话。照桌面聊天软件的新消息提示，外观和时长照 docs/design/lol-app.md 第四节 3：
  // 320 × 72，Kotone 自己的配色（亮暗跟外观设置），从下往上滑入 180 ms，停 6 秒（鼠标停在上面不走，移开后至少再等 2 秒），
  // 淡出 160 ms；底部 2 px 倒计时线。点它打开聊天、定位到那张卡片；点 × 只关掉。窗口不抢焦点（后端 message_popup.rs）。
  import { onMount } from "svelte";
  import { X } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import KotoneAvatar from "$lib/components/KotoneAvatar.svelte";
  import { getSettings } from "../../lib/ipc";
  import { workspaceAvatar } from "../../lib/workspace";
  import { bundledFaces, placeholderLetter, type RoleFaces } from "../../lib/roles";
  import { appearancePreferences, initializeAppearancePreferences } from "../../lib/appearance/appearancePreferences";
  import { resolveTheme } from "../../lib/appearance/appearanceRuntime";
  import { resolveFontFamily, type AppearanceSettings } from "../../lib/appearance/appearanceSettings";
  import { pluginText } from "../../lib/plugins";
  import PluginLogo from "$lib/components/plugins/PluginLogo.svelte";
  import { messagePopupClose, messagePopupCurrent, messagePopupOpen, subscribeMessagePopup, type MessagePopup } from "../../lib/messagePopup";

  /** 停多久（毫秒）；鼠标移开后至少再等多久。 */
  const DWELL = 6_000;
  const AFTER_HOVER = 2_000;
  const LEAVE_MS = 160;

  let popup = $state<MessagePopup | null>(null);
  /** in = 显示着；out = 正在淡出。 */
  let phase = $state<"in" | "out">("in");
  let left = $state(DWELL);
  let hovering = $state(false);
  let custom = $state<string | null>(null);
  let letter = $state<string | null>(null);
  let faces = $state<RoleFaces | null>(null);

  const plugin = $derived(popup ? pluginText(popup.plugin, "name", popup.pluginName) : "");

  async function refreshAvatar() {
    try {
      const agent = (await getSettings()).agent;
      letter = placeholderLetter(agent?.roleId, agent?.identity?.name, agent?.identity?.avatar);
      faces = bundledFaces(agent?.roleId, agent?.identity?.avatar);
      const name = agent?.identity?.avatar ?? "";
      const bytes = name ? await workspaceAvatar(name) : null;
      if (custom) URL.revokeObjectURL(custom);
      custom = bytes ? URL.createObjectURL(new Blob([bytes as BlobPart])) : null;
    } catch { /* 读不到就用内置表情 */ }
  }

  function accept(next: MessagePopup | null) {
    if (!next || (popup && next.revision < popup.revision)) return;
    popup = next;
    phase = "in";
    left = DWELL;
    void refreshAvatar();
  }

  let leaving: ReturnType<typeof setTimeout> | undefined;
  function leave(then: (revision: number) => Promise<void>) {
    if (!popup || phase === "out") return;
    const revision = popup.revision;
    phase = "out";
    clearTimeout(leaving);
    leaving = setTimeout(() => {
      void then(revision).catch(() => {});
      if (popup?.revision === revision) popup = null;
    }, LEAVE_MS);
  }

  /** 和 Kotone 主窗口一样的配色、亮暗、字体（外观偏好存在本机，窗口之间共用）；不跟缩放（小窗大小是定的）。 */
  function applyAppearance(settings: AppearanceSettings, prefersDark: boolean) {
    const root = document.documentElement;
    const theme = resolveTheme(settings.mode, prefersDark);
    root.dataset.appearance = "enabled";
    root.dataset.palette = settings.palette;
    root.dataset.theme = theme;
    root.style.colorScheme = theme;
    root.style.setProperty("--appearance-ui-font", resolveFontFamily(settings));
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const releasePreferences = initializeAppearancePreferences();
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    let current: AppearanceSettings | null = null;
    const apply = () => { if (current) applyAppearance(current, media?.matches ?? true); };
    const unsubscribe = appearancePreferences.subscribe((s) => { current = s; apply(); });
    media?.addEventListener("change", apply);
    void subscribeMessagePopup(accept).then((u) => { if (disposed) u(); else unlisten = u; });
    void messagePopupCurrent().then((p) => { if (!disposed) accept(p); }).catch(() => {});
    // 倒计时：鼠标在上面时不走
    let last = performance.now();
    let frame = requestAnimationFrame(function tick(now) {
      const step = now - last;
      last = now;
      if (popup && phase === "in" && !hovering) {
        left = Math.max(0, left - step);
        if (left === 0) leave(messagePopupClose);
      }
      frame = requestAnimationFrame(tick);
    });
    return () => {
      disposed = true;
      unlisten?.();
      cancelAnimationFrame(frame);
      clearTimeout(leaving);
      unsubscribe();
      media?.removeEventListener("change", apply);
      releasePreferences();
    };
  });
</script>

<div class="stage">
  {#if popup}
    {#key popup.revision}
      <div class="card" class:out={phase === "out"} role="button" tabindex="-1" data-testid="message-popup"
        data-revision={popup.revision} aria-label={m.chat_popup_label({ plugin })} title={m.chat_popup_open()}
        onmouseenter={() => (hovering = true)}
        onmouseleave={() => { hovering = false; left = Math.max(left, AFTER_HOVER); }}
        onclick={() => leave(messagePopupOpen)}
        onkeydown={(e) => { if (e.key === "Enter") leave(messagePopupOpen); }}>
        <div class="avatar">
          <KotoneAvatar size={40} mood="speaking" ring={false} {custom} {faces} {letter} />
          {#if popup}<span class="badge" aria-hidden="true"><PluginLogo id={popup.plugin} name={plugin} size={18} /></span>{/if}
        </div>
        <div class="body">
          <p class="who" data-testid="message-popup-who">{popup.name} · {plugin}</p>
          <p class="text" data-testid="message-popup-text">{popup.text}</p>
        </div>
        <button type="button" class="close" aria-label={m.chat_popup_close()} title={m.chat_popup_close()} data-testid="message-popup-close"
          onclick={(e) => { e.stopPropagation(); leave(messagePopupClose); }}><X class="size-3.5" /></button>
        <span class="countdown" aria-hidden="true"><span style="transform: scaleX({left / DWELL})"></span></span>
      </div>
    {/key}
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent !important; overflow: hidden; }
  /* 窗口比小窗四周大 12，给阴影留地方；小窗贴着窗口右下（离屏幕边缘 24）。 */
  .stage {
    position: fixed; inset: 0; display: flex; align-items: flex-end; justify-content: flex-end;
    padding: 12px; box-sizing: border-box; user-select: none;
    font-family: var(--appearance-ui-font, "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif);
  }
  .card {
    position: relative; display: flex; align-items: center; gap: 12px; box-sizing: border-box;
    width: 320px; height: 72px; padding: 0 14px; overflow: hidden; cursor: pointer;
    background: var(--card); color: var(--foreground); border: 1px solid var(--hairline, var(--border));
    border-radius: 12px; box-shadow: 0 10px 28px rgb(0 0 0 / 0.16);
    animation: enter 180ms cubic-bezier(.2, .8, .3, 1) both;
    transition: opacity 160ms ease, transform 160ms ease;
  }
  :global(:root[data-theme="dark"]) .card { box-shadow: 0 10px 28px rgb(0 0 0 / 0.55); }
  .card.out { opacity: 0; transform: translateY(4px); }
  .card:focus { outline: none; }
  .avatar { position: relative; flex: none; width: 40px; height: 40px; }
  /* 插件 logo 角标 18，2 px 卡片色描边（设计文档第四节 3） */
  .badge {
    position: absolute; right: -5px; bottom: -5px; display: flex; border-radius: 25%; overflow: hidden;
    box-shadow: 0 0 0 2px var(--card);
  }
  .body { min-width: 0; flex: 1; }
  .who { margin: 0; font-size: 12px; line-height: 16px; color: var(--muted-foreground); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text { margin: 2px 0 0; font-size: 14px; font-weight: 500; line-height: 20px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .close {
    position: absolute; top: 6px; right: 6px; display: flex; width: 20px; height: 20px; align-items: center; justify-content: center;
    border: 0; border-radius: 6px; background: transparent; color: var(--muted-foreground); cursor: pointer; opacity: 0;
    transition: opacity 120ms ease;
  }
  .card:hover .close, .close:focus-visible { opacity: 1; }
  .close:hover { background: var(--muted); color: var(--foreground); }
  .countdown { position: absolute; left: 0; right: 0; bottom: 0; height: 2px; }
  .countdown span { display: block; height: 100%; background: var(--primary); transform-origin: left center; }
  @keyframes enter { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) {
    .card { animation: fade 180ms ease both; }
    .card.out { transform: none; }
    @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  }
</style>
