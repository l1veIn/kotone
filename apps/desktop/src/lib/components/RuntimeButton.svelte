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

  // 颜色全用主题变量：停止时是 Kotone 品红的主行动按钮，运行中是青色描边，待重启是警示色。
  const stateClass = $derived(
    busy || acting
      ? "bg-muted text-muted-foreground"
      : !rt || rt.phase === "stopped"
        ? "bg-brand-pink text-[#fff] shadow-[0_0_22px_-6px_var(--color-brand-pink)] hover:brightness-110"
        : rt.restartNeeded
          ? "bg-warning text-warning-foreground hover:brightness-110"
          : "bg-primary/12 text-primary ring-1 ring-primary/45 hover:bg-primary/20",
  );
</script>

<button
  class={variant === "hero"
    ? `flex h-10 w-fit min-w-40 items-center justify-center gap-2 rounded-xl px-5 text-sm font-semibold
       tracking-wide transition active:scale-[0.98] disabled:cursor-not-allowed ${stateClass}`
    : `mr-1 flex h-7 items-center gap-1.5 rounded-lg px-3 text-xs font-semibold
       transition active:scale-95 disabled:cursor-not-allowed ${stateClass}`}
  disabled={busy || acting || !rt}
  onclick={() => void onClick()}
>
  {#if busy || acting}
    <span
      class="animate-spin rounded-full border-2 border-current/25 border-t-current
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
