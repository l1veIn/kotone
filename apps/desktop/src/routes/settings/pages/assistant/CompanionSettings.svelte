<script lang="ts">
  import { onMount } from "svelte";
  import {
    companionClearChatRegion, companionStatus, updateSettings,
    type CompanionStatus, type QuickMessage,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { listAutomations, subscribeAutomations, type Automation } from "../../../../lib/automation";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import { CircleAlert, CircleCheck, Plus, Trash2 } from "@lucide/svelte";
  import ArgsForm from "../automation/ArgsForm.svelte";

  let status = $state<CompanionStatus | null>(null);
  let automations = $state<Automation[]>([]);
  let saving = $state(false);
  const config = $derived($settingsStore?.assistant);
  const quick = $derived(config?.companion?.quickMessages ?? []);
  const MAX = 8;

  export async function refresh() {
    try { status = await companionStatus(); } catch (error) { toast(false, `读取陪玩状态失败：${errText(error)}`); }
  }

  async function loadAutomations() {
    try { automations = await listAutomations(); } catch { /* 列表读不到时只是没有可选项 */ }
  }

  onMount(() => {
    void refresh();
    void loadAutomations();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void subscribeAutomations(() => { void loadAutomations(); }).then((fn) => { if (disposed) fn(); else unlisten = fn; });
    return () => { disposed = true; unlisten?.(); };
  });
  $effect(() => { config?.connectionId; config?.companion; $settingsStore?.ocr?.activeModelId; void refresh(); });

  async function saveQuick(next: QuickMessage[], message = "快捷指令已保存") {
    if (saving) return;
    saving = true;
    try { await patchSettings({ assistant: { companion: { quickMessages: next } } }, message); }
    finally { saving = false; }
  }

  function addQuick() {
    if (quick.length >= MAX) return;
    const id = `qm-${Date.now().toString(36)}`;
    void saveQuick([...quick, { id, label: "", text: "帮我看看队友说了什么", hotkey: "" }], "已添加快捷指令");
  }

  function patchQuick(index: number, patch: Partial<QuickMessage>) {
    void saveQuick(quick.map((item, i) => (i === index ? { ...item, ...patch } : item)));
  }

  function removeQuick(index: number) {
    void saveQuick(quick.filter((_, i) => i !== index), "已删除快捷指令");
  }

  /** 切换“交给 AI 的一句话” / “运行自动化”。 */
  function setKind(index: number, kind: string) {
    const item = quick[index];
    if (kind === "automation") {
      if (!automations.length) { toast(false, "还没有保存的自动化：先在「自动化」页新建一个"); return; }
      patchQuick(index, { automationId: automations[0].id, args: {} });
    } else {
      patchQuick(index, { automationId: "", args: {}, text: item.text.trim() || "帮我看看队友说了什么" });
    }
  }

</script>

{#if config}
  <div class="space-y-6" data-testid="companion-settings">
    <SettingsSection title="快捷指令与读屏" id="companion-title" description="呼叫琴音或按快捷指令热键时，她会先截取游戏窗口、识别聊天，再帮你设提醒、帮你打字，或直接用语音 / 弹幕回答。">
      {#if status}
        <div class="py-3.5">
          <p class="m-0 flex items-start gap-2 rounded-lg border px-3 py-2 text-xs leading-relaxed {status.ready ? 'border-success/35 bg-success/8 text-success' : 'border-warning/35 bg-warning/8 text-foreground/85'}">
            {#if status.ready}<CircleCheck class="mt-0.5 size-3.5 shrink-0" />{:else}<CircleAlert class="mt-0.5 size-3.5 shrink-0 text-warning" />{/if}
            <span data-testid="companion-status">{status.ready ? "陪玩已就绪。" : status.reason}</span>
          </p>
          {#if status.ocr.state?.dataEgress === "selected-image"}
            <p class="mt-2 mb-0 text-xs leading-relaxed text-brand-pink" data-testid="companion-upload-warning">
              当前文字识别用的是在线模型：每次呼叫琴音都会把游戏截图上传到服务商。
            </p>
          {/if}
        </div>
      {/if}
      <SettingsRow label="帮忙打字后自动发送" description="默认打完直接发出去；关掉后只打进聊天框，由你确认后自己按回车。">
        {#snippet control()}
          <Switch aria-label="帮忙打字后自动发送" disabled={saving}
            bind:checked={() => config.companion.autoSend, (checked) => void patchSettings({ assistant: { companion: { autoSend: checked } } }, "已保存")} />
        {/snippet}
      </SettingsRow>
      {#if status?.profile.chatRegion}
        <SettingsRow label="聊天框位置" description="已记住当前游戏的聊天框位置，下次只识别那块区域。">
          {#snippet control()}
            <Button variant="outline" size="sm" onclick={() => companionClearChatRegion().then(refresh).catch((error) => toast(false, errText(error)))}>重新整窗识别</Button>
          {/snippet}
        </SettingsRow>
      {/if}
    </SettingsSection>

    <SettingsSection title="快捷指令" id="companion-quick-title"
      description={`绑定热键，在游戏里一按就执行。可以是交给 AI 的一句话（等于替你说了这句话，不经过语音识别，所以更快），也可以直接运行一个已保存的自动化（不调用 AI，在按下时的游戏窗口里运行）。最多 ${MAX} 条。`}>
      {#snippet actions()}
        <Button variant="outline" size="sm" disabled={saving || quick.length >= MAX} onclick={addQuick}><Plus class="size-3.5" />添加</Button>
      {/snippet}
      {#if !quick.length}
        <p class="my-0 py-6 text-center text-xs text-muted-foreground">还没有快捷指令</p>
      {/if}
      {#each quick as item, index (item.id)}
        {@const runsAutomation = !!item.automationId}
        {@const target = automations.find((a) => a.id === item.automationId)}
        <div class="space-y-3 py-3.5" data-testid="quick-message">
          <div class="flex flex-wrap items-center gap-2">
            <Input class="h-8 w-28 text-[13px]" placeholder="名字（可选）" value={item.label} aria-label={`快捷指令 ${index + 1} 名字`}
              onchange={(event) => patchQuick(index, { label: event.currentTarget.value.trim() })} />
            <div class="w-40"><SelectField ariaLabel={`快捷指令 ${index + 1} 类型`} value={runsAutomation ? "automation" : "ask"} disabled={saving}
              options={[{ value: "ask", label: "交给 AI 的一句话" }, { value: "automation", label: "运行自动化" }]}
              onchange={(kind) => setKind(index, kind)} /></div>
            {#if runsAutomation}
              <div class="min-w-0 flex-1"><SelectField ariaLabel={`快捷指令 ${index + 1} 自动化`} value={item.automationId ?? ""} disabled={saving}
                options={[...(target ? [] : [{ value: item.automationId ?? "", label: "（自动化已删除）" }]), ...automations.map((a) => ({ value: a.id, label: a.name }))]}
                onchange={(automationId) => patchQuick(index, { automationId, args: {} })} /></div>
            {:else}
              <Input class="h-8 min-w-0 flex-1 text-[13px]" value={item.text} aria-label={`快捷指令 ${index + 1} 内容`} maxlength={200}
                onchange={(event) => {
                  const text = event.currentTarget.value.trim();
                  if (!text) { toast(false, "这句话不能为空"); event.currentTarget.value = item.text; return; }
                  patchQuick(index, { text });
                }} />
            {/if}
            <Button variant="ghost" size="sm" class="text-destructive hover:bg-destructive/10 hover:text-destructive" disabled={saving} onclick={() => removeQuick(index)}><Trash2 class="size-3.5" />删除</Button>
          </div>
          {#if runsAutomation && target?.params.length}
            <div class="rounded-xl border border-hairline bg-muted/40 p-3">
              <ArgsForm params={target.params} values={item.args ?? {}} disabled={saving} prefix={`快捷指令 ${index + 1} `}
                onchange={(args) => patchQuick(index, { args })} />
            </div>
          {/if}
          <HotkeyField id={`quick-message-${index + 1}`} title={`快捷指令 ${index + 1} 热键`} value={item.hotkey} placeholder="如 Ctrl+F10" compact disabled={saving}
            onSave={async (hotkey) => {
              const next = quick.map((q, i) => (i === index ? { ...q, hotkey } : q));
              settingsStore.set(await updateSettings({ assistant: { companion: { quickMessages: next } } }));
            }} />
        </div>
      {/each}
    </SettingsSection>
  </div>
{/if}
