<script lang="ts">
  /*
   * 「设置 → 热键」（ADR 040 D2、D3）：Kotone 里所有的热键都在这一页——Kotone 自己的、每个插件的、
   * 玩家自己加的快捷指令，还有固定的 Esc；能搜索，每行标来源，冲突标红。**热键只在这里改**：点一行的键录新的
   * （或者直接打），后端查冲突，撞了就不存并说清楚和谁撞了（主会话定的第 1 条）；别的页面只显示键和“去改”。
   * 提醒助手热键照旧在提醒助手页改（D10），Esc 不能改，这个版本找不到的命令只列出来。游戏配置里的聊天键、
   * 控制台触发键不进表，但参与冲突检查。表和冲突都是后端给的（`get_hotkey_table`），这里只显示。
   * 照 VS Code 的“键盘快捷方式”、Obsidian 的“设置 → 快捷键”。
   */
  import { tick } from "svelte";
  import { Search } from "@lucide/svelte";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import { getHotkeyTable, setHotkey, type HotkeyTable, type KeyUse } from "../../../../lib/ipc";
  import { hotkeyLabel } from "../../../../lib/hotkeyCombo";
  import { keyUseDetail, keyUseName, keyUseNames, keyUseSource } from "../../../../lib/hotkeyNames";
  import { openLocation } from "../../../../lib/stores/navigation";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { focus = null }: {
    /** 别的页面点“去改”过来的那一行（`hotkey-<行 id>`）：高亮，搜索框清空。 */
    focus?: string | null;
  } = $props();

  let table = $state<HotkeyTable | null>(null);
  let loadError = $state("");
  let query = $state("");
  let resetting = $state("");

  // 设置变了（这里改了键、她改了键、别的窗口改了）就重读
  $effect(() => {
    void $settingsStore;
    let stale = false;
    getHotkeyTable()
      .then((next) => { if (!stale) { table = next; loadError = ""; } })
      .catch((error) => { if (!stale) loadError = m.hotkeys_load_failed({ error: errText(error) }); });
    return () => { stale = true; };
  });

  // 从别的页面点“去改”过来：清掉搜索，表读好以后滚到那一行
  $effect(() => { if (focus) query = ""; });
  $effect(() => {
    const target = focus;
    if (!target || !table) return;
    void tick().then(() => document.getElementById(target)?.scrollIntoView({ block: "center" }));
  });

  type Row = { use: KeyUse; name: string; source: string; detail: string; key: string; conflicts: KeyUse[]; editable: boolean };

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
      // 提醒助手热键在提醒助手页改（D10）；Esc 固定；找不到的命令不登记、不改
      editable: (use.source === "core" || use.source === "plugin" || use.source === "custom") && use.id !== "legacy-reminder",
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

  async function save(row: Row, key: string) {
    settingsStore.set(await setHotkey(row.use.id, key));
  }

  async function resetToDefault(row: Row) {
    const key = row.use.defaultKey ?? "";
    if (!key || resetting) return;
    resetting = row.use.id;
    try {
      await save(row, key);
      toast(true, m.hotkeys_reset_done({ title: row.name, key: hotkeyLabel(key) }));
    } catch (error) {
      toast(false, errText(error));
    } finally {
      resetting = "";
    }
  }
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
    {@const focused = focus === `hotkey-${row.use.id}`}
    <div id={`hotkey-${row.use.id}`} data-testid={`hotkeys-row-${row.use.id}`} data-source={row.use.source}
      data-conflict={conflict ? "true" : "false"} data-focused={focused ? "true" : "false"}
      class="scroll-mt-16 px-4 py-3 {conflict ? 'bg-destructive/5' : ''} {focused ? 'ring-2 ring-inset ring-primary/50' : ''}">
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-1.5">
            <span class="text-[13px] font-medium {conflict ? 'text-destructive' : 'text-foreground'}">{row.name}</span>
            <span class="rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground" data-testid={`hotkeys-source-${row.use.id}`}>{row.source}</span>
            {#if !row.use.enabled && row.use.source === "plugin"}
              <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] text-muted-foreground">{m.hotkeys_plugin_off()}</span>
            {/if}
          </div>
          {#if row.detail}<p class="mt-0.5 mb-0 truncate text-xs text-muted-foreground">{row.detail}</p>{/if}
          {#if conflict}
            <p class="mt-1 mb-0 text-xs text-destructive" data-testid={`hotkeys-conflict-${row.use.id}`}>{m.hotkeys_conflicts_with({ list: keyUseNames(row.conflicts) })}</p>
          {/if}
        </div>
        {#if !row.editable}
          <kbd data-testid={`hotkeys-key-${row.use.id}`}
            class="rounded-md border px-2 py-0.5 font-sans text-xs {conflict ? 'border-destructive/60 text-destructive' : row.key ? 'border-hairline bg-background/60 text-foreground' : 'border-dashed border-hairline text-muted-foreground'}">
            {row.key || m.voice_hotkey_unset()}
          </kbd>
          <div class="w-16 text-right">
            {#if row.use.page && row.use.source !== "unknown"}
              <button type="button" data-testid={`hotkeys-goto-${row.use.id}`} class="text-xs font-medium text-primary hover:underline"
                onclick={() => openLocation(row.use.page ?? "")}>{m.hotkeys_goto()}</button>
            {:else}
              <span class="text-xs text-muted-foreground">{m.hotkeys_fixed_note()}</span>
            {/if}
          </div>
        {:else if row.use.defaultKey && row.use.key !== row.use.defaultKey}
          <button type="button" data-testid={`hotkeys-reset-${row.use.id}`} disabled={!!resetting}
            class="text-xs font-medium text-primary hover:underline disabled:opacity-50"
            onclick={() => void resetToDefault(row)}>{m.hotkeys_reset_default({ key: hotkeyLabel(row.use.defaultKey) })}</button>
        {/if}
      </div>
      {#if row.editable}
        <HotkeyField id={row.use.id} statusId={row.use.statusId} title={row.name} value={row.use.key} compact
          onSave={(key) => save(row, key)} />
      {/if}
    </div>
  {:else}
    {#if table && query.trim()}
      <p class="m-0 px-4 py-6 text-center text-xs text-muted-foreground" data-testid="hotkeys-empty">{m.hotkeys_search_empty({ query: query.trim() })}</p>
    {/if}
  {/each}
</SettingsSection>
