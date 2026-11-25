<script lang="ts">
  /*
   * 「设置 → 热键」“自己加的”那一组的“加一个” / 改一条（ADR 040 第 3 期 D5、D9）：做什么（四选一：跟她说一句、运行自动化、
   * 调一个工具、打开一个页面）→ 填内容 → 名字（可以不填）→ 录键 → 保存。调的工具要问的，上面写清“这个热键会：…”和
   * 调哪个工具带什么参数，按钮是“允许并保存”（按的时候不再问）；不能绑的说原因、不让存。调工具的列表里“发到游戏聊天”
   * 在最上面（P3）。就地展开，不是新页面（原则 2）。存在后端（`save_custom_hotkey`）：冲突、校验都在那里，错误说人话。
   */
  import { onDestroy, untrack } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import ArgsForm from "../automation/ArgsForm.svelte";
  import ToolArgsForm from "./ToolArgsForm.svelte";
  import { checkHotkey, saveCustomHotkey, type BindablePage, type HotkeyBinding, type HotkeyBindingOptions, type Settings } from "../../../../lib/ipc";
  import type { Automation } from "../../../../lib/automation";
  import { captureHotkey } from "../../../../lib/hotkeyCapture";
  import { firstConflictName } from "../../../../lib/hotkeyNames";
  import { settingsTitle } from "../../../../lib/stores/uiOpen";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  type CustomKind = "say" | "automation" | "tool" | "page";

  let { entry = null, kind: firstKind = "say", options, automations, onDone, onCancel }: {
    /** 改的那一条（热键表里的原样）；加一个时为 null。 */
    entry?: HotkeyBinding | null;
    kind?: CustomKind;
    options: HotkeyBindingOptions | null;
    automations: Automation[];
    onDone: (settings: Settings, name: string) => void;
    onCancel: () => void;
  } = $props();

  function kindOf(b: HotkeyBinding): CustomKind {
    if (typeof b.say === "string") return "say";
    if (typeof b.page === "string") return "page";
    const args = b.args as Record<string, unknown> | undefined;
    return b.tool === "automation.run" && args && Object.keys(args).every((k) => k === "id" || k === "args") ? "automation" : "tool";
  }

  // 改的时候照原来的填；加的时候照传进来的种类（只看打开表单那一刻的）
  const start = untrack(() => entry);
  const startArgs = (start?.args as Record<string, unknown> | undefined) ?? {};
  let kind = $state<CustomKind>(start ? kindOf(start) : untrack(() => firstKind));
  let text = $state(start && typeof start.say === "string" ? start.say : "");
  let automationId = $state(start && kindOf(start) === "automation" ? String(startArgs.id ?? "") : "");
  let automationArgs = $state<Record<string, unknown>>(start && kindOf(start) === "automation" ? ((startArgs.args as Record<string, unknown>) ?? {}) : {});
  let toolName = $state(start && kindOf(start) === "tool" ? String(start.tool ?? "") : "");
  let toolArgs = $state<Record<string, unknown>>(start && kindOf(start) === "tool" ? { ...startArgs } : {});
  let toolQuery = $state("");
  let pageUri = $state(start && typeof start.page === "string" ? start.page : "");
  let label = $state(String(start?.label ?? ""));
  let key = $state(String(start?.key ?? ""));
  let conflict = $state<string | null>(null);
  let capturing = $state(false);
  let saving = $state(false);
  let disposed = false;
  let captureAbort: AbortController | null = null;
  let checkVersion = 0;

  const rowId = start?.id ? `custom.${start.id}` : undefined;
  const tool = $derived(options?.tools.find((t) => t.name === toolName) ?? null);
  const automation = $derived(automations.find((a) => a.id === automationId) ?? null);
  const shownTools = $derived.by(() => {
    const terms = toolQuery.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const all = options?.tools ?? [];
    if (!terms.length) return all;
    return all.filter((t) => {
      const hay = [t.title, t.name, t.pluginName ?? "", t.description].join(" ").toLowerCase();
      return terms.every((term) => hay.includes(term));
    });
  });
  /** 这一条按的时候的权限：调工具照工具；运行自动化默认能按；说一句、打开页面不问。 */
  const decision = $derived(kind === "tool" ? (tool?.decision ?? null) : "run");
  const ready = $derived(
    !!key.trim() && !conflict
      && (kind === "say" ? !!text.trim()
        : kind === "automation" ? !!automation
        : kind === "tool" ? !!tool && decision !== "refused"
        : !!pageUri),
  );

  function pageName(p: BindablePage): string {
    if (p.section) return settingsTitle(p.section);
    return m.hotkeys_custom_plugin_page({ plugin: p.pluginName ?? p.plugin ?? "", page: p.title ?? p.uri });
  }

  // 键变了：后端查一次冲突（D6），撞了就不让存、说和谁撞了
  $effect(() => {
    const trimmed = key.trim();
    const version = ++checkVersion;
    if (!trimmed) { conflict = null; return; }
    checkHotkey(trimmed, rowId)
      .then((result) => { if (!disposed && version === checkVersion) conflict = firstConflictName(result); })
      .catch(() => { if (!disposed && version === checkVersion) conflict = null; });
  });

  async function capture() {
    if (capturing || saving) return;
    capturing = true;
    const controller = new AbortController();
    captureAbort = controller;
    await captureHotkey((result) => {
      if (disposed || controller.signal.aborted) return;
      capturing = false;
      captureAbort = null;
      if (result.kind === "combo") key = result.combo;
      else if (result.kind === "error") toast(false, result.message);
      else if (result.kind === "timeout") toast(false, m.voice_hotkey_capture_timeout());
    }, { signal: controller.signal });
  }

  function build(): HotkeyBinding {
    const out: HotkeyBinding = { key: key.trim() };
    if (start?.id) out.id = start.id;
    if (label.trim()) out.label = label.trim();
    if (kind === "say") out.say = text.trim();
    else if (kind === "automation") {
      out.tool = "automation.run";
      out.args = Object.keys(automationArgs).length ? { id: automationId, args: automationArgs } : { id: automationId };
    } else if (kind === "tool") {
      out.tool = toolName;
      out.args = toolArgs;
    } else out.page = pageUri;
    return out;
  }

  /** 这一条叫什么（存好以后的提示里用）。 */
  function nameOf(): string {
    if (label.trim()) return label.trim();
    if (kind === "say") return text.trim();
    if (kind === "automation") return automation?.name ?? automationId;
    if (kind === "tool") return tool?.title || toolName;
    const p = options?.pages.find((x) => x.uri === pageUri);
    return p ? pageName(p) : pageUri;
  }

  async function save() {
    if (!ready || saving) return;
    saving = true;
    try {
      const settings = await saveCustomHotkey(build(), decision === "confirm");
      if (!disposed) onDone(settings, nameOf());
    } catch (error) {
      toast(false, errText(error));
    } finally { saving = false; }
  }

  onDestroy(() => { disposed = true; captureAbort?.abort(); });
