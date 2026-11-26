<script lang="ts">
  // “操作客户端”的偏好（ADR 038 第 3.5 步，照 LeagueAkari）：自动接受对局、自动选 / 禁英雄（按排好的顺序）。
  // 数据开关 `control` 关着（默认）时只写一句“先打开上面的开关”；开关和说明在 PluginSwitches 里，风险提示在插件的
  // “风险和许可”弹窗里（ADR 043 D8：第一次打开它时弹一次）。
  // 偏好存在设置 plugins.lol.settings，改了马上生效（后端每 2 秒看一次客户端）。
  import { onMount } from "svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { Input } from "$lib/components/ui/input";
  import { pluginCall, sourceEnabled, type PluginInfo } from "$lib/plugins";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import ChampionOrder from "./ChampionOrder.svelte";
  import { ACCEPT_DELAY_MAX, SELECT_DELAY_MAX, controlPrefs, saveControl } from "./control";
  import type { StatChampion } from "./types";

  let { plugin }: { plugin: PluginInfo } = $props();
  const on = $derived(sourceEnabled($settingsStore, plugin.manifest, "control"));
  const clientOn = $derived(sourceEnabled($settingsStore, plugin.manifest, "client"));
  const prefs = $derived(controlPrefs($settingsStore));

  let champions = $state<StatChampion[]>([]);
  onMount(async () => {
    try {
      const result = await pluginCall(plugin.manifest.id, "lol.champions");
      const data = result.structuredContent as { champions?: StatChampion[] } | undefined;
      champions = data?.champions ?? [];
    } catch { /* 名字拿不到时显示编号 */ }
  });

  async function save(patch: Record<string, unknown>) {
    try { await saveControl(patch); } catch (error) { toast(false, m.system_plugins_save_failed({ error: errText(error) })); }
  }
  const clamp = (raw: string, max: number) => Math.min(Math.max(Math.round(Number(raw) || 0), 0), max);
</script>

{#snippet delay(value: number, max: number, testid: string, disabled: boolean, onchange: (v: number) => void)}
  <label class="flex items-center gap-2 text-xs text-muted-foreground">
    <span>{m.plugin_lol_control_delay()}</span>
    <Input type="number" min="0" max={max} value={String(value)} class="h-7 w-16 text-xs" {disabled} data-testid={testid}
      onchange={(e) => onchange(clamp((e.currentTarget as HTMLInputElement).value, max))} />
    <span>{m.plugin_lol_control_seconds({ max })}</span>
  </label>
{/snippet}

{#if !on}
  <p class="m-0 rounded-xl border border-hairline bg-card/40 px-4 py-3 text-xs text-muted-foreground" data-testid="lol-control-off">
    {m.plugin_lol_control_off()}
  </p>
{:else}
  <div class="space-y-4 rounded-2xl border border-hairline bg-card/70 px-4 py-4" data-testid="lol-control">
    {#if !clientOn}
      <p class="m-0 text-xs text-amber-700 dark:text-amber-200/90" data-testid="lol-control-needs-client">{m.plugin_lol_control_needs_client()}</p>
    {/if}
    <div class="space-y-2">
      <ToggleField checked={prefs.autoAccept} label={m.plugin_lol_control_accept()} desc={m.plugin_lol_control_accept_desc()}
        testid="lol-control-auto-accept" onchange={(v) => void save({ autoAccept: v })} />
      {@render delay(prefs.acceptDelaySeconds, ACCEPT_DELAY_MAX, "lol-control-accept-delay", !prefs.autoAccept,
        (v) => void save({ acceptDelaySeconds: v }))}
    </div>

    <div class="space-y-2 border-t border-hairline pt-4" data-testid="lol-control-pick">
      <ToggleField checked={prefs.pick.enabled} label={m.plugin_lol_control_pick()} desc={m.plugin_lol_control_pick_desc()}
        testid="lol-control-pick-enabled" onchange={(v) => void save({ pick: { enabled: v } })} />
      <ChampionOrder ids={prefs.pick.champions} {champions} disabled={!prefs.pick.enabled} testid="lol-control-pick-champions"
        onchange={(ids) => void save({ pick: { champions: ids } })} />
      <ToggleField checked={prefs.pick.lockIn} label={m.plugin_lol_control_lock()} desc={m.plugin_lol_control_lock_desc()}
        disabled={!prefs.pick.enabled} testid="lol-control-pick-lock" onchange={(v) => void save({ pick: { lockIn: v } })} />
      {@render delay(prefs.pick.delaySeconds, SELECT_DELAY_MAX, "lol-control-pick-delay", !prefs.pick.enabled,
        (v) => void save({ pick: { delaySeconds: v } }))}
    </div>

    <div class="space-y-2 border-t border-hairline pt-4" data-testid="lol-control-ban">
      <ToggleField checked={prefs.ban.enabled} label={m.plugin_lol_control_ban()} desc={m.plugin_lol_control_ban_desc()}
        testid="lol-control-ban-enabled" onchange={(v) => void save({ ban: { enabled: v } })} />
      <ChampionOrder ids={prefs.ban.champions} {champions} disabled={!prefs.ban.enabled} testid="lol-control-ban-champions"
        onchange={(ids) => void save({ ban: { champions: ids } })} />
      {@render delay(prefs.ban.delaySeconds, SELECT_DELAY_MAX, "lol-control-ban-delay", !prefs.ban.enabled,
        (v) => void save({ ban: { delaySeconds: v } }))}
    </div>

    <p class="m-0 border-t border-hairline pt-3 text-xs leading-relaxed text-muted-foreground" data-testid="lol-control-note">
      {m.plugin_lol_control_note()}
    </p>
  </div>
{/if}
