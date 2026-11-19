<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    activateCatalogModel,
    cancelDownload,
    deleteCatalogModel,
    downloadCatalogModel,
    getCatalogInstallGuide,
    importCatalogModel,
    isDownloadCancelled,
    isTauri,
    listCatalogModels,
    listConnections,
    openCatalogModelDir,
    openExternal,
    type CatalogModel,
    type ConnectionInfo,
    type DownloadProgress,
    type ModelCategory,
    type ModelInstallGuide,
  } from "../../../../lib/ipc";
  import { settingsStore, patchSettings, toast, toastInfo, toastWarn, errText } from "../../../../lib/stores/ui";
  import SelectField from "$lib/components/settings/SelectField.svelte";

  let { category }: { category: Exclude<ModelCategory, "asr"> } = $props();

  let models = $state<CatalogModel[] | null>(null);
  let connections = $state<ConnectionInfo[]>([]);
  let dlProgress = $state<Record<string, number | null>>({});
  let dlErrors = $state<Record<string, string>>({});
  let confirmingDelete = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let guide = $state<ModelInstallGuide | null>(null);

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);
  const title = $derived(category === "ocr" ? "文字识别模型" : category === "search" ? "联网搜索" : "语音合成模型");

  onMount(() => {
    let un: (() => void) | undefined;
    void (async () => {
      await reload();
      if (!isTauri) return;
      un = await listen<DownloadProgress>("kotone://download", (ev) => {
        const { id, downloaded, total } = ev.payload;
        if (id in dlProgress) {
          dlProgress[id] = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
        }
      });
    })();
    return () => un?.();
  });

  async function reload() {
    try {
      const [list, cn] = await Promise.all([
        listCatalogModels(category),
        listConnections().catch(() => [] as ConnectionInfo[]),
      ]);
      models = list;
      connections = cn;
    } catch (e) {
      models = [];
      toast(false, `加载${title}失败：${errText(e)}`);
    }
  }

  async function onDownload(m: CatalogModel) {
    if (downloadingAny) return;
    delete dlErrors[m.id];
    dlProgress[m.id] = null;
    try {
      await downloadCatalogModel(category, m.id);
      toast(true, `下载完成：${m.displayName}`);
    } catch (e) {
      if (isDownloadCancelled(e) || errText(e).includes("已取消")) {
        toastInfo("已取消下载，可随时继续");
      } else {
        dlErrors[m.id] = errText(e);
        toast(false, "下载失败，可调整下载源后重试，或手动下载后导入");
      }
    } finally {
      delete dlProgress[m.id];
      await reload();
    }
  }

  async function onImport(m: CatalogModel) {
    const dir = await openDialog({ directory: true, title: `选择包含 ${m.displayName} 文件的文件夹` }).catch(() => null);
    if (!dir) return;
    busy = m.id;
    try {
      await importCatalogModel(category, m.id, dir);
      delete dlErrors[m.id];
      toast(true, "已导入并通过校验");
    } catch (e) {
      toast(false, `导入失败：${errText(e)}`);
    } finally {
      busy = null;
      await reload();
    }
  }

  async function onShowGuide(m: CatalogModel) {
    try {
      guide = await getCatalogInstallGuide(category, m.id);
    } catch (e) {
      toast(false, errText(e));
    }
  }

  async function onActivate(m: CatalogModel) {
    if (m.active || busy) return;
    busy = m.id;
    try {
      const next = await activateCatalogModel(category, m.id);
      settingsStore.set(next);
      toast(true, `已切换为：${m.displayName}`);
      if (m.dataEgress === "selected-image") {
        toastWarn("在线文字识别每次都会上传截图");
      }
    } catch (e) {
      toast(false, `切换失败：${errText(e)}`);
    } finally {
      busy = null;
      await reload();
    }
  }

  async function onDelete(m: CatalogModel) {
    busy = m.id;
    confirmingDelete = null;
    try {
      const outcome = await deleteCatalogModel(category, m.id);
      settingsStore.set(outcome.settings);
      toast(true, outcome.wasActive ? `已删除：${m.displayName}（已改回默认）` : `已删除：${m.displayName}`);
    } catch (e) {
      toast(false, `删除失败：${errText(e)}`);
    } finally {
      busy = null;
      await reload();
    }
  }

  function fieldValue(modelId: string, key: string, fallback: string): string {
    const stored = $settingsStore?.modelConfigs?.[modelId]?.[key];
    return typeof stored === "string" ? stored : fallback;
  }

  async function saveField(modelId: string, key: string, value: string) {
    const current = $settingsStore?.modelConfigs?.[modelId] ?? {};
    await patchSettings({ modelConfigs: { [modelId]: { ...current, [key]: value } } }, "模型配置已保存");
    await reload();
  }

  function formatSize(bytes: number): string {
    if (bytes <= 0) return "无需下载";
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
  }

  function sourceLabel(m: CatalogModel): string {
    if (m.backend === "remote") return m.dataEgress === "selected-image" ? "在线 · 会上传截图" : "在线";
    return m.sizeBytes > 0 ? "本机" : "系统自带";
  }

  function stateLabel(m: CatalogModel): string {
    if (m.state === "invalid") return "文件校验失败，请重新下载";
    if (m.state === "not-configured") return m.reason ?? "尚未配置";
    if (m.state === "unavailable") return m.reason ?? "当前不可用";
    return "";
  }
