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
      if (elevation?.elevated) toast(true, "已通过管理员权限运行");
    }
  });

  /** 开机自启开关：写注册表后回读真值（用户在任务管理器禁用后开关也会回到关闭） */
  async function onAutostartChange(enabled: boolean) {
    try {
      await setAutostartEnabled(enabled);
      autostartEnabled = await getAutostartEnabled().catch(() => enabled);
      toast(true, enabled ? "已开启开机自动启动" : "已关闭开机自动启动");
    } catch (error) {
      autostartEnabled = await getAutostartEnabled().catch(() => autostartEnabled ?? false);
      toast(false, `开机自启设置失败：${errText(error)}`);
    }
  }

  async function onHotkeyBackendChange(hotkeyBackend: string) {
    if (hotkeyBackend !== "auto" && hotkeyBackend !== "llhook" && hotkeyBackend !== "register") return;
    await patchSettings({ hotkeyBackend }, "热键兼容模式已更新");
    hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
  }

  async function onExportDiagnostics() {
    if (exportingDiagnostics) return;
    exportingDiagnostics = true;
    try {
      const defaultName = `kotone-diagnostics-${new Date().toISOString().slice(0, 10)}.zip`;
      const path = isTauri
        ? await saveDialog({
            title: "导出 Kotone 诊断包",
            defaultPath: defaultName,
            filters: [{ name: "ZIP 诊断包", extensions: ["zip"] }],
          })
        : defaultName;
      if (!path) return;
      const result = await exportDiagnostics(path, exportWindow);
      const windowLabel =
        result.window === "24h" ? "最近 24 小时" : result.window === "7d" ? "最近 7 天" : "全部";
      toast(true, `诊断包已导出（${windowLabel}）：${result.reportId}`);
    } catch (error) {
      toast(false, `导出诊断包失败：${errText(error)}`);
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
      toast(true, "本地诊断信息已清除");
    } catch (error) {
      toast(false, `清除诊断信息失败：${errText(error)}`);
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
  <SettingsSection title="通用">
    <SettingsRow label="语言" description="界面语言。更多语言即将到来。">
      {#snippet control()}
        <SelectField
          class="w-32"
          ariaLabel="语言"
          value={$settingsStore.language}
          options={[{ value: "zh", label: "中文" }]}
          onchange={(language) => void patchSettings({ language }, "语言已切换")}
        />
      {/snippet}
    </SettingsRow>
    <div id="system-start-page" class="scroll-mt-16">
      <SettingsRow label="打开软件时显示" description="打开 Kotone 时先看到哪一页。">
        {#snippet control()}
          <SelectField
            class="w-32"
            ariaLabel="打开软件时显示"
            testid="start-page-select"
            value={startPageOf($settingsStore.ui)}
            options={[{ value: "chat", label: "聊天" }, { value: "voiceInput", label: "语音输入" }]}
            onchange={(startPage) => void patchSettings({ ui: { startPage } }, startPage === "voiceInput" ? "打开时显示语音输入" : "打开时显示聊天")}
          />
        {/snippet}
      </SettingsRow>
    </div>
    <SettingsRow label="引导" description="重新看一遍引导：添加 DeepSeek Key、设呼叫键。">
      {#snippet control()}
        <Button variant="outline" size="sm" data-testid="reopen-onboarding" onclick={onOpenOnboarding}>重新打开引导</Button>
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField
        checked={autostartEnabled ?? false}
        disabled={autostartEnabled === null}
        label="开机自动启动"
        desc="登录 Windows 后自动打开 Kotone（仅当前用户）· 与系统启动项实时同步"
        onchange={(v) => void onAutostartChange(v)}
      />
    </div>
  </SettingsSection>

  {#if !usesLegacyVoiceInput($settingsStore.ui)}
    <div id="system-stop" class="scroll-mt-16">
      <SettingsSection title="停下" testid="system-stop"
        description={`停止${$companionName}正在做的事和所有自动化。四种方式：她操作游戏时按 Esc、停止热键、说“停下”、托盘菜单「停下」。`}>
        <SettingsRow label="停止热键" description="Kotone 没在听的时候也有效；自动化按住的键会一起松开。" stacked testid="hotkey-section-automation-stop">
          {#snippet control()}
            <HotkeyField id="automation-stop" title="停止热键" value={$settingsStore.automationStopHotkey ?? ""} placeholder="如 Ctrl+Shift+F12" compact
              onSave={async (key) => { settingsStore.set(await updateSettings({ automationStopHotkey: key })); }} />
          {/snippet}
        </SettingsRow>
      </SettingsSection>
    </div>
  {/if}

  <SettingsSection title="Windows 兼容性">
    <SettingsRow label="热键实现" for="advanced-hotkey-backend">
      {#snippet description()}
        除非特定游戏收不到热键，否则保持“自动”。当前：{hotkeyStatus?.backend === "llhook"
          ? "低层钩子"
          : hotkeyStatus?.backend === "register"
            ? "系统热键"
            : "未注册"}
      {/snippet}
      {#snippet control()}
        <SelectField
          class="w-44"
          id="advanced-hotkey-backend"
          ariaLabel="热键实现"
          value={$settingsStore.hotkeyBackend}
          options={[
            { value: "auto", label: "自动（推荐）" },
            { value: "llhook", label: "低层键鼠钩子" },
            { value: "register", label: "系统热键" },
          ]}
          onchange={(backend) => void onHotkeyBackendChange(backend)}
        />
      {/snippet}
    </SettingsRow>
    <SettingsRow label="当前进程权限" description={elevation === null ? "检测中…" : elevation.elevated ? "管理员" : "普通用户"}>
      {#snippet control()}
        {#if elevation && !elevation.elevated}
          <Button variant="outline" size="sm" disabled={restartingAsAdmin} onclick={() => void onRestartAsAdmin()}>
            {restartingAsAdmin ? "正在重启…" : "以管理员身份重启"}
          </Button>
        {/if}
      {/snippet}
    </SettingsRow>
    <div class="py-3.5">
      <ToggleField
        checked={$settingsStore.runAsAdminOnStart}
        label="启动时自动请求管理员权限"
        desc="开启后每次启动都会自动发起 Windows UAC 确认；普通桌面应用不能静默获得管理员权限"
        onchange={(value) =>
          void patchSettings(
            {
              runAsAdminOnStart: value,
              ...(value ? { adminPromptDismissed: true, autoAdminPromptDismissed: true } : {}),
            },
            value ? "已开启启动时自动权限请求" : "已关闭启动时自动权限请求",
          )}
      />
    </div>
  </SettingsSection>

  <SettingsSection
    title="诊断包"
    description="不包含录音、识别文本和热词，可安全分享给测试群管理员。默认记录流程事件，便于排查无法复现的热键/全屏问题。"
  >
    <div class="py-3.5">
      <ToggleField
        checked={$settingsStore.diagnostics.recording}
        label="记录诊断信息"
        desc="写入流程事件（热键、悬浮窗显隐、注入结果）。关闭后不再落盘，已有文件需手动清除"
        onchange={(value) =>
          void patchSettings(
            { diagnostics: { recording: value } },
            value ? "已开启诊断记录" : "已关闭诊断记录",
          )}
      />
    </div>
    <div class="flex flex-wrap items-center gap-2 py-3.5">
      <SelectField
        class="w-36"
        testid="diagnostics-window"
        ariaLabel="导出时间范围"
        value={exportWindow}
        options={[
          { value: "24h", label: "最近 24 小时" },
          { value: "7d", label: "最近 7 天" },
          { value: "all", label: "全部" },
        ]}
        onchange={(value) => (exportWindow = value as DiagnosticWindow)}
      />
      <Button size="sm" data-testid="diagnostics-export" disabled={exportingDiagnostics} onclick={() => void onExportDiagnostics()}>
        <Download class="size-3.5" />
        {exportingDiagnostics ? "正在生成…" : "导出诊断包"}
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
        清除本地诊断信息
      </Button>
    </div>
  </SettingsSection>
  </div>
{/if}

<Dialog.Root open={confirmClear} onOpenChange={(open) => { if (!open && !clearingDiagnostics) confirmClear = false; }}>
  <Dialog.Content class="ui-scope" data-testid="diagnostics-clear-dialog">
    <Dialog.Header>
      <Dialog.Title>清除本地诊断信息？</Dialog.Title>
      <Dialog.Description>会删除流程事件和 kotone.log，不影响识别历史、设置和录音。此操作不可恢复。</Dialog.Description>
    </Dialog.Header>
    <label class="flex cursor-pointer items-start gap-2 text-[13px] text-foreground">
      <input
        data-testid="diagnostics-stop-recording"
        type="checkbox"
        class="mt-0.5 accent-[var(--color-primary)]"
        bind:checked={stopRecordingOnClear}
      />
      <span>以后不再记录诊断信息</span>
    </label>
    <Dialog.Footer>
      <Button variant="outline" size="sm" onclick={() => (confirmClear = false)}>取消</Button>
      <Button variant="destructive" size="sm" data-testid="diagnostics-clear-confirm" disabled={clearingDiagnostics} onclick={() => void onClearDiagnostics()}>
        {clearingDiagnostics ? "正在清除…" : "确认清除"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
