<script lang="ts">
  /*
   * 文字处理设置页：后端注册表负责模块发现，前端只编辑线性 pipeline 配置。
   * 所有变更仍走 updateSettings 的事务入口，并以返回的完整 Settings 更新共享 store。
   * 版式：SettingsPage / SettingsSection（参照 DeepSeek Harness 设置页），对话框用 shadcn Dialog。
  */
  import { onMount } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    builtinBlocklistCsv,
    exportBuiltinBlocklist,
    isTauri,
    listConnections,
    listPostProcessors,
    testPostProcessing,
    type ConnectionInfo,
    type PostProcessorConfigField,
    type PostProcessorInfo,
    defaultPostProcessing,
    type PipelineConfig,
    type PostProcessingConfig,
    type PostProcessingTestResult,
    type PostProcessStepConfig,
  } from "../../../lib/ipc";
  import { ArrowDown, ArrowUp, FlaskConical, Plus, Settings2 } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Switch } from "$lib/components/ui/switch";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import { errText, patchSettings, settingsStore, toast } from "../../../lib/stores/ui";

  let { onOpenConnections }: { onOpenConnections?: () => void } = $props();

  let processors = $state<PostProcessorInfo[]>([]);
  let loadingProcessors = $state(true);
  let connections = $state<ConnectionInfo[]>([]);
  let saving = $state(false);
  let adding = $state(false);
  let testInput = $state("对面那个傻逼打野太牛逼了");
  let testing = $state(false);
  let tryoutResult = $state<PostProcessingTestResult | null>(null);

  const postProcessing = $derived($settingsStore?.postProcessing ?? defaultPostProcessing());
  const pipelines = $derived(postProcessing.pipelines);
  const activePipeline = $derived(
    pipelines.find((item) => item.id === postProcessing.activePipelineId) ?? pipelines[0],
  );
  const pipeline = $derived(activePipeline?.steps ?? []);
  const enabled = $derived(postProcessing.enabled);
  const activeStepCount = $derived(pipeline.filter((step) => step.enabled).length);
  let renaming = $state(false);
  let renameDraft = $state("");
  let managing = $state(false);
  let tryingOut = $state(false);
  /** 输入框统一样式（主题变量，浅色下也清楚）。 */
  const fieldClass =
    "min-w-0 flex-1 rounded-lg border border-input bg-background/60 px-2.5 py-1.5 text-xs text-foreground outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40 disabled:opacity-50";
  const visibleProcessors = $derived(
    processors.filter((processor) => !processor.developerOnly),
  );

  onMount(async () => {
    try {
      [processors, connections] = await Promise.all([listPostProcessors(), listConnections()]);
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
    if (access === "local") return "访问本地资源";
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

  function configRecord(config: unknown): Record<string, unknown> {
    return config !== null && typeof config === "object" && !Array.isArray(config)
      ? (config as Record<string, unknown>)
      : {};
  }

  function configValue(config: unknown, key: string): string {
    const value = configRecord(config)[key];
    return typeof value === "string" ? value : "";
  }

  function connectionsForField(field: PostProcessorConfigField): ConnectionInfo[] {
    const allowed = field.compatibleProviders ?? [];
    if (allowed.length === 0) return connections;
    return connections.filter((connection) => allowed.includes(connection.provider));
  }

  function requiredConfigComplete(
    processor: PostProcessorInfo | undefined,
    config: unknown,
  ): boolean {
    if (!processor) return false;
    const values = configRecord(config);
    return processor.configFields
      .filter((field) => field.required)
      .every((field) => {
        const value = values[field.key];
        return typeof value === "string" && value.trim() !== "";
      });
  }

  async function savePostProcessing(
    next: PostProcessingConfig,
    successMessage: string,
  ): Promise<boolean> {
    if (saving) return false;
    saving = true;
    tryoutResult = null;
    try {
      return await patchSettings({ postProcessing: next }, successMessage);
    } finally {
      saving = false;
    }
  }

  function currentConfig(): PostProcessingConfig {
    return $settingsStore?.postProcessing ?? defaultPostProcessing();
  }

  function configWithPipelines(
    patch: Partial<PostProcessingConfig> & { pipelines?: PipelineConfig[] },
  ): PostProcessingConfig {
    const current = currentConfig();
    return {
      enabled: patch.enabled ?? current.enabled,
      activePipelineId: patch.activePipelineId ?? current.activePipelineId,
      pipelines: patch.pipelines ?? current.pipelines,
    };
  }

  function configWithSteps(steps: PostProcessStepConfig[]): PostProcessingConfig {
    const current = currentConfig();
    const activeId = activePipeline?.id ?? current.activePipelineId;
    return configWithPipelines({
      pipelines: current.pipelines.map((item) =>
        item.id === activeId ? { ...item, steps } : item,
      ),
    });
  }

  function pipelineLabel(item: PipelineConfig): string {
    const name = item.displayName.trim();
    if (name) return name;
    if (item.id === "blocklist") return "屏蔽词";
    return item.id === "default" ? "自定义" : item.id;
  }

  function nextPipelineId(): string {
    const ids = new Set(pipelines.map((item) => item.id));
    let index = pipelines.length + 1;
    let id = `pipeline-${index}`;
    while (ids.has(id)) id = `pipeline-${++index}`;
    return id;
  }

  function nextPipelineName(): string {
    const names = new Set(pipelines.map((item) => pipelineLabel(item)));
    let index = pipelines.length + 1;
    let name = `流程 ${index}`;
    while (names.has(name)) name = `流程 ${++index}`;
    return name;
  }

  async function switchPipeline(id: string) {
    if (id === postProcessing.activePipelineId) return;
    renaming = false;
    await savePostProcessing(configWithPipelines({ activePipelineId: id }), "已切换处理流程");
  }

  async function addPipeline() {
    const id = nextPipelineId();
    const next: PipelineConfig = { id, displayName: nextPipelineName(), steps: [] };
    renaming = false;
    await savePostProcessing(
      configWithPipelines({
        enabled: true,
        activePipelineId: id,
        pipelines: [...pipelines, next],
      }),
      `已新建：${next.displayName}`,
    );
  }

  async function startRename(item: PipelineConfig) {
    if (item.id !== postProcessing.activePipelineId) {
      await switchPipeline(item.id);
    }
    renameDraft = pipelineLabel(item);
    renaming = true;
  }

  async function renamePipeline() {
    const name = renameDraft.trim();
    const targetId = postProcessing.activePipelineId;
    if (!name) {
      renaming = false;
      return;
    }
    const next = configWithPipelines({
      pipelines: pipelines.map((item) =>
        item.id === targetId ? { ...item, displayName: name } : item,
      ),
    });
    renaming = false;
    await savePostProcessing(next, "流程名称已更新");
  }

  async function deletePipeline(id: string) {
    if (pipelines.length <= 1) return;
    const target = pipelines.find((item) => item.id === id);
    if (!target) return;
    const remaining = pipelines.filter((item) => item.id !== id);
    renaming = false;
    await savePostProcessing(
      configWithPipelines({
        activePipelineId:
          postProcessing.activePipelineId === id
            ? remaining[0].id
            : postProcessing.activePipelineId,
        pipelines: remaining,
      }),
      `已删除：${pipelineLabel(target)}`,
    );
  }

  async function setEnabled(nextEnabled: boolean) {
    if (!nextEnabled) {
      adding = false;
      managing = false;
    }
    const next = configWithSteps([...pipeline]);
    next.enabled = nextEnabled;
    await savePostProcessing(next, nextEnabled ? "文字处理已开启" : "文字处理已关闭");
  }

  async function addProcessor(processor: PostProcessorInfo) {
    const needsConfiguration = processor.configFields.some((field) => field.required);
    const internet = processor.networkAccess === "internet";
    const step: PostProcessStepConfig = {
      id: nextStepId(processor.id),
      processorId: processor.id,
      enabled: !needsConfiguration,
      config: processor.id === "translation.qwen-mt" ? { targetLang: "English" } : {},
      timeoutMs: internet ? 1_500 : 5_000,
      onError: internet ? "best-effort" : "required",
    };
    const next = configWithSteps([...pipeline, step]);
    next.enabled = true;
    const message = needsConfiguration
      ? `已添加：${processor.displayName}，请完成必填配置`
      : `已添加：${processor.displayName}`;
    if (await savePostProcessing(next, message)) adding = false;
  }

  async function updateStep(index: number, patch: Partial<PostProcessStepConfig>, message: string) {
    const steps = pipeline.map((step, stepIndex) =>
      stepIndex === index ? { ...step, ...patch } : { ...step },
    );
    await savePostProcessing(configWithSteps(steps), message);
  }

  async function setStepEnabled(index: number, nextEnabled: boolean) {
    const step = pipeline[index];
    const processor = descriptor(step.processorId);
    if (nextEnabled && !requiredConfigComplete(processor, step.config)) {
      toast(false, "请先完成这个步骤的必填配置");
      return;
    }
    await updateStep(index, { enabled: nextEnabled }, nextEnabled ? "步骤已启用" : "步骤已停用");
  }

  async function updateConfigField(index: number, field: PostProcessorConfigField, value: string) {
    const step = pipeline[index];
    const config = { ...configRecord(step.config), [field.key]: value };
    const processor = descriptor(step.processorId);
    const wasComplete = requiredConfigComplete(processor, step.config);
    const complete = requiredConfigComplete(processor, config);
    await updateStep(
      index,
      { config, enabled: complete ? step.enabled || !wasComplete : false },
      complete && !wasComplete ? "处理步骤配置已更新并启用" : "处理步骤配置已更新",
    );
  }

  async function chooseConfigFile(index: number, field: PostProcessorConfigField) {
    if (!isTauri) return;
    try {
      const selected = await openDialog({
        multiple: false,
        title: `选择${field.displayName}`,
        filters: field.fileExtensions.length
          ? [{ name: field.displayName, extensions: field.fileExtensions }]
          : undefined,
      });
      if (typeof selected === "string") await updateConfigField(index, field, selected);
    } catch (error) {
      toast(false, `选择文件失败：${errText(error)}`);
    }
  }

  async function onExportBlocklist() {
    try {
      if (isTauri) {
        const path = await saveDialog({
          defaultPath: "kotone-blocklist.csv",
          filters: [{ name: "屏蔽词表", extensions: ["csv"] }],
        });
        if (!path) return;
        const count = await exportBuiltinBlocklist(path);
        toast(true, `已导出 ${count} 条屏蔽词 → ${path}`);
      } else {
        const blob = new Blob([builtinBlocklistCsv()], { type: "text/csv;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = "kotone-blocklist.csv";
        anchor.click();
        URL.revokeObjectURL(url);
        toast(true, "已导出内置屏蔽词库");
      }
    } catch (error) {
      toast(false, `导出屏蔽词库失败：${errText(error)}`);
    }
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

  async function runTryout() {
    if (testing || !testInput.trim() || activeStepCount === 0 || !$settingsStore) return;
    testing = true;
    tryoutResult = null;
    try {
      tryoutResult = await testPostProcessing(testInput, activePipeline);
    } catch (error) {
      toast(false, `试跑失败：${errText(error)}`);
    } finally {
      testing = false;
    }
  }
</script>

<SettingsPage title="文字处理" description="识别完先处理，再发到游戏：屏蔽词、AI 润色、翻译……按流程从上到下依次处理。">
  {#if $settingsStore}
    <SettingsSection>
      <div class="py-3.5">
        <ToggleField
          checked={enabled}
          label="启用文字处理"
          desc={enabled ? "下一条语音开始生效" : "关掉后原文直发"}
          onchange={(value) => void setEnabled(value)}
        />
      </div>
    </SettingsSection>

    {#if enabled}
      <SettingsSection title="当前流程" description="说话时只用这一套。默认的屏蔽词开箱即用。">
        <div class="flex flex-wrap items-center gap-2 py-3.5">
          <div class="min-w-48 flex-1">
            <SelectField
              testid="postprocess-pipeline-select"
              size="md"
              ariaLabel="当前流程"
              value={activePipeline?.id ?? ""}
              disabled={saving}
              options={pipelines.map((item) => ({
                value: item.id,
                label: pipelineLabel(item),
              }))}
              onchange={(id) => void switchPipeline(id)}
            />
          </div>
          <Button variant="outline" data-testid="open-postprocess-tryout" onclick={() => (tryingOut = true)}>
            <FlaskConical class="size-3.5 text-primary" />试跑流程
          </Button>
          <Button variant="ghost" data-testid="manage-postprocess-pipelines" onclick={() => (managing = true)}>
            <Settings2 class="size-3.5" />管理流程
          </Button>
        </div>
      </SettingsSection>

      <section class="min-w-0" aria-labelledby="pipeline-steps-title">
        <div class="mb-2.5 px-1">
          <h2 id="pipeline-steps-title" class="m-0 text-sm font-semibold text-foreground">这套流程会做什么</h2>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">从上到下依次处理。</p>
        </div>

        {#if pipeline.length === 0}
          <div class="rounded-2xl border border-dashed border-hairline px-4 py-8 text-center">
            <p class="m-0 text-sm text-foreground/80">还没有处理步骤</p>
            <p class="mt-1 mb-0 text-xs text-muted-foreground">加一步之后，识别结果会先过这一套再发送。</p>
          </div>
        {:else}
          <div class="flex flex-col">
            {#each pipeline as step, index (step.id)}
              {@const processor = descriptor(step.processorId)}
              <article
                data-testid={`postprocess-step-${step.processorId}`}
                class="rounded-2xl border border-hairline bg-card/70 p-4 transition-opacity {step.enabled ? '' : 'opacity-75'}"
              >
                <div class="flex items-start gap-3">
                  <div class="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary/12 text-xs font-semibold text-primary ring-1 ring-primary/25">
                    {index + 1}
                  </div>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <h3 class="m-0 truncate text-[13px] font-semibold text-foreground">
                        {processor?.displayName ?? step.processorId}
                      </h3>
                      {#if !processor}
                        <span class="rounded-full bg-destructive/12 px-2 py-0.5 text-[10px] text-destructive">模块不可用</span>
                      {/if}
                    </div>
                    <p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">
                      {processor?.description ?? "此步骤对应的模块尚未注册；运行时会给出明确提示。"}
                    </p>
                  </div>
                  <Switch
                    bind:checked={() => step.enabled, (value) => void setStepEnabled(index, value)}
                    aria-label={`启用${processor?.displayName ?? step.processorId}`}
                  />
                </div>

                {#if processor && processor.configFields.length > 0}
                  <div class="mt-3 space-y-3 rounded-xl border border-hairline bg-muted/30 p-3">
                    {#each processor.configFields as field (field.key)}
                      {@const currentValue = configValue(step.config, field.key)}
                      {@const fieldPlaceholder =
                        field.placeholder ||
                        (field.kind === "file" ? `选择或输入${field.displayName}路径` : `输入${field.displayName}`)}
                      {@const multiline =
                        field.placeholder?.includes("\n") || (field.placeholder?.length ?? 0) > 48}
                      <div>
                        <div class="flex items-center gap-1.5 text-xs font-medium text-foreground/85">
                          <span>{field.displayName}</span>
                          {#if field.required}
                            <span class="text-[10px] text-brand-pink">必填</span>
                          {/if}
                        </div>
                        <div class="mt-1.5 flex gap-2">
                          {#if field.kind === "connection"}
                            <div class="min-w-0 flex-1">
                              <SelectField
                                testid={`postprocess-config-${step.id}-${field.key}`}
                                ariaLabel={field.displayName}
                                placeholder="选择连接"
                                value={currentValue}
                                disabled={saving}
                                options={[
                                  { value: "", label: "选择连接" },
                                  ...connectionsForField(field).map((connection) => ({
                                    value: connection.id,
                                    label: `${connection.displayName}${connection.hasApiKey ? "" : "（缺密钥）"}`,
                                  })),
                                ]}
                                onchange={(next) => void updateConfigField(index, field, next.trim())}
                              />
                            </div>
                            {#if onOpenConnections}
                              <Button size="sm" variant="outline" data-testid="open-advanced-connections" onclick={onOpenConnections}>
                                {connectionsForField(field).length === 0 ? "去添加 API 连接" : "管理连接"}
                              </Button>
                            {/if}
                          {:else if multiline}
                            <textarea
                              data-testid={`postprocess-config-${step.id}-${field.key}`}
                              aria-label={field.displayName}
                              rows="4"
                              class="{fieldClass} min-h-[5.5rem] resize-y leading-relaxed"
                              placeholder={fieldPlaceholder}
                              value={currentValue}
                              disabled={saving}
                              onchange={(event) =>
                                void updateConfigField(
                                  index,
                                  field,
                                  (event.target as HTMLTextAreaElement).value.trim(),
                                )}
                            ></textarea>
                          {:else}
                            <input
                              data-testid={`postprocess-config-${step.id}-${field.key}`}
                              aria-label={field.displayName}
                              class="{fieldClass} h-8"
                              placeholder={fieldPlaceholder}
                              value={currentValue}
                              disabled={saving}
                              onchange={(event) =>
                                void updateConfigField(
                                  index,
                                  field,
                                  (event.target as HTMLInputElement).value.trim(),
                                )}
                            />
                          {/if}
                          {#if field.kind === "file" && isTauri}
                            <Button size="sm" variant="outline" disabled={saving} onclick={() => void chooseConfigFile(index, field)}>选择文件</Button>
                          {/if}
                        </div>
                        {#if field.presets && field.presets.length > 0}
                          <div class="mt-1.5 flex flex-wrap gap-1">
                            {#each field.presets as preset}
                              {@const selected = currentValue === preset.value}
                              <button
                                type="button"
                                class="rounded-full px-2.5 py-0.5 text-[11px] ring-1 transition-colors {selected
                                  ? 'bg-primary/12 text-primary ring-primary/40'
                                  : 'bg-background/50 text-muted-foreground ring-hairline hover:text-foreground'}"
                                disabled={saving}
                                onclick={() => void updateConfigField(index, field, preset.value)}
                              >
                                {preset.displayName}
                              </button>
                            {/each}
                          </div>
                        {/if}
                        <p class="mt-1.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">
                          {field.kind === "connection" && connectionsForField(field).length === 0
                            ? "还没有可用连接。点右边按钮去添加 API key。"
                            : field.description}
                        </p>
                      </div>
                    {/each}
                    {#if processor.id === "builtin.blocklist-filter"}
                      <div class="border-t border-hairline pt-3">
                        <p class="m-0 text-xs leading-relaxed text-muted-foreground">
                          想改词？先导出默认这份当模板，改完再选回来。
                        </p>
                        <Button size="sm" variant="outline" class="mt-2" data-testid="postprocess-export-blocklist" disabled={saving} onclick={() => void onExportBlocklist()}>
                          导出默认词库
                        </Button>
                      </div>
                    {/if}
                    {#if !requiredConfigComplete(processor, step.config)}
                      <p class="m-0 text-[11px] text-brand-pink">
                        完成必填配置后，这个步骤会自动启用。
                      </p>
                    {/if}
                  </div>
                {/if}

                <div class="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-hairline pt-3">
                  <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                    失败时
                    <div class="w-44">
                      <SelectField
                        testid={`postprocess-on-error-${step.id}`}
                        ariaLabel="失败时"
                        value={step.onError}
                        options={[
                          { value: "required", label: "停止，不发送" },
                          { value: "best-effort", label: "继续使用上一步结果" },
                        ]}
                        onchange={(next) => {
                          const onError = next as "required" | "best-effort";
                          void updateStep(
                            index,
                            { onError },
                            onError === "required" ? "失败时将停止发送" : "失败时将继续使用上一步结果",
                          );
                        }}
                      />
                    </div>
                  </div>
                  <div class="flex items-center gap-1">
                    <Button size="icon-sm" variant="ghost" aria-label="上移" title="上移" disabled={saving || index === 0} onclick={() => void moveStep(index, -1)}>
                      <ArrowUp class="size-3.5" />
                    </Button>
                    <Button size="icon-sm" variant="ghost" aria-label="下移" title="下移" disabled={saving || index === pipeline.length - 1} onclick={() => void moveStep(index, 1)}>
                      <ArrowDown class="size-3.5" />
                    </Button>
                    <Button size="sm" variant="ghost" class="ml-1 text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={saving} onclick={() => void removeStep(index)}>移除</Button>
                  </div>
                </div>
              </article>
              {#if index < pipeline.length - 1}
                <div class="flex justify-center py-1 text-primary/50" aria-hidden="true"><ArrowDown class="size-3.5" /></div>
              {/if}
            {/each}
          </div>
        {/if}

        <div class="mt-3 flex items-center gap-2">
          <Button data-testid="add-postprocess-step" disabled={saving || loadingProcessors} onclick={() => (adding = true)}>
            <Plus class="size-3.5" />添加步骤
          </Button>
          <Button variant="outline" data-testid="open-postprocess-tryout-bottom" disabled={saving || activeStepCount === 0} onclick={() => (tryingOut = true)}>
            试跑效果
          </Button>
        </div>
      </section>
    {/if}
  {/if}

  <Dialog.Root bind:open={adding}>
    <Dialog.Content class="ui-scope sm:max-w-xl" showCloseButton={false}>
      <Dialog.Header>
        <Dialog.Title>添加步骤</Dialog.Title>
        <Dialog.Description>点一行即可加到当前流程。</Dialog.Description>
      </Dialog.Header>
      <div class="max-h-80 overflow-y-auto">
        {#if visibleProcessors.length === 0}
          <p class="m-0 rounded-lg bg-muted/50 px-3 py-3 text-xs text-muted-foreground">
            {loadingProcessors ? "正在发现模块…" : "当前没有可用的文字处理模块。"}
          </p>
        {:else}
          <ul class="m-0 divide-y divide-hairline p-0">
            {#each visibleProcessors as processor}
              <li class="flex list-none items-start gap-3 py-2.5">
                <div class="min-w-0 flex-1">
                  <p class="m-0 text-[13px] text-foreground">{processor.displayName}</p>
                  <p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">{processor.description}</p>
                  <p class="mt-1 mb-0 text-[11px] text-muted-foreground/80">
                    {categoryLabel(processor.category)} · {accessLabel(processor.networkAccess)}
                  </p>
                </div>
                <Button size="sm" variant="outline" data-testid={`processor-option-${processor.id}`} disabled={saving} onclick={() => void addProcessor(processor)}>
                  添加
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <Dialog.Footer>
        <Button variant="ghost" size="sm" onclick={() => (adding = false)}>取消</Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

  <Dialog.Root bind:open={managing}>
    <Dialog.Content class="ui-scope sm:max-w-lg" showCloseButton={false}>
      <Dialog.Header>
        <Dialog.Title>管理流程</Dialog.Title>
        <Dialog.Description>新建、改名或删掉一套流程。切换请回上一页用下拉框。</Dialog.Description>
      </Dialog.Header>
      <div class="max-h-72 overflow-y-auto">
        <table class="w-full text-left text-xs">
          <thead>
            <tr class="text-[11px] text-muted-foreground">
              <th class="pb-2 font-medium">名称</th>
              <th class="pb-2 font-medium">步骤</th>
              <th class="pb-2 font-medium">状态</th>
              <th class="pb-2 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {#each pipelines as item (item.id)}
              <tr class="border-t border-hairline">
                <td class="py-2 pr-2">
                  {#if renaming && item.id === postProcessing.activePipelineId}
                    <input
                      data-testid="postprocess-pipeline-name"
                      class="{fieldClass} h-7 w-full"
                      bind:value={renameDraft}
                      disabled={saving}
                      onkeydown={(event) => {
                        if (event.key === "Enter") void renamePipeline();
                        if (event.key === "Escape") { event.stopPropagation(); renaming = false; }
                      }}
                      onblur={() => void renamePipeline()}
                    />
                  {:else}
                    <span class="text-foreground">{pipelineLabel(item)}</span>
                  {/if}
                </td>
                <td class="py-2 text-muted-foreground">{item.steps.length}</td>
                <td class="py-2">
                  {#if item.id === postProcessing.activePipelineId}
                    <span class="text-primary">使用中</span>
                  {:else}
                    <button type="button" class="text-muted-foreground transition-colors hover:text-foreground" disabled={saving} onclick={() => void switchPipeline(item.id)}>
                      切换
                    </button>
                  {/if}
                </td>
                <td class="py-2 text-right whitespace-nowrap">
                  <Button size="xs" variant="ghost" data-testid="rename-postprocess-pipeline" disabled={saving} onclick={() => void startRename(item)}>重命名</Button>
                  <Button size="xs" variant="ghost" class="text-destructive hover:bg-destructive/10 hover:text-destructive" data-testid="delete-postprocess-pipeline"
                    disabled={saving || pipelines.length <= 1} onclick={() => void deletePipeline(item.id)}>删除</Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <Dialog.Footer class="sm:justify-between">
        <Button variant="outline" size="sm" data-testid="add-postprocess-pipeline" disabled={saving} onclick={() => void addPipeline()}>
          <Plus class="size-3.5" />新建流程
        </Button>
        <Button size="sm" onclick={() => (managing = false)}>完成</Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

  <Dialog.Root bind:open={tryingOut}>
    <Dialog.Content class="ui-scope max-h-[85vh] overflow-y-auto sm:max-w-lg" showCloseButton={false}>
      <Dialog.Header>
        <div class="flex items-center gap-2">
          <Dialog.Title>试跑「{pipelineLabel(activePipeline)}」</Dialog.Title>
          <span class="rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">{activeStepCount} 个启用步骤</span>
        </div>
        <Dialog.Description>输入一段测试文本查看文字处理效果，不会发送到游戏。</Dialog.Description>
      </Dialog.Header>

      <label class="block">
        <span class="text-xs text-muted-foreground">输入测试文本</span>
        <textarea
          data-testid="postprocess-tryout-input"
          class="{fieldClass} mt-1.5 min-h-24 w-full resize-y text-sm leading-relaxed"
          placeholder="输入用于验证流程的文字"
          bind:value={testInput}
        ></textarea>
      </label>

      {#if tryoutResult}
        <div data-testid="postprocess-tryout-result" class="rounded-xl border border-primary/25 bg-primary/6 p-3.5">
          <div class="flex items-center justify-between gap-3">
            <span class="text-xs font-semibold text-primary">处理结果</span>
            <span class="text-[11px] text-muted-foreground tabular-nums">{tryoutResult.durationMs} ms</span>
          </div>
          <p class="mt-2 mb-0 text-sm leading-relaxed break-all text-foreground">{tryoutResult.finalText}</p>
          <div class="mt-3 flex flex-col gap-1.5 border-t border-hairline pt-3">
            {#each tryoutResult.steps as step, index (step.stepId)}
              <div>
                <span
                  class="inline-flex rounded-full px-2 py-0.5 text-[11px] {step.outcome === 'succeeded'
                    ? 'bg-primary/10 text-primary'
                    : 'bg-brand-pink/10 text-brand-pink'}"
                  title={`${step.durationMs} ms`}
                >
                  {index + 1}. {step.displayName} · {step.outcome === "succeeded" ? "完成" : "已跳过"}
                </span>
                {#if step.error}
                  <p class="mt-1 mb-0 text-[11px] leading-relaxed text-brand-pink">{step.error}</p>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <Dialog.Footer>
        <Button variant="ghost" size="sm" data-testid="postprocess-tryout-close" onclick={() => (tryingOut = false)}>关闭</Button>
        <Button size="sm" data-testid="postprocess-tryout-run" disabled={testing || !testInput.trim() || activeStepCount === 0} onclick={() => void runTryout()}>
          {testing ? "处理中…" : "运行试跑"}
        </Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>
</SettingsPage>
