<script lang="ts">
  /*
   * 游戏上方的覆盖层（ADR 038 D5 的第三个显示位置，index.html#/plugin-overlay）：只有一个，在本体里。
   * 宿主（overlay_layer.rs）把窗口摆在要显示的几项外面一圈（不盖住整个游戏：照 aramhex 的排查，大块透明置顶窗口
   * 会让无边框游戏掉出 Independent Flip、限帧），每项带好 `rect`（这个窗口里的比例）。这里只按 rect 摆、交给插件登记的
   * 覆盖层页面画。只读：不能点（窗口点击穿透），没有动画，页面底色透明。
   * 浏览器预览：`#/plugin-overlay?demo=augments`（海斗三选一）/ `?demo=champ-select`（选英雄）显示示例，
   * 或者调 `window.kotonePluginOverlay(payload)`（Playwright 用）。
   */
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { isTauri } from "../../lib/ipc";
  import { overlayOf } from "../../lib/pluginUi";
  import { sampleOverlay } from "../../lib/ipc/lolStatsMock";

  interface Entry {
    plugin: string;
    item: { id: string; uri: string; data: unknown };
    /** 在这个窗口里的位置（比例）：[x, y, w, h]。 */
    rect: [number, number, number, number];
  }
  interface Payload { items: Entry[]; width?: number; height?: number }

  const demo = /[?&]demo=([a-z-]+)/.exec(window.location.hash)?.[1] ?? "";
  let entries = $state<Entry[]>([]);

  function show(payload: Payload | null | undefined) {
    entries = Array.isArray(payload?.items) ? payload.items.filter((e) => Array.isArray(e?.rect) && e.rect.length === 4) : [];
  }

  onMount(() => {
    if (!isTauri) {
      (window as unknown as { kotonePluginOverlay?: (p: Payload) => void }).kotonePluginOverlay = show;
      if (demo) show(sampleOverlay(demo) as unknown as Payload);
      return () => { delete (window as unknown as { kotonePluginOverlay?: unknown }).kotonePluginOverlay; };
    }
    const off = listen<Payload>("kotone://plugin-overlay", (event) => show(event.payload));
    return () => { void off.then((unlisten) => unlisten()); };
  });

  const pct = (v: number) => `${Math.max(0, Math.min(1, v)) * 100}%`;
</script>

<div class="stage" class:demo={demo !== ""} data-testid="plugin-overlay-stage">
  {#each entries as entry (`${entry.plugin}/${entry.item.id}`)}
    {@const View = overlayOf(entry.plugin, entry.item.uri)}
    {#if View}
      <div class="item" data-testid="plugin-overlay-item" data-uri={entry.item.uri} data-id={entry.item.id}
        style:left={pct(entry.rect[0])} style:top={pct(entry.rect[1])} style:width={pct(entry.rect[2])} style:height={pct(entry.rect[3])}>
        <View data={entry.item.data} />
      </div>
    {/if}
  {/each}
</div>

<style>
  .stage { position: fixed; inset: 0; overflow: hidden; pointer-events: none; user-select: none; background: transparent; }
  /* 预览：假装下面是游戏画面，看得出叠在上面的效果 */
  .stage.demo { background: radial-gradient(circle at 30% 40%, #3b4b2a 0%, #1b2516 45%, #0c0f0a 100%); }
  .item { position: absolute; overflow: hidden; container-type: size; pointer-events: none; }
</style>
