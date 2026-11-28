<script lang="ts">
  // 悬浮窗里她的那句话（ADR 043 D6；原来是单独的对话气泡窗口 routes/companion，ADR 018）：状态行 + 正文，是贴着头像的
  // 气泡（头像在 Avatar.svelte）。状态由后端 bubble.rs 推送（kotone://bubble）。
  // `compact`：聊天小窗最下面的一行（在听、听清了、等你回答、收尾、出错——对话里还没有的），不画气泡底。
  import { onMount } from "svelte";
  import { askRemaining, bubbleBody, bubbleLook, type BubbleSnapshot } from "../../lib/bubble";

  let { snap, compact = false }: { snap: BubbleSnapshot; compact?: boolean } = $props();

  let now = $state(Date.now());
  const look = $derived(snap.phase !== "hidden" ? bubbleLook[snap.phase] : null);
  const body = $derived(bubbleBody(snap));
  const remaining = $derived(askRemaining(snap, now));

  onMount(() => {
    const timer = setInterval(() => { if (snap.phase === "asking") now = Date.now(); }, 100);
    return () => clearInterval(timer);
  });
</script>

{#if look}
  <div class="bubble phase-{snap.phase}" class:compact role="status" aria-live="polite" data-phase={snap.phase}
    data-testid={compact ? "floating-her-status" : "floating-her-bubble"}>
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

<style>
  .bubble {
    position: relative; max-width: 300px; min-width: 96px; padding: 8px 12px 9px; border-radius: 14px;
    background: var(--floating-bubble, #1d201d); color: var(--floating-bubble-ink, #eeefe6);
    border: 1px solid var(--floating-bubble-border, #ffffff22);
    box-shadow: var(--floating-shadow, 0 1px 2px rgb(0 0 0 / .3), 0 3px 6px -1px rgb(0 0 0 / .45));
  }
  /* 聊天小窗最下面的一行：跟着小窗的配色 */
  .bubble.compact {
    max-width: none; min-width: 0; padding: 4px 2px 2px; border: 0; border-radius: 0; background: none; box-shadow: none;
    color: var(--foreground); animation: none;
  }
  .label { display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; color: #b9b3d6; letter-spacing: .02em; }
  .phase-speaking .label, .phase-asking .label { color: #ff9fcb; }
  .phase-error .label { color: #ffab94; }
  .phase-listening .label { color: #7fe6dd; }
  .compact .label { color: var(--muted-foreground); }
  .compact.phase-listening .label, .compact.phase-recognizing .label { color: var(--primary); }
  .compact.phase-error .label { color: var(--destructive); }
  .text {
    margin: 3px 0 0; font-size: 14px; line-height: 1.45; word-break: break-word;
    display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden;
  }
  .text.quoted { color: #cfcae3; font-size: 13px; }
  .compact .text { font-size: 13px; -webkit-line-clamp: 2; line-clamp: 2; }
  .compact .text.quoted { color: var(--muted-foreground); }
  .heard { margin: 3px 0 0; font-size: 12px; color: #7fe6dd; }
  .compact .heard { color: var(--primary); }
  .countdown { margin-top: 6px; height: 3px; border-radius: 2px; background: #ffffff1f; overflow: hidden; }
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

  @keyframes hop { 0%, 100% { transform: translateY(0); opacity: .5; } 50% { transform: translateY(-3px); opacity: 1; } }
  @keyframes eq { 0%, 100% { transform: scaleY(.35); } 50% { transform: scaleY(1); } }
  @media (prefers-reduced-motion: reduce) {
    .bubble, .dots i, .bars i { animation: none !important; }
  }
</style>
