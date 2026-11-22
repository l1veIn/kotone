<script lang="ts">
  // 「琴音陪玩 → 设置 → 语音唤醒」（ADR 017）：开关、唤醒模型下载、唤醒词、灵敏度、应答方式。
  // 设置在 config.json `assistant.wake`；状态与下载走 wake_status / wake_download_model。
  import { onMount } from "svelte";
  import { Download, LoaderCircle, Mic } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import {
    subscribeWake, subscribeWakeModel, wakeAcks, wakeDownloadModel, wakePhrases, wakeSensitivities,
    wakeStateText, wakeStatus, type WakeModelProgress, type WakeStatus,
  } from "../../../../lib/wake";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import type { WakeConfig } from "../../../../lib/ipc";
  import { DEFAULT_ROLE, rolesList, type RoleInfo } from "../../../../lib/roles";
  import { DEFAULT_COMPANION_NAME, companionName } from "$lib/stores/companion";
  import { m } from "$lib/paraglide/messages.js";

  const wake = $derived<WakeConfig>($settingsStore?.assistant?.wake ?? {
    enabled: false, phrase: "qinyin-qinyin", sensitivity: "normal", ack: "voice", noSpeechTimeoutMs: 5000,
  });
  let status = $state<WakeStatus | null>(null);
  let progress = $state<WakeModelProgress | null>(null);
  let downloading = $state(false);
  // i18n-ignore-start：唤醒词是玩家要说的话
  const phraseLabel = $derived(wakePhrases.find((p) => p.value === wake.phrase)?.label ?? "琴音琴音");
  // i18n-ignore-end
  // 唤醒词跟着角色走：只有没改过名字和唤醒词的琴音用这里的预置唤醒词，其他角色用自己在「角色」里设的。
  let role = $state<RoleInfo | null>(null);
  const roleKey = $derived(`${$settingsStore?.agent?.roleId ?? ""}|${$companionName}|${wake.phrase}`);
  $effect(() => {
    void roleKey;
    void rolesList().then((list) => { role = list.roles.find((r) => r.current) ?? null; }).catch(() => {});
  });
  const usesPreset = $derived(!role || (role.id === DEFAULT_ROLE && !role.wake.text.trim() && role.name === DEFAULT_COMPANION_NAME));
  const word = $derived(usesPreset || !role ? phraseLabel : role.wakeCheck.text);
  const percent = $derived(progress?.totalBytes ? Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100)) : null);

  onMount(() => {
    let disposed = false;
    const unlisten: (() => void)[] = [];
    void wakeStatus(wake.enabled).then((s) => { if (!disposed) status = s; }).catch(() => {});
    void subscribeWake((s) => { status = s; }).then((u) => { if (disposed) u(); else unlisten.push(u); });
    void subscribeWakeModel((p) => { progress = p; }).then((u) => { if (disposed) u(); else unlisten.push(u); });
    return () => { disposed = true; unlisten.forEach((u) => u()); };
  });

  async function save(patch: Partial<WakeConfig>, message: string) {
    await patchSettings({ assistant: { wake: { ...wake, ...patch } } }, message);
    status = await wakeStatus({ ...wake, ...patch }.enabled).catch(() => status);
  }

  async function download() {
    if (downloading) return;
    downloading = true;
    progress = null;
    try {
      status = await wakeDownloadModel(wake.enabled);
      toast(true, m.companion_wake_model_downloaded());
    } catch (error) {
      toast(false, m.companion_wake_model_download_failed({ error: errText(error) }));
    } finally {
      downloading = false;
    }
  }
</script>

<SettingsSection title={m.companion_wake_title()} description={m.companion_wake_desc({ word, name: $companionName })} id="wake-settings">
  <div data-testid="wake-settings" class="contents">
    <SettingsRow label={m.companion_wake_enabled_label()} description={m.companion_wake_enabled_desc()}>
      {#snippet control()}
        <Switch data-testid="wake-enabled" bind:checked={() => wake.enabled, (checked) => void save({ enabled: checked }, checked ? m.companion_wake_enabled_on() : m.companion_wake_enabled_off())} aria-label={m.companion_wake_enabled_label()} />
      {/snippet}
      {#if status}
        <p class="m-0 flex items-center gap-1.5 text-xs {status.state === 'listening' ? 'text-success' : status.state === 'error' ? 'text-destructive' : 'text-muted-foreground'}" data-testid="wake-status" role="status">
          <Mic class="size-3.5 shrink-0" />{wakeStateText(status)}{status.state === "listening" && status.detections ? m.companion_wake_detections({ count: status.detections }) : ""}
        </p>
      {/if}
    </SettingsRow>
    {#if status && !status.modelReady}
      <SettingsRow label={m.companion_wake_model_label()} description={m.companion_wake_model_desc()}>
        {#snippet control()}
          <Button size="sm" variant="outline" data-testid="wake-download" disabled={downloading || status?.downloading} onclick={() => void download()}>
            {#if downloading || status?.downloading}<LoaderCircle class="size-3.5 animate-spin" />{percent !== null ? `${percent}%` : m.companion_wake_model_downloading()}{:else}<Download class="size-3.5" />{m.companion_wake_model_download()}{/if}
          </Button>
        {/snippet}
      </SettingsRow>
    {/if}
    {#if usesPreset}
      <SettingsRow label={m.companion_wake_phrase_label()} description={m.companion_wake_phrase_desc()}>
        {#snippet control()}
          <div class="w-44 max-w-full">
            <SelectField ariaLabel={m.companion_wake_phrase_label()} value={wake.phrase} options={[...wakePhrases]}
              onchange={(phrase) => void save({ phrase: phrase as WakeConfig["phrase"] }, m.companion_wake_phrase_saved({ phrase: String(wakePhrases.find((p) => p.value === phrase)?.label) }))} />
          </div>
        {/snippet}
      </SettingsRow>
    {:else}
      <SettingsRow label={m.companion_wake_phrase_label()} description={m.companion_wake_role_phrase_desc({ name: $companionName })}>
        {#snippet control()}
          <span class="text-sm font-medium" data-testid="wake-role-word">{word}</span>
        {/snippet}
      </SettingsRow>
    {/if}
    <SettingsRow label={m.companion_wake_sensitivity_label()} description={m.companion_wake_sensitivity_desc()}>
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel={m.companion_wake_sensitivity_aria()} value={wake.sensitivity} options={[...wakeSensitivities]}
            onchange={(sensitivity) => void save({ sensitivity: sensitivity as WakeConfig["sensitivity"] }, m.companion_wake_sensitivity_saved())} />
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_wake_ack_label()} description={m.companion_wake_ack_desc()}>
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel={m.companion_wake_ack_label()} value={wake.ack} options={[...wakeAcks]}
            onchange={(ack) => void save({ ack: ack as WakeConfig["ack"] }, m.companion_wake_ack_saved())} />
        </div>
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
