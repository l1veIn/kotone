<script lang="ts">
  /*
   * 悬浮窗路由视图（index.html#/overlay）。
   * 窗口初始 invisible；非 idle 状态自动弹出，回 idle 自动隐藏，
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

  $effect(() => {
    const state = $appState.state;
    if (!isTauri) return;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (state === "idle") {
        await win.hide();
      } else {
        // 只显示不抢焦点：录音时不打断游戏/当前输入，
        // preview 编辑由用户点击文本框自然获得焦点
        await win.show();
      }
    })();
  });
</script>

<div class="h-full">
  <OverlayBar />
</div>
