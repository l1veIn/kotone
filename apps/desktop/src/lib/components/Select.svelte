<script lang="ts">
  /*
   * 自绘下拉。原生 <select> 在 Windows WebView 里弹出系统菜单，
   * 和深色面板对不上。选项列表用同一套面板色。
   */
  export type SelectOption = { value: string; label: string };

  let {
    value,
    options,
    disabled = false,
    placeholder = "请选择",
    ariaLabel = "",
    testid = "",
    id = "",
    size = "sm",
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
    onchange: (value: string) => void;
  } = $props();

  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  const selected = $derived(options.find((option) => option.value === value));

  $effect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (root && !root.contains(event.target as Node)) open = false;
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
    };
  });

  function pick(next: string) {
    open = false;
    if (next !== value) onchange(next);
  }
</script>

<div class="relative min-w-0" bind:this={root}>
  <button
    type="button"
    {id}
    data-testid={testid || undefined}
    data-value={value}
    aria-label={ariaLabel || undefined}
    aria-haspopup="listbox"
    aria-expanded={open}
    {disabled}
    class="flex w-full items-center justify-between gap-2 rounded-lg bg-white/8 text-left ring-1 ring-white/15 outline-none transition hover:bg-white/10 focus-visible:ring-kotone-cyan/60 disabled:opacity-50 {size ===
    'md'
      ? 'px-3 py-2 text-sm'
      : 'px-2.5 py-1.5 text-xs'}"
    onclick={() => {
      if (!disabled) open = !open;
    }}
  >
    <span class="min-w-0 truncate {selected ? 'text-white/85' : 'text-white/35'}">
      {selected?.label ?? placeholder}
    </span>
    <svg
      viewBox="0 0 12 12"
      class="h-3 w-3 shrink-0 text-white/40 transition {open ? 'rotate-180' : ''}"
      aria-hidden="true"
    >
      <path
        d="M2.5 4.5 L6 8 L9.5 4.5"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  </button>
  {#if open}
    <div
      role="listbox"
      class="kotone-scroll absolute z-[90] mt-1 max-h-48 w-full overflow-y-auto rounded-lg bg-[#212140] py-1 shadow-glow-cyan ring-1 ring-white/12"
    >
      {#each options as option (option.value)}
        {@const active = option.value === value}
        <button
          type="button"
          role="option"
          aria-selected={active}
          class="flex w-full px-2.5 py-1.5 text-left text-xs transition {active
            ? 'bg-kotone-cyan/12 text-kotone-cyan'
            : 'text-white/75 hover:bg-white/8 hover:text-white'}"
          onclick={() => pick(option.value)}
        >
          {option.label}
        </button>
      {/each}
    </div>
  {/if}
</div>
