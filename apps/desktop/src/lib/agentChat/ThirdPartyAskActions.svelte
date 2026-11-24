<script lang="ts">
  // 确认卡片上给第三方插件的工具多的一个按钮（MCP 方案第七节第 3 条，照 Codex 的 writes 和 OpenClaw 的“总是允许”）：
  // - 它说只读、你还没信任这个插件：“信任这个插件”（它说只读的工具以后都不问），这一次也允许；
  // - 别的：“总是允许这个工具”（权限里这个工具一条“允许”，角色页 → 权限里看得到、改得回来），这一次也允许。
  // 不是第三方插件的工具时什么都不显示。
  import { onMount } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { alwaysAllow, thirdPartyTool, trustServer } from "$lib/mcpServers";
  import { listPlugins, pluginsStore } from "$lib/plugins";
  import { errText } from "$lib/stores/ui";
  import type { Answer } from "../agent";
  import { m } from "$lib/paraglide/messages.js";

  let { tool, disabled = false, onAnswer }: {
    tool: string;
    disabled?: boolean;
    /** 照常回答这一次的确认。 */
    onAnswer: (answer: Answer) => Promise<void>;
  } = $props();

  const info = $derived(thirdPartyTool(tool, $pluginsStore));
  const trustInstead = $derived(!!info?.tool?.readOnly && !info.plugin.remote.trusted);
  let busy = $state(false);
  let error = $state("");

  onMount(() => {
    if (!$pluginsStore.length) void listPlugins().catch(() => {});
  });

  async function go() {
    if (!info || busy) return;
    busy = true;
    error = "";
    try {
      if (trustInstead) await trustServer(info.plugin.manifest.id, true);
      else await alwaysAllow(tool);
      await onAnswer("allow-once");
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if info}
  <Button size="sm" variant="outline" data-testid={trustInstead ? "agent-ask-trust-plugin" : "agent-ask-always-allow"}
    title={m.system_mcp_ask_note({ id: info.plugin.manifest.id })} disabled={disabled || busy} onclick={() => void go()}>
    {trustInstead ? m.system_mcp_ask_trust() : m.system_mcp_ask_always()}
  </Button>
  {#if error}<span role="alert" class="basis-full text-xs text-destructive">{error}</span>{/if}
{/if}
