<script lang="ts">
  // 排好顺序的英雄（自动选 / 禁用）：按顺序列出来，能上移、下移、移除；下面搜英雄加进来。
  // 英雄的名字来自 `lol.champions`（客户端开着时用客户端的资料，不然用腾讯的英雄表）；没有名字时显示编号。
  import { ArrowDown, ArrowUp, Plus, X } from "@lucide/svelte";
  import { Input } from "$lib/components/ui/input";
  import { m } from "$lib/paraglide/messages.js";
  import type { StatChampion } from "./types";
  import { move } from "./control";

  let { ids, champions, disabled = false, testid, onchange }: {
    ids: number[];
    champions: StatChampion[];
    disabled?: boolean;
    testid: string;
    onchange: (ids: number[]) => void;
  } = $props();

  let query = $state("");
  const nameOf = (id: number) => champions.find((c) => c.id === id)?.short ?? m.plugin_lol_control_champion_id({ id });
  const matches = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return champions
      .filter((c) => !ids.includes(c.id) && (c.short.toLowerCase().includes(q) || c.label.toLowerCase().includes(q)))
      .slice(0, 8);
  });
  function add(id: number) {
    if (!ids.includes(id)) onchange([...ids, id]);
    query = "";
  }
  const iconButton = "inline-flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-35";
</script>

<div class="space-y-2" data-testid={testid}>
  {#if ids.length}
    <ol class="m-0 list-none space-y-1 p-0" data-testid={`${testid}-list`}>
      {#each ids as id, index (id)}
        <li class="flex items-center gap-2 rounded-lg bg-muted/40 px-2 py-1 text-[13px]" data-testid={`${testid}-item`} data-champion={id}>
          <span class="w-5 text-right text-xs tabular-nums text-muted-foreground">{index + 1}</span>
          <span class="min-w-0 flex-1 truncate text-foreground">{nameOf(id)}</span>
          <button type="button" class={iconButton} disabled={disabled || index === 0} aria-label={m.plugin_lol_control_up()}
            data-testid={`${testid}-up`} onclick={() => onchange(move(ids, index, -1))}><ArrowUp class="size-3.5" /></button>
          <button type="button" class={iconButton} disabled={disabled || index === ids.length - 1} aria-label={m.plugin_lol_control_down()}
            data-testid={`${testid}-down`} onclick={() => onchange(move(ids, index, 1))}><ArrowDown class="size-3.5" /></button>
          <button type="button" class={iconButton} {disabled} aria-label={m.plugin_lol_control_remove()}
            data-testid={`${testid}-remove`} onclick={() => onchange(ids.filter((x) => x !== id))}><X class="size-3.5" /></button>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="m-0 text-xs text-muted-foreground" data-testid={`${testid}-empty`}>{m.plugin_lol_control_list_empty()}</p>
  {/if}
  <div class="relative">
    <Input class="h-8 text-xs" placeholder={m.plugin_lol_control_add_placeholder()} bind:value={query} {disabled}
      data-testid={`${testid}-search`} />
    {#if matches.length}
      <div class="mt-1 flex flex-wrap gap-1" data-testid={`${testid}-matches`}>
        {#each matches as c (c.id)}
          <button type="button" class="inline-flex h-7 items-center gap-1 rounded-md border border-hairline px-2 text-xs text-foreground transition-colors hover:border-primary/50 hover:bg-muted"
            data-testid={`${testid}-add`} data-champion={c.id} onclick={() => add(c.id)}><Plus class="size-3" />{c.short}</button>
        {/each}
      </div>
    {/if}
  </div>
</div>
