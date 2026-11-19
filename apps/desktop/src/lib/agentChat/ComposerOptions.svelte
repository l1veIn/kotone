<script lang="ts">
  // 输入框工具行右侧：权限模式与模型切换（参照 DeepSeek Harness 输入框的“权限 / 模型”两个下拉）。
  // 权限写 agent.permissions.mode（整体替换，规则保留）；模型 = 选一条已保存密钥的连接 + 该连接的模型。
  // 模型清单照 DeepSeek Harness：读服务商的模型列表（GET /models，不花 token），读不到时用预置的模型名。
  import { onMount } from "svelte";
  import { Check, ChevronDown, Cpu, LoaderCircle, Shield, ShieldCheck, ShieldEllipsis } from "@lucide/svelte";
  import * as Menu from "$lib/components/ui/dropdown-menu";
  import {
    companionBindingPatch, companionBindings, connectionModels, listConnectionPresets, listConnections, upsertConnection,
    type ConnectionInfo, type ConnectionPreset,
  } from "../ipc";
  import { agentSetPermissions, type PermissionMode, type Permissions } from "../agent";
  import { errText, patchSettings, settingsStore, toast } from "../stores/ui";

  let { onchanged }: { onchanged?: () => void } = $props();

  const modes: { id: PermissionMode; name: string; description: string; icon: typeof Shield }[] = [
    { id: "read-only", name: "仅可查看", description: "只看状态、读屏幕和说话，不动任何东西", icon: ShieldCheck },
    { id: "default", name: "先问我", description: "按键、发言、改设置、下载、打开游戏前先问你", icon: ShieldEllipsis },
    { id: "allow-all", name: "完全权限", description: "直接去做，不再询问（你设为“禁止”的仍然禁止）", icon: Shield },
  ];

  let connections = $state<ConnectionInfo[]>([]);
  let presets = $state<ConnectionPreset[]>([]);
  let busy = $state<"" | "mode" | "model">("");
  const permissions = $derived<Permissions>($settingsStore?.agent?.permissions ?? { mode: "default", rules: {} });
  const mode = $derived(modes.find((m) => m.id === permissions.mode) ?? modes[1]);
  const connectionId = $derived(companionBindings($settingsStore?.assistant)?.connectionId ?? "");
  const current = $derived(connections.find((c) => c.id === connectionId) ?? null);
  const usable = $derived(connections.filter((c) => c.hasApiKey && c.kind === "remote"));

  /** 服务商实际列出的模型（每条连接读一次，本次打开设置窗口内复用）。 */
  let listed = $state<Record<string, string[]>>({});

  function modelsOf(c: ConnectionInfo): string[] {
    const list = [...(listed[c.id]?.length ? listed[c.id] : presets.find((p) => p.id === c.provider)?.models ?? [])];
    if (c.model && !list.includes(c.model)) list.unshift(c.model);
    return list;
  }

  async function load() {
    [connections, presets] = await Promise.all([
      listConnections().catch(() => [] as ConnectionInfo[]),
      listConnectionPresets().catch(() => [] as ConnectionPreset[]),
    ]);
  }

  async function loadModels() {
    await load();
    await Promise.all(usable.filter((c) => !(c.id in listed)).map(async (c) => {
      const reply = await connectionModels(c.id).catch(() => null);
      listed = { ...listed, [c.id]: reply?.supported ? reply.models : [] };
    }));
  }

  onMount(() => { void load(); });

  async function setMode(id: PermissionMode) {
    if (busy || id === permissions.mode) return;
    busy = "mode";
    try {
      const saved = await agentSetPermissions({ ...permissions, mode: id });
      settingsStore.update((s) => s && { ...s, agent: { hiddenAutomations: [], inGame: true, ...s.agent, permissions: saved } });
      toast(true, `权限：${modes.find((m) => m.id === id)?.name}`);
      onchanged?.();
    } catch (error) {
      toast(false, `保存失败：${errText(error)}`);
    } finally {
      busy = "";
    }
  }

  async function setModel(c: ConnectionInfo, model: string) {
    if (busy || (c.id === connectionId && c.model === model)) return;
    busy = "model";
    try {
      if (c.id !== connectionId) await patchSettings({ assistant: companionBindingPatch($settingsStore?.assistant, { connectionId: c.id }) }, "");
      if (c.model !== model) {
        const { hasApiKey: _key, ...connection } = c;
        await upsertConnection({ ...connection, model });
      }
      await load();
      toast(true, `改用 ${model}`);
    } catch (error) {
      toast(false, `切换模型失败：${errText(error)}`);
    } finally {
      busy = "";
      onchanged?.();
    }
  }
