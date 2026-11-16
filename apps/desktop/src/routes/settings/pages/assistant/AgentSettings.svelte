<script lang="ts">
  // 「琴音 → 设置 → 琴音 Agent」：游戏内入口、权限模式与按工具的规则（config.json `agent` 段）。
  // 工具清单来自后端 agent_tools（含分组、默认规则与说明）；权限要整体替换（agent_set_permissions），
  // inGame 用普通设置补丁。
  import { onMount } from "svelte";
  import { ChevronRight } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import {
    agentSetPermissions, agentState, agentTools, toolGroupLabel, toolLabel,
    type AgentTool, type PermissionMode, type PermissionRule, type Permissions,
  } from "../../../../lib/agent";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";

  const modes: { id: PermissionMode; name: string; description: string }[] = [
    { id: "default", name: "默认", description: "看状态、读聊天等直接做；有风险的（按键、发言、改设置……）先问你" },
    { id: "allow-all", name: "全部允许", description: "全部直接执行（你设为“禁止”的仍然禁止）" },
    { id: "read-only", name: "只读", description: "只能看状态、读屏幕和播报，其余一律禁止" },
  ];
  const ruleNames: Record<PermissionRule | "default", string> = { default: "按默认", allow: "允许", ask: "询问", deny: "禁止" };

  let permissions = $state<Permissions | null>(null);
  let tools = $state<AgentTool[]>([]);
  let toolsError = $state("");
  let saving = $state(false);
  let loadError = $state("");
  const inGame = $derived($settingsStore?.agent?.inGame ?? true);
  /** 按分组排好（核心工具在前）；已有规则但清单里没有的工具也列出来，方便删掉。 */
  const groups = $derived.by(() => {
    const known = new Set(tools.map((tool) => tool.name));
    const extra: AgentTool[] = Object.keys(permissions?.rules ?? {}).filter((name) => !known.has(name)).map((name) => ({
      name, group: "app", description: "当前不可用的工具（规则仍会保留）", sideEffects: [], defaultRule: "ask", rule: null, core: false,
    }));
    const map = new Map<string, AgentTool[]>();
    for (const tool of [...tools, ...extra]) {
      const list = map.get(tool.group) ?? [];
      list.push(tool);
      map.set(tool.group, list);
    }
    return [...map.entries()].map(([group, list]) => ({
      group, tools: list.sort((a, b) => Number(b.core) - Number(a.core) || a.name.localeCompare(b.name)),
    }));
  });
  const ruleCount = $derived(Object.keys(permissions?.rules ?? {}).length);

  onMount(() => {
    let disposed = false;
    // 优先用设置里的值；没有时从琴音状态里取。
    const fromSettings = $settingsStore?.agent?.permissions;
    if (fromSettings) permissions = structuredClone(fromSettings);
    else void agentState().then((state) => { if (!disposed) permissions = state.permissions; })
      .catch((error) => { if (!disposed) loadError = `读取权限设置失败：${errText(error)}`; });
    void agentTools().then((list) => { if (!disposed) tools = list; })
      .catch((error) => { if (!disposed) toolsError = `读取工具清单失败：${errText(error)}`; });
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

<div class="space-y-6" data-testid="agent-settings">
  <SettingsSection title="琴音 Agent" description="琴音能用 Kotone 的全部工具。这里决定游戏里的话交给谁，以及哪些事直接做、哪些先问你。">
    <SettingsRow label="游戏内交给琴音" description="陪玩热键与快捷指令里的“一句话”交给琴音（多步、能用全部工具）；关闭时用旧的单次陪玩。">
      {#snippet control()}
        <Switch data-testid="agent-in-game" bind:checked={() => inGame, (checked) => void patchSettings({ agent: { inGame: checked } }, checked ? "游戏内入口交给琴音" : "游戏内入口改用单次陪玩")} aria-label="游戏内交给琴音" />
      {/snippet}
    </SettingsRow>
    {#if loadError}<p role="alert" class="my-3 text-xs text-destructive">{loadError}</p>{/if}
    {#if permissions}
      {@const current = permissions}
      <SettingsRow label="权限模式" description="“禁止”的规则在任何模式下都生效。" stacked>
        {#snippet control()}
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-3" role="group" aria-label="权限模式">
            {#each modes as mode (mode.id)}
              {@const active = current.mode === mode.id}
              <button type="button" data-testid={`agent-mode-${mode.id}`} aria-pressed={active} disabled={saving}
                class="rounded-xl border px-3 py-2.5 text-left transition-colors hover:bg-muted/60 disabled:opacity-60 {active ? 'border-primary bg-primary/8' : 'border-hairline'}"
                onclick={() => setMode(mode.id)}>
                <p class="m-0 text-[13px] font-semibold {active ? 'text-primary' : 'text-foreground'}">{mode.name}</p>
                <p class="mt-1 mb-0 text-[11px] leading-relaxed text-muted-foreground">{mode.description}</p>
              </button>
            {/each}
          </div>
        {/snippet}
      </SettingsRow>
    {/if}
  </SettingsSection>

  {#if permissions}
    {@const current = permissions}
    <details class="group/rules" data-testid="agent-rules">
      <summary class="flex cursor-pointer list-none items-center gap-1.5 px-1 text-sm font-semibold text-foreground">
        <ChevronRight class="size-4 text-muted-foreground transition-transform group-open/rules:rotate-90" />
        按工具单独设置
        <span class="ml-1 text-xs font-normal text-muted-foreground">（{ruleCount} 条自定义）</span>
      </summary>
      <p class="mt-1.5 mb-3 px-1 text-xs leading-relaxed text-muted-foreground">“按默认”由工具的副作用决定。已保存的自动化是否交给琴音，在「自动化 → 已保存」里逐个设置。</p>
      {#if toolsError}<p role="alert" class="mb-3 px-1 text-xs text-destructive">{toolsError}</p>{/if}
      <div class="space-y-4">
        {#each groups as group (group.group)}
          <SettingsSection title={toolGroupLabel(group.group)} id={`agent-tools-${group.group}`}>
            {#each group.tools as tool (tool.name)}
              {@const value = current.rules[tool.name] ?? "default"}
              <div class="flex items-center justify-between gap-4 py-2.5" data-testid="agent-rule" data-tool={tool.name}>
                <div class="min-w-0">
                  <p class="m-0 flex flex-wrap items-baseline gap-x-2 text-[13px]">
                    <span class="font-medium text-foreground">{toolLabel(tool.name)}</span>
                    <span class="font-mono text-[11px] text-muted-foreground">{tool.name}</span>
                    {#if !tool.core}<span class="rounded border border-hairline px-1 text-[10px] text-muted-foreground" title="经“查找工具”启用">按需</span>{/if}
                  </p>
                  {#if tool.description}<p class="mt-0.5 mb-0 line-clamp-2 text-xs text-muted-foreground">{tool.description}</p>{/if}
                </div>
                <div class="flex shrink-0 items-center gap-2">
                  <span class="hidden text-[11px] text-muted-foreground sm:inline" data-testid="agent-rule-default">默认：{ruleNames[tool.defaultRule]}</span>
                  <SelectField class="w-28" size="sm" ariaLabel={`${toolLabel(tool.name)} 的规则`} {value} disabled={saving}
                    options={(["default", "allow", "ask", "deny"] as const).map((option) => ({ value: option, label: ruleNames[option] }))}
                    onchange={(next) => setRule(tool.name, next)} />
                </div>
              </div>
            {/each}
          </SettingsSection>
        {/each}
      </div>
    </details>
  {/if}
</div>
