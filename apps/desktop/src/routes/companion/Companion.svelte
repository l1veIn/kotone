<script lang="ts">
  // 琴音的对话气泡窗口（透明、置顶、鼠标穿透、不抢焦点）。只展示后端推来的状态。
  import { onMount } from "svelte";
  import {
    askRemaining, bubbleBody, bubbleLook, bubbleSnapshot, subscribeBubble, type BubbleSnapshot,
  } from "../../lib/bubble";

  let snap = $state<BubbleSnapshot | null>(null);
  let now = $state(Date.now());
  const visible = $derived(!!snap && snap.phase !== "hidden");
  const look = $derived(snap && snap.phase !== "hidden" ? bubbleLook[snap.phase] : null);
  const body = $derived(snap ? bubbleBody(snap) : { text: "", quoted: false });
  const remaining = $derived(snap ? askRemaining(snap, now) : null);
  const left = $derived(snap?.corner === "bottom-left" || snap?.corner === "top-left");
  const top = $derived(snap?.corner === "top-right" || snap?.corner === "top-left");

  function accept(next: BubbleSnapshot) {
    if (snap && next.revision < snap.revision) return;
    snap = next;
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void subscribeBubble(accept).then((u) => { if (disposed) u(); else unlisten = u; });
    void bubbleSnapshot().then((s) => { if (s && !disposed) accept(s); }).catch(() => {});
    const timer = setInterval(() => { if (snap?.phase === "asking") now = Date.now(); }, 100);
    return () => { disposed = true; unlisten?.(); clearInterval(timer); };
  });
</script>

