<script lang="ts">
  import { onMount } from "svelte";
  import {
    companionClearChatRegion, companionSelfCheck, companionStatus, updateSettings,
    type CompanionStatus, type QuickMessage,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";

  let { checking = $bindable(false) }: { checking?: boolean } = $props();
  let status = $state<CompanionStatus | null>(null);
  let saving = $state(false);
  const config = $derived($settingsStore?.assistant);
  const quick = $derived(config?.companion?.quickMessages ?? []);
  const check = $derived(config?.companion?.check ?? null);
  const MAX = 8;

  export async function refresh() {
    try { status = await companionStatus(); } catch (error) { toast(false, `读取陪玩状态失败：${errText(error)}`); }
  }

  onMount(() => { void refresh(); });
  $effect(() => { config?.connectionId; config?.companion; $settingsStore?.ocr?.activeModelId; void refresh(); });

  export async function runCheck() {
    if (checking) return;
    checking = true;
    try {
      const result = await companionSelfCheck();
      settingsStore.update((s) => (s ? { ...s, assistant: { ...s.assistant, companion: { ...s.assistant.companion, check: result } } } : s));
      toast(result.passed, result.passed ? "能力自检通过" : `能力自检未通过：${result.message}`);
    } catch (error) {
      toast(false, `能力自检失败：${errText(error)}`);
    } finally {
      checking = false;
      await refresh();
    }
  }

  async function saveQuick(next: QuickMessage[], message = "快捷消息已保存") {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: { companion: { quickMessages: next } } }, message); }
    finally { saving = false; }
  }

  function addQuick() {
    if (quick.length >= MAX) return;
    const id = `qm-${Date.now().toString(36)}`;
    void saveQuick([...quick, { id, label: "", text: "帮我看看队友说了什么", hotkey: "" }], "已添加快捷消息");
  }

  function patchQuick(index: number, patch: Partial<QuickMessage>) {
    void saveQuick(quick.map((item, i) => (i === index ? { ...item, ...patch } : item)));
  }

  function removeQuick(index: number) {
    void saveQuick(quick.filter((_, i) => i !== index), "已删除快捷消息");
  }

  function checkLabel(): string {
    if (!config?.connectionId) return "未选择 AI 连接";
    if (!check || check.connectionId !== config.connectionId) return "尚未自检";
    const when = new Date(check.checkedAtMs).toLocaleString();
    return `${check.passed ? "已通过" : "未通过"}（${check.model || "未知模型"}，${when}）：${check.message}`;
  }
</script>

{#if config}
  <section class="panel" aria-labelledby="companion-title" data-testid="companion-settings">
    <h2 id="companion-title">陪玩</h2>
    <p class="mt-1 text-[11px] leading-relaxed text-white/45">
      按住陪玩热键说一句话（或按快捷消息热键），琴音会截取游戏窗口、识别聊天、调一次 AI，然后选择：帮你设提醒、帮你打字，或直接用语音 / 弹幕回答。
    </p>

    {#if status}
      <div class="notice mt-3" class:ok={status.ready} data-testid="companion-status">
        {status.ready ? "陪玩已就绪。" : status.reason}
      </div>
      {#if status.ocr.state?.dataEgress === "selected-image"}
        <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink" data-testid="companion-upload-warning">
          当前文字识别用的是在线模型：每按一次陪玩热键都会把游戏截图上传到服务商。
        </p>
      {/if}
    {/if}

    <div class="setting-row mt-3">
      <div><h3>AI 能力自检</h3><p data-testid="companion-check">{checkLabel()}</p></div>
      <button type="button" class="secondary" disabled={checking || !config.connectionId} onclick={() => void runCheck()}>
        {checking ? "自检中…" : "能力自检"}
      </button>
    </div>
    <p class="mt-1 text-[11px] text-white/40">选择连接后会自动自检一次（两次付费请求）：先确认能调用工具，再看能否从一段固定聊天里挑出队友的话。没通过时热键不工作。</p>

    <div class="setting-row mt-3">
      <div><h3>帮忙打字后自动发送</h3><p>默认只打进聊天框，由你确认后自己发。</p></div>
      <input type="checkbox" class="accent-cyan-300" aria-label="帮忙打字后自动发送" disabled={saving}
        checked={config.companion.autoSend}
        onchange={(event) => void patchSettings({ assistant: { companion: { autoSend: event.currentTarget.checked } } }, "已保存")} />
    </div>

    <div class="mt-4">
      <div class="flex items-center justify-between">
        <h3>快捷消息</h3>
        <button type="button" class="secondary" disabled={saving || quick.length >= MAX} onclick={addQuick}>添加</button>
      </div>
      <p class="mt-1 text-[11px] text-white/40">提前写好一句话并绑定热键，按下就等于替你说了这句话，不经过语音识别，所以更快。最多 {MAX} 条。</p>
      <div class="mt-2 flex flex-col gap-2">
        {#each quick as item, index (item.id)}
          <div class="rounded-lg bg-white/4 p-3 ring-1 ring-white/10" data-testid="quick-message">
            <div class="flex items-center gap-2">
              <input class="field w-28" placeholder="名字（可选）" value={item.label} aria-label={`快捷消息 ${index + 1} 名字`}
                onchange={(event) => patchQuick(index, { label: event.currentTarget.value.trim() })} />
              <input class="field min-w-0 flex-1" value={item.text} aria-label={`快捷消息 ${index + 1} 内容`} maxlength="200"
                onchange={(event) => {
                  const text = event.currentTarget.value.trim();
                  if (!text) { toast(false, "快捷消息不能为空"); event.currentTarget.value = item.text; return; }
                  patchQuick(index, { text });
                }} />
              <button type="button" class="secondary" disabled={saving} onclick={() => removeQuick(index)}>删除</button>
            </div>
            <div class="mt-2">
              <HotkeyField id={`quick-message-${index + 1}`} title={`快捷消息 ${index + 1} 热键`} value={item.hotkey} placeholder="如 Ctrl+F10" compact disabled={saving}
                onSave={async (hotkey) => {
                  const next = quick.map((q, i) => (i === index ? { ...q, hotkey } : q));
                  settingsStore.set(await updateSettings({ assistant: { companion: { quickMessages: next } } }));
                }} />
            </div>
          </div>
        {/each}
      </div>
    </div>

    {#if status?.profile.chatRegion}
      <div class="setting-row mt-4">
        <div><h3>聊天框位置</h3><p>已记住当前游戏的聊天框位置，下次只识别那块区域。</p></div>
        <button type="button" class="secondary" onclick={() => companionClearChatRegion().then(refresh).catch((error) => toast(false, errText(error)))}>重新整窗识别</button>
      </div>
    {/if}
  </section>
{/if}

<style>
  .notice.ok { color: #7dffd9; }
  .field { border-radius: .5rem; background: #ffffff14; padding: .35rem .6rem; font-size: .75rem; outline: none; box-shadow: inset 0 0 0 1px #ffffff26; }
  .field:focus { box-shadow: inset 0 0 0 1px #00e5ff99; }
</style>
