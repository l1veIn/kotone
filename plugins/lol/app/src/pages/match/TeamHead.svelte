<script lang="ts">
  // 队伍表头（36 px）：队伍记号（我方 ■、对面 ◆）+ 名字 + 平均胜率、平均 KDA、几人开黑。
  import { one } from "../../format";
  import { m } from "../../i18n.svelte";
  import type { MatchPlayer } from "../../types";
  import { partiesIn, teamStats, type PartyInfo } from "./match";

  let {
    ally,
    team,
    party,
    stats = true,
  }: { ally: boolean; team: MatchPlayer[]; party: (p: MatchPlayer) => PartyInfo | null; stats?: boolean } = $props();

  const st = $derived(teamStats(team));
  const groups = $derived(partiesIn(team, party));
</script>

<div class="team-head">
  <span class="label"><span class="mark" class:diamond={!ally}></span>{ally ? m.plugin_lol_allies() : m.plugin_lol_enemies()}</span>
  {#if stats}
    <span class="stats num">
      {#if st.known > 0}
        <span>{m.plugin_lol_avg_rate()} <b class={st.rateTone}>{st.rate}%</b></span>
        <span>{m.plugin_lol_avg_kda()} <b>{one(st.kda)}</b></span>
      {/if}
      {#each groups as g (g.letter)}<span>{m.plugin_lol_party_group({ letter: g.letter, count: g.size })}</span>{/each}
    </span>
  {/if}
</div>