<div class="stage" class:left class:top data-testid="companion-bubble">
  {#if visible && snap && look}
    {#key look.face}
      <div class="avatar ring-{look.ring}" aria-hidden="true">
        <div class="face"><img src={look.face} alt="" style="margin: calc(50% - {look.fy * 170}%) 0 0 calc(50% - {look.fx * 170}%)" /></div>
      </div>
    {/key}
    <div class="bubble phase-{snap.phase}" role="status" aria-live="polite" data-phase={snap.phase}>
      <div class="label">
        {#if snap.phase === "thinking" || snap.phase === "recognizing"}
          <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
        {:else if snap.phase === "listening"}
          <span class="bars" aria-hidden="true"><i></i><i></i><i></i><i></i></span>
        {/if}
        <span data-testid="companion-label">{snap.label}</span>
      </div>
      {#if body.text}
        <p class="text" class:quoted={body.quoted} data-testid="companion-text">{body.quoted ? `“${body.text}”` : body.text}</p>
      {/if}
      {#if snap.phase === "asking" && snap.heard}
        <p class="heard">“{snap.heard}”</p>
      {/if}
      {#if remaining !== null}
        <div class="countdown" aria-hidden="true"><span style="transform: scaleX({remaining})"></span></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { background: transparent !important; overflow: hidden; }
  .stage {
    position: fixed; inset: 0; display: flex; align-items: flex-end; justify-content: flex-end; gap: 10px;
    padding: 14px; box-sizing: border-box; pointer-events: none; user-select: none;
    flex-direction: row-reverse;
    font-family: "Segoe UI", "Microsoft YaHei UI", "PingFang SC", sans-serif;
  }
  .stage.left { flex-direction: row; justify-content: flex-start; }
  .stage.top { align-items: flex-start; }
  .avatar {
    position: relative; flex: none; width: 64px; height: 64px; border-radius: 50%;
    background: #1d1a2b; box-shadow: 0 4px 14px #0007;
    animation: pop .2s cubic-bezier(.2, .9, .3, 1.2) both;
  }
  .face { width: 100%; height: 100%; border-radius: 50%; overflow: hidden; background: radial-gradient(circle at 50% 35%, #3a3358, #1d1a2b); }
  .face img { display: block; width: 170%; height: 170%; max-width: none; object-fit: cover; }
  .avatar::after {
    content: ""; position: absolute; inset: -4px; border-radius: 50%; border: 3px solid var(--ring, #8b8fa8);
  }
  .ring-listening { --ring: #3fd2c7; }
  .ring-listening::after { animation: breathe 1.4s ease-in-out infinite; }
  .ring-thinking { --ring: #9c7bff; }
  .ring-thinking::after { border-color: #9c7bff #9c7bff33 #9c7bff33 #9c7bff33; animation: spin 1s linear infinite; }
  .ring-speaking { --ring: #ff6fb1; }
  .ring-speaking::after { animation: pulse .9s ease-in-out infinite; }
  .ring-asking { --ring: #ff6fb1; }
  .ring-asking::after { animation: breathe 1s ease-in-out infinite; }
  .ring-error { --ring: #ff7a59; }
  .ring-idle { --ring: #8b8fa8; }

  .bubble {
    position: relative; max-width: 320px; min-width: 120px; padding: 9px 13px 10px; border-radius: 14px;
    background: rgba(22, 20, 34, .9); color: #f4f2fb; border: 1px solid rgba(255, 255, 255, .12);
    box-shadow: 0 6px 22px #0008; margin-bottom: 8px;
    animation: rise .18s ease-out both;
  }
  .stage.top .bubble { margin-bottom: 0; margin-top: 8px; }
  /* 尾巴指向头像 */
  .bubble::after {
    content: ""; position: absolute; bottom: 16px; right: -7px; width: 12px; height: 12px;
    background: inherit; border-right: 1px solid rgba(255, 255, 255, .12); border-top: 1px solid rgba(255, 255, 255, .12);
    transform: rotate(45deg);
  }
  .stage.left .bubble::after { right: auto; left: -7px; transform: rotate(225deg); }
  .stage.top .bubble::after { bottom: auto; top: 16px; }
  .label { display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; color: #b9b3d6; letter-spacing: .02em; }
  .phase-speaking .label, .phase-asking .label { color: #ff9fcb; }
  .phase-error .label { color: #ffab94; }
  .phase-listening .label { color: #7fe6dd; }
  .text {
    margin: 4px 0 0; font-size: 15px; line-height: 1.45; word-break: break-word;
    display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden;
  }
  .text.quoted { color: #cfcae3; font-size: 14px; }
  .heard { margin: 4px 0 0; font-size: 13px; color: #7fe6dd; }
  .countdown { margin-top: 8px; height: 3px; border-radius: 2px; background: #ffffff1f; overflow: hidden; }
  .countdown span { display: block; height: 100%; background: #ff6fb1; transform-origin: left; transition: transform .1s linear; }
  .dots { display: inline-flex; gap: 3px; }
  .dots i { width: 5px; height: 5px; border-radius: 50%; background: currentColor; animation: hop 1s ease-in-out infinite; }
  .dots i:nth-child(2) { animation-delay: .15s; }
  .dots i:nth-child(3) { animation-delay: .3s; }
  .bars { display: inline-flex; align-items: center; gap: 2px; height: 12px; }
  .bars i { width: 3px; height: 100%; border-radius: 2px; background: currentColor; animation: eq .8s ease-in-out infinite; }
  .bars i:nth-child(2) { animation-delay: .2s; }
  .bars i:nth-child(3) { animation-delay: .4s; }
  .bars i:nth-child(4) { animation-delay: .1s; }

  @keyframes pop { from { transform: scale(.6); opacity: 0; } to { transform: scale(1); opacity: 1; } }
  @keyframes rise { from { transform: translateY(6px); opacity: 0; } to { transform: none; opacity: 1; } }
  @keyframes breathe { 0%, 100% { opacity: .45; transform: scale(1); } 50% { opacity: 1; transform: scale(1.06); } }
  @keyframes pulse { 0%, 100% { opacity: .7; } 50% { opacity: 1; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes hop { 0%, 100% { transform: translateY(0); opacity: .5; } 50% { transform: translateY(-3px); opacity: 1; } }
  @keyframes eq { 0%, 100% { transform: scaleY(.35); } 50% { transform: scaleY(1); } }
  @media (prefers-reduced-motion: reduce) {
    .avatar, .bubble, .avatar::after, .dots i, .bars i { animation: none !important; }
  }
</style>
