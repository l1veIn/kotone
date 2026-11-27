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
  import { m } from "$lib/paraglide/messages.js";

  // 语音合成不在这里配了（ADR 048：她的声音只在声音与弹幕页）
  let { category }: { category: Exclude<ModelCategory, "asr" | "tts"> } = $props();

  let models = $state<CatalogModel[] | null>(null);
  let connections = $state<ConnectionInfo[]>([]);
  let dlProgress = $state<Record<string, number | null>>({});
  let dlErrors = $state<Record<string, string>>({});
  let confirmingDelete = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let guide = $state<ModelInstallGuide | null>(null);

  const downloadingAny = $derived(Object.keys(dlProgress).length > 0);
  const title = $derived(category === "ocr" ? m.system_catalog_title_ocr() : m.system_catalog_title_search());

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
      toast(false, m.system_catalog_load_failed({ name: title, error: errText(e) }));
    }
  }

  async function onDownload(model: CatalogModel) {
    if (downloadingAny) return;
    delete dlErrors[model.id];
    dlProgress[model.id] = null;
    try {
      await downloadCatalogModel(category, model.id);
      toast(true, m.system_catalog_download_done({ name: model.displayName }));
    } catch (e) {
      // i18n-ignore-start：匹配后端的错误文字
      if (isDownloadCancelled(e) || errText(e).includes("已取消")) {
      // i18n-ignore-end
        toastInfo(m.system_catalog_download_cancelled());
      } else {
        dlErrors[model.id] = errText(e);
        toast(false, m.system_catalog_download_failed_toast());
      }
    } finally {
      delete dlProgress[model.id];
      await reload();
    }
  }

  async function onImport(model: CatalogModel) {
    const dir = await openDialog({ directory: true, title: m.system_catalog_import_dialog_title({ name: model.displayName }) }).catch(() => null);
    if (!dir) return;
    busy = model.id;
    try {
      await importCatalogModel(category, model.id, dir);
      delete dlErrors[model.id];
      toast(true, m.system_catalog_import_done());
    } catch (e) {
      toast(false, m.system_catalog_import_failed({ error: errText(e) }));
    } finally {
      busy = null;
      await reload();
    }
  }

  async function onShowGuide(model: CatalogModel) {
    try {
      guide = await getCatalogInstallGuide(category, model.id);
    } catch (e) {
      toast(false, errText(e));
    }
  }

  async function onActivate(model: CatalogModel) {
    if (model.active || busy) return;
    busy = model.id;
    try {
      const next = await activateCatalogModel(category, model.id);
      settingsStore.set(next);
      toast(true, m.system_catalog_activated({ name: model.displayName }));
      if (model.dataEgress === "selected-image") {
        toastWarn(m.system_catalog_ocr_upload_warning());
      }
    } catch (e) {
      toast(false, m.system_catalog_activate_failed({ error: errText(e) }));
    } finally {
      busy = null;
      await reload();
    }
  }

  async function onDelete(model: CatalogModel) {
    busy = model.id;
    confirmingDelete = null;
    try {
      const outcome = await deleteCatalogModel(category, model.id);
      settingsStore.set(outcome.settings);
      toast(true, outcome.wasActive ? m.system_catalog_deleted_was_active({ name: model.displayName }) : m.system_catalog_deleted({ name: model.displayName }));
    } catch (e) {
      toast(false, m.system_catalog_delete_failed({ error: errText(e) }));
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
    await patchSettings({ modelConfigs: { [modelId]: { ...current, [key]: value } } }, m.system_catalog_config_saved());
    await reload();
  }

  function formatSize(bytes: number): string {
    if (bytes <= 0) return m.system_catalog_size_none();
    if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
    return `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
  }

  function sourceLabel(model: CatalogModel): string {
    if (model.backend === "remote") return model.dataEgress === "selected-image" ? m.system_catalog_source_online_upload() : m.system_catalog_source_online();
    return model.sizeBytes > 0 ? m.system_catalog_source_local() : m.system_catalog_source_builtin();
  }

  function stateLabel(model: CatalogModel): string {
    if (model.state === "invalid") return m.system_catalog_state_invalid();
    if (model.state === "not-configured") return model.reason ?? m.system_catalog_state_not_configured();
    if (model.state === "unavailable") return model.reason ?? m.system_catalog_state_unavailable();
    return "";
  }
</script>

<section class="rounded-2xl border border-hairline bg-card/70 p-2" data-testid="catalog-list-{category}">
  <h2 class="m-0 px-2 pt-1.5 text-sm font-semibold text-foreground">{title}</h2>
  {#if category === "ocr"}
    <p class="mx-2 mt-1 mb-2 text-xs leading-relaxed text-muted-foreground">
      {m.system_catalog_ocr_intro_before()}<b class="font-medium text-brand-pink">{m.system_catalog_ocr_intro_strong()}</b>{m.system_catalog_ocr_intro_after()}
    </p>
  {:else}
    <p class="mx-2 mt-1 mb-2 text-xs leading-relaxed text-muted-foreground">
      {m.system_catalog_search_intro_before()}<b class="font-medium text-brand-pink">{m.system_catalog_search_intro_strong()}</b>{m.system_catalog_search_intro_after()}
    </p>
  {/if}
  {#if models === null}
    <p class="mx-2 my-3 text-xs text-muted-foreground">{m.system_catalog_loading()}</p>
  {:else}
    <div class="flex flex-col gap-1" role="radiogroup" aria-label={title}>
      {#each models as model (model.id)}
        {@const usable = model.downloaded}
        <div
          role="radio"
          aria-checked={model.active}
          tabindex="0"
          class="rounded-xl border px-3 py-2.5 transition-colors
            {model.active
            ? 'border-primary/50 bg-primary/8'
            : usable
              ? 'cursor-pointer border-transparent hover:bg-muted/60'
              : 'border-transparent'}"
          onclick={() => usable && void onActivate(model)}
          onkeydown={(e) => {
            if (e.key === "Enter" && usable) void onActivate(model);
          }}
        >
          <div class="flex items-center gap-2.5">
            <span
              class="flex size-4 shrink-0 items-center justify-center rounded-full border-2 {model.active
                ? 'border-primary'
                : 'border-muted-foreground/40'}"
            >
              {#if model.active}<span class="size-1.5 rounded-full bg-primary"></span>{/if}
            </span>
            <div class="min-w-0 flex-1">
              <p class="m-0 truncate text-[13px] {model.active ? 'font-medium text-primary' : usable ? 'text-foreground' : 'text-muted-foreground'}">
                {model.displayName}{model.recommended ? m.system_catalog_recommended_suffix() : ""}
              </p>
              <p class="mt-0.5 mb-0 text-[11px] text-muted-foreground">
                {formatSize(model.sizeBytes)} · {sourceLabel(model)} · {model.description}
              </p>
              {#if stateLabel(model)}
                <p class="mt-0.5 mb-0 text-[11px] text-warning">{stateLabel(model)}</p>
              {/if}
            </div>
            {#if model.active}
              <span class="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">{m.system_catalog_active_badge()}</span>
            {/if}
            {#if model.backend === "local" && model.sizeBytes > 0}
              {#if !model.downloaded}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md border border-hairline px-2 text-xs text-foreground transition-colors hover:bg-muted disabled:opacity-50"
                  disabled={downloadingAny || busy !== null}
                  onclick={(e) => {
                    e.stopPropagation();
                    void onDownload(model);
                  }}
                >
                  {model.id in dlProgress ? m.system_catalog_downloading() : m.system_catalog_download_button()}
                </button>
                {#if category === "ocr"}
                  <button
                    class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
                    disabled={downloadingAny || busy !== null}
                    title={m.system_catalog_import_tooltip()}
                    onclick={(e) => {
                      e.stopPropagation();
                      void onImport(model);
                    }}
                  >
                    {m.system_catalog_import_button()}
                  </button>
                {/if}
              {:else if confirmingDelete === model.id}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md bg-destructive/15 px-2 text-xs font-medium text-destructive transition-colors hover:bg-destructive/25 disabled:opacity-50"
                  disabled={busy !== null}
                  onclick={(e) => {
                    e.stopPropagation();
                    void onDelete(model);
                  }}
                >
                  {m.system_catalog_delete_confirm()}
                </button>
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground hover:bg-muted"
                  onclick={(e) => {
                    e.stopPropagation();
                    confirmingDelete = null;
                  }}
                >
                  {m.system_common_cancel()}
                </button>
              {:else}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                  onclick={(e) => {
                    e.stopPropagation();
                    confirmingDelete = model.id;
                  }}
                >
                  {m.system_common_delete()}
                </button>
              {/if}
            {/if}
          </div>
          {#if model.id in dlProgress}
            <div class="mt-2 flex items-center gap-2 pl-6.5">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                {#if dlProgress[model.id] !== null}
                  <div class="h-full rounded-full bg-primary transition-[width]" style:width="{dlProgress[model.id]}%"></div>
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
                {m.system_common_cancel()}
              </button>
            </div>
          {/if}
          {#if dlErrors[model.id]}
            <div class="mt-2 ml-6.5 flex items-start gap-2 rounded-lg border border-destructive/35 bg-destructive/8 px-2.5 py-2">
              <p class="m-0 min-w-0 flex-1 text-[11px] leading-relaxed break-all text-destructive">{m.system_catalog_download_failed({ error: dlErrors[model.id] })}</p>
              {#if category === "ocr"}
                <button
                  class="inline-flex h-6 shrink-0 items-center rounded-md border border-hairline px-2 text-xs text-foreground transition-colors hover:bg-muted disabled:opacity-50"
                  onclick={(e) => {
                    e.stopPropagation();
                    void onShowGuide(model);
                  }}
                >
                  {m.system_catalog_manual_download_button()}
                </button>
              {/if}
            </div>
          {/if}
          {#if model.configSchema.length > 0 && (model.active || model.state === "not-configured")}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="mt-2 rounded-xl border border-hairline bg-muted/40 p-3" onclick={(e) => e.stopPropagation()}>
              <div class="flex items-center justify-between gap-2">
                <p class="m-0 text-xs font-medium text-foreground">{m.system_catalog_config_title()}</p>
                {#if model.apiKeyUrl}
                  <button type="button" class="text-xs font-medium text-primary hover:underline" onclick={() => void openExternal(model.apiKeyUrl!)}>
                    {m.system_catalog_get_api_key()}
                  </button>
                {/if}
              </div>
              <div class="mt-2 flex flex-col gap-2.5">
                {#each model.configSchema as field}
                  <label class="block">
                    <span class="text-xs text-muted-foreground">{field.label}</span>
                    {#if field.kind === "enum"}
                      <div class="mt-1">
                        <SelectField
                          ariaLabel={field.label}
                          value={fieldValue(model.id, field.key, field.default)}
                          options={field.options.map((option) => ({ value: option.value, label: option.label }))}
                          onchange={(next) => void saveField(model.id, field.key, next)}
                        />
                      </div>
                    {:else if field.kind === "connection"}
                      <div class="mt-1">
                        <SelectField
                          ariaLabel={field.label}
                          placeholder={m.system_catalog_connection_placeholder()}
                          value={fieldValue(model.id, field.key, field.default)}
                          options={[
                            { value: "", label: m.system_catalog_connection_placeholder() },
                            ...connections.map((c) => ({
                              value: c.id,
                              label: `${c.displayName}${c.hasApiKey ? "" : m.system_catalog_connection_no_key_suffix()}`,
                            })),
                          ]}
                          onchange={(next) => void saveField(model.id, field.key, next)}
                        />
                      </div>
                      <p class="mt-1 mb-0 text-[11px] text-muted-foreground/80">{m.system_catalog_connection_hint()}</p>
                    {:else}
                      <input
                        class="mt-1 h-8 w-full rounded-lg border border-input bg-background/60 px-2.5 text-[13px] text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/30"
                        value={fieldValue(model.id, field.key, field.default)}
                        onchange={(e) => void saveField(model.id, field.key, (e.target as HTMLInputElement).value)}
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
      <h3 class="m-0 text-[15px] font-semibold">{m.system_catalog_guide_title({ name: guide.displayName })}</h3>
      <p class="mt-2 mb-0 text-xs leading-relaxed text-muted-foreground">
        {m.system_catalog_guide_body()}
      </p>
      <ul class="mt-2 flex flex-col gap-1">
        {#each guide.files as f}
          <li class="font-mono text-[11px] break-all text-muted-foreground">{m.system_catalog_guide_file({ name: f.name, size: Math.round(f.sizeBytes / 1_000_000) })}<br />{f.url}</li>
        {/each}
      </ul>
      <div class="mt-3 flex justify-end gap-2">
        <button class="h-8 rounded-lg border border-hairline px-3 text-xs text-foreground hover:bg-muted" onclick={() => void openExternal(guide!.pageUrl)}>{m.system_common_open_download_page()}</button>
        <button class="h-8 rounded-lg border border-hairline px-3 text-xs text-foreground hover:bg-muted" onclick={() => void openCatalogModelDir(category, guide!.id)}>{m.system_catalog_guide_open_dir()}</button>
        <button class="h-8 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground hover:brightness-110" onclick={() => (guide = null)}>{m.system_common_close()}</button>
      </div>
    </div>
  </div>
{/if}
