<script lang="ts">
  import { updateSettings, type AssistantConfig, type ConnectionInfo } from "../../../../lib/ipc";
  import { previewReminder, stopSpeech, type DeliveryKind } from "../../../../lib/assistant";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import Select from "../../../../lib/components/Select.svelte";
  import AssistantInteraction from "./AssistantInteraction.svelte";
  import SpeechSettings from "./SpeechSettings.svelte";
  import DanmakuSettings from "./DanmakuSettings.svelte";

  let { connections, ready, onOpenConnections }: {
    connections: ConnectionInfo[]; ready: boolean; onOpenConnections: () => void;
  } = $props();
  let saving = $state(false);
  let previewing = $state<DeliveryKind | null>(null);
  const config = $derived($settingsStore?.assistant);

  async function save(patch: Partial<AssistantConfig>) {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: patch }, "提醒助手设置已保存"); }
    finally { saving = false; }
  }

  function toggleDelivery(kind: DeliveryKind, checked: boolean) {
    if (!config) return;
    const next = checked ? [...config.defaultDelivery, kind] : config.defaultDelivery.filter((item) => item !== kind);
    if (!next.length) { toast(false, "请至少保留一种默认提醒方式"); return; }
    void save({ defaultDelivery: [...new Set(next)] });
  }

  async function preview(kind: DeliveryKind) {
    if (previewing || saving) return;
    previewing = kind;
    try { await previewReminder(kind); }
    catch (error) { toast(false, `测试失败：${errText(error)}`); }
    finally { previewing = null; }
  }
</script>

{#if config}
  <div data-testid="assistant-settings">
    <section class="panel" aria-labelledby="assistant-input-settings-title">
      <h2 id="assistant-input-settings-title">输入设置</h2>
      <div class="setting-row">
        <div><h3>AI 连接</h3><p>需支持工具调用。</p></div>
        <div class="w-60 max-w-full">
          <Select ariaLabel="提醒助手 AI 连接" value={config.connectionId} disabled={saving}
            options={[{ value: "", label: "请选择连接" }, ...connections.map((item) => ({ value: item.id, label: `${item.displayName}${item.hasApiKey ? "" : "（未保存密钥）"}` }))]}
            onchange={(connectionId) => { void save({ connectionId }); }} />
        </div>
      </div>
      {#if !ready}
        <div class="notice">{connections.length ? "请选择连接并配置密钥。" : "请先添加 AI 连接。"}<button type="button" class="text-kotone-cyan hover:underline" onclick={onOpenConnections}>管理连接 →</button></div>
      {/if}
      <div class="mt-4">
        <h3>助手快捷键</h3>
        <HotkeyField id="assistant" title="助手快捷键" value={config.hotkey} placeholder="如 Ctrl+F9" disabled={saving} compact
          onSave={async (hotkey) => { settingsStore.set(await updateSettings({ assistant: { hotkey } })); }} />
      </div>
      <AssistantInteraction />
    </section>

    <section class="panel" aria-labelledby="assistant-delivery-title">
      <h2 id="assistant-delivery-title">提醒方式</h2>
      <div class="setting-row">
        <h3>默认方式</h3>
        <div class="flex gap-4 text-sm text-white/80">
          {#each [{ id: "voice", label: "语音播报" }, { id: "danmaku", label: "弹幕" }] as item}
            <label class="flex items-center gap-2"><input type="checkbox" class="accent-cyan-300" disabled={saving}
              checked={config.defaultDelivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
          {/each}
        </div>
      </div>
      <div class="mt-5"><SpeechSettings /></div>
      <div class="mt-5"><DanmakuSettings /></div>
      <p id="danmaku-fullscreen-hint" class="mt-3 text-[11px] leading-relaxed text-amber-200/85">弹幕不支持独占全屏，请使用无边框全屏或窗口化。</p>
      <div class="mt-4 flex flex-wrap gap-2">
        <button type="button" class="secondary" disabled={!!previewing || saving} onclick={() => preview("voice")}>{previewing === "voice" ? "播报中…" : "试听语音"}</button>
        <button type="button" class="secondary" aria-describedby="danmaku-fullscreen-hint" disabled={!!previewing || saving} onclick={() => preview("danmaku")}>测试弹幕</button>
        <button type="button" class="secondary" onclick={() => stopSpeech().catch((error) => toast(false, errText(error)))}>停止播报 / 弹幕</button>
      </div>
    </section>
  </div>
{/if}
