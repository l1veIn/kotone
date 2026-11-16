<script lang="ts">
  // 「琴音 → 设置 → 琴音 Agent」：权限（整体模式 + 按工具的规则）与游戏内入口（config.json `agent` 段）。
  // 权限要整体替换（agent_set_permissions）；inGame 用普通设置补丁。
  import { onMount } from "svelte";
  import * as Select from "$lib/components/ui/select";
  import { Switch } from "$lib/components/ui/switch";
  import {
    agentSetPermissions, agentState, ruleTools, toolLabel,
    type PermissionMode, type PermissionRule, type Permissions,
  } from "../../../../lib/agent";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";

  const modes: { id: PermissionMode; name: string; description: string }[] = [
    { id: "default", name: "默认", description: "看状态、读聊天等直接做；有风险的（按键、发言、改设置……）先问你" },
    { id: "allow-all", name: "全部允许", description: "全部直接执行（你设为“禁止”的仍然禁止）" },
    { id: "read-only", name: "只读", description: "只能看状态、读屏幕和播报，其余一律禁止" },
  ];
  const ruleNames: Record<PermissionRule | "default", string> = { default: "按默认", allow: "允许", ask: "询问", deny: "禁止" };

  let permissions = $state<Permissions | null>(null);
  let saving = $state(false);
  let loadError = $state("");
  const inGame = $derived($settingsStore?.agent?.inGame ?? true);
  const tools = $derived([...new Set([...ruleTools, ...Object.keys(permissions?.rules ?? {})])]);

  onMount(() => {
    let disposed = false;
    // 优先用设置里的值；没有时从琴音状态里取。
    const fromSettings = $settingsStore?.agent?.permissions;
    if (fromSettings) permissions = structuredClone(fromSettings);
    else void agentState().then((state) => { if (!disposed) permissions = state.permissions; })
      .catch((error) => { if (!disposed) loadError = `读取权限设置失败：${errText(error)}`; });
    return () => { disposed = true; };
  });

  async function save(next: Permissions, message: string) {
    if (saving) return;
    saving = true;
    try {
      permissions = await agentSetPermissions(next);
      const saved = permissions;
      settingsStore.update((s) => s && { ...s, agent: { hiddenAutomations: [], inGame: true, ...s.agent, permissions: saved } });
      toast(true, message);
    } catch (error) {
      toast(false, `保存失败：${errText(error)}`);
    } finally {
      saving = false;
    }
  }

  function setMode(mode: PermissionMode) {
    if (!permissions || permissions.mode === mode) return;
    void save({ ...permissions, mode }, `权限模式：${modes.find((m) => m.id === mode)?.name}`);
  }

  function setRule(tool: string, value: string) {
    if (!permissions) return;
    const rules = { ...permissions.rules };
    if (value === "default") delete rules[tool];
    else rules[tool] = value as PermissionRule;
    void save({ ...permissions, rules }, `「${toolLabel(tool)}」：${ruleNames[value as PermissionRule | "default"]}`);
  }
</script>

<section class="panel ui-scope" aria-labelledby="agent-settings-title" data-testid="agent-settings">
  <h2 id="agent-settings-title">琴音 Agent</h2>
  <p class="mt-1 text-xs leading-relaxed text-muted-foreground">琴音能用 Kotone 的全部工具。这里决定哪些事直接做、哪些先问你。</p>

  <div class="setting-row">
    <div><h3>游戏内交给琴音</h3><p>陪玩热键与快捷指令里的“一句话”交给琴音（多步、能用全部工具）；关闭时用旧的单次陪玩。</p></div>
    <Switch data-testid="agent-in-game" checked={inGame} aria-label="游戏内交给琴音"
      onCheckedChange={(checked) => void patchSettings({ agent: { inGame: checked } }, checked ? "游戏内入口交给琴音" : "游戏内入口改用单次陪玩")} />
  </div>

  {#if loadError}<p role="alert" class="notice text-kotone-pink">{loadError}</p>{/if}
  {#if permissions}
    {@const current = permissions}
    <div class="mt-4">
      <h3>权限模式</h3>
      <div class="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-3" role="group" aria-label="权限模式">
        {#each modes as mode (mode.id)}
          {@const active = current.mode === mode.id}
          <button type="button" data-testid={`agent-mode-${mode.id}`} aria-pressed={active} disabled={saving}
            class="rounded-lg border px-3 py-2.5 text-left transition-colors hover:bg-accent disabled:opacity-60 {active ? 'border-primary bg-primary/10' : 'border-border'}"
            onclick={() => setMode(mode.id)}>
            <p class="text-sm font-semibold {active ? 'text-primary' : 'text-foreground'}">{mode.name}</p>
            <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">{mode.description}</p>
          </button>
        {/each}
      </div>
    </div>

    <details class="mt-4 group/rules" data-testid="agent-rules">
      <summary class="cursor-pointer text-xs text-muted-foreground hover:text-foreground">按工具单独设置（{Object.keys(current.rules).length} 条）</summary>
      <ul class="mt-2 divide-y divide-border rounded-lg border">
        {#each tools as tool (tool)}
          {@const value = current.rules[tool] ?? "default"}
          <li class="flex items-center justify-between gap-3 px-3 py-1.5" data-testid="agent-rule" data-tool={tool}>
            <span class="min-w-0 text-xs"><span class="text-foreground">{toolLabel(tool)}</span> <span class="font-mono text-[10px] text-muted-foreground">{tool}</span></span>
            <Select.Root type="single" {value} onValueChange={(next) => setRule(tool, next)} disabled={saving}>
              <Select.Trigger size="sm" class="w-28" aria-label={`${toolLabel(tool)} 的规则`}>{ruleNames[value]}</Select.Trigger>
              <Select.Content>
                {#each ["default", "allow", "ask", "deny"] as option (option)}
                  <Select.Item value={option} label={ruleNames[option as PermissionRule | "default"]} />
                {/each}
              </Select.Content>
            </Select.Root>
          </li>
        {/each}
      </ul>
      <p class="mt-1.5 text-[11px] text-muted-foreground">“按默认”由工具的副作用决定；“禁止”在任何模式下都生效。已保存的自动化是否交给琴音，在「自动化 → 已保存」里逐个设置。</p>
    </details>
  {/if}
</section>
