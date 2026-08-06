<script lang="ts">
  /*
   * 启动 / 停止 / 重启生效 —— 全局唯一的运行时主按钮。
   * 标题栏（variant="titlebar"）与首页 Hero（variant="hero"）共用同一套
   * 状态机与交互逻辑：stopped → startRuntime；running → stopRuntime；
   * running + restartNeeded → startRuntime（壳侧编排 stop+start）。
   * 模型缺失类错误会回调 onOpenAdvanced 引导用户去高级页处理。
   */
  import { startRuntime, stopRuntime } from "../ipc";
  import { runtimeStore } from "../stores/runtime";
  import { toast, toastInfo, pushToast, errText } from "../stores/ui";

  let {
    variant = "hero",
    onOpenAdvanced,
  }: { variant?: "titlebar" | "hero"; onOpenAdvanced?: () => void } = $props();

  const rt = $derived($runtimeStore);
  const busy = $derived(rt?.phase === "starting" || rt?.phase === "stopping");
  /** 防连点（busy 已由相位覆盖，acting 覆盖事件未回的瞬间） */
  let acting = $state(false);

  async function onClick() {
    if (!rt || busy || acting) return;
    acting = true;
    const restarting = rt.phase === "running" && rt.restartNeeded;
    try {
      if (rt.phase === "running" && !rt.restartNeeded) {
        await stopRuntime();
        toastInfo("已停止，热键已注销");
      } else {
        await startRuntime();
        pushToast(
          "success",
          restarting ? "已按新配置重启，Kotone 运行中 ✨" : "模型已加载，Kotone 已启动 ✨",
        );
      }
    } catch (e) {
      const message = errText(e);
      toast(false, message);
      if (/模型.*(未下载|不齐备)|未就绪|模型文件|recognizer 创建失败/.test(message)) {
        onOpenAdvanced?.();
      }
    } finally {
      acting = false;
    }
  }

  const stateClass = $derived(
    busy || acting
      ? variant === "hero"
        ? "bg-white/10 text-white/50"
        : "bg-white/10 text-white/50"
      : !rt || rt.phase === "stopped"
        ? "bg-kotone-pink text-white shadow-glow-pink-lg hover:brightness-110"
        : rt.restartNeeded
          ? "bg-yellow-400/90 text-kotone-deep shadow-[0_0_14px_rgba(250,204,21,0.4)] hover:brightness-110"
          : "bg-kotone-cyan/15 text-kotone-cyan ring-1 ring-kotone-cyan/50 hover:bg-kotone-cyan/25 hover:shadow-glow-cyan",
  );
</script>

<button
  class={variant === "hero"
    ? `flex h-12 w-1/3 min-w-44 items-center justify-center gap-2 rounded-[var(--radius-kotone-card)] text-sm font-bold
       tracking-wide transition active:scale-[0.98] disabled:cursor-not-allowed ${stateClass}`
    : `mr-2 flex h-7 items-center gap-1.5 rounded-[var(--radius-kotone-card)] px-3 text-[11px] font-bold
       transition active:scale-95 disabled:cursor-not-allowed ${stateClass}`}
  disabled={busy || acting || !rt}
  onclick={() => void onClick()}
>
  {#if busy || acting}
    <span
      class="animate-spin rounded-full border-2 border-white/25 border-t-white/80
        {variant === 'hero' ? 'h-4 w-4' : 'h-3 w-3'}"
    ></span>
    {rt?.phase === "stopping" ? "停止中" : "启动中"}
  {:else if !rt || rt.phase === "stopped"}
    {#if variant === "hero"}▶ 启动 Kotone{:else}▶ 启动{/if}
  {:else if rt.restartNeeded}
    重启生效
  {:else}
    {#if variant === "hero"}■ 停止 Kotone{:else}■ 停止{/if}
  {/if}
</button>
