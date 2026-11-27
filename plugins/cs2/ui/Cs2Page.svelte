<script lang="ts">
  // CS2 插件的 App（左侧栏「插件 → CS2」）：这一局（比分、回合、你的数据、炸弹）和游戏的配置文件（写在哪、有没有写上、开关）。
  // 数据只来自 Valve 官方的游戏状态接口（GSI）；没有风险说明（官方接口，没有“风险和许可”弹窗，ADR 043 D8）。
  import { onMount } from "svelte";
  import { Crosshair, FileCog } from "@lucide/svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { m } from "$lib/paraglide/messages.js";
  import type { PluginBridge, PluginPageProps } from "$lib/pluginUi";
  import { pluginEnabled, setPluginEnabled } from "$lib/plugins";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import BombCard from "./BombCard.svelte";
  import MatchCard from "./MatchCard.svelte";
  import { cs2Status, PAGE_CS2_BOMB, PAGE_CS2_MATCH, setWriteCfg, type Cs2Status } from "./cs2";

  let { plugin, embedded = false }: PluginPageProps & { embedded?: boolean } = $props();
  const on = $derived(pluginEnabled($settingsStore, plugin.manifest));
  let status = $state<Cs2Status | null>(null);
  let error = $state("");
  let busy = $state(false);
  let request = 0;
  // 页面里的卡片只显示，不调工具
  const bridge: PluginBridge = { call: () => Promise.reject(new Error("cs2")), openPage: () => {}, replace: () => {} };

  async function refresh() {
    const id = ++request;
    try {
      const next = await cs2Status();
      if (id !== request) return;
      status = next;
      error = "";
    } catch (e) {
      if (id === request) error = errText(e);
    }
  }
  $effect(() => {
    if (on) void refresh();
    else status = null;
    return () => { request++; };
  });
  onMount(() => {
    const timer = setInterval(() => { if (on && document.visibilityState === "visible") void refresh(); }, 1000);
    return () => clearInterval(timer);
  });

  async function toggleCfg(write: boolean) {
    if (busy) return;
    busy = true;
    try { status = await setWriteCfg(write); }
    catch (e) { toast(false, m.shell_toast_save_failed({ error: errText(e) })); }
    finally { busy = false; }
  }
  async function enable() {
    try { await setPluginEnabled(plugin.manifest.id, true); }
    catch (e) { toast(false, m.system_plugins_save_failed({ error: errText(e) })); }
  }

  const match = $derived(status?.match);
  const cfgState = $derived.by(() => {
    switch (status?.cfg.state) {
      case "written": return m.plugin_cs2_cfg_written();
      case "absent": return m.plugin_cs2_cfg_absent();
      case "wait_write": return m.plugin_cs2_cfg_wait_write();
      case "wait_remove": return m.plugin_cs2_cfg_wait_remove();
      case "foreign": return m.plugin_cs2_cfg_foreign();
      default: return "";
    }
  });
  // 写上了、游戏开着、还没收到数据：是游戏启动以后才写的，要重启游戏
  const needsRestart = $derived(!!status && status.cfg.state === "written" && status.gameRunning && !status.connected);
</script>

