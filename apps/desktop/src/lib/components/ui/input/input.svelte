<script lang="ts">
	import type { HTMLInputAttributes } from "svelte/elements";
	import { cn, type WithElementRef } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		value = $bindable(),
		type,
		class: className,
		"data-slot": dataSlot = "input",
		...restProps
	}: WithElementRef<Omit<HTMLInputAttributes, "type" | "files"> & { type?: Exclude<HTMLInputAttributes["type"], "file"> }> = $props();
</script>

<input
	bind:this={ref}
	data-slot={dataSlot}
	class={cn(
		"h-8 w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:bg-input/30 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40",
		className
	)}
	{type}
	bind:value
	{...restProps}
/>
