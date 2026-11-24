<script lang="ts">
  /*
   * 设置 → 模型 → 语音识别 最上面：现在用的语音识别和它准备好没有（ADR 028 W8 / W9）。
   * 语音识别归本体（交接第十一节第 6 条）：语音输入插件只显示、引到这里；启动 / 停止、启动热键、
   * “打开 Kotone 时自动启动”从语音输入页的卡片搬来。在线识别打开就能用，看不到“启动”；
   * 本地识别才有 未启动 / 准备中 / 已启动 / 出问题了。换回在线识别时这些收起，启动热键和自动启动的值保留。
   * 换模型、下载在下面的“模型切换”；“按新设置重新启动”在页面最上面的提示条里。
   */
  import { startRuntime, stopRuntime, updateSettings, type ModelInfo } from "$lib/ipc";
  import { LOCAL_ASR_LABELS } from "$lib/ipc/contracts";
  import { asrStore, runtimeStore } from "$lib/stores/runtime";
  import { errText, patchSettings, settingsStore, toast } from "$lib/stores/ui";
  import HotkeyField from "$lib/components/HotkeyField.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";

  let { models }: {
    /** 识别模型清单（ModelsSection 已经读过；用来显示名字、判断本地模型下好没有）。 */
    models: ModelInfo[];
  } = $props();

  const asrState = $derived($asrStore);
  const rt = $derived($runtimeStore);
  const local = $derived(asrState.startsWith("local-"));
  const active = $derived(models.find((model) => model.id === rt?.modelId) ?? null);
  const activeName = $derived((active?.displayName ?? rt?.modelId ?? "").replace(/（.*$/, ""));
  const localMissing = $derived(local && active !== null && !active.downloaded);
  let acting = $state(false);

  async function run(action: () => Promise<unknown>, done?: string) {
    if (acting) return;
    acting = true;
    try {
      await action();
      if (done) toast(true, done);
    } catch (error) {
      toast(false, errText(error));
    } finally {
      acting = false;
    }
  }
</script>

<section id="models-asr-runtime" class="min-w-0" data-testid="models-asr-runtime" data-asr-state={asrState}>
  <div class="rounded-2xl border border-hairline bg-card/70 px-4 [&>*+*]:border-t [&>*+*]:border-hairline">
    <div class="flex flex-wrap items-center gap-3 py-3.5">
      <div class="min-w-0 flex-1">
        <p class="m-0 truncate text-[13px] font-medium text-foreground" data-testid="models-asr-current">{activeName || "…"} · {local ? m.voice_asr_mode_local() : m.voice_asr_mode_online()}</p>
        {#if asrState === "online-ready"}
          <p class="mt-0.5 mb-0 text-xs text-success" data-testid="models-asr-status">{m.voice_asr_online_ready()}</p>
        {:else if asrState === "online-preparing"}
          <p class="mt-0.5 mb-0 text-xs text-muted-foreground" data-testid="models-asr-status">{m.voice_asr_preparing()}</p>
        {:else if asrState === "online-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="models-asr-status">{m.voice_asr_online_error({ error: rt?.error ?? "" })}</p>
        {:else if localMissing}
          <p class="mt-0.5 mb-0 text-xs text-warning" data-testid="models-asr-status">{m.voice_asr_not_downloaded({ name: activeName })}</p>
        {:else if asrState === "local-error"}
          <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="models-asr-status">{m.voice_asr_labeled_error({ label: LOCAL_ASR_LABELS["local-error"], error: rt?.error ?? "" })}</p>
        {:else if local}
          <p class="mt-0.5 mb-0 text-xs {asrState === 'local-running' ? 'text-success' : 'text-muted-foreground'}" data-testid="models-asr-status">
            {asrState === "local-running" ? "● " : ""}{LOCAL_ASR_LABELS[asrState as keyof typeof LOCAL_ASR_LABELS] ?? ""}{asrState === "local-starting" ? "…" : ""}
          </p>
        {/if}
      </div>
      <div class="flex shrink-0 items-center gap-2">
        {#if asrState === "online-error"}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-retry" onclick={() => void run(startRuntime)}>{m.voice_action_retry()}</Button>
        {:else if asrState === "local-stopped" || asrState === "local-error"}
          <Button size="sm" disabled={acting || localMissing} data-testid="asr-start" onclick={() => void run(startRuntime, m.voice_asr_started())}>{asrState === "local-error" ? m.voice_action_retry() : m.voice_asr_start()}</Button>
        {:else if asrState === "local-starting"}
          <Button size="sm" variant="outline" disabled data-testid="asr-start">{m.voice_asr_starting()}</Button>
        {:else if asrState === "local-running"}
          <Button size="sm" variant="outline" disabled={acting} data-testid="asr-stop" onclick={() => void run(stopRuntime, m.voice_asr_stopped())}>{m.voice_asr_stop()}</Button>
        {/if}
      </div>
    </div>

    {#if local}
      <div class="py-3.5" data-testid="asr-toggle-hotkey">
        <p class="m-0 text-[13px] font-medium text-foreground">{m.voice_asr_toggle_hotkey_title()}</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.voice_asr_toggle_hotkey_desc()}</p>
        <HotkeyField id="runtime-toggle" title={m.voice_hotkey_name_runtime_toggle()} value={$settingsStore?.runtimeToggleHotkey ?? ""} placeholder={m.voice_hotkey_placeholder({ example: "Ctrl+Alt+K" })} compact
          onSave={async (key) => { settingsStore.set(await updateSettings({ runtimeToggleHotkey: key })); }} />
      </div>
      <div class="py-3.5">
        <ToggleField checked={$settingsStore?.ui.autoStart ?? false} label={m.voice_asr_autostart_label()} testid="asr-autostart"
          desc={m.voice_asr_autostart_desc()}
          onchange={(on) => void patchSettings({ ui: { autoStart: on } }, on ? m.voice_asr_autostart_on() : m.voice_asr_autostart_off())} />
      </div>
    {/if}
  </div>
</section>
