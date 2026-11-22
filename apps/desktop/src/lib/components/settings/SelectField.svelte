<script lang="ts" module>
  export type SelectOption = { value: string; label: string; description?: string };
</script>

<script lang="ts">
  // 下拉选择：shadcn-svelte Select 的简单封装（取代旧的手写 Select，属性保持一致：
  // value / options / onchange / ariaLabel / testid，触发器带 data-value）。
  import * as Select from "$lib/components/ui/select";
  import { cn } from "$lib/utils";
  import { m } from "$lib/paraglide/messages.js";

  let {
    value,
    options,
    disabled = false,
    placeholder = m.shell_select_placeholder(),
    ariaLabel = "",
    testid = "",
    id = "",
    size = "sm",
    class: className = "",
    onchange,
  }: {
    value: string;
    options: SelectOption[];
    disabled?: boolean;
    placeholder?: string;
    ariaLabel?: string;
    testid?: string;
    id?: string;
    size?: "sm" | "md";
    class?: string;
    onchange: (value: string) => void;
  } = $props();

  const selected = $derived(options.find((option) => option.value === value));
</script>

<Select.Root type="single" {value} {disabled} onValueChange={(next) => { if (next !== value) onchange(next); }}>
  <Select.Trigger
    id={id || undefined}
    data-testid={testid || undefined}
    data-value={value}
    aria-label={ariaLabel || undefined}
    size={size === "md" ? "default" : "sm"}
    class={cn("w-full bg-background/60 text-[13px]", className)}
  >
    <span class={selected ? "text-foreground" : "text-muted-foreground"}>{selected?.label ?? placeholder}</span>
  </Select.Trigger>
  <Select.Content class="ui-scope max-h-72">
    {#each options as option (option.value)}
      <Select.Item value={option.value} label={option.label}>
        {#if option.description}
          <span class="flex flex-col"><span>{option.label}</span><span class="text-[11px] text-muted-foreground">{option.description}</span></span>
        {:else}{option.label}{/if}
      </Select.Item>
    {/each}
  </Select.Content>
</Select.Root>
