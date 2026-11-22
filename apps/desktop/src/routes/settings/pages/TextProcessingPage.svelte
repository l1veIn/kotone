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
  import { m } from "$lib/paraglide/messages.js";

  let { onOpenConnections }: { onOpenConnections?: () => void } = $props();

  let processors = $state<PostProcessorInfo[]>([]);
  let loadingProcessors = $state(true);
  let connections = $state<ConnectionInfo[]>([]);
  let saving = $state(false);
  let adding = $state(false);
  let testInput = $state(m.voice_processing_sample_input());
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
      toast(false, m.voice_processing_load_failed({ error: errText(error) }));
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
        return m.voice_processing_category_writing();
      case "translation":
        return m.voice_processing_category_translation();
      case "utility":
        return m.voice_processing_category_utility();
      default:
        return m.voice_processing_category_developer();
    }
  }

  function accessLabel(access: PostProcessorInfo["networkAccess"]): string {
    if (access === "internet") return m.voice_processing_access_internet();
    if (access === "local") return m.voice_processing_access_local();
    return m.voice_processing_access_offline();
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
    if (item.id === "blocklist") return m.voice_pipeline_blocklist_name();
    return item.id === "default" ? m.voice_pipeline_default_name() : item.id;
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
    let name = m.voice_processing_pipeline_name({ number: index });
    while (names.has(name)) name = m.voice_processing_pipeline_name({ number: ++index });
    return name;
  }

  async function switchPipeline(id: string) {
    if (id === postProcessing.activePipelineId) return;
    renaming = false;
    await savePostProcessing(configWithPipelines({ activePipelineId: id }), m.voice_processing_switched());
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
      m.voice_processing_created({ name: next.displayName }),
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
    await savePostProcessing(next, m.voice_processing_renamed());
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
      m.voice_processing_deleted({ name: pipelineLabel(target) }),
    );
  }

  async function setEnabled(nextEnabled: boolean) {
    if (!nextEnabled) {
      adding = false;
      managing = false;
    }
    const next = configWithSteps([...pipeline]);
    next.enabled = nextEnabled;
    await savePostProcessing(next, nextEnabled ? m.voice_pp_notice_enabled() : m.voice_pp_notice_disabled());
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
      ? m.voice_processing_added_needs_config({ name: processor.displayName })
      : m.voice_processing_added({ name: processor.displayName });
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
      toast(false, m.voice_processing_step_needs_config());
      return;
    }
    await updateStep(index, { enabled: nextEnabled }, nextEnabled ? m.voice_processing_step_enabled() : m.voice_processing_step_disabled());
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
      complete && !wasComplete ? m.voice_processing_step_config_saved_enabled() : m.voice_processing_step_config_saved(),
    );
  }

  async function chooseConfigFile(index: number, field: PostProcessorConfigField) {
    if (!isTauri) return;
    try {
      const selected = await openDialog({
        multiple: false,
        title: m.voice_processing_choose_file_title({ name: field.displayName }),
        filters: field.fileExtensions.length
          ? [{ name: field.displayName, extensions: field.fileExtensions }]
          : undefined,
      });
      if (typeof selected === "string") await updateConfigField(index, field, selected);
    } catch (error) {
      toast(false, m.voice_processing_choose_file_failed({ error: errText(error) }));
    }
  }

  async function onExportBlocklist() {
    try {
      if (isTauri) {
        const path = await saveDialog({
          defaultPath: "kotone-blocklist.csv",
          filters: [{ name: m.voice_processing_blocklist_filter_name(), extensions: ["csv"] }],
        });
        if (!path) return;
        const count = await exportBuiltinBlocklist(path);
        toast(true, m.voice_processing_blocklist_exported({ count, path }));
      } else {
        const blob = new Blob([builtinBlocklistCsv()], { type: "text/csv;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = "kotone-blocklist.csv";
        anchor.click();
        URL.revokeObjectURL(url);
        toast(true, m.voice_processing_blocklist_exported_builtin());
      }
    } catch (error) {
      toast(false, m.voice_processing_blocklist_export_failed({ error: errText(error) }));
    }
  }

  async function removeStep(index: number) {
    const name = descriptor(pipeline[index].processorId)?.displayName ?? pipeline[index].processorId;
    const steps = pipeline.filter((_, stepIndex) => stepIndex !== index).map((step) => ({ ...step }));
    await savePostProcessing(configWithSteps(steps), m.voice_processing_step_removed({ name }));
  }

  async function moveStep(index: number, offset: -1 | 1) {
    const target = index + offset;
    if (target < 0 || target >= pipeline.length) return;
    const steps = pipeline.map((step) => ({ ...step }));
    [steps[index], steps[target]] = [steps[target], steps[index]];
    await savePostProcessing(configWithSteps(steps), m.voice_processing_order_saved());
  }

  async function runTryout() {
    if (testing || !testInput.trim() || activeStepCount === 0 || !$settingsStore) return;
    testing = true;
    tryoutResult = null;
    try {
      tryoutResult = await testPostProcessing(testInput, activePipeline);
    } catch (error) {
      toast(false, m.voice_processing_tryout_failed({ error: errText(error) }));
    } finally {
      testing = false;
    }
  }
</script>

<SettingsPage embedded title={m.voice_tab_processing()} description={m.voice_processing_page_desc()}>
  {#if $settingsStore}
    <SettingsSection>
      <div class="py-3.5">
        <ToggleField
          checked={enabled}
          label={m.voice_processing_enable_label()}
          desc={enabled ? m.voice_processing_enable_on_desc() : m.voice_processing_enable_off_desc()}
          onchange={(value) => void setEnabled(value)}
        />
      </div>
    </SettingsSection>

    {#if enabled}
      <SettingsSection title={m.voice_processing_current_title()} description={m.voice_processing_current_desc()}>
        <div class="flex flex-wrap items-center gap-2 py-3.5">
          <div class="min-w-48 flex-1">
            <SelectField
              testid="postprocess-pipeline-select"
              size="md"
              ariaLabel={m.voice_processing_current_title()}
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
            <FlaskConical class="size-3.5 text-primary" />{m.voice_processing_tryout_open()}
          </Button>
          <Button variant="ghost" data-testid="manage-postprocess-pipelines" onclick={() => (managing = true)}>
            <Settings2 class="size-3.5" />{m.voice_processing_manage()}
          </Button>
        </div>
      </SettingsSection>

      <section class="min-w-0" aria-labelledby="pipeline-steps-title">
        <div class="mb-2.5 px-1">
          <h2 id="pipeline-steps-title" class="m-0 text-sm font-semibold text-foreground">{m.voice_processing_steps_title()}</h2>
          <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.voice_processing_steps_desc()}</p>
        </div>

        {#if pipeline.length === 0}
          <div class="rounded-2xl border border-dashed border-hairline px-4 py-8 text-center">
            <p class="m-0 text-sm text-foreground/80">{m.voice_processing_steps_empty_title()}</p>
            <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.voice_processing_steps_empty_desc()}</p>
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
                        <span class="rounded-full bg-destructive/12 px-2 py-0.5 text-[10px] text-destructive">{m.voice_processing_module_missing()}</span>
                      {/if}
                    </div>
                    <p class="mt-0.5 mb-0 text-xs leading-relaxed text-muted-foreground">
                      {processor?.description ?? m.voice_processing_module_missing_desc()}
                    </p>
                  </div>
                  <Switch
                    bind:checked={() => step.enabled, (value) => void setStepEnabled(index, value)}
                    aria-label={m.voice_processing_step_enable_aria({ name: processor?.displayName ?? step.processorId })}
                  />
                </div>

                {#if processor && processor.configFields.length > 0}
                  <div class="mt-3 space-y-3 rounded-xl border border-hairline bg-muted/30 p-3">
                    {#each processor.configFields as field (field.key)}
                      {@const currentValue = configValue(step.config, field.key)}
                      {@const fieldPlaceholder =
                        field.placeholder ||
                        (field.kind === "file" ? m.voice_processing_field_file_placeholder({ name: field.displayName }) : m.voice_processing_field_placeholder({ name: field.displayName }))}
                      {@const multiline =
                        field.placeholder?.includes("\n") || (field.placeholder?.length ?? 0) > 48}
                      <div>
                        <div class="flex items-center gap-1.5 text-xs font-medium text-foreground/85">
                          <span>{field.displayName}</span>
                          {#if field.required}
                            <span class="text-[10px] text-brand-pink">{m.voice_processing_required()}</span>
                          {/if}
                        </div>
                        <div class="mt-1.5 flex gap-2">
                          {#if field.kind === "connection"}
                            <div class="min-w-0 flex-1">
                              <SelectField
                                testid={`postprocess-config-${step.id}-${field.key}`}
                                ariaLabel={field.displayName}
                                placeholder={m.voice_processing_choose_connection()}
                                value={currentValue}
                                disabled={saving}
                                options={[
                                  { value: "", label: m.voice_processing_choose_connection() },
                                  ...connectionsForField(field).map((connection) => ({
                                    value: connection.id,
                                    label: connection.hasApiKey ? connection.displayName : m.voice_processing_connection_missing_key({ name: connection.displayName }),
                                  })),
                                ]}
                                onchange={(next) => void updateConfigField(index, field, next.trim())}
                              />
                            </div>
                            {#if onOpenConnections}
                              <Button size="sm" variant="outline" data-testid="open-advanced-connections" onclick={onOpenConnections}>
                                {connectionsForField(field).length === 0 ? m.voice_processing_add_connection() : m.voice_processing_manage_connections()}
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
                            <Button size="sm" variant="outline" disabled={saving} onclick={() => void chooseConfigFile(index, field)}>{m.voice_processing_choose_file()}</Button>
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
                            ? m.voice_processing_no_connections()
                            : field.description}
                        </p>
                      </div>
                    {/each}
                    {#if processor.id === "builtin.blocklist-filter"}
                      <div class="border-t border-hairline pt-3">
                        <p class="m-0 text-xs leading-relaxed text-muted-foreground">
                          {m.voice_processing_blocklist_hint()}
                        </p>
                        <Button size="sm" variant="outline" class="mt-2" data-testid="postprocess-export-blocklist" disabled={saving} onclick={() => void onExportBlocklist()}>
                          {m.voice_processing_blocklist_export()}
                        </Button>
                      </div>
                    {/if}
                    {#if !requiredConfigComplete(processor, step.config)}
                      <p class="m-0 text-[11px] text-brand-pink">
                        {m.voice_processing_config_incomplete()}
                      </p>
                    {/if}
                  </div>
                {/if}

                <div class="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-hairline pt-3">
                  <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                    {m.voice_processing_on_error_label()}
                    <div class="w-44">
                      <SelectField
                        testid={`postprocess-on-error-${step.id}`}
                        ariaLabel={m.voice_processing_on_error_label()}
                        value={step.onError}
                        options={[
                          { value: "required", label: m.voice_processing_on_error_required() },
                          { value: "best-effort", label: m.voice_processing_on_error_best_effort() },
                        ]}
                        onchange={(next) => {
                          const onError = next as "required" | "best-effort";
                          void updateStep(
                            index,
                            { onError },
                            onError === "required" ? m.voice_processing_on_error_required_saved() : m.voice_processing_on_error_best_effort_saved(),
                          );
                        }}
                      />
                    </div>
                  </div>
                  <div class="flex items-center gap-1">
                    <Button size="icon-sm" variant="ghost" aria-label={m.voice_processing_move_up()} title={m.voice_processing_move_up()} disabled={saving || index === 0} onclick={() => void moveStep(index, -1)}>
                      <ArrowUp class="size-3.5" />
                    </Button>
                    <Button size="icon-sm" variant="ghost" aria-label={m.voice_processing_move_down()} title={m.voice_processing_move_down()} disabled={saving || index === pipeline.length - 1} onclick={() => void moveStep(index, 1)}>
                      <ArrowDown class="size-3.5" />
                    </Button>
                    <Button size="sm" variant="ghost" class="ml-1 text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={saving} onclick={() => void removeStep(index)}>{m.voice_processing_remove_step()}</Button>
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
            <Plus class="size-3.5" />{m.voice_processing_add_step()}
          </Button>
          <Button variant="outline" data-testid="open-postprocess-tryout-bottom" disabled={saving || activeStepCount === 0} onclick={() => (tryingOut = true)}>
            {m.voice_processing_tryout_open_bottom()}
          </Button>
        </div>
      </section>
    {/if}
  {/if}

  <Dialog.Root bind:open={adding}>
    <Dialog.Content class="ui-scope sm:max-w-xl" showCloseButton={false}>
      <Dialog.Header>
        <Dialog.Title>{m.voice_processing_add_step()}</Dialog.Title>
        <Dialog.Description>{m.voice_processing_add_step_desc()}</Dialog.Description>
      </Dialog.Header>
      <div class="max-h-80 overflow-y-auto">
        {#if visibleProcessors.length === 0}
          <p class="m-0 rounded-lg bg-muted/50 px-3 py-3 text-xs text-muted-foreground">
            {loadingProcessors ? m.voice_processing_modules_loading() : m.voice_processing_modules_empty()}
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
                  {m.voice_processing_add()}
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <Dialog.Footer>
        <Button variant="ghost" size="sm" onclick={() => (adding = false)}>{m.voice_action_cancel()}</Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

  <Dialog.Root bind:open={managing}>
    <Dialog.Content class="ui-scope sm:max-w-lg" showCloseButton={false}>
      <Dialog.Header>
        <Dialog.Title>{m.voice_processing_manage()}</Dialog.Title>
        <Dialog.Description>{m.voice_processing_manage_desc()}</Dialog.Description>
      </Dialog.Header>
      <div class="max-h-72 overflow-y-auto">
        <table class="w-full text-left text-xs">
          <thead>
            <tr class="text-[11px] text-muted-foreground">
              <th class="pb-2 font-medium">{m.voice_processing_col_name()}</th>
              <th class="pb-2 font-medium">{m.voice_processing_col_steps()}</th>
              <th class="pb-2 font-medium">{m.voice_processing_col_status()}</th>
              <th class="pb-2 text-right font-medium">{m.voice_processing_col_actions()}</th>
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
                    <span class="text-primary">{m.voice_processing_in_use()}</span>
                  {:else}
                    <button type="button" class="text-muted-foreground transition-colors hover:text-foreground" disabled={saving} onclick={() => void switchPipeline(item.id)}>
                      {m.voice_processing_switch()}
                    </button>
                  {/if}
                </td>
                <td class="py-2 text-right whitespace-nowrap">
                  <Button size="xs" variant="ghost" data-testid="rename-postprocess-pipeline" disabled={saving} onclick={() => void startRename(item)}>{m.voice_processing_rename()}</Button>
                  <Button size="xs" variant="ghost" class="text-destructive hover:bg-destructive/10 hover:text-destructive" data-testid="delete-postprocess-pipeline"
                    disabled={saving || pipelines.length <= 1} onclick={() => void deletePipeline(item.id)}>{m.voice_processing_delete()}</Button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <Dialog.Footer class="sm:justify-between">
        <Button variant="outline" size="sm" data-testid="add-postprocess-pipeline" disabled={saving} onclick={() => void addPipeline()}>
          <Plus class="size-3.5" />{m.voice_processing_new_pipeline()}
        </Button>
        <Button size="sm" onclick={() => (managing = false)}>{m.voice_action_done()}</Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

  <Dialog.Root bind:open={tryingOut}>
    <Dialog.Content class="ui-scope max-h-[85vh] overflow-y-auto sm:max-w-lg" showCloseButton={false}>
      <Dialog.Header>
        <div class="flex items-center gap-2">
          <Dialog.Title>{m.voice_processing_tryout_title({ name: pipelineLabel(activePipeline) })}</Dialog.Title>
          <span class="rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">{m.voice_processing_tryout_step_count({ count: activeStepCount })}</span>
        </div>
        <Dialog.Description>{m.voice_processing_tryout_desc()}</Dialog.Description>
      </Dialog.Header>

      <label class="block">
        <span class="text-xs text-muted-foreground">{m.voice_processing_tryout_input_label()}</span>
        <textarea
          data-testid="postprocess-tryout-input"
          class="{fieldClass} mt-1.5 min-h-24 w-full resize-y text-sm leading-relaxed"
          placeholder={m.voice_processing_tryout_input_placeholder()}
          bind:value={testInput}
        ></textarea>
      </label>

      {#if tryoutResult}
        <div data-testid="postprocess-tryout-result" class="rounded-xl border border-primary/25 bg-primary/6 p-3.5">
          <div class="flex items-center justify-between gap-3">
            <span class="text-xs font-semibold text-primary">{m.voice_processing_tryout_result()}</span>
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
                  {index + 1}. {step.displayName} · {step.outcome === "succeeded" ? m.voice_processing_step_succeeded() : m.voice_processing_step_skipped()}
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
        <Button variant="ghost" size="sm" data-testid="postprocess-tryout-close" onclick={() => (tryingOut = false)}>{m.voice_action_close()}</Button>
        <Button size="sm" data-testid="postprocess-tryout-run" disabled={testing || !testInput.trim() || activeStepCount === 0} onclick={() => void runTryout()}>
          {testing ? m.voice_processing_tryout_running() : m.voice_processing_tryout_run()}
        </Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>
</SettingsPage>
