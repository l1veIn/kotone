<script lang="ts">
  // 「琴音 → 交互 → 悬浮窗」（ADR 043 D6）：游戏所在屏幕上的悬浮窗——她的气泡、插件推送的一行和小窗。原来的“气泡”一节和
  // “推送时弹出消息提示”（ADR 039 D4）并到了这里。设置在 config.json `assistant.bubble`（显示悬浮窗、没拖过时的角和
  // 离边距离、拖到的位置、回答同时用弹幕）和 `agent.autoCards.popup`（推送时弹出小窗；写的时候 `enabled` 原样带上，
  // 不动旧值）。每条推送的开关在插件页的「设置 → 推送」。
  import { Eye, RotateCcw } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { bubbleCorners, bubblePreview } from "../../../../lib/bubble";
  import { floatingReposition } from "../../../../lib/floating";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import type { BubbleConfig } from "../../../../lib/ipc";

  const bubble = $derived<BubbleConfig>($settingsStore?.assistant?.bubble ?? {
    enabled: true, corner: "bottom-right", offsetY: 0, alsoDanmaku: false,
  });
  const pushes = $derived({ enabled: true, popup: true, ...($settingsStore?.agent?.autoCards ?? {}) });
  const dragged = $derived(!!bubble.position);
  const offsets = [
    { value: "0", label: m.companion_bubble_offset_edge() },
    { value: "120", label: m.companion_bubble_offset_120() },
    { value: "240", label: m.companion_bubble_offset_240() },
    { value: "360", label: m.companion_bubble_offset_360() },
  ];
  const offsetOptions = $derived(
    offsets.some((o) => o.value === String(bubble.offsetY))
      ? offsets
      : [...offsets, { value: String(bubble.offsetY), label: m.companion_bubble_offset_custom({ offset: bubble.offsetY }) }],
  );

  /** 只写改了的那几项（拖到的位置这类后端写的不带上，免得盖掉）。 */
  async function save(patch: Partial<BubbleConfig>, message: string) {
    await patchSettings({ assistant: { bubble: patch } }, message);
  }

  async function savePopup(popup: boolean) {
    await patchSettings({ agent: { autoCards: { ...pushes, popup } } }, popup ? m.companion_pushes_popup_on() : m.companion_pushes_popup_off());
  }

  async function reset() {
    await patchSettings({ assistant: { bubble: { position: null } } }, m.companion_bubble_reset_done());
    await floatingReposition().catch(() => {});
  }

  async function preview() {
    try {
      await bubblePreview();
    } catch (error) {
      toast(false, m.companion_bubble_preview_failed({ error: errText(error) }));
    }
  }
</script>

<SettingsSection title={m.companion_bubble_title()} description={m.companion_bubble_desc()} id="bubble-settings">
  <div data-testid="bubble-settings" class="contents">
    <SettingsRow label={m.companion_bubble_enabled_label()} description={m.companion_bubble_enabled_desc()}>
      {#snippet control()}
        <Switch data-testid="bubble-enabled" bind:checked={() => bubble.enabled, (checked) => void save({ enabled: checked }, checked ? m.companion_bubble_enabled_on() : m.companion_bubble_enabled_off())} aria-label={m.companion_bubble_enabled_label()} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_pushes_popup_label()} description={m.companion_pushes_popup_desc()}>
      {#snippet control()}
        <Switch data-testid="push-popup" aria-label={m.companion_pushes_popup_label()} disabled={!bubble.enabled}
          bind:checked={() => pushes.popup, (checked) => void savePopup(checked)} />
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_bubble_corner_label()} description={m.companion_bubble_corner_desc()}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <div class="w-32 max-w-full">
            <SelectField ariaLabel={m.companion_bubble_corner_aria()} value={bubble.corner} options={bubbleCorners}
              onchange={(corner) => void save({ corner: corner as BubbleConfig["corner"] }, m.companion_bubble_position_saved())} />
          </div>
          <Button size="sm" variant="outline" data-testid="bubble-preview" disabled={!bubble.enabled} onclick={() => void preview()}><Eye class="size-3.5" />{m.companion_bubble_preview()}</Button>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_bubble_offset_label()} description={m.companion_bubble_offset_desc()}>
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel={m.companion_bubble_offset_aria()} value={String(bubble.offsetY)} options={offsetOptions}
            onchange={(v) => void save({ offsetY: Number(v) }, m.companion_bubble_position_saved())} />
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_bubble_reset_label()} description={m.companion_bubble_reset_desc()}>
      {#snippet control()}
        <Button size="sm" variant="outline" data-testid="bubble-reset" disabled={!dragged} onclick={() => void reset()}><RotateCcw class="size-3.5" />{m.companion_bubble_reset()}</Button>
      {/snippet}
    </SettingsRow>
    <SettingsRow label={m.companion_bubble_danmaku_label()} description={m.companion_bubble_danmaku_desc()}>
      {#snippet control()}
        <Switch data-testid="bubble-danmaku" bind:checked={() => bubble.alsoDanmaku, (checked) => void save({ alsoDanmaku: checked }, checked ? m.companion_bubble_danmaku_on() : m.companion_bubble_danmaku_off())} aria-label={m.companion_bubble_danmaku_label()} />
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
