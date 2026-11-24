<script lang="ts">
  // 信任提示里一个服务器的事实（MCP 方案 4.4，照 VS Code 和 Claude Desktop）：它会在你电脑上运行什么程序（哪个包）、
  // 或者连哪个网址；给它哪些环境变量、请求头（只有名字，值里可能是 Key）；要哪些 Key；配置里写着的 Key 会搬走。
  import { Globe, Terminal } from "@lucide/svelte";
  import type { TrustFacts } from "$lib/mcpServers";
  import { m } from "$lib/paraglide/messages.js";

  let { facts, moved = [], testid }: { facts: TrustFacts; moved?: string[]; testid?: string } = $props();
  const keys = $derived(facts.keys.filter((k) => !moved.includes(k)));
</script>

<div class="space-y-1.5 text-xs leading-relaxed" data-testid={testid}>
  {#if facts.kind === "stdio"}
    <p class="m-0 flex items-center gap-1.5 font-medium text-foreground"><Terminal class="size-3.5 text-muted-foreground" />{m.system_mcp_trust_runs()}</p>
    <code class="block rounded-md bg-muted px-2 py-1.5 font-mono text-[11px] break-all text-foreground" data-testid={testid ? `${testid}-command` : undefined}>{facts.summary}</code>
    {#if facts.package}<p class="m-0 text-muted-foreground" data-testid={testid ? `${testid}-package` : undefined}>{m.system_mcp_trust_package({ package: facts.package })}</p>{/if}
    {#if facts.envNames?.length}<p class="m-0 text-muted-foreground">{m.system_mcp_trust_env({ names: facts.envNames.join(", ") })}</p>{/if}
  {:else}
    <p class="m-0 flex items-center gap-1.5 font-medium text-foreground"><Globe class="size-3.5 text-muted-foreground" />{m.system_mcp_trust_connects()}</p>
    <code class="block rounded-md bg-muted px-2 py-1.5 font-mono text-[11px] break-all text-foreground" data-testid={testid ? `${testid}-url` : undefined}>{facts.url}</code>
    {#if facts.headerNames?.length}<p class="m-0 text-muted-foreground">{m.system_mcp_trust_headers({ names: facts.headerNames.join(", ") })}</p>{/if}
  {/if}
  {#if moved.length}<p class="m-0 text-muted-foreground" data-testid={testid ? `${testid}-moved` : undefined}>{m.system_mcp_trust_moved({ names: moved.join(", ") })}</p>{/if}
  {#if keys.length}<p class="m-0 text-warning" data-testid={testid ? `${testid}-keys` : undefined}>{m.system_mcp_trust_keys({ names: keys.join(", ") })}</p>{/if}
</div>
