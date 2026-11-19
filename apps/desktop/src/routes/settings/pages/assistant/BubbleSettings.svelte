<script lang="ts">
  // 「琴音陪玩 → 设置 → 对话气泡」：语音和琴音说话时，屏幕角落的气泡。设置在 config.json `assistant.bubble`。
  import { Eye } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { bubbleCorners, bubblePreview } from "../../../../lib/bubble";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import type { BubbleConfig } from "../../../../lib/ipc";

  const bubble = $derived<BubbleConfig>($settingsStore?.assistant?.bubble ?? {
    enabled: true, corner: "bottom-right", offsetY: 0, alsoDanmaku: false,
  });
  const offsets = [
    { value: "0", label: "贴边" },
    { value: "120", label: "往里挪一点（120）" },
    { value: "240", label: "往里挪多些（240）" },
    { value: "360", label: "离开小地图（360）" },
  ];
  const offsetOptions = $derived(
    offsets.some((o) => o.value === String(bubble.offsetY))
      ? offsets
      : [...offsets, { value: String(bubble.offsetY), label: `自定义（${bubble.offsetY}）` }],
  );

  async function save(patch: Partial<BubbleConfig>, message: string) {
    await patchSettings({ assistant: { bubble: { ...bubble, ...patch } } }, message);
  }

  async function preview() {
    try {
      await bubblePreview();
    } catch (error) {
      toast(false, `预览失败：${errText(error)}`);
    }
  }
</script>

<SettingsSection title="对话气泡" description="用语音说话时（呼叫、唤醒、她追问你），在游戏所在屏幕的角落显示她在听、在想、在说什么。鼠标会穿过气泡，不挡操作。" id="bubble-settings">
  <div data-testid="bubble-settings" class="contents">
    <SettingsRow label="显示对话气泡" description="关掉后只用声音和弹幕回答你；悬浮窗只属于语音输入，不会拿来显示她。">
      {#snippet control()}
        <Switch data-testid="bubble-enabled" bind:checked={() => bubble.enabled, (checked) => void save({ enabled: checked }, checked ? "对话气泡已开启" : "对话气泡已关闭")} aria-label="显示对话气泡" />
      {/snippet}
    </SettingsRow>
    <SettingsRow label="位置" description="游戏全屏独占时气泡显示不出来，这时只听语音。">
      {#snippet control()}
        <div class="flex items-center gap-2">
          <div class="w-32 max-w-full">
            <SelectField ariaLabel="气泡位置" value={bubble.corner} options={bubbleCorners}
              onchange={(corner) => void save({ corner: corner as BubbleConfig["corner"] }, "气泡位置已保存")} />
          </div>
          <Button size="sm" variant="outline" data-testid="bubble-preview" disabled={!bubble.enabled} onclick={() => void preview()}><Eye class="size-3.5" />试一下</Button>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="离边缘的距离" description="右下角是小地图的游戏（比如英雄联盟），把气泡往屏幕中间挪开一些。">
      {#snippet control()}
        <div class="w-44 max-w-full">
          <SelectField ariaLabel="气泡离边缘的距离" value={String(bubble.offsetY)} options={offsetOptions}
            onchange={(v) => void save({ offsetY: Number(v) }, "气泡位置已保存")} />
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="回答同时用弹幕" description="默认只在气泡里显示回答。定时提醒照旧用弹幕。">
      {#snippet control()}
        <Switch data-testid="bubble-danmaku" bind:checked={() => bubble.alsoDanmaku, (checked) => void save({ alsoDanmaku: checked }, checked ? "回答会同时用弹幕" : "回答只在气泡里显示")} aria-label="回答同时用弹幕" />
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
