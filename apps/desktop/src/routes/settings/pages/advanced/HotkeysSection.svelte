<script lang="ts">
  /*
   * 「设置 → 热键」（ADR 040 D2、D3）：Kotone 里所有的热键都在这一页——Kotone 自己的、每个插件的、
   * 玩家自己加的，还有固定的 Esc；能搜索，每行标来源，冲突标红。**热键只在这里改**：点一行的键录新的
   * （或者直接打），后端查冲突，撞了就不存并说清楚和谁撞了（主会话定的第 1 条）；别的页面只显示键和“去改”。
   * 提醒助手热键照旧在提醒助手页改（D10），Esc 不能改，这个版本找不到的命令只列出来。游戏配置里的聊天键、
   * 控制台触发键不进表，但参与冲突检查。表和冲突都是后端给的（`get_hotkey_table`），这里只显示。
   * 照 VS Code 的“键盘快捷方式”、Obsidian 的“设置 → 快捷键”。
   *
   * “自己加的”那一组（第 3 期，D5、D9）：顶上“加一个”就地展开表单（`CustomHotkeyForm`：跟她说一句、运行自动化、
   * 调一个工具、打开一个页面）；每一行能改、能删；调的工具要问、还没确认的标“等你确认”，点“确认”写清会做什么、
   * 调哪个工具带什么参数，玩家点“允许”才登记（`confirm_hotkey`，只有这里能确认）；运行自动化那行写自动化的名字。
   * 别的页面（唤醒页的“快捷指令”）点“去加”过来：锚点 `hotkeys-add-<种类>`，打开表单。
   */
  import { onMount, tick } from "svelte";
  import { Pencil, Plus, Search, Trash2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import CustomHotkeyForm from "./CustomHotkeyForm.svelte";
  import {
    confirmHotkey, getHotkeyBindingOptions, getHotkeyTable, removeCustomHotkey, setHotkey, usesLegacyVoiceInput,
    type HotkeyBinding, type HotkeyBindingOptions, type HotkeyTable, type KeyUse, type Settings,
  } from "../../../../lib/ipc";
  import { listAutomations, subscribeAutomations, type Automation } from "../../../../lib/automation";
  import { hotkeyLabel } from "../../../../lib/hotkeyCombo";
  import { keyUseDetail, keyUseName, keyUseNames, keyUseSource } from "../../../../lib/hotkeyNames";
  import { openLocation } from "../../../../lib/stores/navigation";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  type Kind = "say" | "automation" | "tool" | "page";

  let { focus = null }: {
    /** 别的页面点“去改”过来的那一行（`hotkey-<行 id>`）：高亮，搜索框清空；`hotkeys-add-<种类>` 打开“加一个”。 */
    focus?: string | null;
  } = $props();

  let table = $state<HotkeyTable | null>(null);
  let loadError = $state("");
  let query = $state("");
  let resetting = $state("");
  /** 正在加一个（种类）/ 正在改的那一行 / 正在确认的那一行。 */
  let adding = $state<Kind | null>(null);
  let editing = $state<string | null>(null);
  let confirming = $state<string | null>(null);
  let busy = $state("");
  let options = $state<HotkeyBindingOptions | null>(null);
  let automations = $state<Automation[]>([]);

  // 设置变了（这里改了键、她改了键、别的窗口改了）就重读
  $effect(() => {
    void $settingsStore;
    let stale = false;
    getHotkeyTable()
      .then((next) => { if (!stale) { table = next; loadError = ""; } })
      .catch((error) => { if (!stale) loadError = m.hotkeys_load_failed({ error: errText(error) }); });
    return () => { stale = true; };
  });

  async function loadAutomations() {
    try { automations = await listAutomations(); } catch { /* 读不到时运行自动化那几行只写 id */ }
  }

  onMount(() => {
    void loadAutomations();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void subscribeAutomations(() => { void loadAutomations(); }).then((fn) => { if (disposed) fn(); else unlisten = fn; });
    return () => { disposed = true; unlisten?.(); };
  });

  /** 能选的工具和页面：打开表单时读（权限、插件开关可能刚改过）。 */
  async function loadOptions() {
    try { options = await getHotkeyBindingOptions(); } catch (error) { toast(false, errText(error)); }
  }

  // 从别的页面点“去改”过来：清掉搜索，表读好以后滚到那一行；“去加”过来：打开“加一个”
  $effect(() => { if (focus) query = ""; });
  $effect(() => {
    const target = focus;
    const add = /^hotkeys-add(?:-(say|automation|tool|page))?$/.exec(target ?? "");
    if (add) { void startAdding((add[1] as Kind | undefined) ?? "say"); return; }
    if (!target || !table) return;
    void tick().then(() => document.getElementById(target)?.scrollIntoView({ block: "center" }));
  });

  type Row = {
    use: KeyUse; name: string; source: string; detail: string; key: string; conflicts: KeyUse[]; editable: boolean;
    binding: HotkeyBinding | null;
  };

  /** 表里自己加的那一条（行 id `custom.<id>`）。 */
  function bindingOf(use: KeyUse): HotkeyBinding | null {
    if (use.source !== "custom") return null;
    const id = use.id.slice("custom.".length);
    return ($settingsStore?.hotkeys?.bindings ?? []).find((b) => b.id === id) ?? null;
  }

  /** 运行自动化那一行：写自动化的名字（后端只知道 id）。 */
  function automationText(binding: HotkeyBinding | null): { name: string | null; id: string } {
    const id = String((binding?.args as { id?: string } | undefined)?.id ?? "");
    return { name: automations.find((a) => a.id === id)?.name ?? null, id };
  }

  const rows = $derived.by((): Row[] => {
    if (!table) return [];
    const byId = new Map([...table.bindings, ...table.gameKeys].map((use) => [use.id, use]));
    return table.bindings.map((use) => {
      const binding = bindingOf(use);
      let name = keyUseName(use);
      let detail = keyUseDetail(use, $settingsStore);
      if (use.action === "automation") {
        const auto = automationText(binding);
        const named = !!String(binding?.label ?? "").trim();
        if (!named) name = m.hotkeys_custom_automation_detail({ name: auto.name ?? auto.id });
        detail = auto.name === null && automations.length
          ? m.hotkeys_custom_automation_gone({ id: auto.id })
          : named ? m.hotkeys_custom_automation_detail({ name: auto.name ?? auto.id }) : "";
      }
      return {
        use, name, detail, binding,
        source: keyUseSource(use),
        key: use.key ? hotkeyLabel(use.combo || use.key) : "",
        conflicts: (use.conflicts ?? []).map((id) => byId.get(id)).filter((other): other is KeyUse => !!other),
        // 提醒助手热键在提醒助手页改（D10）；Esc 固定；找不到的命令不登记、不改
        editable: (use.source === "core" || use.source === "plugin" || use.source === "custom") && use.id !== "legacy-reminder",
      };
    });
  });

  const shown = $derived.by(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return rows;
    return rows.filter((row) => {
      const hay = [row.name, row.source, row.detail, row.key, row.use.key, row.use.combo].join(" ").toLowerCase();
      return terms.every((term) => hay.includes(term));
    });
  });
  /** 0.1.x 升级、只用语音输入的人没有自己加的（后端也不列）。 */
  const legacyOnly = $derived(!!$settingsStore && usesLegacyVoiceInput($settingsStore.ui));
  /** 自己加的那一组夹在插件的和“找不到”的中间（表的顺序）。 */
  const before = $derived(shown.filter((row) => row.use.source !== "custom" && row.use.source !== "unknown"));
  const custom = $derived(shown.filter((row) => row.use.source === "custom"));
  const after = $derived(shown.filter((row) => row.use.source === "unknown"));

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

  async function startAdding(kind: Kind) {
    editing = null;
    confirming = null;
    adding = kind;
    await loadOptions();
    await tick();
    document.getElementById("hotkeys-custom")?.scrollIntoView({ block: "start" });
  }

  async function startEditing(row: Row) {
    adding = null;
    confirming = null;
    editing = row.use.id;
    await loadOptions();
  }

  function done(settings: Settings, name: string) {
    settingsStore.set(settings);
    adding = null;
    editing = null;
    toast(true, m.hotkeys_custom_saved({ name }));
  }

  async function remove(row: Row) {
    if (busy) return;
    busy = row.use.id;
    try {
      settingsStore.set(await removeCustomHotkey(row.use.id));
      toast(true, m.hotkeys_custom_removed({ name: row.name }));
    } catch (error) {
      toast(false, errText(error));
    } finally { busy = ""; }
  }

  async function confirm(row: Row) {
    if (busy) return;
    busy = row.use.id;
    try {
      settingsStore.set(await confirmHotkey(row.use.id));
      confirming = null;
      toast(true, m.hotkeys_custom_confirmed({ name: row.name }));
    } catch (error) {
      toast(false, errText(error));
    } finally { busy = ""; }
  }
