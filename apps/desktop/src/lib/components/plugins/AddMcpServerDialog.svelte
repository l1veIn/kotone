<script lang="ts">
  // “加 MCP 服务器”（MCP 方案第 2 期）：粘贴配置，或者从 Claude Desktop、VS Code 导入 → 勾选要加的、改名字 →
  // 信任提示（它会在你电脑上运行什么 / 连哪个网址、要哪些 Key、风险；照 VS Code 和 Claude Desktop）→ 信任并添加。
  // 玩家在信任提示上点了同意才调 `mcp_server_add`（后端把明文 Key 搬进 Windows 的安全存储、写 mcp.json、打开开关、记信任）。
  import { ShieldAlert } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Textarea } from "$lib/components/ui/textarea";
  import { addServer, importSources, previewServers, type McpCandidate, type McpImportSource } from "$lib/mcpServers";
  import { companionName } from "$lib/stores/companion";
  import { errText, toast } from "$lib/stores/ui";
  import McpTrustFacts from "./McpTrustFacts.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { open = $bindable(false), onAdded }: { open?: boolean; /** 加了至少一个（插件页接着隔一会儿重读清单，看它连上没有）。 */ onAdded?: () => void } = $props();

  type Tab = "paste" | "claude-desktop" | "vscode";
  let tab = $state<Tab>("paste");
  let step = $state<"choose" | "trust">("choose");
  let text = $state("");
  let readError = $state("");
  let pasted = $state<McpCandidate[]>([]);
  let sources = $state<McpImportSource[] | null>(null);
  let scanning = $state(false);
  /** 勾上的候选（key = 来源 + 序号）和改过的名字。 */
  let chosen = $state<Record<string, boolean>>({});
  let names = $state<Record<string, string>>({});
  let adding = $state(false);

  const APPS: { id: Exclude<Tab, "paste">; name: string }[] = [
    { id: "claude-desktop", name: "Claude Desktop" },
    { id: "vscode", name: "VS Code" },
  ];

  /** 当前标签下的候选（带一个稳定的 key）。 */
  const candidates = $derived.by((): { key: string; c: McpCandidate }[] => {
    if (tab === "paste") return pasted.map((c, i) => ({ key: `paste:${i}`, c }));
    const source = sources?.find((s) => s.source === tab);
    return (source?.files ?? []).flatMap((f, fi) => f.servers.map((c, i) => ({ key: `${tab}:${fi}:${i}`, c })));
  });
  const files = $derived(sources?.find((s) => s.source === tab)?.files ?? []);
  const usable = (c: McpCandidate) => !c.error && !c.alreadyAdded;
  const picked = $derived(candidates.filter(({ key, c }) => chosen[key] && usable(c)));

  $effect(() => {
    if (!open) {
      step = "choose";
      readError = "";
      return;
    }
  });

  async function read() {
    readError = "";
    try {
      pasted = await previewServers(text);
      chosen = Object.fromEntries(pasted.map((c, i) => [`paste:${i}`, usable(c)]));
      names = Object.fromEntries(pasted.map((c, i) => [`paste:${i}`, c.id]));
    } catch (error) {
      pasted = [];
      readError = errText(error);
    }
  }

  async function scan(next: Tab) {
    tab = next;
    if (next === "paste" || sources || scanning) return;
    scanning = true;
    try {
      sources = await importSources();
      for (const s of sources) s.files.forEach((f, fi) => f.servers.forEach((c, i) => {
        const key = `${s.source}:${fi}:${i}`;
        chosen[key] = false;
        names[key] = c.id;
      }));
    } catch (error) {
      toast(false, errText(error));
      sources = [];
    } finally {
      scanning = false;
    }
  }

  async function confirm() {
    adding = true;
    const done: string[] = [];
    try {
      for (const { key, c } of picked) {
        const id = (names[key] ?? c.id).trim();
        await addServer(id, c.entry);
        done.push(id);
      }
      toast(true, m.system_mcp_added({ names: done.join(", ") }));
      onAdded?.();
      open = false;
      pasted = [];
      text = "";
      chosen = {};
    } catch (error) {
      toast(false, m.system_mcp_add_failed({ error: errText(error) }));
      if (done.length) {
        toast(true, m.system_mcp_added({ names: done.join(", ") }));
        onAdded?.();
      }
    } finally {
      adding = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="ui-scope max-h-[85vh] overflow-y-auto sm:max-w-2xl" data-testid="mcp-add-dialog">
    <Dialog.Header>
      <Dialog.Title>{step === "trust" ? m.system_mcp_trust_title() : m.system_mcp_dialog_title()}</Dialog.Title>
      {#if step === "choose"}<Dialog.Description>{m.system_mcp_dialog_description()}</Dialog.Description>{/if}
    </Dialog.Header>

    {#if step === "choose"}
      <div class="flex flex-wrap gap-1 rounded-lg bg-muted p-1 text-xs" role="tablist">
        {#each [{ id: "paste" as Tab, label: m.system_mcp_tab_paste() }, ...APPS.map((a) => ({ id: a.id as Tab, label: m.system_mcp_tab_import({ app: a.name }) }))] as t (t.id)}
          <button type="button" role="tab" aria-selected={tab === t.id} data-testid={`mcp-add-tab-${t.id}`}
            class="rounded-md px-2.5 py-1 transition-colors {tab === t.id ? 'bg-background font-medium text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            onclick={() => void scan(t.id)}>{t.label}</button>
        {/each}
      </div>

      {#if tab === "paste"}
        <p class="m-0 text-xs leading-relaxed text-muted-foreground">{m.system_mcp_paste_hint()}</p>
        <Textarea bind:value={text} rows={7} spellcheck={false} class="font-mono text-xs" data-testid="mcp-add-paste"
          placeholder={'{ "mcpServers": { "everything": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"] } } }'} />
        <div class="flex items-center gap-2">
          <Button size="sm" variant="outline" disabled={!text.trim()} data-testid="mcp-add-read" onclick={() => void read()}>{m.system_mcp_read()}</Button>
          {#if readError}<span role="alert" class="text-xs text-destructive" data-testid="mcp-add-read-error">{m.system_mcp_read_failed({ error: readError })}</span>{/if}
        </div>
      {:else if scanning}
        <p class="m-0 text-xs text-muted-foreground">{m.system_mcp_scanning()}</p>
      {:else if !files.length}
        <p class="m-0 text-xs text-muted-foreground" data-testid="mcp-add-none">{m.system_mcp_none_found({ app: APPS.find((a) => a.id === tab)?.name ?? "" })}</p>
      {:else}
        {#each files as file (file.path)}
          <p class="m-0 font-mono text-[11px] break-all text-muted-foreground">{file.path}</p>
          {#if file.error}<p class="m-0 text-xs text-destructive">{m.system_mcp_read_failed({ error: file.error })}</p>{/if}
        {/each}
      {/if}

      {#if candidates.length}
        <ul class="m-0 list-none space-y-2 p-0" data-testid="mcp-add-candidates">
          {#each candidates as { key, c } (key)}
            <li class="rounded-xl border border-hairline p-3 {usable(c) ? '' : 'opacity-60'}" data-testid="mcp-add-candidate" data-name={c.name}>
              <div class="flex items-center gap-2">
                <input type="checkbox" class="size-4 accent-primary" disabled={!usable(c)} bind:checked={chosen[key]}
                  aria-label={c.name} data-testid="mcp-add-candidate-check" />
                <span class="text-sm font-medium text-foreground">{c.name}</span>
                {#if usable(c)}
                  <label class="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">{m.system_mcp_name_label()}
                    <Input class="h-7 w-40 font-mono text-xs" bind:value={names[key]} data-testid="mcp-add-candidate-id" />
                  </label>
                {/if}
              </div>
              {#if c.error}
                <p class="m-0 mt-1.5 text-xs text-destructive" data-testid="mcp-add-candidate-error">{m.system_mcp_config_error({ error: c.error })}</p>
              {:else if c.alreadyAdded}
                <p class="m-0 mt-1.5 text-xs text-muted-foreground">{m.system_mcp_already_added({ id: c.alreadyAdded })}</p>
              {:else if c.facts}
                <p class="m-0 mt-1.5 truncate font-mono text-[11px] text-muted-foreground">{c.facts.summary}</p>
              {/if}
              {#each c.warnings as warning (warning)}<p class="m-0 mt-1 text-xs text-warning">{warning}</p>{/each}
            </li>
          {/each}
        </ul>
      {/if}

      <Dialog.Footer>
        <Button variant="ghost" onclick={() => (open = false)}>{m.system_mcp_cancel()}</Button>
        <Button disabled={!picked.length} data-testid="mcp-add-next" onclick={() => (step = "trust")}>{m.system_mcp_next()}</Button>
      </Dialog.Footer>
    {:else}
      <!-- 信任提示：风险在最上面，每个要加的服务器一块 -->
      <div class="flex gap-2 rounded-xl border border-warning/45 bg-warning/10 p-3 text-xs leading-relaxed text-foreground" data-testid="mcp-trust-risk">
        <ShieldAlert class="mt-0.5 size-4 shrink-0 text-warning" />
        <span>{m.system_mcp_risk()}</span>
      </div>
      <ul class="m-0 list-none space-y-2 p-0" data-testid="mcp-trust-list">
        {#each picked as { key, c } (key)}
          <li class="rounded-xl border border-hairline p-3" data-testid="mcp-trust-item" data-id={names[key] ?? c.id}>
            <p class="m-0 mb-2 text-sm font-semibold text-foreground">{names[key] ?? c.id}</p>
            {#if c.facts}<McpTrustFacts facts={c.facts} moved={c.movedKeys} testid="mcp-trust-facts" />{/if}
          </li>
        {/each}
      </ul>
      <p class="m-0 text-xs leading-relaxed text-muted-foreground">{m.system_mcp_trust_after({ companion: $companionName })}</p>
      <Dialog.Footer>
        <Button variant="ghost" disabled={adding} onclick={() => (step = "choose")}>{m.system_mcp_back()}</Button>
        <Button disabled={adding || !picked.length} data-testid="mcp-trust-confirm" onclick={() => void confirm()}>{m.system_mcp_trust_confirm()}</Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>
