<script lang="ts">
  /*
   * 关于页（琴音角色展示页）：在设置页容器内向下滚动。
   * ① Hero：立绘 + 「Kotone 琴音」+ slogan + 版本号（桌面端动态读取）
   * ② 角色介绍：职业 / 性格 / 战绩 / 口头禅要点卡片
   * ③ 招牌母题：声波 / 聊天气泡 / 键轴十字（pattern 底纹）+ 品牌色板
   * ④ 贴纸画廊：九宫格，hover 轻微放大摇摆
   * ⑤ 关于：版本 / GitHub / 版权
   * 分区进入视口时淡入上移（IntersectionObserver），reduced-motion 降级。
   */
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { isTauri } from "../../../lib/ipc";
  import cutout from "../../../assets/brand/kotone-cutout.png";
  import patternWave from "../../../assets/brand/patterns/wave.png";
  import patternBubble from "../../../assets/brand/patterns/bubble.png";
  import patternSwitch from "../../../assets/brand/patterns/switch.png";
  import stickerHello from "../../../assets/brand/stickers/hello.png";
  import stickerCheering from "../../../assets/brand/stickers/cheering.png";
  import stickerRelax from "../../../assets/brand/stickers/relax.png";
  import stickerProud from "../../../assets/brand/stickers/proud.png";
  import stickerAmazed from "../../../assets/brand/stickers/amazed.png";
  import stickerCurious from "../../../assets/brand/stickers/curious.png";
  import stickerPointing from "../../../assets/brand/stickers/pointing.png";
  import stickerThinking from "../../../assets/brand/stickers/thinking.png";
  import stickerSleepy from "../../../assets/brand/stickers/sleepy.png";

  /** 静态兜底版本（与 package.json 同步）；桌面端启动后替换为真实版本 */
  let version = $state("0.1.1");

  onMount(async () => {
    if (!isTauri) return;
    version = await getVersion().catch(() => version);
  });

  /** 滚动进入视口时淡入上移（一次性） */
  function reveal(node: HTMLElement) {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          node.classList.add("reveal-in");
          io.disconnect();
        }
      },
      { threshold: 0.15 },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  const traits = [
    {
      title: "职业",
      text: "游戏主播——以超快打字速度闻名，观众说她是「人形语音输入法」",
    },
    {
      title: "性格",
      text: "活力四射、热情温暖，带一点好胜心；下播后会窝在椅子上安静看回放",
    },
    {
      title: "战绩",
      text: "极速打字比赛最好成绩 214 WPM——练手速就像别人练枪法，日复一日",
    },
    {
      title: "口头禅",
      text: "「收到，已发送！✨」",
    },
  ];

  const motifs = [
    {
      img: patternWave,
      name: "声波波形",
      desc: "青到品红渐变的水平波形线条，源自语音音频波形",
    },
    {
      img: patternBubble,
      name: "游戏聊天气泡",
      desc: "深色底上散落的霓虹青气泡轮廓，周边与页脚底纹",
    },
    {
      img: patternSwitch,
      name: "机械键轴十字",
      desc: "键轴俯视的十字 stem 结构，四方连续排列",
    },
  ];

  const colors = [
    { hex: "#00E5FF", name: "霓虹青", role: "主色 · 高亮强调" },
    { hex: "#FF2D78", name: "品红", role: "强调 · 交互反馈" },
    { hex: "#7B2FFF", name: "紫", role: "点缀" },
    { hex: "#1A1A2E", name: "蓝黑", role: "暗色背景基调" },
  ];

  const stickers = [
    { src: stickerHello, name: "打招呼" },
    { src: stickerCheering, name: "打 call" },
    { src: stickerRelax, name: "放松" },
    { src: stickerProud, name: "得意" },
    { src: stickerAmazed, name: "惊呆" },
    { src: stickerCurious, name: "好奇" },
    { src: stickerPointing, name: "指点" },
    { src: stickerThinking, name: "思考" },
    { src: stickerSleepy, name: "犯困" },
  ];
</script>

