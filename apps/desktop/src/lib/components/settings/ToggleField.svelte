<script lang="ts">
  // 开关行：标题 + 说明 + shadcn-svelte Switch（取代旧的手写 Toggle，属性保持一致）。
  import { Switch } from "$lib/components/ui/switch";

  let { checked, label, desc = "", ariaLabel = label, disabled = false, testid, onchange }: {
    checked: boolean;
    label: string;
    desc?: string;
    ariaLabel?: string;
    disabled?: boolean;
    testid?: string;
    onchange: (checked: boolean) => void;
  } = $props();
  const id = `toggle-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="flex items-center justify-between gap-4">
  <div class="min-w-0">
    <label for={id} class="block cursor-pointer text-[13px] font-medium text-foreground">{label}</label>
    {#if desc}<p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">{desc}</p>{/if}
  </div>
  <!-- 受控：显示的总是传进来的值，保存失败时不会停在错误的位置 -->
  <Switch {id} bind:checked={() => checked, (next) => onchange(next)} aria-label={ariaLabel} data-testid={testid} {disabled} />
</div>
