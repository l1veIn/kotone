<script lang="ts">
  /*
   * 文字处理设置页：后端注册表负责模块发现，前端只编辑线性 pipeline 配置。
   * 所有变更仍走 updateSettings 的事务入口，并以返回的完整 Settings 更新共享 store。
   */
  import { onMount } from "svelte";
  import {
    listPostProcessors,
    type PostProcessorInfo,
    type PostProcessingConfig,
    type PostProcessStepConfig,
  } from "../../../lib/ipc";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import { errText, patchSettings, settingsStore, toast } from "../../../lib/stores/ui";

  let processors = $state<PostProcessorInfo[]>([]);
  let loadingProcessors = $state(true);
  let saving = $state(false);
  let adding = $state(false);

  const pipeline = $derived($settingsStore?.postProcessing.pipeline.steps ?? []);
  const enabled = $derived($settingsStore?.postProcessing.enabled ?? false);
  const visibleProcessors = $derived(
    processors.filter((processor) => !processor.developerOnly || import.meta.env.DEV),
  );

  onMount(async () => {
    try {
      processors = await listPostProcessors();
    } catch (error) {
      toast(false, `加载文字处理模块失败：${errText(error)}`);
    } finally {
      loadingProcessors = false;
    }
  });

  function descriptor(processorId: string): PostProcessorInfo | undefined {
    return processors.find((processor) => processor.id === processorId);
  }

  function categoryLabel(category: PostProcessorInfo["category"]): string {
    switch (category) {
      case "writing":
        return "表达";
      case "translation":
        return "语言";
      case "utility":
        return "基础处理";
      default:
        return "开发测试";
    }
  }

  function accessLabel(access: PostProcessorInfo["networkAccess"]): string {
    if (access === "internet") return "需要联网";
    if (access === "local") return "访问本地服务";
    return "完全离线";
  }

  function nextStepId(processorId: string): string {
    const base = processorId.replace(/[^a-zA-Z0-9_-]+/g, "-");
    let index = pipeline.length + 1;
    let id = `${base}-${index}`;
    const ids = new Set(pipeline.map((step) => step.id));
    while (ids.has(id)) id = `${base}-${++index}`;
    return id;
  }

  async function savePostProcessing(
    next: PostProcessingConfig,
    successMessage: string,
  ): Promise<boolean> {
    if (saving) return false;
    saving = true;
    try {
      return await patchSettings({ postProcessing: next }, successMessage);
    } finally {
      saving = false;
    }
  }

  function configWithSteps(steps: PostProcessStepConfig[]): PostProcessingConfig {
    const current = $settingsStore?.postProcessing ?? {
      enabled: false,
      pipeline: { id: "default", steps: [] },
    };
    return {
      enabled: current.enabled,
      pipeline: { ...current.pipeline, steps },
    };
  }

  async function setEnabled(nextEnabled: boolean) {
    const next = configWithSteps([...pipeline]);
    next.enabled = nextEnabled;
    await savePostProcessing(next, nextEnabled ? "文字处理已开启" : "文字处理已关闭");
  }

  async function addProcessor(processor: PostProcessorInfo) {
    const step: PostProcessStepConfig = {
      id: nextStepId(processor.id),
      processorId: processor.id,
      enabled: true,
      config: null,
      timeoutMs: 5_000,
      onError: "required",
    };
    const next = configWithSteps([...pipeline, step]);
    next.enabled = true;
    if (await savePostProcessing(next, `已添加：${processor.displayName}`)) adding = false;
  }

  async function updateStep(index: number, patch: Partial<PostProcessStepConfig>, message: string) {
    const steps = pipeline.map((step, stepIndex) =>
      stepIndex === index ? { ...step, ...patch } : { ...step },
    );
    await savePostProcessing(configWithSteps(steps), message);
  }

  async function removeStep(index: number) {
    const name = descriptor(pipeline[index].processorId)?.displayName ?? pipeline[index].processorId;
    const steps = pipeline.filter((_, stepIndex) => stepIndex !== index).map((step) => ({ ...step }));
    await savePostProcessing(configWithSteps(steps), `已移除：${name}`);
  }

  async function moveStep(index: number, offset: -1 | 1) {
    const target = index + offset;
    if (target < 0 || target >= pipeline.length) return;
    const steps = pipeline.map((step) => ({ ...step }));
    [steps[index], steps[target]] = [steps[target], steps[index]];
    await savePostProcessing(configWithSteps(steps), "处理顺序已更新");
  }
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">文字处理</h1>
  <p class="mt-0.5 text-[11px] text-white/45">识别后、发送前，按顺序加工文字</p>

  {#if $settingsStore}
    <section class="kotone-panel mt-4 p-4">
      <Toggle
        checked={enabled}
        label="启用文字处理"
        desc={pipeline.length === 0
          ? "添加处理步骤后，识别文本会按顺序加工再发送"
          : `当前共 ${pipeline.length} 个步骤，下一条语音开始生效`}
        onchange={(value) => void setEnabled(value)}
      />
    </section>

    <section class="mt-4">
      <div class="flex items-end justify-between gap-4">
        <div>
          <h2 class="text-sm font-semibold text-kotone-cyan/90">处理流程</h2>
          <p class="mt-1 text-[11px] text-white/45">从上到下依次执行，后一步会接收前一步的结果。</p>
        </div>
        <button
          data-testid="add-postprocess-step"
          class="rounded-lg bg-kotone-cyan/15 px-3 py-1.5 text-xs font-semibold text-kotone-cyan ring-1 ring-kotone-cyan/35 transition hover:bg-kotone-cyan/25 active:scale-95 disabled:opacity-50"
          disabled={saving || loadingProcessors}
          onclick={() => (adding = !adding)}
        >
          {adding ? "收起" : "+ 添加步骤"}
        </button>
      </div>

      {#if adding}
        <div class="mt-3 rounded-xl bg-white/4 p-3 ring-1 ring-white/10">
          <p class="px-1 text-[11px] text-white/45">可用模块由后端注册表自动发现</p>
          {#if visibleProcessors.length === 0}
            <p class="mt-3 rounded-lg bg-white/5 px-3 py-3 text-xs text-white/45">
              {loadingProcessors ? "正在发现模块…" : "当前没有可用的文字处理模块。"}
            </p>
          {:else}
            <div class="mt-2 grid grid-cols-2 gap-2">
              {#each visibleProcessors as processor}
                <button
                  data-testid={`processor-option-${processor.id}`}
                  class="rounded-lg bg-white/5 p-3 text-left ring-1 ring-white/10 transition hover:bg-white/9 hover:ring-kotone-cyan/35 active:scale-[0.99] disabled:opacity-50"
                  disabled={saving}
                  onclick={() => void addProcessor(processor)}
                >
                  <div class="flex items-start justify-between gap-2">
                    <span class="text-xs font-semibold text-white/90">{processor.displayName}</span>
                    {#if processor.developerOnly}
                      <span class="rounded bg-kotone-violet/15 px-1.5 py-0.5 text-[9px] text-kotone-violet ring-1 ring-kotone-violet/25">开发测试</span>
                    {/if}
                  </div>
                  <p class="mt-1 text-[10px] text-kotone-cyan/70">
                    {categoryLabel(processor.category)} · {accessLabel(processor.networkAccess)}
                  </p>
                  <p class="mt-1.5 text-[11px] leading-relaxed text-white/45">{processor.description}</p>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if pipeline.length === 0}
        <div class="mt-3 rounded-xl border border-dashed border-white/15 px-4 py-8 text-center">
          <p class="text-sm text-white/65">还没有处理步骤</p>
          <p class="mt-1 text-[11px] text-white/35">添加后，Kotone 会按列表顺序处理识别文本。</p>
        </div>
      {:else}
        <div class="mt-3 flex flex-col gap-2">
          {#each pipeline as step, index (step.id)}
            {@const processor = descriptor(step.processorId)}
            <article
              data-testid={`postprocess-step-${step.processorId}`}
              class="kotone-panel p-4 transition {step.enabled ? '' : 'opacity-55'}"
            >
              <div class="flex items-start gap-3">
                <div class="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-kotone-cyan/12 text-xs font-bold text-kotone-cyan ring-1 ring-kotone-cyan/25">
                  {index + 1}
                </div>
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2">
                    <h3 class="truncate text-sm font-semibold text-white/90">
                      {processor?.displayName ?? step.processorId}
                    </h3>
                    {#if !processor}
                      <span class="rounded bg-kotone-pink/12 px-1.5 py-0.5 text-[9px] text-kotone-pink ring-1 ring-kotone-pink/25">模块不可用</span>
                    {/if}
                  </div>
                  <p class="mt-0.5 text-[11px] leading-relaxed text-white/45">
                    {processor?.description ?? "此步骤对应的模块尚未注册；运行时会给出明确提示。"}
                  </p>
                </div>
                <Toggle
                  checked={step.enabled}
                  label=""
                  ariaLabel={`启用${processor?.displayName ?? step.processorId}`}
                  onchange={(value) =>
                    void updateStep(index, { enabled: value }, value ? "步骤已启用" : "步骤已停用")}
                />
              </div>

              <div class="mt-3 flex items-center justify-between gap-3 border-t border-white/8 pt-3">
                <label class="flex items-center gap-2 text-[11px] text-white/45">
                  失败时
                  <select
                    class="rounded-md bg-white/7 px-2 py-1 text-[11px] text-white/75 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/50 [&>option]:bg-kotone-deep"
                    value={step.onError}
                    onchange={(event) => {
                      const onError = (event.target as HTMLSelectElement).value as
                        | "required"
                        | "best-effort";
                      void updateStep(
                        index,
                        { onError },
                        onError === "required" ? "失败时将停止发送" : "失败时将继续使用上一步结果",
                      );
                    }}
                  >
                    <option value="required">停止，不发送</option>
                    <option value="best-effort">继续使用上一步结果</option>
                  </select>
                </label>
                <div class="flex items-center gap-1">
                  <button
                    class="rounded-md bg-white/6 px-2 py-1 text-xs text-white/55 ring-1 ring-white/10 transition hover:bg-white/12 hover:text-white disabled:opacity-25"
                    aria-label="上移"
                    title="上移"
                    disabled={saving || index === 0}
                    onclick={() => void moveStep(index, -1)}
                  >↑</button>
                  <button
                    class="rounded-md bg-white/6 px-2 py-1 text-xs text-white/55 ring-1 ring-white/10 transition hover:bg-white/12 hover:text-white disabled:opacity-25"
                    aria-label="下移"
                    title="下移"
                    disabled={saving || index === pipeline.length - 1}
                    onclick={() => void moveStep(index, 1)}
                  >↓</button>
                  <button
                    class="ml-1 rounded-md bg-kotone-pink/8 px-2 py-1 text-[11px] text-kotone-pink/80 ring-1 ring-kotone-pink/15 transition hover:bg-kotone-pink/16 hover:text-kotone-pink disabled:opacity-40"
                    disabled={saving}
                    onclick={() => void removeStep(index)}
                  >移除</button>
                </div>
              </div>
            </article>
            {#if index < pipeline.length - 1}
              <div class="-my-1 text-center text-xs text-kotone-cyan/35">↓</div>
            {/if}
          {/each}
        </div>
      {/if}
    </section>
  {/if}
</div>
