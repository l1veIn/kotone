<script lang="ts">
  import { onMount } from "svelte";
  import { save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    clearDiagnostics,
    exportDiagnostics,
    getAutostartEnabled,
    getElevationStatus,
    getHotkeyStatus,
    getSettings,
    isTauri,
    restartAsAdmin,
    setAutostartEnabled,
    type DiagnosticWindow,
    type ElevationStatus,
    type HotkeyStatus,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { startPageOf, usesLegacyVoiceInput } from "../../../../lib/ipc/contracts";
  import { companionName } from "$lib/stores/companion";
  import { updateSettings } from "../../../../lib/ipc";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import { Download } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenOnboarding }: { onOpenOnboarding: () => void } = $props();

  const ADMIN_RESTART_FLAG = "kotone:admin-restart-pending";

  let elevation = $state<ElevationStatus | null>(null);
  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  let restartingAsAdmin = $state(false);
  let exportingDiagnostics = $state(false);
  let exportWindow = $state<DiagnosticWindow>("24h");
  let clearingDiagnostics = $state(false);
  let confirmClear = $state(false);
  let stopRecordingOnClear = $state(false);
  /** 开机自启真实状态（注册表）；null = 读取中 */
  let autostartEnabled = $state<boolean | null>(null);

  onMount(async () => {
    [elevation, hotkeyStatus, autostartEnabled] = await Promise.all([
      getElevationStatus().catch(() => null),
      getHotkeyStatus().catch(() => null),
      getAutostartEnabled().catch(() => null),
    ]);
    if (localStorage.getItem(ADMIN_RESTART_FLAG)) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      if (elevation?.elevated) toast(true, m.system_compat_now_elevated());
    }
  });

  /** 开机自启开关：写注册表后回读真值（用户在任务管理器禁用后开关也会回到关闭） */
  async function onAutostartChange(enabled: boolean) {
    try {
      await setAutostartEnabled(enabled);
      autostartEnabled = await getAutostartEnabled().catch(() => enabled);
      toast(true, enabled ? m.system_general_autostart_on() : m.system_general_autostart_off());
    } catch (error) {
      autostartEnabled = await getAutostartEnabled().catch(() => autostartEnabled ?? false);
      toast(false, m.system_general_autostart_failed({ error: errText(error) }));
    }
  }

  async function onHotkeyBackendChange(hotkeyBackend: string) {
    if (hotkeyBackend !== "auto" && hotkeyBackend !== "llhook" && hotkeyBackend !== "register") return;
    await patchSettings({ hotkeyBackend }, m.system_compat_hotkey_backend_saved());
    hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
  }

  async function onExportDiagnostics() {
    if (exportingDiagnostics) return;
    exportingDiagnostics = true;
    try {
      const defaultName = `kotone-diagnostics-${new Date().toISOString().slice(0, 10)}.zip`;
      const path = isTauri
        ? await saveDialog({
            title: m.system_diagnostics_save_dialog_title(),
            defaultPath: defaultName,
            filters: [{ name: m.system_diagnostics_save_dialog_filter(), extensions: ["zip"] }],
          })
        : defaultName;
      if (!path) return;
      const result = await exportDiagnostics(path, exportWindow);
      const windowLabel =
        result.window === "24h" ? m.system_diagnostics_window_24h() : result.window === "7d" ? m.system_diagnostics_window_7d() : m.system_diagnostics_window_all();
      toast(true, m.system_diagnostics_exported({ range: windowLabel, reportId: result.reportId }));
    } catch (error) {
      toast(false, m.system_diagnostics_export_failed({ error: errText(error) }));
    } finally {
      exportingDiagnostics = false;
    }
  }

  async function onClearDiagnostics() {
    if (clearingDiagnostics) return;
    clearingDiagnostics = true;
    try {
      await clearDiagnostics(stopRecordingOnClear);
      if (stopRecordingOnClear) {
        settingsStore.set(await getSettings());
      }
      confirmClear = false;
      stopRecordingOnClear = false;
      toast(true, m.system_diagnostics_cleared());
    } catch (error) {
      toast(false, m.system_diagnostics_clear_failed({ error: errText(error) }));
    } finally {
      clearingDiagnostics = false;
    }
  }

  async function onRestartAsAdmin() {
    restartingAsAdmin = true;
    try {
      localStorage.setItem(ADMIN_RESTART_FLAG, "1");
      await restartAsAdmin();
    } catch (error) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      restartingAsAdmin = false;
      toast(false, errText(error));
    }
  }