</script>

<div class="flex min-w-0 items-center gap-1" data-testid="composer-options">
  <Menu.DropdownMenu>
    <Menu.DropdownMenuTrigger data-testid="composer-permission" disabled={!!busy}
      class="flex h-8 max-w-40 items-center gap-1 rounded-full px-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60 {permissions.mode === 'allow-all' ? 'text-warning' : ''}">
      {#if busy === "mode"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<mode.icon class="size-3.5 shrink-0" />{/if}
      <span class="truncate">{mode.name}</span><ChevronDown class="size-3 shrink-0 opacity-70" />
    </Menu.DropdownMenuTrigger>
    <Menu.DropdownMenuContent align="end" side="top" class="w-64">
      {#each modes as m (m.id)}
        <Menu.DropdownMenuItem data-testid={`composer-permission-${m.id}`} onSelect={() => void setMode(m.id)} class="items-start gap-2 py-2">
          <m.icon class="mt-0.5 size-4 shrink-0" />
          <span class="min-w-0 flex-1">
            <span class="block text-[13px] font-medium">{m.name}</span>
            <span class="block text-xs leading-snug text-muted-foreground">{m.description}</span>
          </span>
          {#if m.id === permissions.mode}<Check class="mt-0.5 size-4 shrink-0" />{/if}
        </Menu.DropdownMenuItem>
      {/each}
    </Menu.DropdownMenuContent>
  </Menu.DropdownMenu>

  <Menu.DropdownMenu onOpenChange={(open) => { if (open) void loadModels(); }}>
    <Menu.DropdownMenuTrigger data-testid="composer-model" disabled={!!busy}
      class="flex h-8 max-w-52 items-center gap-1 rounded-full px-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60">
      {#if busy === "model"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Cpu class="size-3.5 shrink-0" />{/if}
      <span class="truncate">{current ? current.model : "选择模型"}</span><ChevronDown class="size-3 shrink-0 opacity-70" />
    </Menu.DropdownMenuTrigger>
    <Menu.DropdownMenuContent align="end" side="top" class="max-h-80 w-72 overflow-y-auto">
      {#if usable.length === 0}
        <p class="m-0 px-2 py-2 text-xs text-muted-foreground">还没有保存了 API key 的连接，请先在「高级 → API 连接」里添加。</p>
      {/if}
      {#each usable as c (c.id)}
        <Menu.DropdownMenuGroup>
          <Menu.DropdownMenuGroupHeading class="text-xs text-muted-foreground">{c.displayName}</Menu.DropdownMenuGroupHeading>
          {#each modelsOf(c) as model (model)}
            {@const active = c.id === connectionId && c.model === model}
            <Menu.DropdownMenuItem data-testid={`composer-model-${c.id}-${model}`} onSelect={() => void setModel(c, model)} class="gap-2 text-[13px]">
              <span class="min-w-0 flex-1 truncate">{model}</span>
              {#if active}<Check class="size-4 shrink-0" />{/if}
            </Menu.DropdownMenuItem>
          {/each}
        </Menu.DropdownMenuGroup>
      {/each}
    </Menu.DropdownMenuContent>
  </Menu.DropdownMenu>
</div>
