<script lang="ts">
  /*
   * 文字处理设置页：后端注册表负责模块发现，前端只编辑线性 pipeline 配置。
   * 所有变更仍走 updateSettings 的事务入口，并以返回的完整 Settings 更新共享 store。
  */
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    isTauri,
    deleteConnection,
    listConnectionPresets,
    listConnections,
    listPostProcessors,
    testPostProcessing,
    upsertConnection,
    type Connection,
    type ConnectionInfo,
    type ConnectionPreset,
    type PostProcessorConfigField,
    type PostProcessorInfo,
    type PostProcessingConfig,
    type PostProcessingTestResult,
    type PostProcessStepConfig,
  } from "../../../lib/ipc";
  import Toggle from "../../../lib/components/Toggle.svelte";
  import { errText, patchSettings, settingsStore, toast } from "../../../lib/stores/ui";

  let processors = $state<PostProcessorInfo[]>([]);
  let loadingProcessors = $state(true);
  let connections = $state<ConnectionInfo[]>([]);
  let presets = $state<ConnectionPreset[]>([]);
  let editingConnection = $state<Connection | null>(null);
  let editingApiKey = $state("");
  let savingConnection = $state(false);
  let saving = $state(false);
  let adding = $state(false);
  let testInput = $state("对面打野在下路");
  let testing = $state(false);
  let tryoutResult = $state<PostProcessingTestResult | null>(null);

  const pipeline = $derived($settingsStore?.postProcessing.pipeline.steps ?? []);
  const enabled = $derived($settingsStore?.postProcessing.enabled ?? false);
  const activeStepCount = $derived(pipeline.filter((step) => step.enabled).length);
  const visibleProcessors = $derived(
    processors.filter((processor) => !processor.developerOnly || import.meta.env.DEV),
  );

  onMount(async () => {
    try {
      [processors, connections, presets] = await Promise.all([
        listPostProcessors(),
        listConnections(),
        listConnectionPresets(),
      ]);
    } catch (error) {
      toast(false, `加载文字处理模块失败：${errText(error)}`);
    } finally {
      loadingProcessors = false;
    }
  });

  function newConnectionDraft(preset: ConnectionPreset): Connection {
    return {
      id: `${preset.id}-${Date.now().toString(36)}`,
      displayName: preset.displayName,
      kind: "remote",
      provider: preset.id,
      baseUrl: preset.defaultBaseUrl,
      model: preset.defaultModel,
    };
  }

  async function saveConnection() {
    if (!editingConnection || savingConnection) return;
    savingConnection = true;
    try {
      const next = await upsertConnection(editingConnection, editingApiKey || undefined);
      settingsStore.set(next);
      connections = await listConnections();
      editingConnection = null;
      editingApiKey = "";
      toast(true, "连接已保存，密钥不会写入配置文件");
    } catch (error) {
      toast(false, `保存连接失败：${errText(error)}`);
    } finally {
      savingConnection = false;
    }
  }

  async function removeConnection(id: string) {
    if (savingConnection) return;
    savingConnection = true;
    try {
      const next = await deleteConnection(id);
      settingsStore.set(next);
      connections = await listConnections();
      if (editingConnection?.id === id) {
        editingConnection = null;
        editingApiKey = "";
      }
      toast(true, "连接已删除");
    } catch (error) {
      toast(false, `删除连接失败：${errText(error)}`);
    } finally {
      savingConnection = false;
    }
  }

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
    const needsConfiguration = processor.configFields.some((field) => field.required);
    const step: PostProcessStepConfig = {
      id: nextStepId(processor.id),
      processorId: processor.id,
      enabled: !needsConfiguration,
      config: {},
      timeoutMs: 5_000,
      onError: "required",
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
      tryoutResult = await testPostProcessing(
        testInput,
        $settingsStore.postProcessing.pipeline,
      );
    } catch (error) {
      toast(false, `试跑失败：${errText(error)}`);
    } finally {
      testing = false;
    }
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
          <h2 class="text-sm font-semibold text-kotone-cyan/90">API 连接</h2>
          <p class="mt-1 text-[11px] text-white/45">
            润色和翻译引用这里的连接。API key 进系统凭据库，不会写入 config.json。
          </p>
        </div>
      </div>

      <div class="mt-3 flex flex-wrap gap-2">
        {#each presets as preset}
          <button
            data-testid={`connection-preset-${preset.id}`}
            class="rounded-lg bg-white/6 px-2.5 py-1.5 text-[11px] text-white/70 ring-1 ring-white/12 transition hover:bg-white/12 disabled:opacity-50"
            disabled={savingConnection}
            onclick={() => {
              editingConnection = newConnectionDraft(preset);
              editingApiKey = "";
            }}
          >
            + {preset.displayName}
          </button>
        {/each}
      </div>

      {#if connections.length === 0 && !editingConnection}
        <p class="mt-3 rounded-xl border border-dashed border-white/15 px-4 py-6 text-center text-[11px] text-white/40">
          还没有连接。先添加一条在线接口，再把它绑到处理步骤上。
        </p>
      {/if}

      {#if connections.length > 0}
        <div class="mt-3 flex flex-col gap-2">
          {#each connections as connection (connection.id)}
            <article
              data-testid={`connection-card-${connection.id}`}
              class="rounded-xl bg-white/4 px-3 py-2.5 ring-1 ring-white/10"
            >
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <p class="truncate text-xs font-semibold text-white/85">{connection.displayName}</p>
                  <p class="mt-0.5 truncate text-[10px] text-white/40">
                    {connection.model} · {connection.hasApiKey ? "已保存密钥" : "缺少密钥"}
                  </p>
                </div>
                <div class="flex shrink-0 gap-1">
                  <button
                    class="rounded-md bg-white/7 px-2 py-1 text-[11px] text-white/65 ring-1 ring-white/10 hover:bg-white/12"
                    onclick={() => {
                      editingConnection = { ...connection };
                      editingApiKey = "";
                    }}
                  >编辑</button>
                  <button
                    class="rounded-md bg-kotone-pink/8 px-2 py-1 text-[11px] text-kotone-pink/80 ring-1 ring-kotone-pink/15 hover:bg-kotone-pink/16"
                    disabled={savingConnection}
                    onclick={() => void removeConnection(connection.id)}
                  >删除</button>
                </div>
              </div>
            </article>
          {/each}
        </div>
      {/if}

      {#if editingConnection}
        <div class="kotone-panel mt-3 space-y-2.5 p-4" data-testid="connection-editor">
          <label class="block">
            <span class="text-[11px] text-white/55">显示名称</span>
            <input
              class="mt-1 w-full rounded-md bg-white/6 px-2.5 py-1.5 text-[12px] text-white/85 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/45"
              bind:value={editingConnection.displayName}
            />
          </label>
          <label class="block">
            <span class="text-[11px] text-white/55">接口地址</span>
            <input
              class="mt-1 w-full rounded-md bg-white/6 px-2.5 py-1.5 text-[12px] text-white/85 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/45"
              bind:value={editingConnection.baseUrl}
            />
          </label>
          <label class="block">
            <span class="text-[11px] text-white/55">模型</span>
            <input
              class="mt-1 w-full rounded-md bg-white/6 px-2.5 py-1.5 text-[12px] text-white/85 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/45"
              bind:value={editingConnection.model}
            />
          </label>
          <label class="block">
            <span class="text-[11px] text-white/55">API key</span>
            <input
              type="password"
              autocomplete="off"
              class="mt-1 w-full rounded-md bg-white/6 px-2.5 py-1.5 text-[12px] text-white/85 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/45"
              placeholder={connections.some((item) => item.id === editingConnection?.id && item.hasApiKey)
                ? "已保存，留空则不修改"
                : "不会写入配置文件"}
              bind:value={editingApiKey}
            />
          </label>
          <div class="flex justify-end gap-2 pt-1">
            <button
              class="rounded-md bg-white/7 px-3 py-1.5 text-[11px] text-white/65 ring-1 ring-white/12"
              onclick={() => {
                editingConnection = null;
                editingApiKey = "";
              }}
            >取消</button>
            <button
              data-testid="connection-save"
              class="rounded-md bg-kotone-cyan px-3 py-1.5 text-[11px] font-semibold text-kotone-deep disabled:opacity-40"
              disabled={savingConnection}
              onclick={() => void saveConnection()}
            >保存连接</button>
          </div>
        </div>
      {/if}
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
              class="kotone-panel p-4 transition {step.enabled ? '' : 'opacity-75'}"
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
                  onchange={(value) => void setStepEnabled(index, value)}
                />
              </div>

              {#if processor && processor.configFields.length > 0}
                <div class="mt-3 rounded-lg bg-white/4 p-3 ring-1 ring-white/10">
                  {#each processor.configFields as field (field.key)}
                    <div>
                      <div class="flex items-center gap-1.5 text-[11px] font-medium text-white/65">
                        <span>{field.displayName}</span>
                        {#if field.required}
                          <span class="text-kotone-pink/80">必填</span>
                        {/if}
                      </div>
                      <div class="mt-1.5 flex gap-2">
                        <input
                          data-testid={`postprocess-config-${step.id}-${field.key}`}
                          aria-label={field.displayName}
                          class="min-w-0 flex-1 rounded-md bg-white/6 px-2.5 py-1.5 text-[11px] text-white/80 ring-1 ring-white/12 outline-none transition placeholder:text-white/25 focus:ring-kotone-cyan/45 disabled:opacity-50"
                          placeholder={field.kind === "file"
                            ? `选择或输入${field.displayName}路径`
                            : `输入${field.displayName}`}
                          value={configValue(step.config, field.key)}
                          disabled={saving}
                          onchange={(event) =>
                            void updateConfigField(
                              index,
                              field,
                              (event.target as HTMLInputElement).value.trim(),
                            )}
                        />
                        {#if field.kind === "file" && isTauri}
                          <button
                            class="shrink-0 rounded-md bg-white/9 px-2.5 py-1.5 text-[11px] text-white/70 ring-1 ring-white/12 transition hover:bg-white/15 disabled:opacity-50"
                            disabled={saving}
                            onclick={() => void chooseConfigFile(index, field)}
                          >选择文件</button>
                        {/if}
                      </div>
                      <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">
                        {field.description}
                      </p>
                    </div>
                  {/each}
                  {#if !requiredConfigComplete(processor, step.config)}
                    <p class="mt-2 text-[10px] text-kotone-pink/75">
                      完成必填配置后，这个步骤会自动启用。
                    </p>
                  {/if}
                </div>
              {/if}

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

    <section class="kotone-panel mt-4 p-4">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-sm font-semibold text-kotone-cyan/90">试跑</h2>
          <p class="mt-1 text-[11px] text-white/45">
            只预览处理结果，不写入历史，也不会发送到当前窗口。
          </p>
        </div>
        <span class="rounded-full bg-white/6 px-2 py-1 text-[10px] text-white/45 ring-1 ring-white/10">
          {activeStepCount} 个启用步骤
        </span>
      </div>

      <label class="mt-3 block">
        <span class="text-[11px] text-white/55">输入一段文本</span>
        <textarea
          data-testid="postprocess-tryout-input"
          class="kotone-scroll mt-1.5 min-h-20 w-full resize-y rounded-lg bg-white/5 px-3 py-2 text-sm leading-relaxed text-white/85 ring-1 ring-white/12 outline-none transition placeholder:text-white/25 focus:ring-kotone-cyan/45"
          placeholder="输入用于验证流程的文字"
          bind:value={testInput}
        ></textarea>
      </label>

      <div class="mt-3 flex items-center justify-between gap-3">
        <p class="text-[10px] text-white/35">
          总开关关闭时仍可试跑，方便先调好流程再启用。
        </p>
        <button
          data-testid="postprocess-tryout-run"
          class="rounded-lg bg-kotone-cyan px-4 py-1.5 text-xs font-bold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={testing || !testInput.trim() || activeStepCount === 0}
          onclick={() => void runTryout()}
        >
          {testing ? "处理中…" : "运行试跑"}
        </button>
      </div>

      {#if tryoutResult}
        <div
          data-testid="postprocess-tryout-result"
          class="mt-4 rounded-xl bg-kotone-cyan/6 p-3 ring-1 ring-kotone-cyan/20"
        >
          <div class="flex items-center justify-between gap-3">
            <span class="text-[11px] font-semibold text-kotone-cyan/80">处理结果</span>
            <span class="text-[10px] text-white/35">{tryoutResult.durationMs} ms</span>
          </div>
          <p class="mt-2 break-all text-sm leading-relaxed text-white/90">
            {tryoutResult.finalText}
          </p>
          <div class="mt-3 flex flex-wrap gap-1.5 border-t border-white/8 pt-3">
            {#each tryoutResult.steps as step, index (step.stepId)}
              <span
                class="rounded-full px-2 py-1 text-[10px] ring-1 {step.outcome === 'succeeded'
                  ? 'bg-kotone-cyan/8 text-kotone-cyan/75 ring-kotone-cyan/20'
                  : 'bg-kotone-pink/8 text-kotone-pink/80 ring-kotone-pink/20'}"
                title={step.error ?? `${step.durationMs} ms`}
              >
                {index + 1}. {step.displayName} · {step.outcome === "succeeded" ? "完成" : "已跳过"}
              </span>
            {/each}
          </div>
        </div>
      {/if}
    </section>
  {/if}
</div>
