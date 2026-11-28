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
    activateCatalogModel, addHotkey, checkHotkey, companionStatus, getElevationStatus, restartAsAdmin, setHotkey, updateSettings,
    type CompanionStatus, type ConnectionInfo, type ElevationStatus,
  } from "../ipc";
  import KeyCaptureButton from "../components/KeyCaptureButton.svelte";
  import { isDeepSeekConnection, switchToDeepSeek } from "../deepseekSetup";
  import { hotkeyLabel, normalizeHotkeyCombo } from "../hotkeyCombo";
  import { firstConflictName } from "../hotkeyNames";
  import { pluginEnabled, pluginName, pluginsStore, setPluginEnabled, setPluginSource, sourceEnabled, sourceName } from "../plugins";
  import { openTarget } from "../stores/navigation";
  import { errText, settingsStore, toast } from "../stores/ui";

  let { connections, onOpenTalk }: {
    /** API 连接清单（和输入框旁的模型菜单同一份）。 */
    connections: ConnectionInfo[];
    /** 角色页「唤醒」的呼叫键。 */
    onOpenTalk: () => void;
  } = $props();

  /** `pickKey`：这一行是“按一下你想用的呼叫键”（录键 + 就用现在这个，ADR 054）。 */
  type Row = { id: string; ok: boolean; text: string; fix?: { label: string; run: () => unknown }; pickKey?: boolean };

  /** 新装时的呼叫键（同后端 DEFAULT_CALL_HOTKEY）。 */
  const DEFAULT_CALL_KEY = "Ctrl+Backquote";

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
  /**
   * 呼叫键是玩家自己定的（ADR 054 第 4 条）：键不是新装的默认、或者加了第二个呼叫键、或者点过“就用这个键”
   * （`ui.callKeyChosen`，新手引导里点“用推荐的”、录过键也算）。没定之前默认键照样能用，只是这一行提醒他绑一个顺手的。
   */
  const callKeyChosen = $derived.by(() => {
    const settings = $settingsStore;
    if (!settings || !callKey) return false;
    const calls = (settings.hotkeys?.bindings ?? []).filter((b) => b.command === "kotone.call").length;
    return !!settings.ui.callKeyChosen || calls > 1 || normalizeHotkeyCombo(callKey) !== normalizeHotkeyCombo(DEFAULT_CALL_KEY);
  });

  /** 录到一个键：鼠标侧键加成只在游戏里管用的第二个呼叫键（默认键留着，桌面上侧键还是“后退”）；别的键换掉现在的呼叫键。 */
  async function pickCallKey(combo: string) {
    if (busy) return;
    busy = "call";
    try {
      const side = /(^|\+)Mouse[45]$/.test(combo);
      const when = side ? "game" : undefined;
      const conflict = firstConflictName(await checkHotkey(combo, side ? undefined : "kotone.call", when));
      if (conflict) {
        toast(false, m.voice_hotkey_conflict({ title: m.voice_hotkey_name_call(), conflict, key: hotkeyLabel(combo) }));
        return;
      }
      settingsStore.set(side ? await addHotkey("kotone.call", combo, { when }) : await setHotkey("kotone.call", combo));
      settingsStore.set(await updateSettings({ ui: { callKeyChosen: true } }));
      toast(true, side && callKey
        ? m.chat_ready_call_saved_game({ key: hotkeyLabel(combo), fallback: hotkeyLabel(callKey) })
        : m.chat_ready_call_saved({ key: hotkeyLabel(combo) }));
    } catch (error) {
      toast(false, m.chat_ready_fix_failed({ error: errText(error) }));
    } finally { busy = null; }
  }
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
    // 呼叫键：玩家自己定过才算 ✓；没定的“按一下你想用的键”（录键、就用现在这个）；没有呼叫键的照旧去「唤醒」
    list.push(callKey && callKeyChosen
      ? { id: "call", ok: true, text: m.chat_ready_call_ok({ key: hotkeyLabel(callKey) }) }
      : callKey
        ? { id: "call", ok: false, text: m.chat_ready_call_pick({ key: hotkeyLabel(callKey) }), pickKey: true }
        : { id: "call", ok: false, text: m.chat_ready_call_missing({ name }), fix: { label: m.chat_ready_call_fix(), run: onOpenTalk }, pickKey: true });
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
            <span class="min-w-0 flex-1 {row.ok ? 'text-muted-foreground' : 'text-foreground'}">
              {row.text}
              {#if !row.ok && row.pickKey}
                <span class="block text-xs text-muted-foreground" data-testid="chat-ready-call-hint">{m.chat_ready_call_hint()}</span>
              {/if}
            </span>
            {#if !row.ok && row.pickKey}
              <KeyCaptureButton label={m.chat_ready_call_capture()} capturingLabel={m.chat_ready_call_capturing()} testid="chat-ready-call-capture"
                disabled={busy !== null} onCombo={pickCallKey} />
              {#if callKey}
                <Button size="sm" variant="ghost" class="h-7 shrink-0" data-testid="chat-ready-call-keep" disabled={busy !== null}
                  onclick={() => void fix("call", async () => { settingsStore.set(await updateSettings({ ui: { callKeyChosen: true } })); })}>
                  {m.chat_ready_call_keep({ key: hotkeyLabel(callKey) })}</Button>
              {/if}
            {/if}
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
