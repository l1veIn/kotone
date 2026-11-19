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

  const groups: { id: ConnectionPresetGroup; label: string; hint: string }[] = [
    { id: "recommended", label: "推荐", hint: "工具调用稳定、国内可直连" },
    { id: "china", label: "国内厂商", hint: "国内平台与编程套餐" },
    { id: "global", label: "海外厂商", hint: "需要能访问对应服务" },
    { id: "aggregator", label: "聚合平台", hint: "一个密钥用多家模型" },
    { id: "custom", label: "自定义", hint: "任意 OpenAI 兼容接口" },
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
      toast(false, `加载连接失败：${errText(error)}`);
    } finally {
      loading = false;
    }
  });

  function host(url: string): string {
    try { return new URL(url).host; } catch { return url; }
  }
  function groupLabel(id: string | undefined): string {
    return groups.find((g) => g.id === id)?.label ?? "自定义";
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
    try {
      settingsStore.set(await upsertConnection(editing, apiKey || undefined));
      connections = await listConnections();
      closeEditor();
      toast(true, "连接已保存，密钥不会写入配置文件");
      if (useAfterSave) await useForAgent(id);
    } catch (error) {
      toast(false, `保存连接失败：${errText(error)}`);
    } finally {
      saving = false;
    }
  }

  async function remove(connection: ConnectionInfo) {
    saving = true;
    try {
      settingsStore.set(await deleteConnection(connection.id));
      connections = await listConnections();
      confirmDelete = null;
      toast(true, "连接已删除");
    } catch (error) {
      toast(false, `删除连接失败：${errText(error)}`);
    } finally {
      saving = false;
    }
  }

  /** 设为琴音用的连接（不需要先测试：选好、有密钥就能用）。 */
  async function useForAgent(id: string) {
    if (!companionEnabled) return;
    try {
      settingsStore.set(await updateSettings({ assistant: companionBindingPatch($settingsStore?.assistant, { connectionId: id }) }));
      toast(true, `已设为${$companionName}的连接`);
    } catch (error) {
      toast(false, `没能设为${$companionName}的连接：${errText(error)}`);
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
      toast(result.passed, result.passed ? "连接正常" : result.message);
    } catch (error) {
      const message = `连不上：${errText(error)}`;
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
    catch (error) { toast(false, `打开 API key 页面失败：${errText(error)}`); }
  }

  function checkedAt(ms: number): string {
    return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(ms);
  }
</script>

<div class="ui-scope space-y-6" data-testid="connections-section">
  <div class="flex items-start gap-3 rounded-2xl border border-primary/25 bg-primary/6 px-4 py-3.5">
    <Sparkles class="mt-0.5 size-4 shrink-0 text-primary" />
    <div class="min-w-0 text-[13px] leading-relaxed">
      <p class="m-0 font-medium text-foreground">给{$companionName}用的主流大模型</p>
      <p class="mt-0.5 mb-0 text-muted-foreground">
        选一家、填好 API key，设为{$companionName}的连接就能用；想确认能不能连上，点<strong class="font-medium text-foreground">测试连接</strong>（读一次模型列表，不花 token）。
        文字处理的润色、翻译也能用这里的连接。API key 存进系统凭据库，不会写入 config.json。
      </p>
    </div>
  </div>

  {#if loading}
    <p class="text-sm text-muted-foreground">正在读取连接…</p>
  {:else}
    <SettingsSection title="你的连接" description={`${$companionName}用其中一条。`} flush testid="connections-list">
      {#if connections.length === 0}
        <div class="flex flex-col items-center gap-1 px-4 py-8 text-center">
          <KeyRound class="mb-1 size-5 text-muted-foreground" />
          <p class="m-0 text-[13px] text-foreground">还没有连接</p>
          <p class="m-0 text-xs text-muted-foreground">在下面选一家提供商，填上 API key 就能用。推荐先试 DeepSeek 或通义千问。</p>
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
                {#if forAgent}<span class="rounded-md bg-primary/15 px-1.5 py-px text-[11px] font-medium text-primary">{$companionName}在用</span>{/if}
                <span class="rounded-md border border-hairline px-1.5 py-px text-[11px] font-normal text-muted-foreground">{groupLabel(preset?.group)}</span>
              </p>
              <p class="mt-0.5 mb-0 flex min-w-0 flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
                <span class="font-mono">{connection.model}</span><span aria-hidden="true">·</span>
                <span class="truncate">{host(connection.baseUrl)}</span><span aria-hidden="true">·</span>
                <span class="inline-flex items-center gap-1 {connection.hasApiKey ? 'text-success' : 'text-destructive'}">
                  <span class="size-1.5 rounded-full {connection.hasApiKey ? 'bg-success' : 'bg-destructive'}"></span>{connection.hasApiKey ? "已保存密钥" : "缺少密钥"}
                </span>
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              {#if companionEnabled && !forAgent}
                <Button size="sm" variant="outline" data-testid="connection-use" disabled={checking} onclick={() => void useForAgent(connection.id)}>设为{$companionName}的连接</Button>
              {/if}
              <Button size="sm" variant={forAgent ? "outline" : "ghost"} data-testid="connection-self-check" disabled={checking || !connection.hasApiKey} onclick={() => void runCheck(connection.id)}>
                {#if cardChecking}<LoaderCircle class="size-3.5 animate-spin" />{:else}<ShieldCheck class="size-3.5" />{/if}测试连接
              </Button>
              <Button size="sm" variant="ghost" onclick={() => openEdit(connection)}>编辑</Button>
              <Button size="icon-sm" variant="ghost" class="text-muted-foreground hover:text-destructive" aria-label={`删除 ${connection.displayName}`} disabled={saving} onclick={() => { confirmDelete = connection; }}><Trash2 class="size-3.5" /></Button>
            </div>
          </div>
          {#if ownCheck || cardChecking}
            <div class="mt-3 ml-12 flex items-start gap-2 rounded-xl bg-muted/50 px-3 py-2 text-xs" data-testid="connection-check-result">
              {#if cardChecking}
                <LoaderCircle class="mt-0.5 size-3.5 shrink-0 animate-spin text-primary" /><span class="text-muted-foreground">正在测试：读一次服务商的模型列表…</span>
              {:else if ownCheck}
                {#if ownCheck.passed}<Check class="mt-0.5 size-3.5 shrink-0 text-success" />{:else}<CircleAlert class="mt-0.5 size-3.5 shrink-0 text-destructive" />{/if}
                <span class="min-w-0">
                  <span class="font-medium {ownCheck.passed ? 'text-success' : 'text-destructive'}">{ownCheck.passed ? "连接正常" : "有问题"}</span>
                  <span class="text-muted-foreground"> · {ownCheck.message} · {checkedAt(ownCheck.checkedAtMs)}{ownCheck.model && ownCheck.model !== connection.model ? `（当时的模型：${ownCheck.model}）` : ""}</span>
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
          <h2 id="connection-providers-title" class="m-0 text-sm font-semibold">添加连接</h2>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">{presets.length} 家提供商，都走 OpenAI 兼容接口。选一家，填上密钥即可。</p>
        </div>
        <div class="relative w-56 max-w-full">
          <Search class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input bind:value={query} placeholder="搜索提供商或模型" aria-label="搜索提供商或模型" class="h-8 pl-8 text-[13px]" />
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
        {#if !filtered.length}<p class="px-1 text-xs text-muted-foreground">没有找到「{query}」。可以用“自定义”接入任意 OpenAI 兼容接口。</p>{/if}
      </div>
    </section>
  {/if}
</div>

<Dialog.Root open={editing !== null} onOpenChange={(open) => { if (!open && !saving) closeEditor(); }}>
  <Dialog.Content class="ui-scope max-h-[88vh] overflow-y-auto sm:max-w-lg" data-testid="connection-editor">
    {#if editing}
      <Dialog.Header>
        <Dialog.Title>{editingIsNew ? "添加 API 连接" : "编辑 API 连接"}</Dialog.Title>
        <Dialog.Description>
          {editingPreset ? `${editingPreset.displayName} · ${groupLabel(editingPreset.group)}` : "自定义"}{editingPreset?.description ? ` — ${editingPreset.description}` : ""}
        </Dialog.Description>
      </Dialog.Header>
      <form class="space-y-4" onsubmit={(event) => { event.preventDefault(); void save(); }}>
        <label class="block space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">显示名称</span>
          <Input bind:value={editing.displayName} data-testid="connection-name" />
        </label>
        <label class="block space-y-1.5">
          <span class="flex items-center justify-between text-xs font-medium text-muted-foreground">
            接口地址
            {#if editingPreset?.defaultBaseUrl && editing.baseUrl !== editingPreset.defaultBaseUrl}
              <button type="button" class="font-normal text-primary hover:underline" onclick={() => { if (editing && editingPreset) editing.baseUrl = editingPreset.defaultBaseUrl; }}>恢复默认</button>
            {/if}
          </span>
          <Input bind:value={editing.baseUrl} placeholder="https://…/v1" class="font-mono text-xs" data-testid="connection-base-url" />
        </label>
        <div class="space-y-1.5">
          <label for="connection-model" class="text-xs font-medium text-muted-foreground">模型</label>
          <Input id="connection-model" bind:value={editing.model} placeholder="模型名，可以手填" class="font-mono text-xs" data-testid="connection-model" />
          {#if editingPreset?.models.length}
            <div class="flex flex-wrap gap-1.5" role="group" aria-label="常用模型">
              {#each editingPreset.models as model (model)}
                {@const active = editing.model === model}
                <button type="button" aria-pressed={active}
                  class="h-6 rounded-full border px-2.5 font-mono text-[11px] transition-colors {active ? 'border-primary bg-primary/12 text-primary' : 'border-hairline text-muted-foreground hover:text-foreground'}"
                  onclick={() => { if (editing) editing.model = model; }}>{model}</button>
              {/each}
            </div>
          {/if}
          <p class="m-0 text-[11px] text-muted-foreground">要选能稳定调用工具的模型；拿不准就用默认的。</p>
        </div>
        <div class="space-y-1.5">
          <span class="flex items-center justify-between text-xs font-medium text-muted-foreground">
            <label for="connection-api-key">API key</label>
            {#if editingPreset?.apiKeyUrl}
              <button type="button" class="inline-flex items-center gap-1 font-normal text-primary hover:underline" onclick={() => void openKeyPage(editingPreset!.apiKeyUrl!)}>获取 API key<ExternalLink class="size-3" /></button>
            {/if}
          </span>
          <Input id="connection-api-key" type="password" autocomplete="off" data-testid="connection-api-key" bind:value={apiKey}
            placeholder={editingSaved?.hasApiKey ? "已保存，留空则不修改" : "不会写入配置文件"} />
          <p class="m-0 text-[11px] text-muted-foreground">只写：保存后存进系统凭据库，界面不会再显示。</p>
        </div>
        {#if companionEnabled && editing.id !== agentConnectionId}
          <label class="flex items-start gap-2 rounded-xl bg-muted/50 px-3 py-2.5 text-xs">
            <input type="checkbox" class="mt-0.5 accent-[var(--color-primary)]" bind:checked={useAfterSave} data-testid="connection-use-after-save" />
            <span><span class="font-medium text-foreground">保存后设为{$companionName}的连接</span></span>
          </label>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" size="sm" onclick={closeEditor}>取消</Button>
          <Button type="submit" size="sm" data-testid="connection-save" disabled={saving}>保存连接</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root open={confirmDelete !== null} onOpenChange={(open) => { if (!open && !saving) confirmDelete = null; }}>
  <Dialog.Content class="ui-scope" data-testid="connection-delete-dialog">
    <Dialog.Header>
      <Dialog.Title>删除这条连接？</Dialog.Title>
      <Dialog.Description>「{confirmDelete?.displayName}」和它的密钥会从本机删除。{confirmDelete?.id === agentConnectionId ? `${$companionName}正在用它，删除后要重新选一条。` : ""}</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" size="sm" disabled={saving} onclick={() => { confirmDelete = null; }}>取消</Button>
      <Button variant="destructive" size="sm" data-testid="connection-delete-confirm" disabled={saving} onclick={() => { if (confirmDelete) void remove(confirmDelete); }}>删除</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