</script>

{#if $settingsStore}
  <div class="space-y-6">
  <SettingsSection title={m.system_general_title()}>
    <SettingsRow label={m.system_general_language()} description={m.system_general_language_description()}>
      {#snippet control()}
        <SelectField
          class="w-32"
          ariaLabel={m.system_general_language()}
          value={$settingsStore.language}
          options={[{ value: "zh", label: m.system_general_language_zh() }]}
          onchange={(language) => void patchSettings({ language }, m.system_general_language_saved())}
        />
      {/snippet}
    </SettingsRow>
    <div id="system-start-page" class="scroll-mt-16">
      <SettingsRow label={m.system_general_start_page()} description={m.system_general_start_page_description()}>
        {#snippet control()}
          <SelectField
            class="w-32"
            ariaLabel={m.system_general_start_page()}
            testid="start-page-select"
            value={startPageOf($settingsStore.ui)}
            options={[{ value: "chat", label: m.system_general_start_page_chat() }, { value: "voiceInput", label: m.system_general_start_page_voice_input() }]}
            onchange={(startPage) => void patchSettings({ ui: { startPage } }, startPage === "voiceInput" ? m.system_general_start_page_voice_input_saved() : m.system_general_start_page_chat_saved())}
          />
        {/snippet}
      </SettingsRow>
    </div>
    <SettingsRow label={m.system_general_onboarding()} description={m.system_general_onboarding_description()}>
      {#snippet control()}
        <Button variant="outline" size="sm" data-testid="reopen-onboarding" onclick={onOpenOnboarding}>{m.system_general_onboarding_button()}</Button>
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField
        checked={autostartEnabled ?? false}
        disabled={autostartEnabled === null}
        label={m.system_general_autostart()}
        desc={m.system_general_autostart_description()}
        onchange={(v) => void onAutostartChange(v)}
      />
    </div>
  </SettingsSection>

  {#if !usesLegacyVoiceInput($settingsStore.ui)}
    <div id="system-stop" class="scroll-mt-16">
      <SettingsSection title={m.system_stop_title()} testid="system-stop"
        description={m.system_stop_description({ name: $companionName })}>
        <SettingsRow label={m.system_stop_hotkey()} description={m.system_stop_hotkey_description()} stacked testid="hotkey-section-automation-stop">
          {#snippet control()}
            <HotkeyField id="automation-stop" title={m.system_stop_hotkey()} value={$settingsStore.automationStopHotkey ?? ""} placeholder={m.system_stop_hotkey_placeholder()} compact
              onSave={async (key) => { settingsStore.set(await updateSettings({ automationStopHotkey: key })); }} />
          {/snippet}
        </SettingsRow>
      </SettingsSection>
    </div>
  {/if}

  <SettingsSection title={m.system_compat_title()}>
    <SettingsRow label={m.system_compat_hotkey_backend()} for="advanced-hotkey-backend">
      {#snippet description()}
        {m.system_compat_hotkey_backend_description({
          current: hotkeyStatus?.backend === "llhook"
            ? m.system_compat_backend_llhook_short()
            : hotkeyStatus?.backend === "register"
              ? m.system_compat_backend_register()
              : m.system_compat_backend_unregistered(),
        })}
      {/snippet}
      {#snippet control()}
        <SelectField
          class="w-44"
          id="advanced-hotkey-backend"
          ariaLabel={m.system_compat_hotkey_backend()}
          value={$settingsStore.hotkeyBackend}
          options={[
            { value: "auto", label: m.system_compat_backend_auto() },
            { value: "llhook", label: m.system_compat_backend_llhook() },
            { value: "register", label: m.system_compat_backend_register() },
          ]}
          onchange={(backend) => void onHotkeyBackendChange(backend)}
        />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.system_compat_elevation()} description={elevation === null ? m.system_compat_elevation_checking() : elevation.elevated ? m.system_compat_elevation_admin() : m.system_compat_elevation_user()}>
      {#snippet control()}
        {#if elevation && !elevation.elevated}
          <Button variant="outline" size="sm" disabled={restartingAsAdmin} onclick={() => void onRestartAsAdmin()}>
            {restartingAsAdmin ? m.system_compat_restarting() : m.system_compat_restart_as_admin()}
          </Button>
        {/if}
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField
        checked={$settingsStore.runAsAdminOnStart}
        label={m.system_compat_auto_admin()}
        desc={m.system_compat_auto_admin_description()}
        onchange={(value) =>
          void patchSettings(
            {
              runAsAdminOnStart: value,
              ...(value ? { adminPromptDismissed: true, autoAdminPromptDismissed: true } : {}),
            },
            value ? m.system_compat_auto_admin_on() : m.system_compat_auto_admin_off(),
          )}
      />
    </div>
  </SettingsSection>

  <SettingsSection
    title={m.system_diagnostics_title()}
    description={m.system_diagnostics_description()}
  >
    <div class="py-3.5">
      <ToggleField
        checked={$settingsStore.diagnostics.recording}
        label={m.system_diagnostics_recording()}
        desc={m.system_diagnostics_recording_description()}
        onchange={(value) =>
          void patchSettings(
            { diagnostics: { recording: value } },
            value ? m.system_diagnostics_recording_on() : m.system_diagnostics_recording_off(),
          )}
      />
    </div>
    <div class="flex flex-wrap items-center gap-2 py-3.5">
      <SelectField
        class="w-36"
        testid="diagnostics-window"
        ariaLabel={m.system_diagnostics_window_label()}
        value={exportWindow}
        options={[
          { value: "24h", label: m.system_diagnostics_window_24h() },
          { value: "7d", label: m.system_diagnostics_window_7d() },
          { value: "all", label: m.system_diagnostics_window_all() },
        ]}
        onchange={(value) => (exportWindow = value as DiagnosticWindow)}
      />
      <Button size="sm" data-testid="diagnostics-export" disabled={exportingDiagnostics} onclick={() => void onExportDiagnostics()}>
        <Download class="size-3.5" />
        {exportingDiagnostics ? m.system_diagnostics_exporting() : m.system_diagnostics_export_button()}
      </Button>
      <Button
        variant="outline"
        size="sm"
        data-testid="diagnostics-clear"
        onclick={() => {
          confirmClear = true;
          stopRecordingOnClear = false;
        }}
      >
        {m.system_diagnostics_clear_button()}
      </Button>
    </div>
  </SettingsSection>
  </div>
{/if}

<Dialog.Root open={confirmClear} onOpenChange={(open) => { if (!open && !clearingDiagnostics) confirmClear = false; }}>
  <Dialog.Content class="ui-scope" data-testid="diagnostics-clear-dialog">
    <Dialog.Header>
      <Dialog.Title>{m.system_diagnostics_clear_dialog_title()}</Dialog.Title>
      <Dialog.Description>{m.system_diagnostics_clear_dialog_description()}</Dialog.Description>
    </Dialog.Header>
    <label class="flex cursor-pointer items-start gap-2 text-[13px] text-foreground">
      <input
        data-testid="diagnostics-stop-recording"
        type="checkbox"
        class="mt-0.5 accent-[var(--color-primary)]"
        bind:checked={stopRecordingOnClear}
      />
      <span>{m.system_diagnostics_clear_stop_recording()}</span>
    </label>
    <Dialog.Footer>
      <Button variant="outline" size="sm" onclick={() => (confirmClear = false)}>{m.system_common_cancel()}</Button>
      <Button variant="destructive" size="sm" data-testid="diagnostics-clear-confirm" disabled={clearingDiagnostics} onclick={() => void onClearDiagnostics()}>
        {clearingDiagnostics ? m.system_diagnostics_clearing() : m.system_diagnostics_clear_confirm()}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
