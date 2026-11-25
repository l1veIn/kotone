<script lang="ts">
  /*
   * 热键调工具时带的固定参数（ADR 040 第 3 期）：照工具参数的 JSON Schema 出一张简单表单——顶层的几选一是下拉、
   * 字是输入框、数是数字框、是非是开关；别的（对象、数组）写 JSON。带 * 的是必填。参数对不对最终由工具自己说
   * （按下时报错，照快捷指令的做法说一句）。
   */
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import { Textarea } from "$lib/components/ui/textarea";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { m } from "$lib/paraglide/messages.js";

  type Prop = { type?: string | string[]; enum?: unknown[]; description?: string; title?: string };
  let { schema, values, disabled = false, onchange }: {
    schema: Record<string, unknown>;
    values: Record<string, unknown>;
    disabled?: boolean;
    onchange: (values: Record<string, unknown>) => void;
  } = $props();

  const properties = $derived(Object.entries((schema?.properties as Record<string, Prop> | undefined) ?? {}));
  const required = $derived(new Set(Array.isArray(schema?.required) ? (schema.required as string[]) : []));
  /** JSON 写到一半的原文（还不是合法 JSON 时不改参数）。 */
  let drafts = $state<Record<string, string>>({});

  function kindOf(p: Prop): "enum" | "string" | "number" | "boolean" | "json" {
    if (Array.isArray(p.enum) && p.enum.length && p.enum.every((v) => typeof v === "string" || typeof v === "number")) return "enum";
    const type = Array.isArray(p.type) ? p.type.find((t) => t !== "null") : p.type;
    if (type === "string") return "string";
    if (type === "number" || type === "integer") return "number";
    if (type === "boolean") return "boolean";
    return "json";
  }

  function set(name: string, value: unknown) {
    const next = { ...values };
    if (value === undefined || value === "") delete next[name];
    else next[name] = value;
    onchange(next);
  }

  function setJson(name: string, text: string) {
    drafts[name] = text;
    if (!text.trim()) { set(name, undefined); return; }
    try { set(name, JSON.parse(text)); } catch { /* 还没写完：等它变成合法的 JSON */ }
  }

  function jsonValid(name: string): boolean {
    const text = drafts[name];
    if (text === undefined || !text.trim()) return true;
    try { JSON.parse(text); return true; } catch { return false; }
  }
</script>

{#if !properties.length}
  <p class="m-0 text-xs text-muted-foreground" data-testid="tool-args-none">{m.hotkeys_custom_tool_no_args()}</p>
{:else}
  <div class="flex flex-col gap-2" data-testid="tool-args">
    {#each properties as [name, prop] (name)}
      {@const kind = kindOf(prop)}
      {@const label = prop.title || name}
      <div class="flex flex-wrap items-center gap-3">
        <span class="w-28 shrink-0 text-[13px] text-foreground">{label}{#if required.has(name)}<span class="text-brand-pink"> *</span>{/if}</span>
        {#if kind === "enum"}
          {@const choices = (prop.enum ?? []).map((v) => ({ value: String(v), label: String(v) }))}
          <div class="w-44">
            <SelectField ariaLabel={label} testid={`tool-arg-${name}`} {disabled}
              value={values[name] === undefined ? "" : String(values[name])}
              options={required.has(name) ? choices : [{ value: "", label: m.hotkeys_custom_args_unset() }, ...choices]}
              onchange={(v) => set(name, v === "" ? undefined : typeof prop.enum?.[0] === "number" ? Number(v) : v)} />
          </div>
        {:else if kind === "string"}
          <Input class="h-8 min-w-0 flex-1 text-[13px]" aria-label={label} data-testid={`tool-arg-${name}`} {disabled}
            value={typeof values[name] === "string" ? values[name] : ""}
            oninput={(event) => set(name, event.currentTarget.value)} />
        {:else if kind === "number"}
          <Input type="number" class="h-8 w-32 text-[13px]" aria-label={label} data-testid={`tool-arg-${name}`} {disabled}
            value={values[name] === undefined ? "" : String(values[name])}
            oninput={(event) => { const text = event.currentTarget.value; set(name, text === "" ? undefined : Number(text)); }} />
        {:else if kind === "boolean"}
          <Switch aria-label={label} data-testid={`tool-arg-${name}`} {disabled}
            bind:checked={() => values[name] === true, (checked) => set(name, checked)} />
        {:else}
          <div class="min-w-0 flex-1">
            <Textarea class="min-h-16 font-mono text-xs" aria-label={label} data-testid={`tool-arg-${name}`} {disabled}
              value={drafts[name] ?? (values[name] === undefined ? "" : JSON.stringify(values[name]))}
              oninput={(event) => setJson(name, event.currentTarget.value)} />
            {#if !jsonValid(name)}<p class="mt-1 mb-0 text-xs text-destructive">{m.hotkeys_custom_json_invalid()}</p>{/if}
          </div>
        {/if}
        {#if prop.description}<span class="text-xs text-muted-foreground">{prop.description}</span>{/if}
      </div>
    {/each}
  </div>
{/if}
