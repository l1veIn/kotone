<script lang="ts">
  // 输入框里的权限按钮：一个盾牌图标，悬停看当前模式，点开换模式（参照 DeepSeek Harness 输入框的“权限”下拉）。
  // 写 agent.permissions.mode（整体替换，规则保留）。选模型不在这里：游戏陪玩不常换模型，放在「设置 → 她怎么回答」。
  import { Check, LoaderCircle, Shield, ShieldCheck, ShieldEllipsis } from "@lucide/svelte";
  import * as Menu from "$lib/components/ui/dropdown-menu";
  import { agentSetPermissions, type PermissionMode, type Permissions } from "../agent";
  import { errText, settingsStore, toast } from "../stores/ui";

  let { onchanged }: { onchanged?: () => void } = $props();

  const modes: { id: PermissionMode; name: string; description: string; icon: typeof Shield }[] = [
    { id: "read-only", name: "仅可查看", description: "只看状态、读屏幕和说话，不动任何东西", icon: ShieldCheck },
    { id: "default", name: "先问我", description: "按键、发言、改设置、下载、打开游戏前先问你", icon: ShieldEllipsis },
    { id: "allow-all", name: "完全权限", description: "直接去做，不再询问（你设为“禁止”的仍然禁止）", icon: Shield },
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
      toast(true, `权限：${modes.find((m) => m.id === id)?.name}`);
      onchanged?.();
    } catch (error) {
      toast(false, `保存失败：${errText(error)}`);
    } finally {
      busy = false;
    }
  }
</script>

<Menu.DropdownMenu>
  <Menu.DropdownMenuTrigger data-testid="composer-permission" disabled={busy} aria-label={`权限：${mode.name}`} title={`权限：${mode.name}`}
    data-mode={mode.id}
    class="flex size-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-60 {permissions.mode === 'allow-all' ? 'text-warning' : ''}">
    {#if busy}<LoaderCircle class="size-4 animate-spin" />{:else}<mode.icon class="size-4" />{/if}
    <span class="sr-only">{mode.name}</span>
  </Menu.DropdownMenuTrigger>
  <Menu.DropdownMenuContent align="end" side="top" class="w-64">
    <Menu.DropdownMenuLabel class="text-xs text-muted-foreground">她能做到哪一步</Menu.DropdownMenuLabel>
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
