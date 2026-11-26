<script lang="ts">
  // 悬浮窗角上她的头像（ADR 043 D6，2026-11-26 晚修订）：和聊天页同一个头像（KotoneAvatar：上传的头像、角色自带的表情、
  // 没有头像时名字的第一个字），外面一圈**进度条**——还剩多久。走完整个悬浮窗收起（后端 floating/model.rs 管时间）；
  // 鼠标停在上面停住（`paused`：这里先停住，不等后端那一版）；她在听 / 在想 / 在说时一直是满的，按她的状态呼吸 / 转圈
  // （颜色和原来气泡头像的光环一样）。
  import KotoneAvatar from "../../lib/components/KotoneAvatar.svelte";
  import { bubbleLook, type BubblePhase } from "../../lib/bubble";
  import type { FloatingRing } from "../../lib/floating";
  import { avatarLetter, customAvatar, roleFaces } from "../../lib/stores/avatar";
  import { ringFraction } from "./shape";

  let { ring, phase, busy = false, paused = false }: {
    ring: FloatingRing | null;
    /** 她现在的状态（换表情、进度条的颜色）；没有她那一行时 null。 */
    phase: BubblePhase | null;
    /** 她在忙（在听、在想、在说）：进度条满着、跟着状态动。 */
    busy?: boolean;
    /** 鼠标在上面：停住。 */
    paused?: boolean;
  } = $props();

  /** 整块、头像、圈的半径（px）。 */
  const SIZE = 56;
  const FACE = 44;
  const R = 26;
  const C = 2 * Math.PI * R;

  const mood = $derived<Exclude<BubblePhase, "hidden">>(phase && phase !== "hidden" ? phase : "asking");
  const tone = $derived(phase && phase !== "hidden" ? bubbleLook[mood].ring : "push");
  let now = $state(Date.now());
  /** 鼠标停上去那一刻：后端那一版到之前按这一刻画。 */
  let pausedAt = $state<number | null>(null);
  $effect(() => {
    if (!paused) { pausedAt = null; return; }
    if (pausedAt === null) pausedAt = Date.now();
  });
  const fraction = $derived(busy ? 1 : ringFraction(ring, pausedAt ?? now));
  const running = $derived(!!ring?.running && !paused && !busy);

  // 走着的时候每 50 毫秒重画一次（一秒 20 次，圈够顺，比每一帧省）
  $effect(() => {
    if (!running) return;
    now = Date.now();
    const timer = setInterval(() => { now = Date.now(); }, 50);
    return () => clearInterval(timer);
  });
</script>

<div class="avatar" style:width="{SIZE}px" style:height="{SIZE}px" data-testid="floating-avatar" data-phase={phase ?? ""}>
  <svg class="meter tone-{tone}" class:busy viewBox="0 0 {SIZE} {SIZE}" aria-hidden="true"
    data-testid="floating-ring" data-running={running} data-left={fraction.toFixed(3)}>
    <circle class="track" cx={SIZE / 2} cy={SIZE / 2} r={R} />
    <circle class="arc" cx={SIZE / 2} cy={SIZE / 2} r={R} stroke-dasharray={C} stroke-dashoffset={C * (1 - fraction)}
      transform="rotate(-90 {SIZE / 2} {SIZE / 2})" />
  </svg>
  <div class="face"><KotoneAvatar size={FACE} {mood} ring={false} custom={$customAvatar} faces={$roleFaces} letter={$avatarLetter} /></div>
</div>

<style>
  .avatar { position: relative; flex: none; cursor: grab; animation: pop .2s cubic-bezier(.2, .9, .3, 1.2) both; }
  .face { position: absolute; inset: 6px; border-radius: 50%; box-shadow: 0 4px 14px #0007; }
  /* 不叫 .ring：那是 Tailwind 的 ring 工具类（一圈 1 px 的白色 box-shadow），打包后的样式里有它，真机上头像外面多一个白框 */
  .meter { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; transform-origin: 50% 50%; }
  .track { fill: none; stroke: rgba(255, 255, 255, .16); stroke-width: 3; }
  .arc { fill: none; stroke: var(--ring, #ff6fb1); stroke-width: 3; stroke-linecap: round; }
  .tone-listening { --ring: #3fd2c7; }
  .tone-thinking { --ring: #9c7bff; }
  .tone-speaking, .tone-asking, .tone-push { --ring: #ff6fb1; }
  .tone-error { --ring: #ff7a59; }
  .tone-idle { --ring: #8b8fa8; }
  /* 她在忙：圈满着，在听 / 在说时呼吸，在想时转一小段 */
  .meter.busy .arc { animation: breathe 1.4s ease-in-out infinite; }
  .meter.busy.tone-speaking .arc { animation-duration: .9s; }
  .meter.busy.tone-thinking { animation: spin 1s linear infinite; }
  .meter.busy.tone-thinking .arc { stroke-dasharray: 44 120; stroke-dashoffset: 0; animation: none; }
  @keyframes breathe { 50% { opacity: .45; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes pop { from { transform: scale(.6); opacity: 0; } to { transform: scale(1); opacity: 1; } }
  @media (prefers-reduced-motion: reduce) {
    .avatar, .meter, .arc { animation: none !important; }
  }
</style>
