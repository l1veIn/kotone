<script lang="ts">
  import { onMount } from "svelte";
  import {
    deleteConnection,
    listConnectionPresets,
    listConnections,
    openExternal,
    upsertConnection,
    type Connection,
    type ConnectionInfo,
    type ConnectionPreset,
  } from "../../../../lib/ipc";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";

  let connections = $state<ConnectionInfo[]>([]);
  let presets = $state<ConnectionPreset[]>([]);
  let editingConnection = $state<Connection | null>(null);
  let editingApiKey = $state("");
  let savingConnection = $state(false);
  let loading = $state(true);

  const editingApiKeyUrl = $derived.by(() => {
    const current = editingConnection;
    if (!current) return "";
    return presets.find((preset) => preset.id === current.provider)?.apiKeyUrl ?? "";
  });

  onMount(async () => {
    try {
      [connections, presets] = await Promise.all([listConnections(), listConnectionPresets()]);
    } catch (error) {
      toast(false, `加载连接失败：${errText(error)}`);
    } finally {
      loading = false;
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

  async function openApiKeyPage(url: string) {
    try {
      await openExternal(url);
    } catch (error) {
      toast(false, `打开 API key 页面失败：${errText(error)}`);
    }
  }
</script>

<div>
  <p class="text-[11px] text-white/45">
    文字处理里的润色和翻译引用这里的连接。API key 进系统凭据库，不会写入 config.json。
  </p>

  {#if loading}
    <p class="mt-4 text-sm text-white/45">正在读取连接…</p>
  {:else}
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
        还没有连接。添加一条在线接口后，回到「文字处理」把它绑到步骤上。
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
      <div
        class="fixed inset-0 z-[80] flex items-center justify-center bg-kotone-deep/85 p-6 backdrop-blur-sm"
        role="presentation"
        onclick={() => {
          editingConnection = null;
          editingApiKey = "";
        }}
      >
        <div
          class="kotone-panel max-h-[85vh] w-full max-w-lg overflow-y-auto p-5 shadow-glow-cyan-lg"
          role="dialog"
          aria-modal="true"
          aria-labelledby="connection-editor-title"
          data-testid="connection-editor"
          onclick={(event) => event.stopPropagation()}
        >
          <div class="flex items-start justify-between gap-3">
            <div>
              <h2 id="connection-editor-title" class="text-base font-bold">
                {connections.some((item) => item.id === editingConnection?.id) ? "编辑 API 连接" : "添加 API 连接"}
              </h2>
              <p class="mt-1 text-[11px] text-white/45">API key 保存到系统凭据库，不会写入配置文件。</p>
            </div>
            <button
              class="rounded-md bg-white/8 px-2 py-1 text-[11px] text-white/70 ring-1 ring-white/12 hover:bg-white/15"
              onclick={() => {
                editingConnection = null;
                editingApiKey = "";
              }}
            >
              取消
            </button>
          </div>

          <div class="mt-4 space-y-3">
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
              <span class="flex items-center justify-between gap-2 text-[11px] text-white/55">
                <span>API key</span>
                {#if editingApiKeyUrl}
                  <button
                    type="button"
                    class="text-[11px] font-semibold text-kotone-cyan hover:underline"
                    onclick={() => void openApiKeyPage(editingApiKeyUrl)}
                  >
                    获取 API key
                  </button>
                {/if}
              </span>
              <input
                type="password"
                autocomplete="off"
                data-testid="connection-api-key"
                class="mt-1 w-full rounded-md bg-white/6 px-2.5 py-1.5 text-[12px] text-white/85 ring-1 ring-white/12 outline-none focus:ring-kotone-cyan/45"
                placeholder={connections.some((item) => item.id === editingConnection?.id && item.hasApiKey)
                  ? "已保存，留空则不修改"
                  : "不会写入配置文件"}
                bind:value={editingApiKey}
              />
            </label>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button
              class="rounded-md bg-white/7 px-3 py-1.5 text-[11px] text-white/65 ring-1 ring-white/12 hover:bg-white/12"
              onclick={() => {
                editingConnection = null;
                editingApiKey = "";
              }}
            >取消</button>
            <button
              data-testid="connection-save"
              class="rounded-md bg-kotone-cyan px-3.5 py-1.5 text-[11px] font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-40"
              disabled={savingConnection}
              onclick={() => void saveConnection()}
            >保存连接</button>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>
