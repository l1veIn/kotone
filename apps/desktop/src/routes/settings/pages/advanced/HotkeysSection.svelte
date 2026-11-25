<script lang="ts">
  /*
   * 「设置 → 热键」（ADR 040 第 1 期，D2）：一页列出 Kotone 里所有的热键——Kotone 自己的、每个插件的、
   * 玩家自己加的快捷指令，还有固定的 Esc；能搜索，每行标来源，冲突标红。这一期只读：每行的“去改”
   * 跳到现在改它的那个页面（第 2 期才把改的地方收进这张表）。游戏配置里的聊天键、控制台触发键不进表，
   * 但参与冲突检查（冲突说明里写“和《游戏》的……冲突”）。表和冲突都是后端给的（`get_hotkey_table`），
   * 这里只显示。照 VS Code 的“键盘快捷方式”、Obsidian 的“设置 → 快捷键”。
   */
  import { Search } from "@lucide/svelte";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { getHotkeyTable, type HotkeyTable, type KeyUse } from "../../../../lib/ipc";
  import { hotkeyLabel } from "../../../../lib/hotkeyCombo";
  import { keyUseDetail, keyUseName, keyUseNames, keyUseSource } from "../../../../lib/hotkeyNames";
  import { openLocation } from "../../../../lib/stores/navigation";
  import { errText, settingsStore } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let table = $state<HotkeyTable | null>(null);
  let loadError = $state("");
  let query = $state("");

  // 设置变了（别的页面改了键、她改了键）就重读
  $effect(() => {
    void $settingsStore;
    let stale = false;
    getHotkeyTable()
      .then((next) => { if (!stale) { table = next; loadError = ""; } })
      .catch((error) => { if (!stale) loadError = m.hotkeys_load_failed({ error: errText(error) }); });
    return () => { stale = true; };
  });

  type Row = { use: KeyUse; name: string; source: string; detail: string; key: string; conflicts: KeyUse[] };

  const rows = $derived.by((): Row[] => {
    if (!table) return [];
    const byId = new Map([...table.bindings, ...table.gameKeys].map((use) => [use.id, use]));
    return table.bindings.map((use) => ({
      use,
      name: keyUseName(use),
      source: keyUseSource(use),
      detail: keyUseDetail(use, $settingsStore),
      key: use.key ? hotkeyLabel(use.combo || use.key) : "",
      conflicts: (use.conflicts ?? []).map((id) => byId.get(id)).filter((other): other is KeyUse => !!other),
    }));
  });

  const shown = $derived.by(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return rows;
    return rows.filter((row) => {
      const hay = [row.name, row.source, row.detail, row.key, row.use.key, row.use.combo].join(" ").toLowerCase();
      return terms.every((term) => hay.includes(term));
    });
  });
</script>

<SettingsSection title={m.hotkeys_title()} description={m.hotkeys_description()} flush testid="hotkeys-page">
  <div class="px-4 py-3">
    <div class="relative max-w-sm">
      <Search class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
      <Input bind:value={query} placeholder={m.hotkeys_search()} aria-label={m.hotkeys_search()} data-testid="hotkeys-search" class="h-8 pl-8 text-[13px]" />
    </div>
  </div>
  {#if loadError}
    <p class="m-0 px-4 py-3 text-xs text-destructive" role="alert">{loadError}</p>
  {/if}
  {#each shown as row (row.use.id)}
    {@const conflict = row.conflicts.length > 0}
    <div data-testid={`hotkeys-row-${row.use.id}`} data-source={row.use.source} data-conflict={conflict ? "true" : "false"}
      class="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-3 {conflict ? 'bg-destructive/5' : ''}">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-1.5">
          <span class="text-[13px] font-medium {conflict ? 'text-destructive' : 'text-foreground'}">{row.name}</span>
          <span class="rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground" data-testid={`hotkeys-source-${row.use.id}`}>{row.source}</span>
          {#if !row.use.enabled}
            <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] text-muted-foreground">{m.hotkeys_plugin_off()}</span>
          {/if}
        </div>
        {#if row.detail}<p class="mt-0.5 mb-0 truncate text-xs text-muted-foreground">{row.detail}</p>{/if}
        {#if conflict}
          <p class="mt-1 mb-0 text-xs text-destructive" data-testid={`hotkeys-conflict-${row.use.id}`}>{m.hotkeys_conflicts_with({ list: keyUseNames(row.conflicts) })}</p>
        {/if}
      </div>
      <kbd data-testid={`hotkeys-key-${row.use.id}`}
        class="rounded-md border px-2 py-0.5 font-sans text-xs {conflict ? 'border-destructive/60 text-destructive' : row.key ? 'border-hairline bg-background/60 text-foreground' : 'border-dashed border-hairline text-muted-foreground'}">
        {row.key || m.voice_hotkey_unset()}
      </kbd>
      <div class="w-16 text-right">
        {#if row.use.page}
          <button type="button" data-testid={`hotkeys-goto-${row.use.id}`} class="text-xs font-medium text-primary hover:underline"
            onclick={() => openLocation(row.use.page ?? "")}>{m.hotkeys_goto()}</button>
        {:else}
          <span class="text-xs text-muted-foreground">{m.hotkeys_fixed_note()}</span>
        {/if}
      </div>
    </div>
  {:else}
    {#if table && query.trim()}
      <p class="m-0 px-4 py-6 text-center text-xs text-muted-foreground" data-testid="hotkeys-empty">{m.hotkeys_search_empty({ query: query.trim() })}</p>
    {/if}
  {/each}
</SettingsSection>
