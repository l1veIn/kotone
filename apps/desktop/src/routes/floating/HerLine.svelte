<script lang="ts">
  // 悬浮窗气泡里她的那一行（ADR 043 D6；原来是单独的对话气泡窗口 routes/companion，ADR 018）：头像 + 状态行 + 正文。
  // 状态由后端 bubble.rs 推送（kotone://bubble）；头像贴着悬浮窗贴着的那一边，字多字少只改变气泡往屏幕里延伸多少。
  import { onMount } from "svelte";
  import { askRemaining, bubbleBody, bubbleLook, type BubbleSnapshot } from "../../lib/bubble";
  import { getSettings } from "../../lib/ipc";
  import { workspaceAvatar } from "../../lib/workspace";
  import { bundledFaces, placeholderLetter, type RoleFaces } from "../../lib/roles";

  let { snap, left = false }: { snap: BubbleSnapshot; left?: boolean } = $props();

  /** 玩家上传的头像（每次出来时读一次设置，换了头像下一句就生效）。 */
  let custom = $state<string | null>(null);
  /** 其他角色没有头像时的占位字。 */
  let letter = $state<string | null>(null);
  /** 自带形象的角色（大肥鱼、Claude大小姐）：按状态换表情。 */
  let faces = $state<RoleFaces | null>(null);
  let customName = "";
  let now = $state(Date.now());

  const look = $derived(snap.phase !== "hidden" ? bubbleLook[snap.phase] : null);
  const body = $derived(bubbleBody(snap));
  const remaining = $derived(askRemaining(snap, now));

  async function refreshAvatar() {
    try {
      const agent = (await getSettings()).agent;
      letter = placeholderLetter(agent?.roleId, agent?.identity?.name, agent?.identity?.avatar);
      faces = bundledFaces(agent?.roleId, agent?.identity?.avatar);
      const name = agent?.identity?.avatar ?? "";
      if (name === customName) return;
      customName = name;
      const bytes = name ? await workspaceAvatar(name) : null;
      if (custom) URL.revokeObjectURL(custom);
      custom = bytes ? URL.createObjectURL(new Blob([bytes as BlobPart])) : null;
    } catch { /* 读不到就用内置表情 */ }
  }

  onMount(() => {
    void refreshAvatar();
    const timer = setInterval(() => { if (snap.phase === "asking") now = Date.now(); }, 100);
    return () => clearInterval(timer);
  });
</script>

{#if look}
  <div class="her" class:left data-testid="floating-her">
    {#key look.face}
      <div class="avatar ring-{look.ring}" aria-hidden="true">
        <div class="face">
          {#if custom}<img src={custom} alt="" class="custom" />
          {:else if faces && snap.phase !== "hidden"}<img src={faces[snap.phase]} alt="" class="custom" />
          {:else if letter}<span class="letter">{letter}</span>
          {:else}<img src={look.face} alt="" style="margin: calc(50% - {look.fy * 170}%) 0 0 calc(50% - {look.fx * 170}%)" />{/if}
        </div>
      </div>
    {/key}
    <div class="bubble phase-{snap.phase}" role="status" aria-live="polite" data-phase={snap.phase} data-testid="floating-her-bubble">
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
  </div>
{/if}

<style>
  /* 头像贴着悬浮窗贴着的那一边：右边的角 row-reverse（头像在右），左边的角 row */
  .her { display: flex; flex-direction: row-reverse; align-items: flex-end; gap: 8px; }
  .her.left { flex-direction: row; }
  .avatar {
    position: relative; flex: none; width: 48px; height: 48px; border-radius: 50%;
    background: #1d1a2b; box-shadow: 0 4px 14px #0007;
    animation: pop .2s cubic-bezier(.2, .9, .3, 1.2) both;
  }
  .face { width: 100%; height: 100%; border-radius: 50%; overflow: hidden; background: radial-gradient(circle at 50% 35%, #3a3358, #1d1a2b); }
  .face img { display: block; width: 170%; height: 170%; max-width: none; object-fit: cover; }
  .face img.custom { width: 100%; height: 100%; margin: 0; }
  .letter { display: flex; width: 100%; height: 100%; align-items: center; justify-content: center; font-size: 18px; font-weight: 600; color: #f1edff; }
  .avatar::after {
    content: ""; position: absolute; inset: -3px; border-radius: 50%; border: 2px solid var(--ring, #8b8fa8);
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
    position: relative; max-width: 300px; min-width: 96px; padding: 8px 12px 9px; border-radius: 14px;
    background: rgba(22, 20, 34, .92); color: #f4f2fb; border: 1px solid rgba(255, 255, 255, .12);
    box-shadow: 0 6px 18px #0007; margin-bottom: 4px;
    animation: rise .18s ease-out both;
  }
  .label { display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; color: #b9b3d6; letter-spacing: .02em; }
  .phase-speaking .label, .phase-asking .label { color: #ff9fcb; }
  .phase-error .label { color: #ffab94; }
  .phase-listening .label { color: #7fe6dd; }
  .text {
    margin: 3px 0 0; font-size: 14px; line-height: 1.45; word-break: break-word;
    display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden;
  }
  .text.quoted { color: #cfcae3; font-size: 13px; }
  .heard { margin: 3px 0 0; font-size: 12px; color: #7fe6dd; }
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
