<script lang="ts" module>
  import type { Summary, Tag } from "../types";

  /** 卡片里一行最多 1 个：连败 > 第一次玩 > 连胜 > 常玩（设计第一节 9）。 */
  const ORDER = ["loss_streak", "first_time", "win_streak", "main"];

  export function kindOf(tag: Tag): string {
    return typeof tag === "string" ? "" : tag.kind;
  }
  export function textOf(tag: Tag): string {
    return typeof tag === "string" ? tag : tag.text;
  }
  export function pickTags(tags: Tag[], limit?: number): Tag[] {
    if (!limit) return tags;
    const rank = (t: Tag) => {
      const i = ORDER.indexOf(kindOf(t));
      return i < 0 ? ORDER.length : i;
    };
    return [...tags].sort((a, b) => rank(a) - rank(b)).slice(0, limit);
  }
</script>

<script lang="ts">
  // 标签（18 px 高、3 px 圆角、11 px 字）：连胜淡绿、连败淡红、第一次玩淡黄、常玩灰。小牌上是短的说法
  // （“3 连胜”“第一次玩”“常玩 8 局”），完整的那句话（和给她的字一样）放 title。旧数据只有一句话时原样显示。
  import { m } from "../i18n.svelte";

  let { tags, limit, summary = null }: { tags: Tag[]; limit?: number; summary?: Summary | null } = $props();
  const shown = $derived(pickTags(tags, limit));
  const tone = (kind: string) =>
    kind === "win_streak" ? "good" : kind === "loss_streak" ? "bad" : kind === "first_time" ? "warn" : "";
  function short(tag: Tag): string {
    const streak = Math.abs(summary?.streak ?? 0);
    switch (kindOf(tag)) {
      case "win_streak":
        return streak ? m.plugin_lol_tag_win_streak({ count: streak }) : textOf(tag);
      case "loss_streak":
        return streak ? m.plugin_lol_tag_loss_streak({ count: streak }) : textOf(tag);
      case "first_time":
        return m.plugin_lol_tag_first_time();
      case "main":
        return summary?.champGames ? m.plugin_lol_tag_main({ count: summary.champGames }) : textOf(tag);
      default:
        return textOf(tag);
    }
  }
</script>

{#each shown as tag, i (i)}
  <span class="chip {tone(kindOf(tag))}" title={textOf(tag)} data-kind={kindOf(tag)}>{short(tag)}</span>
{/each}
