<script lang="ts">
  // 原神插件的 App（左侧栏「插件 → 原神」，ADR 047 第 3 期）：BetterGI 在哪、开没开；它的通知 Webhook 有没有指向 Kotone
  //（一键填好：BetterGI 没开着时才改它的配置）；能跑的一条龙配置、配置组、计划，各一个“跑”按钮（受原神游戏配置的
  // “允许自动化输入”管）；最近的运行、它日志的最后几条、怎么停（BetterGI 没有给外部停止的接口，如实说）。
  // 日志、计划名是 BetterGI 和玩家写的字：Svelte 照文字显示（会转义）。
  import { onMount } from "svelte";
  import { Copy, FolderSearch, Play, RefreshCw, ScrollText, Square, Webhook } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";
  import type { PluginPageProps } from "$lib/pluginUi";
  import { listenPluginEvent, pluginEnabled, setPluginEnabled, sourceEnabled } from "$lib/plugins";
  import { openTarget } from "$lib/stores/navigation";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import RunLine from "./RunLine.svelte";
  import { genshinFillWebhook, genshinLists, genshinRun, genshinStatus, PLUGIN_ID, type GenshinLists, type GenshinStatus } from "./genshin";
  import { kindLabel, planStatusLabel } from "./labels";

  let { plugin, openSettings, embedded = false }: PluginPageProps & { embedded?: boolean } = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest) && sourceEnabled($settingsStore, plugin.manifest, "bettergi"));
  let status = $state<GenshinStatus | null>(null);
  let lists = $state<GenshinLists | null>(null);
  let error = $state("");
  let busy = $state<string | null>(null);
  let copied = $state(false);
  let requestId = 0;

  const found = $derived(status?.status === "ok");
  const running = $derived(!!status?.running);
  const allowed = $derived(status?.automation?.allowed !== false);
  const webhook = $derived(status?.webhook);

  async function refresh() {
    const id = ++requestId;
    try {
      const [next, nextLists] = await Promise.all([genshinStatus(), genshinLists()]);
      if (id !== requestId) return;
      status = next;
      lists = nextLists?.status === "ok" ? nextLists : null;
      error = "";
    } catch (e) {
      if (id === requestId) error = m.plugin_genshin_status_failed({ error: errText(e) });
    }
  }
  $effect(() => {
    if (on) void refresh();
    else { status = null; lists = null; error = ""; }
    return () => { requestId++; };
  });
  onMount(() => {
    const timer = setInterval(() => { if (on && document.visibilityState === "visible") void refresh(); }, 5000);
    let unlisten: (() => void) | null = null;
    void listenPluginEvent(PLUGIN_ID, "runs", () => { if (on) void refresh(); }).then((u) => { unlisten = u; });
    return () => { clearInterval(timer); unlisten?.(); };
  });

  async function enable() {
    try { await setPluginEnabled(plugin.manifest.id, true); }
    catch (e) { toast(false, m.system_plugins_save_failed({ error: errText(e) })); }
  }
  const settings = () => openSettings ? openSettings() : openTarget({ page: "plugin", id: plugin.manifest.id, params: { view: "settings" } });

  async function run(kind: "dragon" | "group" | "plan", name: string, id: string) {
    if (busy) return;
    busy = id;
    try {
      const answer = await genshinRun(kind, name);
      if (answer.status === "started") toast(true, m.plugin_genshin_started({ name: answer.name || kindLabel(kind) }));
      else toast(false, answer.text ?? answer.status);
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    } finally {
      busy = null;
    }
  }

  async function fill() {
    if (busy) return;
    busy = "webhook";
    try {
      const answer = await genshinFillWebhook();
      if (answer.status === "filled") toast(true, m.plugin_genshin_webhook_filled({ backup: answer.backup ?? "" }));
      else toast(false, answer.text ?? answer.status);
      await refresh();
    } catch (e) {
      toast(false, errText(e));
    } finally {
      busy = null;
    }
  }

  async function copy() {
    if (!webhook?.url) return;
    try { await navigator.clipboard.writeText(webhook.url); copied = true; setTimeout(() => (copied = false), 1500); }
    catch (e) { toast(false, errText(e)); }
  }

  const foundLabel = (how: string) => ({ settings: m.plugin_genshin_found_settings(), env: m.plugin_genshin_found_env(),
    common: m.plugin_genshin_found_common(), shortcut: m.plugin_genshin_found_shortcut() } as Record<string, string>)[how] ?? how;
  const clock = (ms: number) => new Date(ms).toLocaleString(undefined, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const disabledRun = (kind: string) => !!busy || !allowed || (running && kind !== "plan");
</script>

<div class="desk" class:embedded data-testid="plugin-page-genshin">
  <header>
    <h1>{m.plugin_genshin_name()}</h1>
    {#if on && found}
      <span class="state" class:live={running} data-testid="genshin-running" role="status"><span class="lamp"></span>
        {running ? m.plugin_genshin_running() : m.plugin_genshin_not_running()}</span>
    {/if}
  </header>

  {#if !on}
    <section class="off" data-testid="plugin-genshin-off">
      <h2>{m.plugin_genshin_off_title()}</h2>
      <p>{m.plugin_genshin_off_note()}</p>
      <button class="action" data-testid="plugin-genshin-enabled" onclick={enable}>{m.plugin_genshin_enable()}</button>
    </section>
  {:else if error}
    <p class="message" role="alert">{error}</p>
  {:else if !status}
    <p class="message">{m.plugin_genshin_reading()}</p>
  {:else if !found}
    <section class="off" data-testid="genshin-not-found">
      <FolderSearch aria-hidden="true" />
      <h2>{m.plugin_genshin_not_found_title()}</h2>
      <p>{m.plugin_genshin_not_found_note()}</p>
      {#if status.searched?.length}<p class="small" data-testid="genshin-searched">{m.plugin_genshin_searched({ places: status.searched.slice(0, 6).join("、") })}</p>{/if}
      <button class="action" data-testid="genshin-open-settings" onclick={settings}>{m.plugin_genshin_open_settings()}</button>
    </section>
  {:else}
    <section class="row" data-testid="genshin-install">
      <div class="min-w-0">
        <span class="caption">{m.plugin_genshin_install_label()}</span>
        <p class="path" title={status.install?.exe}>{status.install?.dir}</p>
        <p class="small">{status.install?.version ? m.plugin_genshin_version({ version: status.install.version }) : m.plugin_genshin_version_unknown()} · {foundLabel(status.install?.found ?? "")}</p>
      </div>
      <button class="ghost" onclick={() => void refresh()} aria-label={m.plugin_genshin_refresh()} title={m.plugin_genshin_refresh()}><RefreshCw /></button>
    </section>

    <section class="block" data-testid="genshin-webhook" data-to-kotone={webhook?.toKotone ? "true" : "false"}>
      <h2><Webhook aria-hidden="true" />{m.plugin_genshin_webhook_title()}</h2>
      <p>{webhook?.toKotone ? m.plugin_genshin_webhook_ok() : m.plugin_genshin_webhook_off()}</p>
      {#if webhook?.error}<p class="small warn" role="alert">{m.plugin_genshin_webhook_error({ error: webhook.error })}</p>{/if}
      {#if webhook?.url}
        <div class="url">
          <code data-testid="genshin-webhook-url">{webhook.url}</code>
          <button class="ghost" onclick={copy} data-testid="genshin-webhook-copy">{#if copied}{m.plugin_genshin_webhook_copied()}{:else}<Copy />{m.plugin_genshin_webhook_copy()}{/if}</button>
        </div>
      {/if}
      {#if !webhook?.toKotone}
        <button class="action" data-testid="genshin-webhook-fill" disabled={!!busy || running} onclick={fill}>{m.plugin_genshin_webhook_fill()}</button>
        {#if running}<p class="small">{m.plugin_genshin_webhook_busy_note()}</p>{/if}
      {/if}
      {#if webhook?.lastNoticeAtMs}<p class="small">{m.plugin_genshin_webhook_last({ time: clock(webhook.lastNoticeAtMs) })}</p>{/if}
    </section>

    <section class="block" data-testid="genshin-runnable">
      <h2><Play aria-hidden="true" />{m.plugin_genshin_runnable_title()}</h2>
      {#if !allowed}
        <p class="warn" data-testid="genshin-automation-off">{m.plugin_genshin_automation_off()}
          <button class="link" onclick={() => openTarget({ page: "game" })}>{m.plugin_genshin_open_games()}</button></p>
      {:else if running}
        <p class="small" data-testid="genshin-busy-note">{m.plugin_genshin_busy_note()}</p>
      {/if}
      <h3>{m.plugin_genshin_dragon_title()}</h3>
      <ul>
        {#each lists?.oneDragon ?? [] as d (d.name)}
          <li data-testid="genshin-dragon">
            <span class="name">{d.name}{#if d.selected}<span class="tag">{m.plugin_genshin_selected()}</span>{/if}</span>
            <button class="run" disabled={disabledRun("dragon")} onclick={() => run("dragon", d.name, `dragon:${d.name}`)} data-testid="genshin-run-dragon">{m.plugin_genshin_run()}</button>
          </li>
        {:else}
          <li class="none">{m.plugin_genshin_none()}</li>
        {/each}
      </ul>
      <h3>{m.plugin_genshin_groups_title()}</h3>
      <ul>
        {#each lists?.groups ?? [] as g (g.name)}
          <li data-testid="genshin-group">
            <span class="name">{g.name}<span class="tag muted">{m.plugin_genshin_projects({ count: g.projects })}</span></span>
            <button class="run" disabled={disabledRun("group")} onclick={() => run("group", g.name, `group:${g.name}`)} data-testid="genshin-run-group">{m.plugin_genshin_run()}</button>
          </li>
        {:else}
          <li class="none">{m.plugin_genshin_none()}</li>
        {/each}
      </ul>
      <h3>{m.plugin_genshin_plans_title()}</h3>
      {#if lists && !lists.plansSupported}
        <p class="small">{m.plugin_genshin_plans_unsupported()}</p>
      {:else}
        <ul>
          {#each lists?.plans ?? [] as p (p.id)}
            <li data-testid="genshin-plan">
              <span class="name" title={p.description}>{p.name}<span class="tag muted">{p.id}</span></span>
              <button class="run" disabled={disabledRun("plan")} onclick={() => run("plan", p.id, `plan:${p.id}`)} data-testid="genshin-run-plan">{m.plugin_genshin_run()}</button>
            </li>
          {:else}
            <li class="none">{m.plugin_genshin_none()}</li>
          {/each}
        </ul>
        {#if status.plans?.queue && (status.plans.queue.active.length || status.plans.queue.pending)}
          <p class="small" data-testid="genshin-plan-queue">{m.plugin_genshin_plan_queue({ active: status.plans.queue.active.map((r) => `${r.planName}（${planStatusLabel(r.status)}）`).join("、") || "-", pending: status.plans.queue.pending })}</p>
        {/if}
      {/if}
    </section>

    <section class="block" data-testid="genshin-recent">
      <h2>{m.plugin_genshin_recent_title()}</h2>
      {#if (status.current?.length ?? 0) + (status.recent?.length ?? 0) === 0}
        <p class="small">{m.plugin_genshin_recent_empty()}</p>
      {:else}
        <ul class="runs">
          {#each [...(status.current ?? []), ...(status.recent ?? [])] as r (r.key)}<li><RunLine run={r} /></li>{/each}
        </ul>
      {/if}
    </section>

    {#if status.log?.length}
      <section class="block" data-testid="genshin-log">
        <h2><ScrollText aria-hidden="true" />{m.plugin_genshin_log_title()}</h2>
        <ol class="log">
          {#each status.log as line, i (i)}<li class:warn={line.level !== "INF"}><span class="time">{line.time}</span>{line.text}</li>{/each}
        </ol>
      </section>
    {/if}

    <section class="block" data-testid="genshin-stop">
      <h2><Square aria-hidden="true" />{m.plugin_genshin_stop_title()}</h2>
      <p class="small">{status.stop}</p>
    </section>
  {/if}
  {#if on}
    <footer><button class="options" data-testid="genshin-options" onclick={settings}>{m.plugin_genshin_options()}</button></footer>
  {/if}
</div>

<style>
  .desk { --signal: light-dark(#a1621a, #f0c27a); --button: light-dark(#8a5314, #f0c27a); --button-ink: light-dark(#fff, #2a1a06); --rule: color-mix(in srgb, var(--foreground) 14%, transparent);
    container-type: inline-size; box-sizing: border-box; width: 100%; max-width: 980px; margin: 0 auto; padding: clamp(20px,4.7vw,58px) clamp(18px,5.2vw,62px); color: var(--foreground); }
  .desk.embedded { padding: 10px 4px 18px; }
  header, .row, .url, footer { display:flex; align-items:center; justify-content:space-between; gap:12px; }
  header { flex-wrap:wrap; padding-bottom: 20px; border-bottom: 1px solid var(--rule); }
  h1, h2, h3, p { margin:0; }
  h1 { font-size:clamp(25px,4.2cqw,38px); line-height:1.17; font-weight:680; letter-spacing:-.045em; }
  h2 { display:flex; align-items:center; gap:8px; font-size:14px; font-weight:650; }
  h2 :global(svg) { width:17px; height:17px; color:var(--signal); }
  h3 { margin-top:14px; font-size:12px; font-weight:600; color:var(--muted-foreground); }
  .state { display:inline-flex; align-items:center; gap:7px; color:var(--muted-foreground); font-size:12px; }
  .lamp { width:7px; height:7px; border:1px solid currentColor; border-radius:50%; }
  .state.live { color:var(--signal); }
  .state.live .lamp { background:currentColor; }
  .row { padding:18px 0; border-bottom:1px solid var(--rule); }
  .caption { color:var(--muted-foreground); font-size:12px; font-weight:550; }
  .path { margin-top:4px; font-size:14px; overflow-wrap:anywhere; }
  .small { margin-top:6px; color:var(--muted-foreground); font-size:12px; line-height:1.6; overflow-wrap:anywhere; }
  .warn { color:light-dark(#b4232f, #ff9aa2); }
  .block { display:grid; gap:8px; padding:18px 0; border-bottom:1px solid var(--rule); }
  .block > p { font-size:13px; line-height:1.6; }
  .url { flex-wrap:wrap; justify-content:flex-start; }
  code { font-size:12px; padding:4px 8px; border-radius:6px; background:var(--muted); overflow-wrap:anywhere; }
  ul, ol { margin:0; padding:0; list-style:none; }
  li { display:flex; align-items:center; justify-content:space-between; gap:10px; min-height:34px; border-bottom:1px solid color-mix(in srgb, var(--foreground) 7%, transparent); }
  li:last-child { border-bottom:none; }
  .none { color:var(--muted-foreground); font-size:12px; }
  .name { min-width:0; font-size:14px; overflow-wrap:anywhere; }
  .tag { margin-left:8px; padding:1px 6px; border-radius:4px; font-size:11px; color:var(--signal); border:1px solid currentColor; }
  .tag.muted { color:var(--muted-foreground); border-color:var(--rule); }
  .runs li { display:block; padding:6px 0; }
  .log li { display:block; min-height:0; padding:3px 0; font-size:12px; overflow-wrap:anywhere; }
  .log .time { margin-right:8px; color:var(--muted-foreground); font-variant-numeric:tabular-nums; }
  .action, .run { display:inline-flex; justify-content:center; align-items:center; gap:8px; background:var(--button); color:var(--button-ink); font-weight:650; border-radius:7px; cursor:pointer; }
  .action { justify-self:start; min-height:38px; padding:0 16px; font-size:14px; }
  .run { min-height:28px; padding:0 12px; font-size:12px; }
  .action:disabled, .run:disabled { opacity:.45; cursor:default; }
  .ghost, .options, .link { display:inline-flex; align-items:center; gap:6px; color:var(--muted-foreground); font-size:12px; cursor:pointer; }
  .ghost :global(svg) { width:14px; height:14px; }
  .ghost:hover, .options:hover { color:var(--foreground); }
  .link { color:var(--signal); text-decoration:underline; margin-left:6px; }
  .message { padding:24px 0; color:var(--muted-foreground); font-size:13px; }
  .off { display:grid; justify-items:start; gap:14px; max-width:520px; padding:32px 0; }
  .off > :global(svg) { width:28px; height:28px; color:var(--signal); }
  .off h2 { font-size:20px; }
  .off p { color:var(--muted-foreground); line-height:1.65; font-size:14px; }
  footer { padding-top:14px; }
</style>
