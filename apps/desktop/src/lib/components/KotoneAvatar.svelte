<script lang="ts">
  // 琴音的头像：和右下角对话气泡同一个样子（圆形、贴纸里的脸、按状态换表情和光环颜色）。
  // 玩家上传了头像就用上传的（没有表情变化，光环照常）。聊天、空白对话页、「她是谁」、气泡都用它。
  import { bubbleLook, type BubblePhase } from "../bubble";

  let { size = 40, mood = "asking", ring = true, custom = null, animate = false }: {
    size?: number;
    /** 表情：和气泡的状态一致（asking = 招手，speaking = 指着说话……）。 */
    mood?: Exclude<BubblePhase, "hidden">;
    ring?: boolean;
    /** 玩家上传的头像（object URL）；null = 用内置表情。 */
    custom?: string | null;
    /** 光环动起来（在听 / 在想 / 在说）。 */
    animate?: boolean;
  } = $props();
  const look = $derived(bubbleLook[mood]);
</script>

<div class="avatar ring-{look.ring}" class:ringed={ring} class:animate style:width={`${size}px`} style:height={`${size}px`}
  style:--ring-width={`${Math.max(1.5, size / 22)}px`} aria-hidden="true" data-testid="kotone-avatar">
  <div class="face">
    {#if custom}
      <img src={custom} alt="" class="custom" />
    {:else}
      <img src={look.face} alt="" style="margin: calc(50% - {look.fy * 170}%) 0 0 calc(50% - {look.fx * 170}%)" />
    {/if}
  </div>
</div>

<style>
  .avatar { position: relative; flex: none; border-radius: 50%; background: #1d1a2b; }
  .face { width: 100%; height: 100%; border-radius: 50%; overflow: hidden; background: radial-gradient(circle at 50% 35%, #3a3358, #1d1a2b); }
  .face img { display: block; width: 170%; height: 170%; max-width: none; object-fit: cover; }
  .face img.custom { width: 100%; height: 100%; margin: 0; }
  .ringed::after {
    content: ""; position: absolute; inset: calc(var(--ring-width) * -1.3); border-radius: 50%;
    border: var(--ring-width) solid var(--ring, #8b8fa8);
  }
  .ring-listening { --ring: #3fd2c7; }
  .ring-thinking { --ring: #9c7bff; }
  .ring-speaking, .ring-asking { --ring: #ff6fb1; }
  .ring-error { --ring: #ff7a59; }
  .ring-idle { --ring: #8b8fa8; }
  .animate.ring-listening::after, .animate.ring-asking::after { animation: breathe 1.4s ease-in-out infinite; }
  .animate.ring-thinking::after { border-color: #9c7bff #9c7bff33 #9c7bff33 #9c7bff33; animation: spin 1s linear infinite; }
  .animate.ring-speaking::after { animation: breathe .9s ease-in-out infinite; }
  @keyframes breathe { 50% { opacity: .45; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .avatar::after { animation: none !important; } }
</style>
