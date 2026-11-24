<script lang="ts">
  // 输入框底栏的“她用的模型”：一个小按钮写着当前的模型，点开按 API 连接分组列出模型（照 DeepSeek Harness 输入框的模型下拉）。
  // 选一个就换。选了别的连接里的模型，会同时换她用的连接（assistant.connectionId；0.1.x 升级用户是 companionBindings.connectionId）
  // 和那条连接的模型——和原来「设置 → 她怎么回答 → 她用的模型」改的是同一组字段。
  // 2026-11-21 挪进设置（ADR 029 待定），2026-11-25 玩家说“模型选择这块，我觉得还是放在 chat 聊天吧”，挪回来（交接第十一节第 8 条）。
  // 模型清单照 DeepSeek Harness：读服务商的模型列表（GET /models，不花 token），读不到时用预置的模型名。换模型不自检。
  import { untrack } from "svelte";
  import { Check, ChevronDown, Cpu, LoaderCircle } from "@lucide/svelte";
  import * as Menu from "$lib/components/ui/dropdown-menu";
  import { m } from "$lib/paraglide/messages.js";
  import {
    companionBindingPatch, companionBindings, connectionModels, listConnectionPresets, updateSettings, upsertConnection,
    type ConnectionInfo, type ConnectionPreset,
  } from "../ipc";
  import { errText, settingsStore, toast } from "../stores/ui";
  import { CHAT_MODEL_ANCHOR } from "../stores/navigation";

  let { connections, openRequest = 0, active = true, onOpenConnections, onchanged }: {
    connections: ConnectionInfo[];
    /** 每加一就打开一次菜单（聊天里“去看看：她用的模型”跳过来时）。 */
    openRequest?: number;
    /** 聊天页在显示（切去角色的分页时聊天藏起来但还挂着：菜单跟着关掉）。 */
    active?: boolean;
    /** 「设置 → API 连接」：添加连接、填 Key。 */
    onOpenConnections: () => void;
    /** 换好了（让聊天页重新读一次她准备好没有）。 */
    onchanged?: () => void;
  } = $props();

  let open = $state(false);
  let busy = $state(false);
  let presets = $state<ConnectionPreset[]>([]);
  /** 服务商实际列出的模型（每条连接读一次，设置窗口开着时复用）。 */
  let listed = $state<Record<string, string[]>>({});
  /** 刚保存的模型（连接清单还没刷新时先显示这个）。 */
  let saved = $state<Record<string, string>>({});

  const connectionId = $derived(companionBindings($settingsStore?.assistant)?.connectionId ?? "");
  const current = $derived(connections.find((c) => c.id === connectionId) ?? null);
  /** 能选模型的连接：有 Key 的在线连接。 */
  const usable = $derived(connections.filter((c) => c.hasApiKey && c.kind === "remote"));
  const modelOf = (c: ConnectionInfo) => saved[c.id] ?? c.model;
  const currentModel = $derived(current?.hasApiKey ? modelOf(current) : "");

  function modelsOf(c: ConnectionInfo): string[] {
    const list = [...(listed[c.id]?.length ? listed[c.id] : presets.find((p) => p.id === c.provider)?.models ?? [])];
    const model = modelOf(c);
    if (model && !list.includes(model)) list.unshift(model);
    return list;
  }

  async function loadModels() {
    if (!presets.length) presets = await listConnectionPresets().catch(() => [] as ConnectionPreset[]);
    await Promise.all(usable.filter((c) => !(c.id in listed)).map(async (c) => {
      const reply = await connectionModels(c.id).catch(() => null);
      listed = { ...listed, [c.id]: reply?.supported ? reply.models : [] };
    }));
  }
  $effect(() => { if (open) untrack(() => void loadModels()); });

  // “去看看：她用的模型”：打开菜单（首次挂载不算）
  let seenRequest = untrack(() => openRequest);
  $effect(() => {
    const request = openRequest;
    if (request === seenRequest) return;
    seenRequest = request;
    open = true;
  });
  $effect(() => { if (!active) open = false; });

  async function choose(c: ConnectionInfo, model: string) {
    if (busy || (c.id === connectionId && modelOf(c) === model)) return;
    busy = true;
    try {
      if (c.id !== connectionId) settingsStore.set(await updateSettings({ assistant: companionBindingPatch($settingsStore?.assistant, { connectionId: c.id }) }));
      if (modelOf(c) !== model) {
        const { hasApiKey: _key, ...connection } = c;
        settingsStore.set(await upsertConnection({ ...connection, model }));
        saved = { ...saved, [c.id]: model };
      }
      toast(true, m.chat_model_switched({ model }));
    } catch (error) {
      toast(false, m.chat_model_switch_failed({ error: errText(error) }));
    } finally {
      busy = false;
      onchanged?.();
    }
  }
</script>

<Menu.DropdownMenu bind:open>
  <Menu.DropdownMenuTrigger id={CHAT_MODEL_ANCHOR} data-testid="composer-model" disabled={busy} data-model={currentModel}
    aria-label={currentModel ? m.chat_model_trigger({ model: currentModel }) : m.chat_model_choose()}
    title={currentModel ? m.chat_model_trigger({ model: currentModel }) : m.chat_model_choose()}
    class="flex h-8 max-w-44 min-w-0 shrink items-center gap-1 rounded-full px-2.5 text-xs transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60
      {currentModel ? 'text-muted-foreground' : 'text-warning'}">
    {#if busy}<LoaderCircle class="size-3.5 shrink-0 animate-spin" />{:else}<Cpu class="size-3.5 shrink-0" />{/if}
    <span class="truncate">{currentModel || m.chat_model_choose()}</span><ChevronDown class="size-3 shrink-0 opacity-70" />
  </Menu.DropdownMenuTrigger>
  <Menu.DropdownMenuContent align="end" side="top" class="max-h-80 w-72 overflow-y-auto" data-testid="composer-model-menu">
    <Menu.DropdownMenuLabel class="text-xs text-muted-foreground">{m.chat_model_menu_label()}</Menu.DropdownMenuLabel>
    {#if usable.length === 0}
      <p class="m-0 px-2 pb-2 text-xs leading-snug text-muted-foreground" data-testid="composer-model-none">{m.chat_model_none()}</p>
    {/if}
    {#each usable as c (c.id)}
      <Menu.DropdownMenuGroup>
        {#if usable.length > 1}
          <Menu.DropdownMenuGroupHeading class="pt-1.5 text-[11px] font-normal text-muted-foreground">{c.displayName}</Menu.DropdownMenuGroupHeading>
        {/if}
        {#each modelsOf(c) as model (model)}
          {@const active = c.id === connectionId && modelOf(c) === model}
          <Menu.DropdownMenuItem data-testid={`composer-model-${c.id}-${model}`} onSelect={() => void choose(c, model)} class="gap-2 text-[13px]">
            <span class="min-w-0 flex-1 truncate">{model}</span>
            {#if active}<Check class="size-4 shrink-0" />{/if}
          </Menu.DropdownMenuItem>
        {/each}
      </Menu.DropdownMenuGroup>
    {/each}
    <Menu.DropdownMenuSeparator />
    <Menu.DropdownMenuItem data-testid="composer-model-connections" onSelect={onOpenConnections} class="text-xs text-muted-foreground">
      {m.chat_model_manage()}
    </Menu.DropdownMenuItem>
  </Menu.DropdownMenuContent>
</Menu.DropdownMenu>