<div class="desk" class:embedded data-testid="plugin-page-cs2">
  <header>
    <h1><Crosshair aria-hidden="true" />{m.plugin_cs2_name()}</h1>
    {#if on}
      <span class="state" class:live={status?.connected} role="status" data-testid="cs2-connection" data-connected={status?.connected ? "true" : "false"}>
        <span class="lamp"></span>{status ? (status.connected ? m.plugin_cs2_status_connected() : m.plugin_cs2_status_waiting()) : m.plugin_cs2_status_reading()}
      </span>
    {/if}
  </header>

  {#if !on}
    <section class="off" data-testid="plugin-cs2-off">
      <Crosshair aria-hidden="true" /><h2>{m.plugin_cs2_off_title()}</h2>
      <p>{m.plugin_cs2_off_note()}</p>
      <button class="action" data-testid="plugin-cs2-enabled" onclick={enable}>{m.plugin_cs2_enable()}</button>
    </section>
  {:else}
    {#if error}<p class="message" role="alert">{error}</p>{/if}
    <section class="sheet" aria-label={m.plugin_cs2_section_match()} data-testid="cs2-this-match">
      <h2>{m.plugin_cs2_section_match()}</h2>
      {#if match?.playing}
        <MatchCard uri={PAGE_CS2_MATCH} params={{}} data={match} {bridge} />
        {#if match.bomb && !match.over}
          <div class="bomb"><BombCard uri={PAGE_CS2_BOMB} params={{}} data={match.bomb} {bridge} /></div>
        {/if}
      {:else if status?.connected}
        <p class="muted" data-testid="cs2-menu">{m.plugin_cs2_menu()}</p>
      {:else}
        <p class="muted" data-testid="cs2-idle">{m.plugin_cs2_idle()}</p>
      {/if}
      {#if !match?.playing && match?.lastMatch}
        <h3>{m.plugin_cs2_last_match()}</h3>
        <MatchCard uri={PAGE_CS2_MATCH} params={{}} data={match.lastMatch} {bridge} />
      {/if}
    </section>

    <section class="sheet" aria-label={m.plugin_cs2_section_cfg()} data-testid="cs2-cfg">
      <h2><FileCog aria-hidden="true" />{m.plugin_cs2_section_cfg()}</h2>
      {#if status}
        <ToggleField checked={status.cfg.writeCfg} disabled={busy || status.cfg.found !== "dir"} label={m.plugin_cs2_cfg_switch()}
          desc={m.plugin_cs2_cfg_desc({ port: status.listening ?? status.cfg.port })} testid="cs2-write-cfg" onchange={(v) => void toggleCfg(v)} />
        {#if status.cfg.found === "isolated"}
          <p class="muted" data-testid="cs2-cfg-isolated">{m.plugin_cs2_cfg_isolated()}</p>
        {:else if status.cfg.found === "not_installed"}
          <p class="muted" data-testid="cs2-cfg-missing">{m.plugin_cs2_cfg_not_installed()}</p>
        {:else}
          <p class="cfg-line"><span data-testid="cs2-cfg-state" data-state={status.cfg.state ?? ""}>{cfgState}</span>
            {#if status.cfg.path}<code data-testid="cs2-cfg-path">{status.cfg.path}</code>{/if}</p>
          {#if status.cfg.error}<p class="muted" role="alert">{m.plugin_cs2_cfg_error({ error: status.cfg.error })}</p>{/if}
          {#if needsRestart}<p class="muted" data-testid="cs2-cfg-restart">{m.plugin_cs2_cfg_restart()}</p>{/if}
        {/if}
        {#if status.listening == null && status.bindError}
          <p class="muted" role="alert">{m.plugin_cs2_listen_failed({ error: status.bindError })}</p>
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .desk { --signal: light-dark(#b45309, #fbbf24); --button: light-dark(#92400e, #fcd34d); --button-ink: light-dark(#fff, #2a1a05);
    --rule: color-mix(in srgb, var(--foreground) 14%, transparent);
    container-type: inline-size; box-sizing: border-box; width: 100%; max-width: 880px; margin: 0 auto;
    padding: clamp(20px,4.7vw,52px) clamp(18px,5.2vw,56px); color: var(--foreground); }
  .desk.embedded { padding: 10px 4px 18px; }
  header { display:flex; flex-wrap:wrap; align-items:center; justify-content:space-between; gap:12px; padding-bottom:20px; border-bottom:1px solid var(--rule); }
  h1, h2, h3, p { margin:0; }
  h1 { display:flex; align-items:center; gap:12px; font-size:clamp(24px,4cqw,34px); font-weight:680; letter-spacing:-.04em; }
  h1 :global(svg) { width:1em; height:1em; color:var(--signal); stroke-width:1.7; }
  .state { display:inline-flex; align-items:center; gap:7px; color:var(--muted-foreground); font-size:12px; }
  .lamp { width:7px; height:7px; border:1px solid currentColor; border-radius:50%; }
  .state.live { color:var(--signal); }
  .state.live .lamp { background:currentColor; }
  .sheet { display:grid; gap:12px; padding:20px 0; border-bottom:1px solid var(--rule); }
  .sheet h2 { display:flex; align-items:center; gap:8px; font-size:14px; font-weight:650; }
  .sheet h2 :global(svg) { width:16px; height:16px; color:var(--signal); }
  .sheet h3 { margin-top:6px; font-size:12px; font-weight:600; color:var(--muted-foreground); }
  .bomb { border-left:3px solid var(--signal); padding-left:12px; }
  .muted, .message { color:var(--muted-foreground); font-size:12px; line-height:1.6; }
  .cfg-line { display:flex; flex-wrap:wrap; align-items:baseline; gap:8px; font-size:12px; }
  .cfg-line code { font-size:11px; color:var(--muted-foreground); overflow-wrap:anywhere; }
  .action { display:inline-flex; align-items:center; min-height:40px; padding:0 16px; background:var(--button); color:var(--button-ink);
    font-size:14px; font-weight:650; border-radius:7px; cursor:pointer; }
  .off { display:grid; justify-items:start; gap:14px; max-width:480px; padding:32px 0; }
  .off > :global(svg) { width:28px; height:28px; color:var(--signal); }
  .off h2 { font-size:20px; font-weight:650; }
  .off p { color:var(--muted-foreground); line-height:1.65; font-size:14px; }
</style>
