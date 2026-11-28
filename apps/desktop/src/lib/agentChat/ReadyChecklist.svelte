<script lang="ts">
  /*
   * 聊天页最上面的“准备好了吗”（ADR 051）：英雄联盟玩家从装好到打完一局要的几样，一行一样，✓ 或 ✗；
   * 每个 ✗ 一个按钮，当场改好或跳到改它的地方。全都 ✓ 就不显示。
   * 还没有能用的 API 连接时聊天页是 Key 卡片（那就是第一步），这一块只在她能回答以后出现（由聊天页决定）。
   * 状态都用现成的：API 连接清单、设置、`companion_status`（文字识别）、`get_elevation_status`、插件开关。
   */
  import { onMount, untrack } from "svelte";
  import { Check, X } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { m } from "$lib/paraglide/messages.js";
  import { companionName } from "$lib/stores/companion";
  import {
    activateCatalogModel, companionStatus, getElevationStatus, restartAsAdmin, updateSettings,
    type CompanionStatus, type ConnectionInfo, type ElevationStatus,
  } from "../ipc";
  import { isDeepSeekConnection, switchToDeepSeek } from "../deepseekSetup";
  import { hotkeyLabel } from "../hotkeyCombo";
  import { pluginEnabled, pluginName, pluginsStore, setPluginEnabled, setPluginSource, sourceEnabled, sourceName } from "../plugins";
  import { openTarget } from "../stores/navigation";
  import { errText, settingsStore, toast } from "../stores/ui";

  let { connections, onOpenTalk }: {
    /** API 连接清单（和输入框旁的模型菜单同一份）。 */
    connections: ConnectionInfo[];
    /** 角色页「唤醒」的呼叫键。 */
    onOpenTalk: () => void;
  } = $props();

  type Row = { id: string; ok: boolean; text: string; fix?: { label: string; run: () => unknown } };

  let status = $state<CompanionStatus | null>(null);
  let elevation = $state<ElevationStatus | null>(null);
  let busy = $state<string | null>(null);

  onMount(() => { void getElevationStatus().then((value) => { elevation = value; }).catch(() => {}); });
  // 设置变了（填 Key、换连接、换文字识别）：重新读一次文字识别的状态
  $effect(() => {
    $settingsStore;
    untrack(() => void companionStatus().then((value) => { status = value; }).catch(() => {}));
  });

  const name = $derived($companionName);
  const deepseek = $derived(connections.find((c) => c.kind === "remote" && c.hasApiKey && isDeepSeekConnection(c)) ?? null);
  const current = $derived(connections.find((c) => c.id === $settingsStore?.assistant.connectionId) ?? null);
  const callKey = $derived($settingsStore?.assistant.hotkey.trim() ?? "");
  const lol = $derived($pluginsStore.find((p) => p.manifest.id === "lol") ?? null);

  async function fix(id: string, run: () => unknown) {
    if (busy) return;
    busy = id;
    try { await run(); }
    catch (error) { toast(false, m.chat_ready_fix_failed({ error: errText(error) })); }
    finally { busy = null; }
  }

  const rows = $derived.by<Row[]>(() => {
    const settings = $settingsStore;
    const list: Row[] = [];
    // DeepSeek Key；没有 DeepSeek 连接时“她用的模型是不是 DeepSeek”并进这一行
    if (!deepseek) {
      list.push({ id: "key", ok: false, text: m.chat_ready_key_missing(), fix: { label: m.chat_ready_key_fix(), run: () => openTarget({ page: "setup-key" }) } });
    } else {
      list.push({ id: "key", ok: true, text: m.chat_ready_key_ok() });
      const onDeepSeek = !!current && isDeepSeekConnection(current);
      list.push(onDeepSeek
        ? { id: "model", ok: true, text: m.chat_ready_model_ok({ model: current!.model }) }
        : {
          id: "model", ok: false, text: m.chat_ready_model_other({ model: current?.model ?? "" }),
          fix: { label: m.chat_ready_model_fix(), run: async () => { settingsStore.set((await switchToDeepSeek(deepseek.id, settings)).settings); } },
        });
    }
    list.push(callKey
      ? { id: "call", ok: true, text: m.chat_ready_call_ok({ key: hotkeyLabel(callKey) }) }
      : { id: "call", ok: false, text: m.chat_ready_call_missing({ name }), fix: { label: m.chat_ready_call_fix(), run: onOpenTalk } });
    // 管理员身份（ADR 044）：不是 Windows 时没有这一行
    if (elevation?.supported) {
      const revision = elevation.adminPromptRevision;
      list.push(elevation.elevated
        ? { id: "admin", ok: true, text: m.chat_ready_admin_ok() }
        : {
          id: "admin", ok: false, text: m.chat_ready_admin_missing(),
          fix: {
            label: m.system_compat_restart_as_admin(),
            // 以后打开也用管理员身份（默认就是这样；玩家关过的这里打开）
            run: async () => { settingsStore.set(await updateSettings({ runAsAdminOnStart: true, adminPromptRevision: revision })); await restartAsAdmin(); },
          },
        });
    }
    if (lol) {
      const plugin = pluginName(lol.manifest);
      const client = lol.manifest.sources.find((s) => s.id === "client");
      const on = pluginEnabled(settings, lol.manifest);
      const clientOn = !client || sourceEnabled(settings, lol.manifest, "client");
      list.push(on && clientOn
        ? { id: "lol", ok: true, text: m.chat_ready_plugin_ok({ plugin }) }
        : {
          id: "lol", ok: false,
          text: !on ? m.chat_ready_plugin_off({ plugin }) : m.chat_ready_source_off({ plugin, source: sourceName(lol.manifest, client!) }),
          fix: {
            label: m.chat_ready_plugin_fix(),
            run: async () => {
              if (!on) await setPluginEnabled("lol", true);
              if (client && !sourceEnabled($settingsStore, lol.manifest, "client")) await setPluginSource("lol", "client", true);
            },
          },
        });
    }
    if (status) {
      list.push(status.ocr.state.ready
        ? { id: "ocr", ok: true, text: m.chat_ready_ocr_ok() }
        : {
          id: "ocr", ok: false, text: m.chat_ready_ocr_missing({ name }),
          fix: deepseek
            ? { label: m.chat_ready_ocr_fix_deepseek(), run: async () => { settingsStore.set(await activateCatalogModel("ocr", "deepseek-ocr")); } }
            : { label: m.chat_ready_ocr_fix_open(), run: () => openTarget({ page: "advanced", tab: "models", anchor: "models-ocr" }) },
        });
    }
    return list;
  });
  const missing = $derived(rows.filter((row) => !row.ok).length);
