<script lang="ts">
  /*
   * 品牌开关（青色调，方向 B）：label + 描述 + 右侧滑动开关。
   * 纯 live HTML；checked 由父组件绑定，onchange 抛给父组件处理 IPC。
   */
  interface Props {
    checked: boolean;
    label: string;
    desc?: string;
    onchange: (checked: boolean) => void;
  }
  let { checked, label, desc = "", onchange }: Props = $props();
</script>

<label class="flex cursor-pointer items-center justify-between gap-4">
  <span>
    <span class="block text-sm">{label}</span>
    {#if desc}
      <span class="block text-[11px] text-white/45">{desc}</span>
    {/if}
  </span>
  <input
    type="checkbox"
    class="peer sr-only"
    {checked}
    onchange={(e) => onchange((e.target as HTMLInputElement).checked)}
  />
  <span
    class="relative h-5 w-9 shrink-0 rounded-full bg-white/15 transition
      peer-checked:bg-kotone-cyan/70 peer-checked:shadow-glow-cyan
      after:absolute after:top-0.5 after:left-0.5 after:h-4 after:w-4 after:rounded-full
      after:bg-white after:transition peer-checked:after:translate-x-4"
  ></span>
</label>
