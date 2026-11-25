<!-- 右侧栏“卡片”里一张等玩家点的页面：`ui.open` 打开时玩家正在看别的，没切过去（stores/uiOpen.ts）。 -->
<!-- 点“打开”才去看（插件的页面在聊天页的右侧栏，设置页跳过去）；“×”不看了。 -->
<script lang="ts">
  import { Settings2, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import PluginLogo from "../components/plugins/PluginLogo.svelte";
  import { pluginText } from "../plugins";
  import { dismissWaiting, openWaiting, type WaitingPage } from "../stores/uiOpen";

  let { item }: { item: WaitingPage } = $props();
</script>

<li class="col-span-full min-w-0 overflow-hidden rounded-xl border border-primary/30 bg-primary/5" data-testid="ui-open-waiting" data-uri={item.uri}>
  <div class="flex min-w-0 items-center gap-2 py-1.5 pr-1 pl-2.5">
    {#if item.plugin}<PluginLogo id={item.plugin} name={pluginText(item.plugin, "name", item.plugin)} size={16} />
    {:else}<Settings2 class="size-4 shrink-0 text-muted-foreground" />{/if}
    <div class="min-w-0 flex-1">
      <p class="m-0 truncate text-[13px] font-medium text-foreground/90" data-testid="ui-open-waiting-title" title={item.title}>{item.title}</p>
      <p class="m-0 truncate text-[11px] leading-4 text-muted-foreground">{m.chat_ui_open_waiting_hint()}</p>
    </div>
    <Button size="sm" variant="ghost" class="h-6 shrink-0 px-2 text-[11px] text-primary" data-testid="ui-open-waiting-open"
      onclick={() => openWaiting(item.key)}>{m.chat_ui_open_open()}</Button>
    <Button size="icon" variant="ghost" class="size-6 shrink-0 text-muted-foreground" aria-label={m.chat_ui_open_dismiss()} title={m.chat_ui_open_dismiss()}
      data-testid="ui-open-waiting-dismiss" onclick={() => dismissWaiting(item.key)}><X class="size-3" /></Button>
  </div>
</li>
