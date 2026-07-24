<script lang="ts">
  /*
   * 悬浮窗路由视图（index.html#/overlay）。
   * 窗口初始 invisible；显隐由后端状态事件驱动（SW_SHOWNA 不抢焦点），
   * 实现「按下热键即弹出悬浮条」（docs/development.md §3.4、§3.6）。
   * 托盘「显示悬浮条」手动唤起时若处于 idle 会保持可见（仅状态迁移触发隐藏）。
   */
  import { appState } from "../../lib/stores/state";
  import { isTauri } from "../../lib/ipc";
  import OverlayBar from "../../lib/components/OverlayBar.svelte";
  import { onMount } from "svelte";

  /*
   * 浏览器 demo 模式（仅 dev:web）：#/overlay?demo 自动播放一遍
   * listening → transcribing → preview → sending → success → idle，
   * 便于脱离 Tauri 验证各状态渲染；Tauri 环境下不生效。
   */
  onMount(() => {
    if (isTauri || !window.location.hash.includes("demo")) return;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const later = (ms: number, fn: () => void) => timers.push(setTimeout(fn, ms));
    const levelTimer = setInterval(() => {
      appState.update((s) =>
        s.state === "listening" ? { ...s, level: 0.08 + Math.random() * 0.5 } : s,
      );
    }, 60);

    later(100, () => appState.update((s) => ({ ...s, state: "listening" })));
    later(600, () => appState.update((s) => ({ ...s, partialText: "对面" })));
    later(1200, () => appState.update((s) => ({ ...s, partialText: "对面打野" })));
    later(1800, () => appState.update((s) => ({ ...s, partialText: "对面打野在下" })));
    later(2400, () => appState.update((s) => ({ ...s, state: "transcribing", level: 0 })));
    later(3200, () =>
      appState.update((s) => ({
        ...s,
        state: "preview",
        partialText: "对面打野在下路",
        finalText: "对面打野在下路",
      })),
    );
    later(5200, () => appState.update((s) => ({ ...s, state: "sending" })));
    later(6200, () => appState.update((s) => ({ ...s, state: "success" })));
    later(7800, () => {
      appState.update((s) => ({
        ...s,
        state: "idle",
        partialText: "",
        finalText: "",
        level: 0,
      }));
      clearInterval(levelTimer);
    });
    return () => {
      timers.forEach(clearTimeout);
      clearInterval(levelTimer);
    };
  });

  /*
   * 窗口显隐完全由后端 TauriEmitter 驱动（lib.rs）：
   * 非 idle 状态用 SW_SHOWNA 显示（不抢焦点，焦点必须留在游戏/目标窗口，
   * 否则注入前台校验会打错窗口），idle 时隐藏。
   * 前端不再调用 win.show()：Tauri show() 走 SW_SHOW 会激活窗口抢焦点，
   * 与「preview 交互不抢焦点」的设计冲突。
   */
</script>

<div class="h-full">
  <OverlayBar />
</div>
