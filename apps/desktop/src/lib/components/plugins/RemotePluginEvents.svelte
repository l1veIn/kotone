<script lang="ts">
  // 第三方插件设置里的“事件”一块（ADR 047 D5，插件标准 0.1 第 3.4 节）：它的观察项一行一个——事件说明、调哪个工具、
  // 多久查一次、怎么算发生了、从哪来（它给的 / Kotone 带的 / 你加的）、开关、现在的样子（没开 / 要确认 / 查不了 / 等游戏开 /
  // 在查 / 停了）。停下的有“重试”，自己加的能删。
  // - 第一次打开（或者内容变了）先确认：说清楚“Kotone 会每 N 秒调一次它的 X 工具”、挂在哪个游戏上、这件事只给自动化；
  //   工具没标只读的要勾上“我知道，照样定时调它”。确认绑定的是那一刻的内容（后端的指纹），之后清单变了要重新确认。
  // - 自己加一个：选工具、填参数 JSON、间隔、怎么算（字段变了 / 列表多了新的一项）、事件名和说明，写进本地的 manifest.json，加完就开。
  // - 这些事只交给「自动化」的“当……的时候”：她看不到，也不推送（风险说明照旧在“风险和许可”弹窗里，这里不摆）。
  // 观察项的样子会变（在查 → 停了）：这一块开着时隔几秒重新读一次插件清单。
  import { onMount } from "svelte";
  import { RotateCcw, Trash2 } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { addWatch, refreshRemote, removeWatch, retryWatch, setWatch, type RemoteInfo, type RemoteWatch } from "$lib/mcpServers";
  import type { PluginInfo } from "$lib/plugins";
  import { companionName } from "$lib/stores/companion";
  import { errText, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin, remote }: { plugin: PluginInfo; remote: RemoteInfo } = $props();
  const id = $derived(plugin.manifest.id);
  const watches = $derived(remote.watches ?? []);
  const games = $derived(remote.kotoneManifest?.manifest.games ?? []);
  /** 它的工具（服务器上的原名）：自己加的时候选。 */
  const tools = $derived((remote.toolList ?? []).map((t) => ({ name: t.name.slice(id.length + 1), readOnly: t.readOnly, title: t.title })));
  let busy = $state(false);
  let confirming = $state<RemoteWatch | null>(null);
  let agree = $state(false);
  let adding = $state(false);
  let draft = $state({ name: "", label: "", tool: "", args: "{}", every: 60, compare: "changed" as "changed" | "new", field: "", to: "", list: "", idField: "id" });
  let draftAgree = $state(false);
  let draftError = $state("");

  onMount(() => {
    const timer = setInterval(() => void refreshRemote().catch(() => {}), 4000);
    return () => clearInterval(timer);
  });

  const listText = (list: string | null | undefined) => (list ? m.system_mcp_events_list_at({ list }) : m.system_mcp_events_list_self());
  function rule(w: { tool: string; intervalSeconds: number; compare: string; field?: string; to?: string[]; list?: string | null }): string {
    if (w.compare === "new") return m.system_mcp_events_rule_new({ seconds: w.intervalSeconds, tool: w.tool, list: listText(w.list) });
    if (w.to?.length) return m.system_mcp_events_rule_changed_to({ seconds: w.intervalSeconds, tool: w.tool, field: w.field ?? "", values: w.to.join(m.system_mcp_manifest_separator()) });
    return m.system_mcp_events_rule_changed({ seconds: w.intervalSeconds, tool: w.tool, field: w.field ?? "" });
  }
  const fromText = (from: string) =>
    ({ server: m.system_mcp_events_from_server(), preset: m.system_mcp_events_from_preset(), local: m.system_mcp_events_from_local() })[from] ?? "";
  function statusText(w: RemoteWatch): { text: string; tone: "muted" | "ok" | "warn" } {
    const s = w.status;
    switch (s.state) {
      case "off": return { text: m.system_mcp_events_status_off(), tone: "muted" };
      case "waiting": return { text: m.system_mcp_events_status_waiting(), tone: "muted" };
      case "polling": return { text: m.system_mcp_events_status_polling(), tone: "ok" };
      case "confirm": return { text: m.system_mcp_events_status_confirm({ reason: s.reason }), tone: "warn" };
      case "blocked": return { text: m.system_mcp_events_status_blocked({ reason: s.reason }), tone: "warn" };
      case "stopped": return { text: m.system_mcp_events_status_stopped({ reason: s.reason }), tone: "warn" };
    }
  }

  async function run(action: () => Promise<void>, done?: string) {
    busy = true;
    try { await action(); if (done) toast(true, done); } catch (error) { toast(false, errText(error)); } finally { busy = false; }
  }

  /** 开关：确认过这份内容的直接开；没确认过的先弹确认。 */
  function toggle(w: RemoteWatch, next: boolean) {
    if (!next) return void run(() => setWatch(id, w.event, false));
    if (w.confirmed && (w.readOnly === true || w.allowUnmarked)) return void run(() => setWatch(id, w.event, true));
    agree = false;
    confirming = w;
  }

  function confirmOn() {
    const w = confirming;
    if (!w) return;
    void run(async () => {
      await setWatch(id, w.event, true, w.digest, w.readOnly === true ? false : agree);
      confirming = null;
    });
  }

  function openAdd() {
    draft = { name: "", label: "", tool: tools.find((t) => t.readOnly)?.name ?? tools[0]?.name ?? "", args: "{}", every: 60, compare: "changed", field: "", to: "", list: "", idField: "id" };
    draftAgree = false;
    draftError = "";
    adding = true;
  }
  const draftTool = $derived(tools.find((t) => t.name === draft.tool));

  function submitAdd() {
    let args: unknown;
    try { args = JSON.parse(draft.args.trim() || "{}"); } catch { args = null; }
    if (!args || typeof args !== "object" || Array.isArray(args)) { draftError = m.system_mcp_events_add_bad_json(); return; }
    draftError = "";
    const name = draft.label.trim();
    void run(async () => {
      try {
        await addWatch(id, {
          name: draft.name.trim(), label: name, tool: draft.tool, arguments: args as Record<string, unknown>,
          everySeconds: Math.max(1, Math.round(Number(draft.every) || 60)), compare: draft.compare,
          field: draft.field.trim(), to: draft.to, list: draft.list.trim(), idField: draft.idField.trim(), allowUnmarked: draftAgree,
        });
        adding = false;
        toast(true, m.system_mcp_events_added({ event: name }));
      } catch (error) {
        draftError = errText(error);
      }
    });
  }
