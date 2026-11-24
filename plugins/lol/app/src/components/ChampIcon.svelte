<script lang="ts">
  // 英雄头像：方形小圆角、裁掉原图的暗边；右下角可以压一块等级小牌（这局里是召唤师等级，战绩里是英雄等级）。
  import { championIconPath } from "../assets";
  import Pic from "./Pic.svelte";

  let {
    id,
    name,
    icon,
    size = 40,
    level,
    title,
    cls = "",
  }: { id: number; name: string; icon?: string; size?: number; level?: number | null; title?: string; cls?: string } = $props();
</script>

<Pic path={icon || championIconPath(id)} {name} {size} {title} {cls} crop>
  {#if typeof level === "number" && level > 0}<span class="lv">{level}</span>{/if}
</Pic>
