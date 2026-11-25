<script lang="ts">
  // 别的页面上的热键：只显示现在的键和“去改”（ADR 040 D3：热键只在「设置 → 热键」改）。点了打开那一页、滚到这一行。
  import { hotkeyLabel } from "../hotkeyCombo";
  import { openTarget } from "../stores/navigation";
  import { m } from "$lib/paraglide/messages.js";

  let { id, value }: {
    /** 热键表里那一行的 id（命令名 `kotone.call`、`quick-message-n`）。 */
    id: string;
    /** 现在的键（空 = 没设）。 */
    value: string;
  } = $props();
</script>

<div class="flex flex-wrap items-center gap-2" data-testid={`hotkey-link-${id}`}>
  <kbd data-testid={`hotkey-link-key-${id}`}
    class="rounded-md border px-2 py-0.5 font-sans text-xs {value.trim() ? 'border-hairline bg-background/60 text-foreground' : 'border-dashed border-hairline text-muted-foreground'}">
    {value.trim() ? hotkeyLabel(value.trim()) : m.voice_hotkey_unset()}
  </kbd>
  <button type="button" data-testid={`hotkey-link-goto-${id}`} class="text-xs font-medium text-primary hover:underline"
    onclick={() => openTarget({ page: "advanced", tab: "hotkeys", anchor: `hotkey-${id}` })}>{m.hotkeys_change_in_table()}</button>
</div>