</script>

{#snippet hotkeyRow(row: Row)}
  {@const conflict = row.conflicts.length > 0}
  {@const focused = focus === `hotkey-${row.use.id}`}
  {@const isCustom = row.use.source === "custom"}
  <div id={`hotkey-${row.use.id}`} data-testid={`hotkeys-row-${row.use.id}`} data-source={row.use.source}
    data-conflict={conflict ? "true" : "false"} data-focused={focused ? "true" : "false"}
    data-confirm={row.use.confirm ? "true" : "false"}
    class="scroll-mt-16 px-4 py-3 {conflict ? 'bg-destructive/5' : ''} {focused ? 'ring-2 ring-inset ring-primary/50' : ''}">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-1.5">
          <span class="text-[13px] font-medium {conflict ? 'text-destructive' : 'text-foreground'}">{row.name}</span>
          <span class="rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground" data-testid={`hotkeys-source-${row.use.id}`}>{row.source}</span>
          {#if !row.use.enabled && (row.use.source === "plugin" || isCustom)}
            <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] text-muted-foreground">{m.hotkeys_plugin_off()}</span>
          {/if}
          {#if row.use.confirm}
            <span class="rounded-md border border-warning/40 px-1.5 py-px text-[11px] text-warning" data-testid={`hotkeys-waiting-${row.use.id}`}>{m.hotkeys_custom_waiting()}</span>
          {/if}
          {#if row.use.blocked}
            <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] text-muted-foreground">{m.hotkeys_custom_blocked()}</span>
          {/if}
        </div>
        {#if row.detail}<p class="mt-0.5 mb-0 truncate text-xs text-muted-foreground" data-testid={`hotkeys-detail-${row.use.id}`}>{row.detail}</p>{/if}
        {#if row.use.blocked}<p class="mt-0.5 mb-0 text-xs text-muted-foreground" data-testid={`hotkeys-blocked-${row.use.id}`}>{row.use.blocked}</p>{/if}
        {#if row.use.lastRun && !row.use.lastRun.ok}
          <p class="mt-0.5 mb-0 text-xs text-destructive">{m.hotkeys_custom_last_failed({ message: row.use.lastRun.message })}</p>
        {/if}
        {#if conflict}
          <p class="mt-1 mb-0 text-xs text-destructive" data-testid={`hotkeys-conflict-${row.use.id}`}>{m.hotkeys_conflicts_with({ list: keyUseNames(row.conflicts) })}</p>
        {/if}
      </div>
      {#if isCustom}
        <div class="flex items-center gap-1">
          {#if row.use.confirm}
            <Button size="sm" variant="outline" data-testid={`hotkeys-confirm-${row.use.id}`} disabled={!!busy}
              onclick={() => (confirming = confirming === row.use.id ? null : row.use.id)}>{m.hotkeys_custom_confirm()}</Button>
          {/if}
          <Button size="sm" variant="ghost" data-testid={`hotkeys-edit-${row.use.id}`} disabled={!!busy}
            onclick={() => void startEditing(row)}><Pencil class="size-3.5" />{m.hotkeys_custom_edit()}</Button>
          <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10 hover:text-destructive"
            data-testid={`hotkeys-remove-${row.use.id}`} disabled={!!busy}
            onclick={() => void remove(row)}><Trash2 class="size-3.5" />{m.hotkeys_custom_remove()}</Button>
        </div>
      {:else if !row.editable}
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
    {#if confirming === row.use.id && row.use.confirm}
      <div class="mt-2 rounded-lg border border-warning/40 bg-warning/8 px-3 py-2 text-xs leading-relaxed text-foreground" data-testid={`hotkeys-confirm-box-${row.use.id}`}>
        <p class="m-0">{m.hotkeys_custom_will_do({ reason: row.use.confirm })}</p>
        <p class="m-0 font-mono text-[11px] text-muted-foreground">{m.hotkeys_custom_args_shown({ tool: String(row.binding?.tool ?? ""), args: JSON.stringify(row.binding?.args ?? {}) })}</p>
        <p class="m-0 text-muted-foreground">{m.hotkeys_custom_no_ask_later()}</p>
        <div class="mt-2 flex gap-2">
          <Button size="sm" data-testid={`hotkeys-allow-${row.use.id}`} disabled={!!busy} onclick={() => void confirm(row)}>{m.hotkeys_custom_confirm_allow()}</Button>
          <Button size="sm" variant="ghost" disabled={!!busy} onclick={() => (confirming = null)}>{m.hotkeys_custom_cancel()}</Button>
        </div>
      </div>
    {/if}
    {#if isCustom && editing === row.use.id}
      <div class="mt-2">
        <CustomHotkeyForm entry={row.binding} {options} {automations} onDone={done} onCancel={() => (editing = null)} />
      </div>
    {:else if row.editable}
      <HotkeyField id={row.use.id} statusId={row.use.statusId} title={row.name} value={row.use.key} compact
        onSave={(key) => save(row, key)} />
    {/if}
  </div>
{/snippet}

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
  {#each before as row (row.use.id)}
    {@render hotkeyRow(row)}
  {/each}
  {#if table && !legacyOnly}
    <div id="hotkeys-custom" class="scroll-mt-16 px-4 pt-4 pb-2" data-testid="hotkeys-custom">
      <div class="flex flex-wrap items-center gap-2">
        <div class="min-w-0 flex-1">
          <p class="m-0 text-[13px] font-semibold text-foreground">{m.hotkeys_custom_group()}</p>
          <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.hotkeys_custom_group_desc()}</p>
        </div>
        {#if !adding}
          <Button size="sm" variant="outline" data-testid="hotkeys-add" onclick={() => void startAdding("say")}>
            <Plus class="size-3.5" />{m.hotkeys_custom_add()}
          </Button>
        {/if}
      </div>
      {#if adding}
        {#key adding}
          <div class="mt-3">
            <CustomHotkeyForm kind={adding} {options} {automations} onDone={done} onCancel={() => (adding = null)} />
          </div>
        {/key}
      {/if}
    </div>
  {/if}
  {#each custom as row (row.use.id)}
    {@render hotkeyRow(row)}
  {/each}
  {#each after as row (row.use.id)}
    {@render hotkeyRow(row)}
  {/each}
  {#if table && query.trim() && !shown.length}
    <p class="m-0 px-4 py-6 text-center text-xs text-muted-foreground" data-testid="hotkeys-empty">{m.hotkeys_search_empty({ query: query.trim() })}</p>
  {/if}
</SettingsSection>
