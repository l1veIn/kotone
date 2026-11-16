<script lang="ts">
  // 按入参声明填运行时的值：没填的用缺省值；没有缺省值的是必填。
  import type { ParamSpec } from "../../../../lib/automation";
  import ParamInput from "./ParamInput.svelte";

  let { params, values, disabled = false, prefix = "", onchange }: {
    params: ParamSpec[]; values: Record<string, unknown>; disabled?: boolean; prefix?: string;
    onchange: (values: Record<string, unknown>) => void;
  } = $props();

  function describe(value: unknown): string {
    if (value === undefined || value === null) return "";
    if (typeof value === "object") {
      if ("image" in (value as object)) return "图片";
      return JSON.stringify(value);
    }
    return String(value);
  }

  function set(name: string, value: unknown) {
    const next = { ...values };
    if (value === undefined) delete next[name];
    else next[name] = value;
    onchange(next);
  }
</script>

{#if params.length}
  <div class="flex flex-col gap-2">
    {#each params as spec (spec.name)}
      {@const required = spec.default === undefined || spec.default === null}
      <div class="flex flex-wrap items-center gap-3">
        <span class="w-28 shrink-0 text-[13px] text-foreground">{spec.label || spec.name}{#if required}<span class="text-brand-pink"> *</span>{/if}</span>
        <ParamInput {spec} {disabled} label={`${prefix}${spec.label || spec.name}`}
          value={values[spec.name]} onchange={(value) => set(spec.name, value)} />
        {#if !required && values[spec.name] === undefined}
          <span class="text-xs text-muted-foreground/80">缺省：{describe(spec.default)}</span>
        {/if}
        {#if spec.description}<span class="text-xs text-muted-foreground">{spec.description}</span>{/if}
      </div>
    {/each}
  </div>
{:else}
  <p class="m-0 text-xs text-muted-foreground">没有参数。</p>
{/if}