<div class="relative px-6 py-5">
  <!-- 底纹：wave 无缝 tile，极低透明度 -->
  <div
    class="pointer-events-none absolute inset-0 opacity-[0.05]"
    style:background-image="url({patternWave})"
    style:background-size="220px"
  ></div>

  <!-- ① Hero：立绘 + 名称 + slogan + 版本 -->
  <section class="relative flex flex-col items-center pt-2 text-center">
    <div class="relative">
      <img
        src={cutout}
        alt="Kotone 立绘"
        class="h-56 object-contain drop-shadow-[0_0_28px_rgba(0,229,255,0.3)]"
      />
      <img src={stickerHello} alt="" class="absolute -left-14 top-6 h-14 w-14 -rotate-12 object-contain" />
      <img src={stickerCheering} alt="" class="absolute -right-14 top-2 h-14 w-14 rotate-12 object-contain" />
    </div>
    <h1 class="mt-4 text-2xl font-bold">
      Kotone <span class="kotone-gradient-text">琴音</span>
    </h1>
    <p class="mt-1 text-[13px] text-white/60">打字比打游戏快的主播</p>
    <span class="mt-3 rounded-full bg-white/8 px-3 py-1 text-[11px] text-white/55 ring-1 ring-white/12">
      v{version}
    </span>
    <blockquote class="kotone-panel mt-4 max-w-sm px-5 py-3 text-[13px] leading-relaxed text-white/75">
      「想说的话，一秒都别等。<br />你专注赢下比赛，聊天框交给我。」
    </blockquote>
  </section>

  <!-- ② 角色介绍 -->
  <section use:reveal class="reveal relative mt-8">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">角色介绍</h2>
    <div class="mt-3 grid grid-cols-2 gap-3">
      {#each traits as t}
        <div class="kotone-card p-3.5">
          <p class="text-xs font-semibold text-kotone-cyan/90">{t.title}</p>
          <p class="mt-1 text-[12px] leading-relaxed text-white/65">{t.text}</p>
        </div>
      {/each}
    </div>
  </section>

  <!-- ③ 招牌母题 + 品牌色板 -->
  <section use:reveal class="reveal relative mt-8">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">招牌元素</h2>
    <div class="mt-3 grid grid-cols-3 gap-3">
      {#each motifs as m}
        <div class="kotone-card overflow-hidden">
          <div
            class="h-16 ring-1 ring-white/10"
            style:background-image="url({m.img})"
            style:background-size="96px"
          ></div>
          <div class="p-3">
            <p class="text-xs font-semibold">{m.name}</p>
            <p class="mt-1 text-[11px] leading-relaxed text-white/45">{m.desc}</p>
          </div>
        </div>
      {/each}
    </div>
    <div class="mt-3 grid grid-cols-4 gap-3">
      {#each colors as c}
        <div class="kotone-card flex items-center gap-2.5 p-3">
          <span
            class="h-7 w-7 shrink-0 rounded-lg ring-1 ring-white/15"
            style:background-color={c.hex}
          ></span>
          <div class="min-w-0">
            <p class="truncate text-[11px] font-semibold">{c.name} <span class="text-white/40">{c.hex}</span></p>
            <p class="truncate text-[10px] text-white/40">{c.role}</p>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <!-- ④ 贴纸画廊 -->
  <section use:reveal class="reveal relative mt-8">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">贴纸画廊</h2>
    <div class="mt-3 grid grid-cols-3 gap-3">
      {#each stickers as s}
        <div
          class="kotone-card flex flex-col items-center gap-2 p-3.5 transition duration-200 hover:-translate-y-1 hover:scale-[1.04] hover:-rotate-2 hover:shadow-glow-cyan"
        >
          <img src={s.src} alt="琴音贴纸·{s.name}" class="h-16 w-16 object-contain" />
          <p class="text-[11px] text-white/50">{s.name}</p>
        </div>
      {/each}
    </div>
  </section>

  <!-- ⑤ 关于 -->
  <section use:reveal class="reveal relative mt-8 pb-2">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">关于</h2>
    <div class="kotone-panel mt-3 flex flex-col items-center gap-2 p-4 text-center text-[12px] text-white/50">
      <p>Kotone 琴音 v{version} · 本地优先 · 隐私安全</p>
      <a
        class="text-kotone-cyan/80 underline underline-offset-2 transition hover:text-kotone-cyan"
        href="https://github.com/l1veIn/kotone"
        target="_blank"
        rel="noreferrer"
      >github.com/l1veIn/kotone</a>
      <p class="mt-1 text-[10px] text-white/30">© 2026 Kotone 项目 · 第三方组件声明见 THIRD_PARTY_NOTICES</p>
    </div>
  </section>
</div>

<style>
  /* 分区滚动进入视口时淡入上移（由 use:reveal 加 .reveal-in 触发） */
  .reveal {
    opacity: 0;
    transform: translateY(14px);
    transition:
      opacity 0.5s ease,
      transform 0.5s ease;
  }
  .reveal:global(.reveal-in) {
    opacity: 1;
    transform: translateY(0);
  }
  @media (prefers-reduced-motion: reduce) {
    .reveal {
      opacity: 1;
      transform: none;
      transition: none;
    }
  }
</style>
