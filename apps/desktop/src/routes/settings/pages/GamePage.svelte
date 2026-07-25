<script lang="ts">
  /*
   * 游戏适配页：profile 游戏卡片（激活开关）+ 前台检测 + 测试发送（品红 CTA）。
   */
  import { onMount } from "svelte";
  import {
    updateSettings,
    listProfiles,
    detectForegroundGame,
    simulateSend,
    type GameProfile,
    type ForegroundGameInfo,
  } from "../../../lib/ipc";
  import { settingsStore, toast, errText } from "../../../lib/stores/ui";

  let profiles = $state<GameProfile[]>([]);
  let foreground = $state<ForegroundGameInfo | null | undefined>(undefined);
  let testing = $state(false);

  onMount(async () => {
    try {
      profiles = await listProfiles();
    } catch (e) {
      toast(false, `加载 profile 失败：${errText(e)}`);
    }
  });

  async function onActivate(id: string) {
    try {
      settingsStore.set(await updateSettings({ activeProfileId: id }));
      toast(true, `已激活 profile：${profiles.find((p) => p.id === id)?.displayName ?? id}`);
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
    }
  }

  async function onDetect() {
    try {
      foreground = await detectForegroundGame();
    } catch (e) {
      toast(false, `检测失败：${errText(e)}`);
    }
  }

  async function onTestSend() {
    testing = true;
    try {
      await simulateSend("Kotone 测试：对面打野在下路");
      toast(true, "测试发送已触发（切到目标窗口看效果）");
    } catch (e) {
      toast(false, `测试发送失败：${errText(e)}`);
    } finally {
      testing = false;
    }
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">
    <span class="kotone-gradient-text">游戏适配</span>
  </h1>
  <p class="mt-0.5 text-[11px] text-white/45">每个游戏一套打法：聊天键、词表、节奏</p>

  <div class="mt-4 flex flex-col gap-3">
    {#each profiles as p}
      {@const active = $settingsStore?.activeProfileId === p.id}
      <div class="kotone-card flex items-center gap-3 p-4 {active ? 'border-kotone-cyan/50 shadow-glow-cyan' : ''}">
        <!-- 图标占位 -->
        <span
          class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-lg
            {active ? 'bg-kotone-cyan/15' : 'bg-white/8'}"
        >
          {p.processNames.length === 0 ? "🌐" : "🎮"}
        </span>
        <div class="min-w-0 flex-1">
          <p class="flex items-center gap-2 text-sm font-semibold">
            <span class="truncate">{p.displayName}</span>
            {#if active}
              <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">激活中</span>
            {/if}
          </p>
          <p class="mt-0.5 truncate text-[11px] text-white/45">
            {p.processNames.length === 0 ? "通配任意前台窗口" : p.processNames.join(" / ")}
            · 热词 {p.hotwords.length} 个
          </p>
        </div>
        {#if !active}
          <button
            class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/80 transition hover:bg-white/20 active:scale-95"
            onclick={() => void onActivate(p.id)}
          >
            激活
          </button>
        {/if}
      </div>
    {/each}
  </div>

  <!-- 前台检测 + 测试发送 -->
  <section class="kotone-panel mt-4 p-4">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-kotone-cyan/90">联调工具</h2>
      <button
        class="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/80 transition hover:bg-white/20 active:scale-95"
        onclick={() => void onDetect()}
      >
        检测前台窗口
      </button>
    </div>
    {#if foreground !== undefined}
      <p class="mt-2 text-[12px] text-white/60">
        {#if foreground}
          前台命中：<span class="text-kotone-cyan">{foreground.displayName}</span>
          {#if foreground.targetElevated === true}
            <span class="text-kotone-pink">（目标进程已提权）</span>
          {/if}
        {:else}
          前台窗口未命中任何 profile（按 generic 通配处理）
        {/if}
      </p>
    {/if}
    <button
      class="mt-3 w-full rounded-lg bg-kotone-pink px-3 py-2.5 text-sm font-bold text-white shadow-glow-pink transition hover:brightness-110 active:scale-95 disabled:opacity-50"
      disabled={testing}
      onclick={() => void onTestSend()}
    >
      {testing ? "发送中…" : "测试发送"}
    </button>
    <p class="mt-1.5 text-[11px] text-white/40">
      向当前前台窗口注入一句测试文本（走真实发送时序：聊天键 → 文本 → 发送键）。
    </p>
  </section>
</div>
