<script lang="ts">
  // 一个入参的输入控件（运行时填值、编辑器里填缺省值共用）。
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import type { Color, ParamSpec, Region } from "../../../../lib/automation";
  import { m } from "$lib/paraglide/messages.js";

  let { spec, value, label, disabled = false, onchange }: {
    spec: ParamSpec; value: unknown; label: string; disabled?: boolean; onchange: (value: unknown) => void;
  } = $props();

  const region = $derived((value ?? { x: 0, y: 0, w: 0.2, h: 0.2 }) as Region);
  const hex = $derived.by(() => {
    const c = value as Color | undefined;
    if (!c || typeof c.r !== "number") return "#ffffff";
    return `#${[c.r, c.g, c.b].map((n) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0")).join("")}`;
  });

  function fromHex(text: string): Color {
    return { r: parseInt(text.slice(1, 3), 16), g: parseInt(text.slice(3, 5), 16), b: parseInt(text.slice(5, 7), 16) };
  }

  function setRegion(key: keyof Region, raw: string) {
    const n = Number(raw);
    if (Number.isFinite(n)) onchange({ ...region, [key]: n });
  }
</script>

{#if spec.type === "number"}
  <input class="field w-32" type="number" aria-label={label} {disabled}
    min={spec.min ?? undefined} max={spec.max ?? undefined} step={spec.integer ? 1 : "any"}
    value={typeof value === "number" ? value : ""}
    onchange={(event) => {
      const raw = event.currentTarget.value;
      onchange(raw === "" ? undefined : Number(raw));
    }} />
{:else if spec.type === "text"}
  <input class="field w-60 max-w-full" aria-label={label} {disabled} maxlength={spec.maxLength ?? 200}
    value={typeof value === "string" ? value : ""}
    onchange={(event) => onchange(event.currentTarget.value === "" ? undefined : event.currentTarget.value)} />
{:else if spec.type === "key"}
  <input class="field w-32" aria-label={label} {disabled} maxlength="32" placeholder={m.auto_param_key_placeholder()}
    value={typeof value === "string" ? value : ""}
    onchange={(event) => onchange(event.currentTarget.value.trim() === "" ? undefined : event.currentTarget.value.trim())} />
{:else if spec.type === "boolean"}
  <input type="checkbox" class="size-4 accent-[var(--color-primary)]" aria-label={label} {disabled} checked={value === true}
    onchange={(event) => onchange(event.currentTarget.checked)} />
{:else if spec.type === "choice"}
  <div class="w-48">
    <SelectField ariaLabel={label} {disabled} value={typeof value === "string" ? value : ""}
      options={[{ value: "", label: m.auto_param_choice_unselected() }, ...(spec.options ?? []).map((o) => ({ value: o, label: o }))]}
      onchange={(next) => onchange(next === "" ? undefined : next)} />
  </div>
{:else if spec.type === "region"}
  <div class="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground" role="group" aria-label={label}>
    {#each [["x", m.auto_param_region_x()], ["y", m.auto_param_region_y()], ["w", m.auto_param_region_w()], ["h", m.auto_param_region_h()]] as [key, name]}
      <label class="flex items-center gap-1">{name}
        <input class="field w-16" type="number" min="0" max="1" step="0.01" {disabled} aria-label={`${label} ${name}`}
          value={value ? region[key as keyof Region] : ""}
          onchange={(event) => setRegion(key as keyof Region, event.currentTarget.value)} />
      </label>
    {/each}
    <span class="text-muted-foreground/75">{m.auto_param_region_hint()}</span>
  </div>
{:else if spec.type === "color"}
  <input type="color" class="h-8 w-12 cursor-pointer rounded-lg border border-input bg-transparent p-0.5" aria-label={label} {disabled} value={hex}
    onchange={(event) => onchange(fromHex(event.currentTarget.value))} />
{:else}
  <span class="text-xs text-muted-foreground">
    {value && typeof value === "object" && "image" in value ? m.auto_param_image_set() : m.auto_param_image_unset()}
  </span>
{/if}

<style>
  .field { height: 2rem; border-radius: .5rem; border: 1px solid var(--color-input); background: color-mix(in oklab, var(--color-background) 60%, transparent); color: var(--color-foreground); padding: 0 .6rem; font-size: 13px; outline: none; transition: border-color 120ms, box-shadow 120ms; }
  .field::placeholder { color: var(--color-muted-foreground); }
  .field:focus { border-color: var(--color-ring); box-shadow: 0 0 0 3px color-mix(in oklab, var(--color-ring) 30%, transparent); }
  .field:disabled { opacity: .5; }
</style>