</script>

<div class="space-y-3 rounded-xl border border-hairline bg-muted/30 p-3" data-testid="custom-hotkey-form">
  <div class="flex flex-wrap items-center gap-2" role="radiogroup" aria-label={m.hotkeys_custom_kind_label()}>
    {#each ([["say", m.hotkeys_custom_kind_say()], ["automation", m.hotkeys_custom_kind_automation()], ["tool", m.hotkeys_custom_kind_tool()], ["page", m.hotkeys_custom_kind_page()]] as const) as [value, name] (value)}
      <button type="button" role="radio" aria-checked={kind === value} data-testid={`custom-kind-${value}`} disabled={saving}
        class="h-8 rounded-lg border px-3 text-xs font-medium transition-colors {kind === value ? 'border-primary/60 bg-primary/15 text-primary' : 'border-hairline bg-background/60 text-foreground hover:bg-muted'}"
        onclick={() => (kind = value)}>{name}</button>
    {/each}
  </div>

  {#if kind === "say"}
    <label class="flex flex-wrap items-center gap-3">
      <span class="w-28 shrink-0 text-[13px] text-foreground">{m.hotkeys_custom_say_label()}</span>
      <Input class="h-8 min-w-0 flex-1 text-[13px]" maxlength={200} bind:value={text} data-testid="custom-say"
        placeholder={m.hotkeys_custom_say_placeholder()} aria-label={m.hotkeys_custom_say_label()} />
    </label>
  {:else if kind === "automation"}
    {#if automations.length}
      <div class="flex flex-wrap items-center gap-3">
        <span class="w-28 shrink-0 text-[13px] text-foreground">{m.hotkeys_custom_automation_label()}</span>
        <div class="min-w-0 flex-1">
          <SelectField ariaLabel={m.hotkeys_custom_automation_label()} testid="custom-automation" value={automationId}
            options={automations.map((a) => ({ value: a.id, label: a.name }))}
            onchange={(id) => { automationId = id; automationArgs = {}; }} />
        </div>
      </div>
      {#if automation?.params.length}
        <div class="rounded-xl border border-hairline bg-background/60 p-3">
          <ArgsForm params={automation.params} values={automationArgs} disabled={saving} onchange={(args) => (automationArgs = args)} />
        </div>
      {/if}
    {:else}
      <p class="m-0 text-xs text-muted-foreground" data-testid="custom-automation-none">{m.hotkeys_custom_automation_none()}</p>
    {/if}
  {:else if kind === "tool"}
    <div class="space-y-2">
      <Input class="h-8 max-w-sm text-[13px]" bind:value={toolQuery} placeholder={m.hotkeys_custom_tool_search()}
        aria-label={m.hotkeys_custom_tool_search()} data-testid="custom-tool-search" />
      <div class="max-h-56 overflow-y-auto rounded-lg border border-hairline bg-background/60" role="listbox" aria-label={m.hotkeys_custom_kind_tool()}>
        {#each shownTools as t (t.name)}
          <button type="button" role="option" aria-selected={toolName === t.name} data-testid={`custom-tool-${t.name}`}
            class="flex w-full items-start gap-2 px-3 py-2 text-left transition-colors hover:bg-muted {toolName === t.name ? 'bg-primary/10' : ''}"
            onclick={() => { if (toolName !== t.name) { toolName = t.name; toolArgs = {}; } }}>
            <span class="min-w-0 flex-1">
              <span class="block truncate text-[13px] text-foreground">{t.title || t.name}</span>
              <span class="block truncate text-[11px] text-muted-foreground">{t.pluginName ? `${t.pluginName} · ` : ""}{t.name}</span>
            </span>
            {#if t.decision === "confirm"}
              <span class="shrink-0 rounded-md border border-warning/40 px-1.5 py-px text-[11px] text-warning">{m.hotkeys_custom_tool_needs_confirm()}</span>
            {:else if t.decision === "refused"}
              <span class="shrink-0 rounded-md border border-hairline px-1.5 py-px text-[11px] text-muted-foreground">{m.hotkeys_custom_tool_refused()}</span>
            {/if}
          </button>
        {:else}
          <p class="m-0 px-3 py-3 text-xs text-muted-foreground">{m.hotkeys_custom_tool_none()}</p>
        {/each}
      </div>
      {#if tool}
        <div class="rounded-xl border border-hairline bg-background/60 p-3">
          <p class="mt-0 mb-2 text-xs text-muted-foreground">{m.hotkeys_custom_tool_args()}</p>
          <ToolArgsForm schema={tool.parameters} values={toolArgs} disabled={saving} onchange={(args) => (toolArgs = args)} />
        </div>
      {/if}
    </div>
  {:else}
    <div class="flex flex-wrap items-center gap-3">
      <span class="w-28 shrink-0 text-[13px] text-foreground">{m.hotkeys_custom_page_label()}</span>
      <div class="min-w-0 flex-1">
        <SelectField ariaLabel={m.hotkeys_custom_page_label()} testid="custom-page" value={pageUri}
          options={(options?.pages ?? []).map((p) => ({ value: p.uri, label: pageName(p) }))}
          onchange={(uri) => (pageUri = uri)} />
      </div>
    </div>
  {/if}

  <label class="flex flex-wrap items-center gap-3">
    <span class="w-28 shrink-0 text-[13px] text-foreground">{m.hotkeys_custom_name_label()}</span>
    <Input class="h-8 w-56 text-[13px]" maxlength={40} bind:value={label} data-testid="custom-label" aria-label={m.hotkeys_custom_name_label()} />
  </label>

  <div class="flex flex-wrap items-center gap-3">
    <span class="w-28 shrink-0 text-[13px] text-foreground">{m.hotkeys_custom_key_label()}</span>
    <input bind:value={key} disabled={saving || capturing} data-testid="custom-key" aria-label={m.hotkeys_custom_key_label()}
      placeholder={m.voice_hotkey_placeholder({ example: "F8" })} spellcheck="false" autocomplete="off"
      class="h-8 w-44 rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/30" />
    <button type="button" data-testid="custom-key-capture" disabled={saving || capturing} onclick={capture}
      class="h-8 rounded-lg border px-3 text-xs font-medium transition-colors {capturing ? 'animate-pulse border-primary/60 bg-primary/15 text-primary' : 'border-hairline bg-muted/60 text-foreground hover:bg-muted'}">
      {capturing ? m.voice_hotkey_capturing() : m.voice_hotkey_capture()}
    </button>
  </div>
  {#if conflict}
    <p class="m-0 text-xs text-destructive" data-testid="custom-key-conflict">{m.hotkeys_custom_key_conflict({ key: key.trim(), conflict })}</p>
  {/if}

  {#if kind === "tool" && tool?.decision === "confirm"}
    <div class="rounded-lg border border-warning/40 bg-warning/8 px-3 py-2 text-xs leading-relaxed text-foreground" data-testid="custom-will-do">
      <p class="m-0">{m.hotkeys_custom_will_do({ reason: tool.reason })}</p>
      <p class="m-0 font-mono text-[11px] text-muted-foreground">{m.hotkeys_custom_args_shown({ tool: tool.name, args: JSON.stringify(toolArgs) })}</p>
      <p class="m-0 text-muted-foreground">{m.hotkeys_custom_no_ask_later()}</p>
    </div>
  {:else if kind === "tool" && tool?.decision === "refused"}
    <p class="m-0 text-xs text-destructive" data-testid="custom-refused">{m.hotkeys_custom_refused_note({ reason: tool.reason })}</p>
  {/if}

  <div class="flex flex-wrap items-center gap-2">
    <Button size="sm" disabled={!ready || saving} data-testid="custom-save" onclick={save}>
      {decision === "confirm" ? m.hotkeys_custom_allow_save() : m.hotkeys_custom_save()}
    </Button>
    <Button size="sm" variant="ghost" disabled={saving} data-testid="custom-cancel" onclick={onCancel}>{m.hotkeys_custom_cancel()}</Button>
    {#if !key.trim()}<span class="text-xs text-muted-foreground">{m.hotkeys_custom_key_needed()}</span>{/if}
  </div>
</div>
