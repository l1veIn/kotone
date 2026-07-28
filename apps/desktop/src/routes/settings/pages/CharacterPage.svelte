<script lang="ts">
  /*
   * 角色详情页（琴音档案 · CHARACTER FILE）——从「关于」页进入的全屏接管视图。
   * 设计参考 repochan starter「character-game-page」的分区语言（编号区头 / HUD
   * 数据面板 / 趣闻档案卡 / 名台词区），按桌面端 800×600 小窗口重做：
   * 单列竖向滚动、约 560px 内容宽、小字号高密度，幽灵大字与弹幕跑马灯做减法。
   * 文案数据源：lib/data/persona.ts（同步自 .repochan/persona/current.json）。
   */
  import { spotlight } from "../../../lib/actions/spotlight";
  import {
    heroChips,
    stats,
    story,
    personality,
    flaws,
    world,
    funFacts,
    abilities,
    passiveSkill,
    likes,
    voiceLines,
    danmaku,
  } from "../../../lib/data/persona";
  import cutout from "../../../assets/brand/kotone-cutout.png";
  import relayRoom from "../../../assets/brand/relay-room-bg.png";
  import stickerHello from "../../../assets/brand/stickers/hello.png";
  import stickerCheering from "../../../assets/brand/stickers/cheering.png";
  import stickerProud from "../../../assets/brand/stickers/proud.png";
  import stickerRelax from "../../../assets/brand/stickers/relax.png";
  import stickerAmazed from "../../../assets/brand/stickers/amazed.png";
  import stickerCurious from "../../../assets/brand/stickers/curious.png";
  import stickerPointing from "../../../assets/brand/stickers/pointing.png";
  import stickerThinking from "../../../assets/brand/stickers/thinking.png";
  import stickerSleepy from "../../../assets/brand/stickers/sleepy.png";
  import propKeycap from "../../../assets/brand/props/keycap.webp";
  import propKeyboard from "../../../assets/brand/props/keyboard.webp";
  import propHeadset from "../../../assets/brand/props/headset.webp";
  import propRamen from "../../../assets/brand/props/ramen.webp";
  import propGarter from "../../../assets/brand/props/garter.webp";
  import propPhone from "../../../assets/brand/props/phone.webp";
  import propLetter from "../../../assets/brand/props/letter.webp";
  import propMic from "../../../assets/brand/props/mic.webp";
  import propDesk from "../../../assets/brand/props/desk.webp";
  import posterRiso from "../../../assets/brand/posters/poster-riso.webp";
  import posterRelay from "../../../assets/brand/posters/poster-relay.webp";
  import hero2026 from "../../../assets/brand/posters/hero-2026.webp";
  import poster2026 from "../../../assets/brand/posters/poster-2026.webp";

  import patternBubble from "../../../assets/brand/patterns/bubble.png";
  import patternSwitch from "../../../assets/brand/patterns/switch.png";
  import patternWave from "../../../assets/brand/patterns/wave.png";
  let { onBack }: { onBack: () => void } = $props();

  /** 滚动进入视口时淡入上移（一次性，与关于页同一模式） */
  function reveal(node: HTMLElement) {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          node.classList.add("reveal-in");
          io.disconnect();
        }
      },
      { threshold: 0.12 },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  const sections = [
    { num: "01", title: "面板数据", sub: "STAT" },
    { num: "02", title: "她的故事", sub: "STORY" },
    { num: "03", title: "性格与弱点", sub: "PERSONA" },
    { num: "04", title: "世界 · 中继站", sub: "WORLD" },
    { num: "05", title: "趣闻档案", sub: "FACTS" },
    { num: "06", title: "招牌技能", sub: "SKILLS" },
    { num: "07", title: "喜好", sub: "LIKES" },
    { num: "08", title: "装备收藏", sub: "GEAR" },
    { num: "09", title: "贴纸画廊", sub: "STICKERS" },
    { num: "10", title: "海报墙", sub: "GALLERY" },
    { num: "11", title: "名台词", sub: "VOICE" },
    { num: "12", title: "招牌元素", sub: "BRAND" },
  ] as const;

  /** 弹幕跑马灯轨道（首尾相接两份，CSS 平移循环） */
  const danmakuTrack = [...danmaku, ...danmaku];

  /** 08 装备收藏（ord-ui-props 九宫格切片，白底拍立得式卡片） */
  const gear = [
    {
      src: propKeycap,
      name: "键帽吊坠",
      desc: "封存 Cherry MX 青轴轴心的手工滴胶",
    },
    {
      src: propKeyboard,
      name: "Kotone Blue 键盘",
      desc: "自润轴体，死也不换的手感",
    },
    { src: propHeadset, name: "游戏耳机", desc: "声波贴纸定制耳罩，多挂少戴" },
    { src: propRamen, name: "夜食泡面", desc: "辛拉面加生鸡蛋，直播标配夜宵" },
    {
      src: propGarter,
      name: "科技腿环",
      desc: "LED 小屏，平时显示时间或颜文字",
    },
    { src: propPhone, name: "弹幕手机", desc: "下播后刷粉丝二创专用" },
    { src: propLetter, name: "粉丝来信", desc: "贴满一整面墙的手写信" },
    { src: propMic, name: "电容麦克风", desc: "懒得开麦，但设备从不含糊" },
    { src: propDesk, name: "中继站桌面", desc: "RGB 双屏 + 猫咪音箱的直播间" },
  ];

  /** 09 贴纸画廊（自「关于」页迁入） */
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

  /** 10 海报墙（riso 印刷风 / 中继站夜景 / 2026 README hero / 2026 波普海报） */
  const posters = [
    { src: posterRiso, name: "KOTONE 琴音 · Riso 印刷海报", tag: "RISO" },
    { src: posterRelay, name: "中继站 · 深夜直播中", tag: "SCENE" },
    { src: hero2026, name: "霓虹直播间 · README Hero 2026", tag: "HERO" },
    { src: poster2026, name: "波普拼贴海报 · 2026", tag: "POP" },
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
</script>

<div class="relative min-h-full">
  <!-- 顶栏：返回 + 档案标头 + 分区跳转（吸顶，滚动时保持可返回/可跳转） -->
  <div
    class="sticky top-0 z-20 border-b border-white/8 bg-kotone-deep/85 backdrop-blur-md"
  >
    <div class="flex items-center gap-3 px-5 py-2.5">
      <button
        class="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-semibold text-white/70
          transition hover:bg-white/8 hover:text-kotone-cyan"
        onclick={onBack}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          class="h-3.5 w-3.5"
          aria-hidden="true"
        >
          <path
            d="M15 6l-6 6 6 6"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
        返回关于
      </button>
      <span
        class="text-[10px] font-semibold tracking-[0.22em] text-kotone-cyan/60"
        >CHARACTER FILE</span
      >
      <span
        class="ml-auto rounded-full bg-white/6 px-2.5 py-0.5 text-[10px] text-white/35 ring-1 ring-white/10"
      >
        repochan.persona.v2
      </span>
    </div>
    <!-- 分区跳转芯片条（横向滚动） -->
    <nav
      class="kotone-scroll flex gap-1.5 overflow-x-auto px-5 pt-1 pb-2"
      aria-label="档案分区"
    >
      {#each sections as s}
        <button
          class="shrink-0 rounded-full bg-white/6 px-2.5 py-1 text-[10px] text-white/55 ring-1 ring-white/10 transition
            hover:bg-kotone-cyan/15 hover:text-kotone-cyan hover:ring-kotone-cyan/40"
          onclick={() =>
            document
              .getElementById(`cf-${s.num}`)
              ?.scrollIntoView({ behavior: "smooth", block: "start" })}
        >
          {s.num} {s.title}
        </button>
      {/each}
    </nav>
  </div>

  <!-- Hero：幽灵大字 + 立绘 + 姓名 + 口头禅 + 信息徽章 -->
  <section
    class="relative flex flex-col items-center overflow-hidden px-6 pt-6 pb-4 text-center"
  >
    <span
      class="pointer-events-none absolute top-2 left-1/2 -translate-x-1/2 text-[92px] font-black tracking-tight whitespace-nowrap select-none"
      style="-webkit-text-stroke: 1.5px rgba(0,229,255,0.14); color: transparent;"
      aria-hidden="true">KOTONE</span
    >

    <div class="relative mt-10">
      <img
        src={cutout}
        alt="Kotone 琴音 立绘"
        class="h-64 object-contain drop-shadow-[0_0_36px_rgba(0,229,255,0.35)]"
      />
      <img
        src={stickerHello}
        alt=""
        class="float-slow absolute -left-16 top-8 h-14 w-14 -rotate-12 object-contain"
        aria-hidden="true"
      />
      <img
        src={stickerCheering}
        alt=""
        class="float-fast absolute -right-16 top-4 h-14 w-14 rotate-12 object-contain"
        aria-hidden="true"
      />
      <img
        src={stickerProud}
        alt=""
        class="float-slow absolute -right-12 bottom-6 h-12 w-12 rotate-6 object-contain"
        aria-hidden="true"
      />
    </div>

    <p
      class="mt-3 text-[11px] font-semibold tracking-[0.18em] text-kotone-cyan/80"
    >
      打字比打游戏快的主播
    </p>
    <h1 class="mt-1 text-3xl font-black">
      琴音 <span class="kotone-gradient-text">Kotone</span>
    </h1>
    <p class="mt-0.5 text-[12px] text-white/40">
      ことね · 游戏主播 · 「人形语音输入法」
    </p>

    <span
      class="mt-3 rounded-full bg-kotone-pink/12 px-4 py-1.5 text-[13px] font-bold text-kotone-pink ring-1 ring-kotone-pink/30"
      >「收到，已发送！✨」</span
    >

    <div class="mt-4 flex flex-wrap justify-center gap-2">
      {#each heroChips as chip}
        <span
          class="rounded-full bg-white/7 px-3 py-1 text-[11px] text-white/60 ring-1 ring-white/12"
          >{chip}</span
        >
      {/each}
    </div>
  </section>

  <div class="px-6 pb-6">
    <!-- 01 面板数据 -->
    {@render secHead(sections[0])}
    <div use:reveal class="reveal mt-3 grid grid-cols-4 gap-2.5">
      {#each stats as s}
        <div class="kotone-card kotone-spotlight p-3 text-center" use:spotlight>
          <p class="text-[10px] text-white/40">{s.k}</p>
          <p class="mt-1 text-xl font-black text-kotone-cyan">
            {s.v}<span
              class="ml-0.5 text-[10px] font-semibold text-kotone-cyan/60"
              >{s.unit}</span
            >
          </p>
          <p class="mt-1 text-[10px] leading-snug text-white/35">{s.note}</p>
        </div>
      {/each}
    </div>

    <!-- 02 她的故事 -->
    {@render secHead(sections[1])}
    <div use:reveal class="reveal kotone-panel mt-3 p-4">
      {#each story.paragraphs as p}
        <p class="mb-3 text-[12.5px] leading-relaxed text-white/70 last:mb-0">
          {p}
        </p>
      {/each}
      <blockquote class="mt-4 border-l-2 border-kotone-cyan/50 pl-3">
        <p class="text-[13px] font-semibold text-white/85">「{story.quote}」</p>
        <cite class="mt-1 block text-[10px] text-white/35 not-italic"
          >{story.quoteCaption}</cite
        >
      </blockquote>
      <p
        class="mt-4 text-right text-[11px] tracking-widest text-kotone-pink/70"
      >
        —— {story.motto}
      </p>
    </div>

    <!-- 03 性格与弱点 -->
    {@render secHead(sections[2])}
    <div use:reveal class="reveal kotone-panel mt-3 p-4">
      <p class="text-[12.5px] leading-relaxed text-white/70">{personality}</p>
    </div>
    <div use:reveal class="reveal mt-3 grid grid-cols-2 gap-3">
      {#each flaws as f}
        <div class="kotone-card kotone-spotlight p-3.5" use:spotlight>
          <div class="flex items-center justify-between gap-2">
            <p class="text-xs font-bold text-kotone-pink/90">{f.name}</p>
            <span
              class="rounded bg-kotone-pink/10 px-1.5 py-0.5 text-[9px] font-semibold tracking-wider text-kotone-pink/60 ring-1 ring-kotone-pink/20"
              >{f.type}</span
            >
          </div>
          <p class="mt-1.5 text-[11.5px] leading-relaxed text-white/55">
            {f.desc}
          </p>
        </div>
      {/each}
    </div>

    <!-- 04 世界 · 中继站 -->
    {@render secHead(sections[3])}
    <div use:reveal class="reveal kotone-card mt-3 overflow-hidden">
      <div class="relative">
        <img
          src={relayRoom}
          alt="中继站——琴音的直播间"
          class="h-44 w-full object-cover"
        />
        <div
          class="absolute inset-0 bg-gradient-to-t from-kotone-deep via-kotone-deep/30 to-transparent"
        ></div>
        <div class="absolute bottom-2.5 left-3.5">
          <p class="text-sm font-black">
            {world.name}
            <span
              class="text-[10px] font-semibold tracking-widest text-kotone-cyan/70"
              >{world.nameEn}</span
            >
          </p>
        </div>
      </div>
      <div class="p-4">
        <p class="text-[12px] leading-relaxed text-white/65">
          {world.atmosphere}
        </p>
        <p
          class="mt-2.5 border-t border-white/8 pt-2.5 text-[11.5px] leading-relaxed text-white/45"
        >
          {world.relationship}
        </p>
      </div>
    </div>

    <!-- 05 趣闻档案 -->
    {@render secHead(sections[4])}
    <div use:reveal class="reveal mt-3 flex flex-col gap-2.5">
      {#each funFacts as fact, i}
        <div
          class="kotone-card kotone-spotlight flex items-start gap-3 p-3.5"
          use:spotlight
        >
          <span class="text-lg font-black text-kotone-violet/70"
            >{String(i + 1).padStart(2, "0")}</span
          >
          <p class="text-[12px] leading-relaxed text-white/65">{fact}</p>
        </div>
      {/each}
    </div>

    <!-- 06 招牌技能 -->
    {@render secHead(sections[5])}
    <div use:reveal class="reveal mt-3 flex flex-col gap-2.5">
      {#each abilities as a}
        <div class="kotone-card kotone-spotlight p-3.5" use:spotlight>
          <div class="flex items-center justify-between gap-2">
            <p class="text-[13px] font-bold text-kotone-cyan">{a.name}</p>
            <span
              class="shrink-0 rounded bg-kotone-cyan/10 px-1.5 py-0.5 text-[9px] font-semibold tracking-wider text-kotone-cyan/70 ring-1 ring-kotone-cyan/25"
              >{a.type}</span
            >
          </div>
          <p class="mt-1.5 text-[11.5px] leading-relaxed text-white/55">
            {a.desc}
          </p>
        </div>
      {/each}
      <div class="kotone-card kotone-spotlight p-3.5" use:spotlight>
        <div class="flex items-center justify-between gap-2">
          <p class="text-[13px] font-bold text-kotone-violet">
            {passiveSkill.name}
          </p>
          <span
            class="shrink-0 rounded bg-kotone-violet/10 px-1.5 py-0.5 text-[9px] font-semibold tracking-wider text-kotone-violet/70 ring-1 ring-kotone-violet/25"
            >{passiveSkill.type}</span
          >
        </div>
        <p class="mt-1.5 text-[11.5px] leading-relaxed text-white/55">
          {passiveSkill.desc}
        </p>
      </div>
    </div>

    <!-- 07 喜好 -->
    {@render secHead(sections[6])}
    <div use:reveal class="reveal mt-3 flex flex-wrap gap-2">
      {#each likes as like}
        <span
          class="kotone-card inline-flex items-baseline gap-1.5 px-3 py-1.5 text-[11.5px]"
        >
          <span class="font-semibold text-white/80">{like.label}</span>
          <span class="text-[10px] text-white/35">{like.note}</span>
        </span>
      {/each}
    </div>

    <!-- 08 装备收藏 -->
    {@render secHead(sections[7])}
    <div use:reveal class="reveal mt-3 grid grid-cols-3 gap-3">
      {#each gear as g}
        <figure class="kotone-card overflow-hidden">
          <div class="aspect-square bg-white">
            <img
              src={g.src}
              alt="琴音的装备·{g.name}"
              class="h-full w-full object-cover"
              loading="lazy"
            />
          </div>
          <figcaption class="p-2.5">
            <p class="text-[11px] font-semibold">{g.name}</p>
            <p class="mt-0.5 text-[10px] leading-snug text-white/40">
              {g.desc}
            </p>
          </figcaption>
        </figure>
      {/each}
    </div>

    <!-- 09 贴纸画廊（自「关于」页迁入） -->
    {@render secHead(sections[8])}
    <div use:reveal class="reveal mt-3 grid grid-cols-3 gap-3">
      {#each stickers as s}
        <div
          class="kotone-card flex flex-col items-center gap-2 p-3.5 transition duration-200 hover:-translate-y-1 hover:scale-[1.04] hover:-rotate-2 hover:shadow-glow-cyan"
        >
          <img
            src={s.src}
            alt="琴音贴纸·{s.name}"
            class="h-16 w-16 object-contain"
            loading="lazy"
          />
          <p class="text-[11px] text-white/50">{s.name}</p>
        </div>
      {/each}
    </div>

    <!-- 10 海报墙 -->
    {@render secHead(sections[9])}
    <div use:reveal class="reveal mt-3 flex flex-col gap-3">
      {#each posters as p}
        <figure class="kotone-card overflow-hidden">
          <div class="relative">
            <img
              src={p.src}
              alt={p.name}
              class="w-full object-cover"
              loading="lazy"
            />
            <span
              class="absolute top-2 right-2 rounded bg-kotone-deep/70 px-2 py-0.5 text-[9px] font-bold tracking-[0.2em] text-kotone-cyan/90 ring-1 ring-kotone-cyan/30 backdrop-blur-sm"
              >{p.tag}</span
            >
          </div>
          <figcaption class="px-3.5 py-2.5 text-[11px] text-white/50">
            {p.name}
          </figcaption>
        </figure>
      {/each}
    </div>

    <!-- 11 名台词 -->
    {@render secHead(sections[10])}
    <div use:reveal class="reveal mt-3 flex flex-col gap-3">
      {#each voiceLines as line}
        <div class="flex items-start gap-2.5">
          <img
            src={stickerProud}
            alt=""
            class="mt-1 h-8 w-8 shrink-0 object-contain"
            aria-hidden="true"
          />
          <div class="kotone-panel relative p-3.5">
            <span
              class="absolute top-3 -left-1 h-2.5 w-2.5 rotate-45 border-l border-b border-white/8 bg-kotone-panel"
              aria-hidden="true"
            ></span>
            <p class="text-[12px] leading-relaxed text-white/70">{line}</p>
          </div>
        </div>
      {/each}
    </div>

    <!-- 12 招牌元素（母题 + 品牌色板） -->
    {@render secHead(sections[11])}
    <div use:reveal class="reveal mt-3">
      <div class="grid grid-cols-3 gap-3">
        {#each motifs as m}
          <div class="kotone-card overflow-hidden">
            <div
              class="h-16 ring-1 ring-white/10"
              style:background-image="url({m.img})"
              style:background-size="96px"
            ></div>
            <div class="p-3">
              <p class="text-xs font-semibold">{m.name}</p>
              <p class="mt-1 text-[11px] leading-relaxed text-white/45">
                {m.desc}
              </p>
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
              <p class="truncate text-[11px] font-semibold">
                {c.name} <span class="text-white/40">{c.hex}</span>
              </p>
              <p class="truncate text-[10px] text-white/40">{c.role}</p>
            </div>
          </div>
        {/each}
      </div>
    </div>

    <!-- 弹幕跑马灯 -->
    <div
      class="relative mt-4 overflow-hidden rounded-xl border border-white/8 bg-kotone-panel/60 py-2"
      aria-hidden="true"
    >
      <div class="marquee-track flex w-max items-center gap-6">
        {#each danmakuTrack as d}
          <span class="text-[11px] whitespace-nowrap text-kotone-cyan/50"
            ><i class="mr-1 text-kotone-pink/60 not-italic">※</i>{d}</span
          >
        {/each}
      </div>
    </div>

    <!-- 档案页脚 -->
    <p class="mt-6 text-center text-[10px] leading-relaxed text-white/25">
      CHARACTER FILE · KOTONE<br />档案由 repochan persona 协议生成 ·
      贴纸与立绘版权归 Kotone 项目
    </p>
  </div>
</div>

{#snippet secHead(s: { num: string; title: string; sub: string })}
  <div
    use:reveal
    id={"cf-" + s.num}
    class="reveal mt-8 flex scroll-mt-24 items-baseline gap-2.5"
  >
    <span class="text-[11px] font-black tracking-widest text-kotone-pink/70"
      >{s.num}</span
    >
    <h2 class="text-sm font-bold">
      {s.title}<span
        class="ml-2 text-[9px] font-semibold tracking-[0.24em] text-white/30"
        >{s.sub}</span
      >
    </h2>
    <span
      class="h-px flex-1 self-center bg-gradient-to-r from-kotone-cyan/30 to-transparent"
    ></span>
  </div>
{/snippet}

<style>
  /* 分区滚动进入视口时淡入上移（与关于页 reveal 同一模式） */
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

  /* Hero 贴纸漂浮 */
  .float-slow {
    animation: float-y 5s ease-in-out infinite;
  }
  .float-fast {
    animation: float-y 3.6s ease-in-out infinite reverse;
  }
  @keyframes float-y {
    0%,
    100% {
      transform: translateY(0) rotate(var(--r, 0deg));
    }
    50% {
      transform: translateY(-8px) rotate(var(--r, 0deg));
    }
  }

  /* 弹幕跑马灯：轨道宽度 = 内容两份，平移 -50% 无缝循环 */
  .marquee-track {
    animation: marquee 22s linear infinite;
  }
  @keyframes marquee {
    from {
      transform: translateX(0);
    }
    to {
      transform: translateX(-50%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .reveal {
      opacity: 1;
      transform: none;
      transition: none;
    }
    .float-slow,
    .float-fast,
    .marquee-track {
      animation: none;
    }
  }
</style>