</script>

<!-- 文字识别的状态读到之前不显示（免得先闪一下） -->
{#if status && missing > 0}
  <div class="mx-auto w-full max-w-[808px] px-6 pt-4">
    <section class="rounded-xl border border-hairline bg-card/70 px-3.5 py-2.5 text-[13px]" data-testid="chat-ready" aria-labelledby="chat-ready-title">
      <p class="m-0 mb-1.5 flex items-baseline gap-2">
        <span id="chat-ready-title" class="font-medium text-foreground">{m.chat_ready_title()}</span>
        <span class="text-xs text-muted-foreground" data-testid="chat-ready-count">{m.chat_ready_count({ count: missing })}</span>
      </p>
      <ul class="m-0 list-none space-y-1 p-0">
        {#each rows as row (row.id)}
          <li class="flex min-h-7 items-center gap-2" data-testid={`chat-ready-${row.id}`} data-ok={row.ok ? "yes" : "no"}>
            {#if row.ok}<Check class="size-3.5 shrink-0 text-success" aria-label="✓" />{:else}<X class="size-3.5 shrink-0 text-destructive" aria-label="✗" />{/if}
            <span class="min-w-0 flex-1 {row.ok ? 'text-muted-foreground' : 'text-foreground'}">{row.text}</span>
            {#if !row.ok && row.fix}
              {@const action = row.fix}
              <Button size="sm" variant="outline" class="h-7 shrink-0" data-testid={`chat-ready-${row.id}-fix`} disabled={busy !== null}
                onclick={() => void fix(row.id, action.run)}>{action.label}</Button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  </div>
{:else if status}
  <!-- 全都 ✓：什么都不显示（留一个看不见的记号给测试） -->
  <span hidden data-testid="chat-ready-all" data-rows={rows.map((row) => row.id).join(" ")}></span>
{/if}
