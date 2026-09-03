<script lang="ts">
  import { patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import { previewSfx } from "../../../../lib/ipc";
  import Select from "../../../../lib/components/Select.svelte";
  import Toggle from "../../../../lib/components/Toggle.svelte";

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
  <section class="kotone-panel flex flex-col gap-4 p-4">
    <div>
      <h2 class="text-sm font-semibold text-kotone-cyan/90">音效提示</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/45">
        不开悬浮窗、或全屏游戏时，也能听到录音与发送是否生效。录制音与发送音可分别开关、分别调音量、分别选音效。
      </p>
    </div>

    {#each cards as card}
      {@const item = $settingsStore.soundFeedback[card.kind]}
      <div class="flex flex-col gap-3 rounded-xl bg-white/4 p-4 ring-1 ring-white/8">
        <div>
          <div class="flex items-center justify-between gap-4">
            <h3 class="text-sm font-semibold">{card.title}</h3>
            <div class="w-44 shrink-0">
              <Toggle
                checked={item.enabled}
                ariaLabel={card.title}
                label=""
                desc=""
                onchange={(value) =>
                  void patchSettings(
                    { soundFeedback: { [card.kind]: { enabled: value } } },
                    `${card.title}已${value ? "开启" : "关闭"}`,
                  )}
              />
            </div>
          </div>
          <p class="mt-0.5 text-[11px] text-white/40">{card.hint}</p>
        </div>

        <div class="flex items-center justify-between gap-4">
          <div class="min-w-0">
            <p class="text-xs text-white/70">音效</p>
            <p class="mt-0.5 text-[11px] text-white/35">点播放键试听当前音效。</p>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <div class="w-44">
              <Select
                ariaLabel={`${card.title}音效`}
                value={item.soundId}
                options={card.options}
                onchange={(soundId) => void onPick(card.kind, soundId)}
              />
            </div>
            <button
              type="button"
              aria-label={`试听${card.title}`}
              class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-kotone-cyan/30 bg-kotone-cyan/8 text-kotone-cyan transition hover:bg-kotone-cyan/16 hover:shadow-glow-cyan"
              onclick={() => void previewSfx(card.kind, item.soundId)}
            >
              <svg viewBox="0 0 24 24" fill="currentColor" class="h-4 w-4" aria-hidden="true">
                <path d="M8 5.5v13l11-6.5-11-6.5Z" />
              </svg>
            </button>
          </div>
        </div>

        <label class="block">
          <div class="flex items-center justify-between">
            <span class="text-xs text-white/70">{card.title}音量</span>
            <span class="rounded bg-kotone-cyan/15 px-2 py-0.5 text-xs font-semibold text-kotone-cyan">
              {item.volume}%
            </span>
          </div>
          <input
            type="range"
            min="10"
            max="100"
            step="5"
            value={item.volume}
            onchange={(e) =>
              void patchSettings(
                { soundFeedback: { [card.kind]: { volume: Number((e.target as HTMLInputElement).value) } } },
                `${card.title}音量已保存`,
              )}
            class="mt-2 w-full accent-kotone-cyan"
          />
        </label>
      </div>
    {/each}

    <p class="text-[11px] leading-relaxed text-white/35">
      自定义提示音（上传自己的音频，录制音与发送音可分别设置）将在后续版本支持。
    </p>
  </section>

  <section class="kotone-panel mt-3 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">触发时机</h2>
    <ul class="mt-2 space-y-1.5 text-[11px] leading-relaxed text-white/50">
      <li>· 按下热键并成功开始录音 → 录制音</li>
      <li>· 消息注入游戏成功（松手 / 再按 / 一句话说完自动发送）→ 发送音</li>
      <li>· 独奏模式逐句连发：每句发送都有发送音，自动续录不再重复播录制音</li>
      <li>· 录音失败或发送失败不会发声（沿用界面错误提示），不会被误认为成功</li>
    </ul>
  </section>
{/if}
