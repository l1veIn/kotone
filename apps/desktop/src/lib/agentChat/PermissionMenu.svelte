<script lang="ts">
  // 输入框里的权限按钮：一个盾牌图标，悬停看当前模式，点开换模式（参照 DeepSeek Harness 输入框的“权限”下拉）。
  // 写 agent.permissions.mode（整体替换，规则保留）。选模型在旁边的 ModelMenu（她用的模型）。
  import { Check, LoaderCircle, Shield, ShieldCheck, ShieldEllipsis } from "@lucide/svelte";
  import * as Menu from "$lib/components/ui/dropdown-menu";
  import { m } from "$lib/paraglide/messages.js";
  import { agentSetPermissions, type PermissionMode, type Permissions } from "../agent";
  import { errText, settingsStore, toast } from "../stores/ui";

  let { onchanged }: { onchanged?: () => void } = $props();

  const modes: { id: PermissionMode; name: string; description: string; icon: typeof Shield }[] = [
    { id: "read-only", name: m.chat_permission_read_only_name(), description: m.chat_permission_read_only_description(), icon: ShieldCheck },
    { id: "default", name: m.chat_permission_default_name(), description: m.chat_permission_default_description(), icon: ShieldEllipsis },
    { id: "allow-all", name: m.chat_permission_allow_all_name(), description: m.chat_permission_allow_all_description(), icon: Shield },
  ];

  let busy = $state(false);
  const permissions = $derived<Permissions>($settingsStore?.agent?.permissions ?? { mode: "default", rules: {} });
  const mode = $derived(modes.find((m) => m.id === permissions.mode) ?? modes[1]);

  async function setMode(id: PermissionMode) {
    if (busy || id === permissions.mode) return;
    busy = true;
    try {
      const saved = await agentSetPermissions({ ...permissions, mode: id });
      settingsStore.update((s) => s && { ...s, agent: { hiddenAutomations: [], inGame: true, ...s.agent, permissions: saved } });
      toast(true, m.chat_permission_mode({ mode: modes.find((item) => item.id === id)?.name ?? "" }));
      onchanged?.();
    } catch (error) {
      toast(false, m.chat_permission_save_failed({ error: errText(error) }));
    } finally {
      busy = false;
    }
  }
</script>

<Menu.DropdownMenu>
  <Menu.DropdownMenuTrigger data-testid="composer-permission" disabled={busy} aria-label={m.chat_permission_mode({ mode: mode.name })} title={m.chat_permission_mode({ mode: mode.name })}
    data-mode={mode.id}
    class="flex size-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60 {permissions.mode === 'allow-all' ? 'text-warning' : ''}">
    {#if busy}<LoaderCircle class="size-4 animate-spin" />{:else}<mode.icon class="size-4" />{/if}
    <span class="sr-only">{mode.name}</span>
  </Menu.DropdownMenuTrigger>
  <Menu.DropdownMenuContent align="end" side="top" class="w-64">
    <Menu.DropdownMenuLabel class="text-xs text-muted-foreground">{m.chat_permission_menu_label()}</Menu.DropdownMenuLabel>
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
