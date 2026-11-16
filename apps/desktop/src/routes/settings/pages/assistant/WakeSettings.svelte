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

  const wake = $derived<WakeConfig>($settingsStore?.assistant?.wake ?? {
    enabled: false, phrase: "qinyin-qinyin", sensitivity: "normal", ack: "voice", noSpeechTimeoutMs: 5000,
  });
  let status = $state<WakeStatus | null>(null);
  let progress = $state<WakeModelProgress | null>(null);
  let downloading = $state(false);
  const phraseLabel = $derived(wakePhrases.find((p) => p.value === wake.phrase)?.label ?? "琴音琴音");
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
      toast(true, "唤醒模型已下载");
    } catch (error) {
      toast(false, `唤醒模型下载失败：${errText(error)}`);
    } finally {
      downloading = false;
    }
  }
</script>

<SettingsSection title="语音唤醒" description={`不用按键：说“${phraseLabel}”叫醒琴音，接着说要做的事，回答用语音播出。`} id="wake-settings">
  <div data-testid="wake-settings" class="contents">
    <SettingsRow label="开启语音唤醒" description="开启后麦克风常开（Windows 任务栏会一直显示麦克风在用），唤醒词只在本机检测，不上传。建议戴耳机，外放时游戏声音可能误唤醒。">
      {#snippet control()}
        <Switch data-testid="wake-enabled" bind:checked={() => wake.enabled, (checked) => void save({ enabled: checked }, checked ? "语音唤醒已开启" : "语音唤醒已关闭")} aria-label="开启语音唤醒" />
      {/snippet}
      {#if status}
        <p class="m-0 flex items-center gap-1.5 text-xs {status.state === 'listening' ? 'text-success' : status.state === 'error' ? 'text-destructive' : 'text-muted-foreground'}" data-testid="wake-status" role="status">
          <Mic class="size-3.5 shrink-0" />{wakeStateText(status)}{status.state === "listening" && status.detections ? `（已唤醒 ${status.detections} 次）` : ""}
        </p>
      {/if}
    </SettingsRow>
    {#if status && !status.modelReady}
      <SettingsRow label="唤醒模型" description="约 5 MB，从 sherpa-onnx 官方发布页下载（压缩包约 33 MB，只保留要用的文件）。模型的训练数据只许非商用，所以不随安装包附带。">
        {#snippet control()}
          <Button size="sm" variant="outline" data-testid="wake-download" disabled={downloading || status?.downloading} onclick={() => void download()}>
            {#if downloading || status?.downloading}<LoaderCircle class="size-3.5 animate-spin" />{percent !== null ? `${percent}%` : "下载中"}{:else}<Download class="size-3.5" />下载{/if}
          </Button>
        {/snippet}
      </SettingsRow>
    {/if}
    <SettingsRow label="唤醒词" description="四个字更不容易被别的话误触发。">
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel="唤醒词" value={wake.phrase} options={[...wakePhrases]}
            onchange={(phrase) => void save({ phrase: phrase as WakeConfig["phrase"] }, `唤醒词：${wakePhrases.find((p) => p.value === phrase)?.label}`)} />
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="灵敏度" description="经常误唤醒就调低；喊了没反应就调高。">
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel="唤醒灵敏度" value={wake.sensitivity} options={[...wakeSensitivities]}
            onchange={(sensitivity) => void save({ sensitivity: sensitivity as WakeConfig["sensitivity"] }, "唤醒灵敏度已保存")} />
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="唤醒后的应答" description="喊完唤醒词停一下，琴音会先接一声再听；连着往下说时不应答。唤醒后 5 秒没说话就安静结束。">
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel="唤醒后的应答" value={wake.ack} options={[...wakeAcks]}
            onchange={(ack) => void save({ ack: ack as WakeConfig["ack"] }, "唤醒应答已保存")} />
        </div>
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
