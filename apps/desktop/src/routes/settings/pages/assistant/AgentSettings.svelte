<script lang="ts">
  // 「琴音 → 设置 → 琴音 Agent」：游戏内入口、权限模式与按工具的规则（config.json `agent` 段）。
  // 工具清单来自后端 agent_tools（含分组、默认规则与说明）；权限要整体替换（agent_set_permissions），
  // inGame 用普通设置补丁。
  import { companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";
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
    { id: "default", name: m.companion_agent_mode_default_name(), description: m.companion_agent_mode_default_desc() },
    { id: "allow-all", name: m.companion_agent_mode_allow_all_name(), description: m.companion_agent_mode_allow_all_desc() },
    { id: "read-only", name: m.companion_agent_mode_read_only_name(), description: m.companion_agent_mode_read_only_desc() },
  ];
  const ruleNames: Record<PermissionRule | "default", string> = {
    default: m.companion_agent_rule_default(), allow: m.companion_agent_rule_allow(), ask: m.companion_agent_rule_ask(), deny: m.companion_agent_rule_deny(),
  };

  let permissions = $state<Permissions | null>(null);
  let tools = $state<AgentTool[]>([]);
  let toolsError = $state("");
  let saving = $state(false);
  let loadError = $state("");
  const inGame = $derived($settingsStore?.agent?.inGame ?? true);
  const progressSpeech = $derived($settingsStore?.agent?.progressSpeech ?? true);
  const matchRecap = $derived($settingsStore?.agent?.matchRecap ?? true);
  const safetyCheck = $derived($settingsStore?.agent?.safetyCheck ?? true);
  const gameEffort = $derived($settingsStore?.agent?.gameEffort ?? "");
  const chatEffort = $derived($settingsStore?.agent?.chatEffort ?? "");
  const efforts = [
    { value: "default", label: m.companion_agent_effort_default_high() }, { value: "low", label: m.companion_agent_effort_low_faster() },
    { value: "high", label: m.companion_agent_effort_high() }, { value: "max", label: m.companion_agent_effort_max() },
    { value: "none", label: m.companion_agent_effort_none() },
  ];
  /** 游戏里（语音叫她）默认不思考（ADR 029：开着思考一句回答要多等十几秒）。 */
  const gameEfforts = [
    { value: "default", label: m.companion_agent_effort_default_none() }, { value: "low", label: m.companion_agent_effort_low() },
    { value: "high", label: m.companion_agent_effort_high() }, { value: "max", label: m.companion_agent_effort_max() },
    { value: "none", label: m.companion_agent_effort_none() },
  ];
  /** 按分组排好（核心工具在前）；已有规则但清单里没有的工具也列出来，方便删掉。 */
  const groups = $derived.by(() => {
    const known = new Set(tools.map((tool) => tool.name));
    const extra: AgentTool[] = Object.keys(permissions?.rules ?? {}).filter((name) => !known.has(name)).map((name) => ({
      name, group: "app", description: m.companion_agent_tool_unavailable_desc(), sideEffects: [], defaultRule: "ask", rule: null, core: false,
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
      .catch((error) => { if (!disposed) loadError = m.companion_agent_permissions_load_failed({ error: errText(error) }); });
    void agentTools().then((list) => { if (!disposed) tools = list; })
      .catch((error) => { if (!disposed) toolsError = m.companion_agent_tools_load_failed({ error: errText(error) }); });
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
      toast(false, m.companion_save_failed({ error: errText(error) }));
    } finally {
      saving = false;
    }
  }

  function setMode(mode: PermissionMode) {
    if (!permissions || permissions.mode === mode) return;
    void save({ ...permissions, mode }, m.companion_agent_mode_saved({ mode: String(modes.find((item) => item.id === mode)?.name) }));
  }

  function setRule(tool: string, value: string) {
    if (!permissions) return;
    const rules = { ...permissions.rules };
    if (value === "default") delete rules[tool];
    else rules[tool] = value as PermissionRule;
    void save({ ...permissions, rules }, m.companion_agent_rule_saved({ tool: toolLabel(tool), rule: ruleNames[value as PermissionRule | "default"] }));
  }
</script>

<div class="space-y-6" data-testid="agent-settings">
  <SettingsSection title={m.companion_agent_section_title()} description={m.companion_agent_section_desc()}>
    <SettingsRow label={m.companion_agent_in_game_label({ name: $companionName })} description={m.companion_agent_in_game_desc()}>
      {#snippet control()}
        <Switch data-testid="agent-in-game" bind:checked={() => inGame, (checked) => void patchSettings({ agent: { inGame: checked } }, checked ? m.companion_agent_in_game_on({ name: $companionName }) : m.companion_agent_in_game_off())} aria-label={m.companion_agent_in_game_label({ name: $companionName })} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_agent_progress_speech_label()} description={m.companion_agent_progress_speech_desc()}>
      {#snippet control()}
        <Switch data-testid="agent-progress-speech" bind:checked={() => progressSpeech, (checked) => void patchSettings({ agent: { progressSpeech: checked } }, checked ? m.companion_agent_progress_speech_on() : m.companion_agent_progress_speech_off())} aria-label={m.companion_agent_progress_speech_label()} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_agent_match_recap_label()} description={m.companion_agent_match_recap_desc()}>
      {#snippet control()}
        <Switch data-testid="agent-match-recap" bind:checked={() => matchRecap, (checked) => void patchSettings({ agent: { matchRecap: checked } }, checked ? m.companion_agent_match_recap_on() : m.companion_agent_match_recap_off())} aria-label={m.companion_agent_match_recap_label()} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_agent_effort_label()} description={m.companion_agent_effort_desc()}>
      {#snippet control()}
        <div class="flex flex-wrap gap-2">
          <label class="flex items-center gap-1.5 text-xs text-muted-foreground">{m.companion_agent_effort_game()}
            <SelectField class="w-32" size="sm" ariaLabel={m.companion_agent_effort_game_aria()} value={gameEffort || "default"} options={gameEfforts}
              onchange={(v) => void patchSettings({ agent: { gameEffort: v === "default" ? "" : v } }, m.companion_agent_effort_game_saved())} /></label>
          <label class="flex items-center gap-1.5 text-xs text-muted-foreground">{m.companion_agent_effort_chat()}
            <SelectField class="w-28" size="sm" ariaLabel={m.companion_agent_effort_chat_aria()} value={chatEffort || "default"} options={efforts}
              onchange={(v) => void patchSettings({ agent: { chatEffort: v === "default" ? "" : v } }, m.companion_agent_effort_chat_saved())} /></label>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_agent_safety_check_label()} description={m.companion_agent_safety_check_desc()}>
      {#snippet control()}
        <Switch data-testid="agent-safety-check" bind:checked={() => safetyCheck, (checked) => void patchSettings({ agent: { safetyCheck: checked } }, checked ? m.companion_agent_safety_check_on() : m.companion_agent_safety_check_off())} aria-label={m.companion_agent_safety_check_label()} />
      {/snippet}
    </SettingsRow>
    {#if loadError}<p role="alert" class="my-3 text-xs text-destructive">{loadError}</p>{/if}
    {#if permissions}
      {@const current = permissions}
      <SettingsRow label={m.companion_agent_permission_mode_label()} description={m.companion_agent_permission_mode_desc()} stacked>
        {#snippet control()}
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-3" role="group" aria-label={m.companion_agent_permission_mode_label()}>
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
        {m.companion_agent_rules_title()}
        <span class="ml-1 text-xs font-normal text-muted-foreground">{m.companion_agent_rules_custom_count({ count: ruleCount })}</span>
      </summary>
      <p class="mt-1.5 mb-3 px-1 text-xs leading-relaxed text-muted-foreground">{m.companion_agent_rules_hint()}</p>
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
                    {#if !tool.core}<span class="rounded border border-hairline px-1 text-[10px] text-muted-foreground" title={m.companion_agent_tool_on_demand_title()}>{m.companion_agent_tool_on_demand()}</span>{/if}
                  </p>
                  {#if tool.description}<p class="mt-0.5 mb-0 line-clamp-2 text-xs text-muted-foreground">{tool.description}</p>{/if}
                </div>
                <div class="flex shrink-0 items-center gap-2">
                  <span class="hidden text-[11px] text-muted-foreground sm:inline" data-testid="agent-rule-default">{m.companion_agent_rule_default_value({ rule: ruleNames[tool.defaultRule] })}</span>
                  <SelectField class="w-28" size="sm" ariaLabel={m.companion_agent_rule_aria({ tool: toolLabel(tool.name) })} {value} disabled={saving}
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
