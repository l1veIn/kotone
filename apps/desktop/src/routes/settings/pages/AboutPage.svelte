<script lang="ts">
  /*
   * 关于页（品牌页）：立绘 + 品牌文案 + 版本与链接，贴纸彩蛋。
   */
  import cutout from "../../../assets/brand/kotone-cutout.png";
  import stickerHello from "../../../assets/brand/stickers/hello.png";
  import stickerCheering from "../../../assets/brand/stickers/cheering.png";
  import stickerRelax from "../../../assets/brand/stickers/relax.png";
  import patternWave from "../../../assets/brand/patterns/wave.png";
</script>

<div class="relative h-full overflow-hidden px-6 py-5">
  <!-- 底纹 -->
  <div
    class="pointer-events-none absolute inset-0 opacity-[0.05]"
    style:background-image="url({patternWave})"
    style:background-size="220px"
  ></div>

  <div class="relative flex h-full flex-col items-center text-center">
    <div class="relative mt-2">
      <img
        src={cutout}
        alt="Kotone 立绘"
        class="h-52 object-contain drop-shadow-[0_0_28px_rgba(0,229,255,0.3)]"
      />
      <!-- 贴纸彩蛋 -->
      <img src={stickerHello} alt="" class="absolute -left-16 top-6 h-16 w-16 -rotate-12 object-contain" />
      <img src={stickerCheering} alt="" class="absolute -right-16 top-2 h-16 w-16 rotate-12 object-contain" />
      <img src={stickerRelax} alt="" class="absolute -right-14 bottom-0 h-14 w-14 rotate-6 object-contain" />
    </div>

    <h1 class="mt-4 text-2xl font-bold">
      Kotone <span class="kotone-gradient-text">琴音</span>
    </h1>
    <p class="mt-1 text-[13px] text-white/60">打字比打游戏快的主播</p>

    <blockquote class="kotone-panel mt-4 max-w-sm px-5 py-3 text-[13px] leading-relaxed text-white/75">
      「想说的话，一秒都别等。<br />你专注赢下比赛，聊天框交给我。」
    </blockquote>

    <div class="mt-5 flex items-center gap-4 text-[12px] text-white/45">
      <span>v0.1.1（MVP）</span>
      <span class="text-white/20">|</span>
      <a
        class="text-kotone-cyan/80 underline underline-offset-2 transition hover:text-kotone-cyan"
        href="https://github.com/kotone-app/kotone"
        target="_blank"
        rel="noreferrer"
      >GitHub</a>
      <span class="text-white/20">|</span>
      <span>本地优先 · 隐私安全</span>
    </div>

    <p class="mt-auto pb-1 text-[10px] text-white/25">
      青 #00E5FF · 蓝黑 #1A1A2E · 品红 #FF2D78 · 紫 #7B2FFF —— 方向 B「中继站」
    </p>
  </div>
</div>