</script>

<section class="space-y-2" data-testid="remote-plugin-events">
  <div class="flex items-center gap-2">
    <h2 class="m-0 text-sm font-semibold text-foreground">{m.system_mcp_events_title()}</h2>
    <Button size="sm" variant="outline" class="ml-auto h-7 text-xs" disabled={busy || !tools.length} data-testid="remote-plugin-events-add" onclick={openAdd}>{m.system_mcp_events_add()}</Button>
  </div>
  <p class="m-0 text-xs leading-relaxed text-muted-foreground">{m.system_mcp_events_hint({ companion: $companionName })}</p>
  {#if !tools.length}<p class="m-0 text-[11px] text-muted-foreground" data-testid="remote-plugin-events-no-tools">{m.system_mcp_events_add_no_tools()}</p>{/if}
  {#if !watches.length}
    <p class="m-0 text-xs text-muted-foreground" data-testid="remote-plugin-events-empty">{m.system_mcp_events_empty()}</p>
  {:else}
    <ul class="m-0 list-none divide-y divide-hairline rounded-xl border border-hairline p-0">
      {#each watches as w (w.event)}
        {@const status = statusText(w)}
        <li class="flex items-start gap-3 px-4 py-2.5" data-testid="remote-plugin-event" data-event={w.event} data-state={w.status.state}>
          <div class="min-w-0 flex-1">
            <p class="m-0 flex flex-wrap items-center gap-2 text-sm text-foreground">
              {w.label}
              <code class="font-mono text-[11px] text-muted-foreground">{w.type}</code>
              <span class="rounded bg-muted px-1.5 text-[10px] text-muted-foreground" data-testid="remote-plugin-event-from">{fromText(w.from)}</span>
            </p>
            <p class="m-0 mt-0.5 text-xs leading-relaxed text-muted-foreground">{rule(w)}</p>
            <p class="m-0 mt-0.5 text-[11px] {status.tone === 'warn' ? 'text-warning' : status.tone === 'ok' ? 'text-primary' : 'text-muted-foreground'}"
              data-testid="remote-plugin-event-status">{status.text}</p>
          </div>
          <div class="flex shrink-0 items-center gap-1.5">
            {#if w.status.state === "stopped"}
              <Button size="sm" variant="ghost" class="h-7 gap-1 text-xs" disabled={busy} data-testid="remote-plugin-event-retry"
                onclick={() => void run(() => retryWatch(id, w.event))}><RotateCcw class="size-3.5" />{m.system_mcp_events_retry()}</Button>
            {/if}
            {#if w.from === "local"}
              <Button size="sm" variant="ghost" class="h-7 px-2 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={busy}
                aria-label={m.system_mcp_events_remove()} data-testid="remote-plugin-event-remove"
                onclick={() => void run(() => removeWatch(id, w.event), m.system_mcp_events_removed({ event: w.label }))}><Trash2 class="size-3.5" /></Button>
            {/if}
            <!-- 受控：显示的总是存着的值（要确认的先弹确认，确认了才开） -->
            <Switch disabled={busy} aria-label={m.system_mcp_events_switch_aria({ event: w.label })} data-testid="remote-plugin-event-switch"
              bind:checked={() => w.on, (next) => toggle(w, next)} />
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<Dialog.Root open={!!confirming} onOpenChange={(open) => { if (!open && !busy) confirming = null; }}>
  <Dialog.Content class="ui-scope" data-testid="remote-plugin-event-confirm">
    {#if confirming}
      {@const w = confirming}
      <Dialog.Header>
        <Dialog.Title>{m.system_mcp_events_confirm_title({ event: w.label })}</Dialog.Title>
      </Dialog.Header>
      <div class="space-y-2 text-sm leading-relaxed text-foreground">
        <p class="m-0" data-testid="remote-plugin-event-confirm-rule">{m.system_mcp_events_confirm_rule({ rule: rule(w) })}</p>
        <p class="m-0 break-all font-mono text-xs text-muted-foreground">{m.system_mcp_events_confirm_args({ args: JSON.stringify(w.arguments) })}</p>
        <p class="m-0 text-xs text-muted-foreground">
          {games.length ? m.system_mcp_events_confirm_games({ games: games.join(m.system_mcp_manifest_separator()) }) : m.system_mcp_events_confirm_always()}
        </p>
        <p class="m-0 text-xs text-muted-foreground">{m.system_mcp_events_confirm_where({ type: w.type, companion: $companionName })}</p>
        {#if !remote.trusted}<p class="m-0 text-xs text-warning">{m.system_mcp_events_untrusted_note()}</p>{/if}
        {#if w.readOnly !== true}
          <p class="m-0 text-xs text-warning" data-testid="remote-plugin-event-unmarked">{w.readOnly === false ? m.system_mcp_events_unmarked() : m.system_mcp_events_unknown_tool()}</p>
          <label class="flex items-center gap-2 text-xs">
            <input type="checkbox" class="size-4 accent-primary" bind:checked={agree} data-testid="remote-plugin-event-agree" />{m.system_mcp_events_agree()}
          </label>
        {/if}
      </div>
      <Dialog.Footer>
        <Button variant="ghost" disabled={busy} onclick={() => (confirming = null)}>{m.system_mcp_cancel()}</Button>
        <Button disabled={busy || (w.readOnly === false && !agree)} data-testid="remote-plugin-event-confirm-on" onclick={confirmOn}>{m.system_mcp_events_turn_on()}</Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root open={adding} onOpenChange={(open) => { if (!open && !busy) adding = false; }}>
  <Dialog.Content class="ui-scope max-h-[90vh] overflow-y-auto" data-testid="remote-plugin-event-add">
    <Dialog.Header>
      <Dialog.Title>{m.system_mcp_events_add_title()}</Dialog.Title>
      <Dialog.Description class="break-all">{m.system_mcp_events_add_hint({ path: remote.kotoneManifest?.localPath ?? "manifest.json" })}</Dialog.Description>
    </Dialog.Header>
    <div class="space-y-2.5 text-xs">
      <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_label()}</span>
        <Input class="h-8 text-xs" bind:value={draft.label} maxlength={64} placeholder={m.system_mcp_events_add_label_placeholder()} data-testid="remote-plugin-event-add-label" /></label>
      <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_name()}</span>
        <Input class="h-8 font-mono text-xs" bind:value={draft.name} maxlength={64} placeholder="farm_done" data-testid="remote-plugin-event-add-name" />
        <span class="block text-[11px] text-muted-foreground">{m.system_mcp_events_add_name_hint({ id })}</span></label>
      <div class="space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_tool()}</span>
        <SelectField ariaLabel={m.system_mcp_events_add_tool()} value={draft.tool} testid="remote-plugin-event-add-tool"
          options={tools.map((t) => ({ value: t.name, label: t.readOnly ? `${t.name} · ${m.system_mcp_tool_read_only()}` : t.name }))}
          onchange={(value) => { draft.tool = value; draftAgree = false; }} /></div>
      <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_arguments()}</span>
        <textarea class="field min-h-14 w-full rounded-md border border-hairline bg-background px-2 py-1 font-mono text-xs" bind:value={draft.args} data-testid="remote-plugin-event-add-args"></textarea></label>
      <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_every()}</span>
        <Input type="number" min={1} class="h-8 w-28 text-xs" bind:value={draft.every} data-testid="remote-plugin-event-add-every" /></label>
      <div class="space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_compare()}</span>
        <SelectField ariaLabel={m.system_mcp_events_add_compare()} value={draft.compare} testid="remote-plugin-event-add-compare"
          options={[{ value: "changed", label: m.system_mcp_events_add_compare_changed() }, { value: "new", label: m.system_mcp_events_add_compare_new() }]}
          onchange={(value) => { draft.compare = value === "new" ? "new" : "changed"; }} /></div>
      {#if draft.compare === "changed"}
        <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_field()}</span>
          <Input class="h-8 font-mono text-xs" bind:value={draft.field} data-testid="remote-plugin-event-add-field" /></label>
        <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_to()}</span>
          <Input class="h-8 font-mono text-xs" bind:value={draft.to} data-testid="remote-plugin-event-add-to" /></label>
      {:else}
        <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_list()}</span>
          <Input class="h-8 font-mono text-xs" bind:value={draft.list} data-testid="remote-plugin-event-add-list" /></label>
        <label class="block space-y-1"><span class="text-muted-foreground">{m.system_mcp_events_add_id()}</span>
          <Input class="h-8 font-mono text-xs" bind:value={draft.idField} data-testid="remote-plugin-event-add-id" /></label>
      {/if}
      {#if draft.tool}
        <p class="m-0 leading-relaxed text-foreground" data-testid="remote-plugin-event-add-rule">
          {m.system_mcp_events_confirm_rule({ rule: rule({ tool: draft.tool, intervalSeconds: Math.max(1, Math.round(Number(draft.every) || 60)), compare: draft.compare,
            field: draft.field, to: draft.to.split(/[,，、]/).map((x) => x.trim()).filter(Boolean), list: draft.list.trim() || null }) })}</p>
      {/if}
      {#if draftTool && !draftTool.readOnly}
        <p class="m-0 text-warning">{m.system_mcp_events_unmarked()}</p>
        <label class="flex items-center gap-2">
          <input type="checkbox" class="size-4 accent-primary" bind:checked={draftAgree} data-testid="remote-plugin-event-add-agree" />{m.system_mcp_events_agree()}
        </label>
      {/if}
      {#if draftError}<p class="m-0 break-all text-destructive" data-testid="remote-plugin-event-add-error">{draftError}</p>{/if}
    </div>
    <Dialog.Footer>
      <Button variant="ghost" disabled={busy} onclick={() => (adding = false)}>{m.system_mcp_cancel()}</Button>
      <Button disabled={busy || !draft.label.trim() || !draft.name.trim() || !draft.tool || (!!draftTool && !draftTool.readOnly && !draftAgree)}
        data-testid="remote-plugin-event-add-submit" onclick={submitAdd}>{m.system_mcp_events_add_submit()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
