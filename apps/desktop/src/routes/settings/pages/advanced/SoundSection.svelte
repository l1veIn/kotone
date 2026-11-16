<script lang="ts">
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { previewSfx } from "../../../../lib/ipc";
  import { Play } from "@lucide/svelte";
  import { Switch } from "$lib/components/ui/switch";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";

  type EventKind = "record" | "send";

  const recordSounds = [
    { value: "rise", label: "上扬" },
    { value: "ding-up", label: "叮咚上行" },
    { value: "chirp-up", label: "啁啾" },
  ];
  const sendSounds = [
    { value: "fall", label: "下沉" },
    { value: "knock", label: "叩～叩" },
    { value: "ding-down", label: "叮咚下行" },
  ];

  /** 切换音效：仅保存（试听走卡片上的播放键，避免切换时被打断/轰炸） */
  async function onPick(kind: EventKind, soundId: string) {
    await patchSettings(
      { soundFeedback: { [kind]: { soundId } } },
      `${kind === "record" ? "录制音" : "发送音"}已切换`,
    );
  }

  const cards: {
    kind: EventKind;
    title: string;
    hint: string;
    options: { value: string; label: string }[];
  }[] = [
    { kind: "record", title: "录制音", hint: "按下热键并成功开始录音时播放", options: recordSounds },
    { kind: "send", title: "发送音", hint: "消息注入游戏成功时播放", options: sendSounds },
  ];
</script>

{#if $settingsStore}
  <div class="space-y-6">
  <p class="m-0 px-1 text-[13px] leading-relaxed text-muted-foreground">
    不开悬浮窗、或全屏游戏时，也能听到录音与发送是否生效。录制音与发送音可分别开关、分别调音量、分别选音效。
  </p>

  {#each cards as card}
    {@const item = $settingsStore.soundFeedback[card.kind]}
    <SettingsSection title={card.title} description={card.hint}>
      {#snippet actions()}
        <Switch
          aria-label={card.title}
          bind:checked={() => item.enabled, (value) =>
            void patchSettings(
              { soundFeedback: { [card.kind]: { enabled: value } } },
              `${card.title}已${value ? "开启" : "关闭"}`,
            )}
        />
      {/snippet}
      <SettingsRow label="音效" description="点播放键试听当前音效。">
        {#snippet control()}
          <div class="flex items-center gap-2">
            <SelectField
              class="w-40"
              ariaLabel={`${card.title}音效`}
              value={item.soundId}
              options={card.options}
              onchange={(soundId) => void onPick(card.kind, soundId)}
            />
            <button
              type="button"
              aria-label={`试听${card.title}`}
              class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-primary/35 bg-primary/10 text-primary transition-colors hover:bg-primary/20"
              onclick={() => void previewSfx(card.kind, item.soundId)}
            >
              <Play class="size-3.5 fill-current" />
            </button>
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={`${card.title}音量`} stacked>
        {#snippet control()}
          <div class="flex items-center gap-3">
            <input
              type="range"
              aria-label={`${card.title}音量`}
              min="10"
              max="100"
              step="5"
              value={item.volume}
              onchange={(e) =>
                void patchSettings(
                  { soundFeedback: { [card.kind]: { volume: Number((e.target as HTMLInputElement).value) } } },
                  `${card.title}音量已保存`,
                )}
              class="min-w-0 flex-1 accent-[var(--color-primary)]"
            />
            <span class="w-14 shrink-0 rounded-md bg-primary/12 px-2 py-0.5 text-center text-xs font-semibold text-primary tabular-nums">{item.volume}%</span>
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>
  {/each}

  <SettingsSection title="触发时机">
    <ul class="m-0 list-none space-y-1.5 py-3.5 pl-0 text-xs leading-relaxed text-muted-foreground">
      <li>· 按下热键并成功开始录音 → 录制音</li>
      <li>· 消息注入游戏成功（松手 / 再按 / 一句话说完自动发送）→ 发送音</li>
      <li>· 独奏模式逐句连发：每句发送都有发送音，自动续录不再重复播录制音</li>
      <li>· 录音失败或发送失败不会发声（沿用界面错误提示），不会被误认为成功</li>
    </ul>
  </SettingsSection>
  <p class="m-0 px-1 text-[11px] leading-relaxed text-muted-foreground/80">
    自定义提示音（上传自己的音频，录制音与发送音可分别设置）将在后续版本支持。
  </p>
  </div>
{/if}