</script>

<section class="rounded-2xl border border-hairline bg-card/70 p-2" data-testid="catalog-list-{category}">
  <h2 class="m-0 px-2 pt-1.5 text-sm font-semibold text-foreground">{title}</h2>
  {#if category === "ocr"}
    <p class="mx-2 mt-1 mb-2 text-xs leading-relaxed text-muted-foreground">
      每次呼叫都会截取游戏窗口，用这里选中的模型识别文字。选在线模型时，<b class="font-medium text-brand-pink">每次识别都会把截图上传</b>到服务商。
    </p>
  {:else if category === "search"}
    <p class="mx-2 mt-1 mb-2 text-xs leading-relaxed text-muted-foreground">
      游戏一两周一个版本，模型自己记得的出装、强化、打法多半过时了；联网后她会先搜再答，并在聊天里列出来源。<b class="font-medium text-brand-pink">搜索词和要读的网址会发给服务商</b>。
    </p>
  {:else}
    <p class="mx-2 mt-1 mb-2 text-xs leading-relaxed text-muted-foreground">陪玩回答与提醒的播报声音。外语聊天会先翻译成中文再播报。</p>
  {/if}
  {#if models === null}
    <p class="mx-2 my-3 text-xs text-muted-foreground">读取中…</p>
  {:else}
    <div class="flex flex-col gap-1" role="radiogroup" aria-label={title}>
      {#each models as m (m.id)}
        {@const usable = m.downloaded}
        <div
          role="radio"
          aria-checked={m.active}
          tabindex="0"
          class="rounded-xl border px-3 py-2.5 transition-colors
            {m.active
            ? 'border-primary/50 bg-primary/8'
            : usable
              ? 'cursor-pointer border-transparent hover:bg-muted/60'
              : 'border-transparent'}"
          onclick={() => usable && void onActivate(m)}
          onkeydown={(e) => {
            if (e.key === "Enter" && usable) void onActivate(m);
          }}
        >
          <div class="flex items-center gap-2.5">
            <span
              class="flex size-4 shrink-0 items-center justify-center rounded-full border-2 {m.active
                ? 'border-primary'
                : 'border-muted-foreground/40'}"
            >
              {#if m.active}<span class="size-1.5 rounded-full bg-primary"></span>{/if}
            </span>
            <div class="min-w-0 flex-1">
              <p class="m-0 truncate text-[13px] {m.active ? 'font-medium text-primary' : usable ? 'text-foreground' : 'text-muted-foreground'}">
                {m.displayName}{m.recommended ? "（推荐）" : ""}
              </p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {formatSize(m.sizeBytes)} · {sourceLabel(m)} · {m.description}
              </p>
              {#if stateLabel(m)}
                <p class="mt-0.5 mb-0 text-[11px] text-warning">{stateLabel(m)}</p>
              {/if}
            </div>
            {#if m.active}
              <span class="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">使用中</span>
            {/if}
            {#if m.backend === "local" && m.sizeBytes > 0}
              {#if !m.downloaded}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md border border-hairline px-2 text-xs text-foreground transition-colors hover:bg-muted disabled:opacity-50"
                  disabled={downloadingAny || busy !== null}
                  onclick={(e) => {
                    e.stopPropagation();
                    void onDownload(m);
                  }}
                >
                  {m.id in dlProgress ? "下载中…" : "下载"}
                </button>
                {#if category === "ocr"}
                  <button
                    class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
                    disabled={downloadingAny || busy !== null}
                    title="已手动下载模型文件时，从文件夹导入（逐文件校验）"
                    onclick={(e) => {
                      e.stopPropagation();
                      void onImport(m);
                    }}
                  >
                    导入
                  </button>
                {/if}
              {:else if confirmingDelete === m.id}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md bg-destructive/15 px-2 text-xs font-medium text-destructive transition-colors hover:bg-destructive/25 disabled:opacity-50"
                  disabled={busy !== null}
                  onclick={(e) => {
                    e.stopPropagation();
                    void onDelete(m);
                  }}
                >
                  确认删除
                </button>
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground hover:bg-muted"
                  onclick={(e) => {
                    e.stopPropagation();
                    confirmingDelete = null;
                  }}
                >
                  取消
                </button>
              {:else}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                  onclick={(e) => {
                    e.stopPropagation();
                    confirmingDelete = m.id;
                  }}
                >
                  删除
                </button>
              {/if}
            {/if}
          </div>
          {#if m.id in dlProgress}
            <div class="mt-2 flex items-center gap-2 pl-6.5">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                {#if dlProgress[m.id] !== null}
                  <div class="h-full rounded-full bg-primary transition-[width]" style:width="{dlProgress[m.id]}%"></div>
                {:else}
                  <div class="h-full w-1/3 animate-pulse rounded-full bg-primary/70"></div>
                {/if}
              </div>
              <button
                class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground hover:bg-muted"
                onclick={(e) => {
                  e.stopPropagation();
                  void cancelDownload();
                }}
              >
                取消
              </button>
            </div>
          {/if}
          {#if dlErrors[m.id]}
            <div class="mt-2 ml-6.5 flex items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2">
              <p class="m-0 min-w-0 flex-1 text-[11px] leading-relaxed break-all text-destructive">下载失败：{dlErrors[m.id]}</p>
              {#if category === "ocr"}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md border border-hairline px-2 text-xs text-foreground transition-colors hover:bg-muted disabled:opacity-50"
                  onclick={(e) => {
                    e.stopPropagation();
                    void onShowGuide(m);
                  }}
                >
                  手动下载
                </button>
              {/if}
            </div>
          {/if}
          {#if m.configSchema.length > 0 && (m.active || m.state === "not-configured")}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="mt-2 rounded-xl border border-hairline bg-muted/40 p-3" onclick={(e) => e.stopPropagation()}>
              <div class="flex items-center justify-between gap-2">
                <p class="m-0 text-xs font-medium text-foreground">模型配置</p>
                {#if m.apiKeyUrl}
                  <button type="button" class="text-xs font-medium text-primary hover:underline" onclick={() => void openExternal(m.apiKeyUrl!)}>
                    获取 API Key
                  </button>
                {/if}
              </div>
              <div class="mt-2 flex flex-col gap-2.5">
                {#each m.configSchema as field}
                  <label class="block">
                    <span class="text-xs text-muted-foreground">{field.label}</span>
                    {#if field.kind === "enum"}
                      <div class="mt-1">
                        <SelectField
                          ariaLabel={field.label}
                          value={fieldValue(m.id, field.key, field.default)}
                          options={field.options.map((option) => ({ value: option.value, label: option.label }))}
                          onchange={(next) => void saveField(m.id, field.key, next)}
                        />
                      </div>
                    {:else if field.kind === "connection"}
                      <div class="mt-1">
                        <SelectField
                          ariaLabel={field.label}
                          placeholder="请选择 API 连接"
                          value={fieldValue(m.id, field.key, field.default)}
                          options={[
                            { value: "", label: "请选择 API 连接" },
                            ...connections.map((c) => ({
                              value: c.id,
                              label: `${c.displayName}${c.hasApiKey ? "" : "（未填密钥）"}`,
                            })),
                          ]}
                          onchange={(next) => void saveField(m.id, field.key, next)}
                        />
                      </div>
                      <p class="mt-1 mb-0 text-[11px] text-muted-foreground/80">地址和 API key 在「设置 → API 连接」里保存，密钥只存在系统凭据管理器。</p>
                    {:else}
                      <input
                        class="mt-1 h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30"
                        value={fieldValue(m.id, field.key, field.default)}
                        onchange={(e) => void saveField(m.id, field.key, (e.target as HTMLInputElement).value)}
                      />
                    {/if}
                  </label>
                {/each}
              </div>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if guide}
  <div class="ui-scope fixed inset-0 z-[110] flex items-center justify-center bg-black/50 p-6" role="dialog" aria-modal="true">
    <div class="max-h-[80vh] w-full max-w-lg overflow-y-auto rounded-2xl border border-hairline bg-popover p-5 text-popover-foreground shadow-2xl">
      <h3 class="m-0 text-[15px] font-semibold">手动下载：{guide.displayName}</h3>
      <p class="mt-2 mb-0 text-xs leading-relaxed text-muted-foreground">
        从下面的地址下载这些文件，放进同一个文件夹，再点该模型的「导入」选择那个文件夹。导入时逐文件校验大小与 SHA-256。
      </p>
      <ul class="mt-2 flex flex-col gap-1">
        {#each guide.files as f}
          <li class="font-mono text-[11px] break-all text-muted-foreground">{f.name}（{Math.round(f.sizeBytes / 1_000_000)} MB）<br />{f.url}</li>
        {/each}
      </ul>
      <div class="mt-3 flex justify-end gap-2">
        <button class="h-8 rounded-lg border border-hairline px-3 text-xs text-foreground hover:bg-muted" onclick={() => void openExternal(guide!.pageUrl)}>打开下载页</button>
        <button class="h-8 rounded-lg border border-hairline px-3 text-xs text-foreground hover:bg-muted" onclick={() => void openCatalogModelDir(category, guide!.id)}>打开安装目录</button>
        <button class="h-8 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground hover:brightness-110" onclick={() => (guide = null)}>关闭</button>
      </div>
    </div>
  </div>
{/if}
