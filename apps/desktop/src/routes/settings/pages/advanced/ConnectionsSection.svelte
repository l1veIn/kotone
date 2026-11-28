<script lang="ts">
  /*
   * 设置 → API 连接：给琴音（内置 Agent）用的主流大模型。
   * 版式参照 DeepSeek Harness 的模型 / 提供商设置（ModelsSection、ProviderEditor、CustomProviderCard、
   * ApiKeyInput，MIT）：上面是“你的连接”卡片列表（密钥状态点、琴音在用、能力自检），下面是按分组展示的
   * 预置提供商；选中一家后在编辑器里改地址、模型（列表里选或手填）、密钥（只写）。Kotone 自己的实现。
   */
  import { companionName } from "$lib/stores/companion";
  import { onMount } from "svelte";
  import {
    Check, CircleAlert, ExternalLink, KeyRound, LoaderCircle, Plus, Search, ShieldCheck, Sparkles, Trash2,
  } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import {
    companionBindingPatch,
    companionBindings,
    connectionSelfCheck,
    deleteConnection,
    listConnectionPresets,
    listConnections,
    openExternal,
    updateSettings,
    upsertConnection,
    usesLegacyVoiceInput,
    type CompanionCheck,
    type Connection,
    type ConnectionInfo,
    type ConnectionPreset,
    type ConnectionPresetGroup,
  } from "../../../../lib/ipc";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import { isDeepSeekConnection, switchToDeepSeek } from "../../../../lib/deepseekSetup";
  import { m } from "$lib/paraglide/messages.js";
  import { getLocale } from "$lib/paraglide/runtime.js";

  const groups: { id: ConnectionPresetGroup; label: string; hint: string }[] = [
    { id: "recommended", label: m.system_connections_group_recommended(), hint: m.system_connections_group_recommended_hint() },
    { id: "china", label: m.system_connections_group_china(), hint: m.system_connections_group_china_hint() },
    { id: "global", label: m.system_connections_group_global(), hint: m.system_connections_group_global_hint() },
    { id: "aggregator", label: m.system_connections_group_aggregator(), hint: m.system_connections_group_aggregator_hint() },
    { id: "custom", label: m.system_connections_group_custom(), hint: m.system_connections_group_custom_hint() },
  ];

  let connections = $state<ConnectionInfo[]>([]);
  let presets = $state<ConnectionPreset[]>([]);
  let loading = $state(true);
  let query = $state("");
  let editing = $state<Connection | null>(null);
  let editingIsNew = $state(false);
  let apiKey = $state("");
  let useAfterSave = $state(false);
  let saving = $state(false);
  let checking = $state(false);
  let confirmDelete = $state<ConnectionInfo | null>(null);

  const companionEnabled = $derived(!usesLegacyVoiceInput($settingsStore?.ui));
  const agentConnectionId = $derived(companionEnabled ? companionBindings($settingsStore?.assistant)?.connectionId ?? "" : "");
  const check = $derived<CompanionCheck | null>($settingsStore?.assistant.companion.check ?? null);
  const presetOf = (id: string) => presets.find((preset) => preset.id === id);
  const editingPreset = $derived(editing ? presetOf(editing.provider) : undefined);
  const editingSaved = $derived(editing ? connections.find((item) => item.id === editing!.id) : undefined);
  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return presets;
    return presets.filter((p) => [p.displayName, p.id, p.description, ...p.models].some((text) => text.toLowerCase().includes(q)));
  });

  onMount(async () => {
    try {
      [connections, presets] = await Promise.all([listConnections(), listConnectionPresets()]);
    } catch (error) {
      toast(false, m.system_connections_load_failed({ error: errText(error) }));
    } finally {
      loading = false;
    }
  });

  function host(url: string): string {
    try { return new URL(url).host; } catch { return url; }
  }
  function groupLabel(id: string | undefined): string {
    return groups.find((g) => g.id === id)?.label ?? m.system_connections_group_custom();
  }

  function openNew(preset: ConnectionPreset) {
    editing = {
      id: `${preset.id}-${Date.now().toString(36)}`,
      displayName: preset.displayName,
      kind: "remote",
      provider: preset.id,
      baseUrl: preset.defaultBaseUrl,
      model: preset.defaultModel,
    };
    editingIsNew = true;
    apiKey = "";
    useAfterSave = false;
  }
  function openEdit(connection: ConnectionInfo) {
    const { hasApiKey: _hasApiKey, ...plain } = connection;
    editing = { ...plain };
    editingIsNew = false;
    apiKey = "";
    useAfterSave = false;
  }
  function closeEditor() {
    editing = null;
    apiKey = "";
  }

  async function save() {
    if (!editing || saving) return;
    saving = true;
    const id = editing.id;
    // 存了 DeepSeek 的 Key（新加或换 Key）：她换成它，其余能力跟着换（ADR 051，和引导里填 Key 一样）
    const newDeepSeekKey = isDeepSeekConnection(editing) && !!apiKey.trim();
    try {
      settingsStore.set(await upsertConnection(editing, apiKey || undefined));
      connections = await listConnections();
      closeEditor();
      if (newDeepSeekKey) await useDeepSeek(id);
      else {
        toast(true, m.system_connections_saved());
        if (useAfterSave) await useForAgent(id);
      }
    } catch (error) {
      toast(false, m.system_connections_save_failed({ error: errText(error) }));
    } finally {
      saving = false;
    }
  }

  /** 测一次刚存的 DeepSeek Key（读模型列表，不花钱）：通过了她就换成它；没通过就留在本页，什么都不换。 */
  async function useDeepSeek(id: string) {
    let result: CompanionCheck;
    try {
      result = await connectionSelfCheck(id);
    } catch (error) {
      result = { connectionId: id, baseUrl: "", model: "", passed: false, checkedAtMs: Date.now(), message: errText(error) } as CompanionCheck;
    }
    if (!result.passed) {
      otherChecks = { ...otherChecks, [id]: result };
      toast(false, m.system_connections_deepseek_not_switched({ name: $companionName, error: result.message }));
      return;
    }
    try {
      const switched = await switchToDeepSeek(id, $settingsStore);
      settingsStore.set(switched.settings);
      const failed = switched.items.filter((item) => !item.ok).map((item) => item.label);
      toast(true, failed.length
        ? m.system_connections_deepseek_switched_partly({ name: $companionName, failed: failed.join("、") })
        : m.system_connections_deepseek_switched({ name: $companionName }));
    } catch (error) {
      toast(false, m.system_connections_use_failed({ name: $companionName, error: errText(error) }));
    }
  }

  async function remove(connection: ConnectionInfo) {
    saving = true;
    try {
      settingsStore.set(await deleteConnection(connection.id));
      connections = await listConnections();
      confirmDelete = null;
      toast(true, m.system_connections_deleted());
    } catch (error) {
      toast(false, m.system_connections_delete_failed({ error: errText(error) }));
    } finally {
      saving = false;
    }
  }

  /** 设为琴音用的连接（不需要先测试：选好、有密钥就能用）。 */
  async function useForAgent(id: string) {
    if (!companionEnabled) return;
    try {
      settingsStore.set(await updateSettings({ assistant: companionBindingPatch($settingsStore?.assistant, { connectionId: id }) }));
      toast(true, m.system_connections_use_done({ name: $companionName }));
    } catch (error) {
      toast(false, m.system_connections_use_failed({ name: $companionName, error: errText(error) }));
    }
  }

  /** 测试连接（照 DeepSeek Harness：读一次模型列表，不花 token）。其他连接的结果只留在本页；琴音在用的那条写进设置。 */
  let otherChecks = $state<Record<string, CompanionCheck>>({});
  let checkingId = $state<string | null>(null);

  async function runCheck(id: string) {
    if (checking) return;
    checking = true;
    checkingId = id;
    try {
      const result = await connectionSelfCheck(id);
      if (result.connectionId === agentConnectionId) {
        settingsStore.update((s) => s && { ...s, assistant: { ...s.assistant, companion: { ...s.assistant.companion, check: result } } });
      } else {
        otherChecks = { ...otherChecks, [id]: result };
      }
      toast(result.passed, result.passed ? m.system_connections_check_passed() : result.message);
    } catch (error) {
      const message = m.system_connections_check_unreachable({ error: errText(error) });
      const failed = { connectionId: id, baseUrl: "", model: "", passed: false, checkedAtMs: Date.now(), message } as CompanionCheck;
      otherChecks = { ...otherChecks, [id]: failed };
      toast(false, message);
    } finally {
      checking = false;
      checkingId = null;
    }
  }

  async function openKeyPage(url: string) {
    try { await openExternal(url); }
    catch (error) { toast(false, m.system_connections_open_key_page_failed({ error: errText(error) })); }
  }

  function checkedAt(ms: number): string {
    return new Intl.DateTimeFormat(getLocale(), { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(ms);
  }
</script>

<div class="ui-scope space-y-6" data-testid="connections-section">
  <div class="flex items-start gap-3 rounded-2xl border border-primary/25 bg-primary/6 px-4 py-3.5">
    <Sparkles class="mt-0.5 size-4 shrink-0 text-primary" />
    <div class="min-w-0 text-[13px] leading-relaxed">
      <p class="m-0 font-medium text-foreground">{m.system_connections_intro_title({ name: $companionName })}</p>
      <p class="mt-0.5 mb-0 text-muted-foreground">
        {m.system_connections_intro_before({ name: $companionName })}<strong class="font-medium text-foreground">{m.system_connections_test_button()}</strong>{m.system_connections_intro_after()}
      </p>
    </div>
  </div>

  {#if loading}
    <p class="text-sm text-muted-foreground">{m.system_connections_loading()}</p>
  {:else}
    <SettingsSection title={m.system_connections_list_title()} description={m.system_connections_list_description({ name: $companionName })} flush testid="connections-list">
      {#if connections.length === 0}
        <div class="flex flex-col items-center gap-1 px-4 py-8 text-center">
          <KeyRound class="mb-1 size-5 text-muted-foreground" />
          <p class="m-0 text-[13px] text-foreground">{m.system_connections_empty_title()}</p>
          <p class="m-0 text-xs text-muted-foreground">{m.system_connections_empty_description()}</p>
        </div>
      {/if}
      {#each connections as connection (connection.id)}
        {@const forAgent = connection.id === agentConnectionId}
        {@const preset = presetOf(connection.provider)}
        {@const ownCheck = forAgent ? (check && check.connectionId === connection.id ? check : null) : (otherChecks[connection.id] ?? null)}
        {@const cardChecking = checking && checkingId === connection.id}
        <article class="px-4 py-3.5" data-testid={`connection-card-${connection.id}`}>
          <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-muted text-sm font-semibold text-foreground/80" aria-hidden="true">
              {(preset?.displayName ?? connection.displayName).slice(0, 1)}
            </div>
            <div class="min-w-0 flex-1">
              <p class="m-0 flex flex-wrap items-center gap-2 text-sm font-medium text-foreground">
                <span class="truncate">{connection.displayName}</span>
                {#if forAgent}<span class="rounded-md bg-primary/15 px-1.5 py-px text-[11px] font-medium text-primary">{m.system_connections_in_use_badge({ name: $companionName })}</span>{/if}
                <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] font-normal text-muted-foreground">{groupLabel(preset?.group)}</span>
              </p>
              <p class="mt-0.5 mb-0 flex min-w-0 flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
                <span class="font-mono">{connection.model}</span><span aria-hidden="true">·</span>
                <span class="truncate">{host(connection.baseUrl)}</span><span aria-hidden="true">·</span>
                <span class="inline-flex items-center gap-1 {connection.hasApiKey ? 'text-success' : 'text-destructive'}">
                  <span class="size-1.5 rounded-full {connection.hasApiKey ? 'bg-success' : 'bg-destructive'}"></span>{connection.hasApiKey ? m.system_connections_key_saved() : m.system_connections_key_missing()}
                </span>
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              {#if companionEnabled && !forAgent}
                <Button size="sm" variant="outline" data-testid="connection-use" disabled={checking} onclick={() => void useForAgent(connection.id)}>{m.system_connections_use_button({ name: $companionName })}</Button>
              {/if}
              <Button size="sm" variant={forAgent ? "outline" : "ghost"} data-testid="connection-self-check" disabled={checking || !connection.hasApiKey} onclick={() => void runCheck(connection.id)}>
                {#if cardChecking}<LoaderCircle class="size-3.5 animate-spin" />{:else}<ShieldCheck class="size-3.5" />{/if}{m.system_connections_test_button()}
              </Button>
              <Button size="sm" variant="ghost" onclick={() => openEdit(connection)}>{m.system_connections_edit_button()}</Button>
              <Button size="icon-sm" variant="ghost" class="text-muted-foreground hover:text-destructive" aria-label={m.system_connections_delete_label({ name: connection.displayName })} disabled={saving} onclick={() => { confirmDelete = connection; }}><Trash2 class="size-3.5" /></Button>
            </div>
          </div>
          {#if ownCheck || cardChecking}
            <div class="mt-3 ml-12 flex items-start gap-2 rounded-xl bg-muted/50 px-3 py-2 text-xs" data-testid="connection-check-result">
              {#if cardChecking}
                <LoaderCircle class="mt-0.5 size-3.5 shrink-0 animate-spin text-primary" /><span class="text-muted-foreground">{m.system_connections_checking()}</span>
              {:else if ownCheck}
                {#if ownCheck.passed}<Check class="mt-0.5 size-3.5 shrink-0 text-success" />{:else}<CircleAlert class="mt-0.5 size-3.5 shrink-0 text-destructive" />{/if}
                <span class="min-w-0">
                  <span class="font-medium {ownCheck.passed ? 'text-success' : 'text-destructive'}">{ownCheck.passed ? m.system_connections_check_passed() : m.system_connections_check_failed()}</span>
                  <span class="text-muted-foreground"> · {ownCheck.message} · {checkedAt(ownCheck.checkedAtMs)}{ownCheck.model && ownCheck.model !== connection.model ? m.system_connections_check_model_at_time({ model: ownCheck.model }) : ""}</span>
                </span>
              {/if}
            </div>
          {/if}
        </article>
      {/each}
    </SettingsSection>

    <section aria-labelledby="connection-providers-title" data-testid="connection-providers">
      <div class="mb-3 flex flex-wrap items-end justify-between gap-3 px-1">
        <div>
          <h2 id="connection-providers-title" class="m-0 text-sm font-semibold">{m.system_connections_add_title()}</h2>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.system_connections_add_description({ count: presets.length })}</p>
        </div>
        <div class="relative w-56 max-w-full">
          <Search class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input bind:value={query} placeholder={m.system_connections_search()} aria-label={m.system_connections_search()} class="h-8 pl-8 text-[13px]" />
        </div>
      </div>
      <div class="space-y-5">
        {#each groups as group (group.id)}
          {@const items = filtered.filter((p) => p.group === group.id)}
          {#if items.length}
            <div>
              <p class="mb-2 flex items-baseline gap-2 px-1 text-xs"><span class="font-medium text-foreground/90">{group.label}</span><span class="text-muted-foreground">{group.hint}</span></p>
              <div class="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
                {#each items as preset (preset.id)}
                  <button type="button" data-testid={`connection-preset-${preset.id}`} disabled={saving}
                    class="group flex min-w-0 items-start gap-3 rounded-xl border border-hairline bg-card/60 px-3.5 py-3 text-left transition-colors hover:border-primary/45 hover:bg-card disabled:opacity-50"
                    onclick={() => openNew(preset)}>
                    <span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-[13px] font-semibold text-foreground/80 group-hover:text-primary" aria-hidden="true">
                      {#if preset.group === "custom"}<Plus class="size-4" />{:else}{preset.displayName.slice(0, 1)}{/if}
                    </span>
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-[13px] font-medium text-foreground">{preset.displayName}</span>
                      <span class="mt-0.5 line-clamp-2 block text-xs leading-snug text-muted-foreground">{preset.description}</span>
                      {#if preset.models.length}<span class="mt-1 block truncate font-mono text-[11px] text-muted-foreground/80">{preset.models.join(" · ")}</span>{/if}
                    </span>
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        {/each}
        {#if !filtered.length}<p class="px-1 text-xs text-muted-foreground">{m.system_connections_search_empty({ query })}</p>{/if}
      </div>
    </section>
  {/if}
</div>

<Dialog.Root open={editing !== null} onOpenChange={(open) => { if (!open && !saving) closeEditor(); }}>
  <Dialog.Content class="ui-scope max-h-[88vh] overflow-y-auto sm:max-w-lg" data-testid="connection-editor">
    {#if editing}
      <Dialog.Header>
        <Dialog.Title>{editingIsNew ? m.system_connections_editor_add_title() : m.system_connections_editor_edit_title()}</Dialog.Title>
        <Dialog.Description>
          {editingPreset ? `${editingPreset.displayName} · ${groupLabel(editingPreset.group)}` : m.system_connections_group_custom()}{editingPreset?.description ? ` — ${editingPreset.description}` : ""}
        </Dialog.Description>
      </Dialog.Header>
      <form class="space-y-4" onsubmit={(event) => { event.preventDefault(); void save(); }}>
        <label class="block space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">{m.system_connections_name_label()}</span>
          <Input bind:value={editing.displayName} data-testid="connection-name" />
        </label>
        <label class="block space-y-1.5">
          <span class="flex items-center justify-between text-xs font-medium text-muted-foreground">
            {m.system_connections_base_url_label()}
            {#if editingPreset?.defaultBaseUrl && editing.baseUrl !== editingPreset.defaultBaseUrl}
              <button type="button" class="font-normal text-primary hover:underline" onclick={() => { if (editing && editingPreset) editing.baseUrl = editingPreset.defaultBaseUrl; }}>{m.system_common_restore_default()}</button>
            {/if}
          </span>
          <Input bind:value={editing.baseUrl} placeholder="https://…/v1" class="font-mono text-xs" data-testid="connection-base-url" />
        </label>
        <div class="space-y-1.5">
          <label for="connection-model" class="text-xs font-medium text-muted-foreground">{m.system_connections_model_label()}</label>
          <Input id="connection-model" bind:value={editing.model} placeholder={m.system_connections_model_placeholder()} class="font-mono text-xs" data-testid="connection-model" />
          {#if editingPreset?.models.length}
            <div class="flex flex-wrap gap-1.5" role="group" aria-label={m.system_connections_model_suggestions()}>
              {#each editingPreset.models as model (model)}
                {@const active = editing.model === model}
                <button type="button" aria-pressed={active}
                  class="h-6 rounded-full border px-2.5 font-mono text-[11px] transition-colors {active ? 'border-primary bg-primary/12 text-primary' : 'border-hairline text-muted-foreground hover:text-foreground'}"
                  onclick={() => { if (editing) editing.model = model; }}>{model}</button>
              {/each}
            </div>
          {/if}
          <p class="m-0 text-[11px] text-muted-foreground">{m.system_connections_model_hint()}</p>
        </div>
        <div class="space-y-1.5">
          <span class="flex items-center justify-between text-xs font-medium text-muted-foreground">
            <label for="connection-api-key">API key</label>
            {#if editingPreset?.apiKeyUrl}
              <button type="button" class="inline-flex items-center gap-1 font-normal text-primary hover:underline" onclick={() => void openKeyPage(editingPreset!.apiKeyUrl!)}>{m.system_connections_get_key()}<ExternalLink class="size-3" /></button>
            {/if}
          </span>
          <Input id="connection-api-key" type="password" autocomplete="off" data-testid="connection-api-key" bind:value={apiKey}
            placeholder={editingSaved?.hasApiKey ? m.system_connections_key_placeholder_saved() : m.system_connections_key_placeholder_new()} />
          <p class="m-0 text-[11px] text-muted-foreground">{m.system_connections_key_hint()}</p>
        </div>
        {#if isDeepSeekConnection(editing)}
          <!-- DeepSeek：存了 Key 她就换成它（ADR 051），不用再勾 -->
          <p class="m-0 rounded-xl bg-muted/50 px-3 py-2.5 text-xs text-foreground/85" data-testid="connection-deepseek-note">{m.system_connections_deepseek_note({ name: $companionName })}</p>
        {:else if companionEnabled && editing.id !== agentConnectionId}
          <label class="flex items-start gap-2 rounded-xl bg-muted/50 px-3 py-2.5 text-xs">
            <input type="checkbox" class="mt-0.5 accent-[var(--color-primary)]" bind:checked={useAfterSave} data-testid="connection-use-after-save" />
            <span><span class="font-medium text-foreground">{m.system_connections_use_after_save({ name: $companionName })}</span></span>
          </label>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" size="sm" onclick={closeEditor}>{m.system_common_cancel()}</Button>
          <Button type="submit" size="sm" data-testid="connection-save" disabled={saving}>{m.system_connections_save_button()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root open={confirmDelete !== null} onOpenChange={(open) => { if (!open && !saving) confirmDelete = null; }}>
  <Dialog.Content class="ui-scope" data-testid="connection-delete-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.system_connections_delete_title()}</Dialog.Title>
      <Dialog.Description>{m.system_connections_delete_description({ name: confirmDelete?.displayName ?? "" })}{confirmDelete?.id === agentConnectionId ? m.system_connections_delete_in_use({ name: $companionName }) : ""}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={saving} onclick={() => { confirmDelete = null; }}>{m.system_common_cancel()}</Button>
      <Button variant="destructive" size="sm" data-testid="connection-delete-confirm" disabled={saving} onclick={() => { if (confirmDelete) void remove(confirmDelete); }}>{m.system_common_delete()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
