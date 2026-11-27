<script lang="ts">
  /*
   * 语音输入页的“语音识别”卡片（交接第十一节第 6 条）：语音识别归本体，语音输入只是借来用。
   * 这里只显示现在用的是哪个（本地模型的名字 / 在线服务的名字）和它准备好没有；
   * 换模型、下载、启动 / 停止、启动热键、自动启动都在「设置 → 模型 → 语音识别」，卡片上只有去那里的链接。
   */
  import { listModels, type ModelInfo } from "$lib/ipc";
  import { LOCAL_ASR_LABELS } from "$lib/ipc/contracts";
  import { asrStore, runtimeStore } from "$lib/stores/runtime";
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenModels }: {
    /** 「去 设置 → 模型 换」：设置 → 模型 → 语音识别（设置只剩原来的“通用”一排，交接第十一节第 8 条）。 */
    onOpenModels: () => void;
  } = $props();

  let models = $state<ModelInfo[]>([]);
  const asrState = $derived($asrStore);
  const rt = $derived($runtimeStore);
  const local = $derived(asrState.startsWith("local-"));
  const active = $derived(models.find((model) => model.id === rt?.modelId) ?? null);
  const activeName = $derived((active?.displayName ?? rt?.modelId ?? "").replace(/（.*$/, ""));
  const localMissing = $derived(local && active !== null && !active.downloaded);

  $effect(() => {
    void rt?.modelId;
    let current = true;
    void listModels().then((items) => { if (current) models = items; }).catch(() => {});
    return () => { current = false; };
  });
</script>

<section id="voice-asr" class="scroll-mt-16 min-w-0" data-testid="asr-card" data-asr-state={asrState}>
  <h2 class="m-0 mb-1 text-xs font-normal text-muted-foreground">{m.plugin_voice_input_asr_title()}</h2>
  <div class="grid gap-1 justify-items-start">
    <div class="min-w-0 flex-1">
      <p class="m-0 break-words text-[13px] font-medium text-foreground" data-testid="asr-model-name">{activeName || "…"} · {local ? m.plugin_voice_input_asr_local() : m.plugin_voice_input_asr_online()}</p>
      {#if asrState === "online-ready"}
        <p class="mt-0.5 mb-0 text-xs text-success" data-testid="asr-status">{m.plugin_voice_input_asr_online_ready()}</p>
      {:else if asrState === "online-preparing"}
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground" data-testid="asr-status">{m.plugin_voice_input_asr_preparing()}</p>
      {:else if asrState === "online-error"}
        <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">{m.plugin_voice_input_asr_online_error({ error: rt?.error ?? "" })}</p>
      {:else if localMissing}
        <p class="mt-0.5 mb-0 text-xs text-warning" data-testid="asr-status">{m.plugin_voice_input_asr_not_downloaded()}</p>
      {:else if asrState === "local-error"}
        <p class="mt-0.5 mb-0 text-xs text-destructive" data-testid="asr-status">{m.plugin_voice_input_asr_local_error({ label: LOCAL_ASR_LABELS["local-error"], error: rt?.error ?? "" })}</p>
      {:else if local}
        <p class="mt-0.5 mb-0 text-xs {asrState === 'local-running' ? 'text-success' : 'text-muted-foreground'}" data-testid="asr-status">
          {asrState === "local-running" ? "● " : ""}{LOCAL_ASR_LABELS[asrState as keyof typeof LOCAL_ASR_LABELS] ?? ""}{asrState === "local-starting" ? "…" : ""}
        </p>
      {/if}
    </div>
    <button class="shrink-0 text-xs text-primary hover:underline" data-testid="asr-open-models" onclick={onOpenModels}>{m.plugin_voice_input_asr_open_models()}</button>
  </div>
</section>
