<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getHotkeyStatus, updateSettings, type HotkeyStatus } from "../../../../lib/ipc";
  import { captureHotkey } from "../../../../lib/hotkeyCapture";
  import { combosConflict } from "../../../../lib/hotkeyCombo";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";

  interface NamedHotkeySection {
    id: string;
    title: string;
    desc: string;
    help: string;
    settingsKey:
      | "channelCycleHotkey"
      | "resendLastHotkey"
      | "togglePostProcessingHotkey"
      | "cyclePostProcessingHotkey";
    placeholder: string;
  }

  /** 与 backend KNOWN_NAMED_HOTKEYS 对齐；新增热键在此加一行 */
  const NAMED_HOTKEY_SECTIONS: NamedHotkeySection[] = [
    {
      id: "channel-cycle",
      title: "频道切换热键",
      desc: "支持多频道的游戏适配（如英雄联盟的「队伍 / 所有人」）按声明顺序循环切换，悬浮窗会显示当前频道。",
      help: "不能与「通用」页的录制热键相同；当前游戏适配只有一个频道时该键不生效。",
      settingsKey: "channelCycleHotkey",
      placeholder: "如 Shift+CapsLock",
    },
    {
      id: "resend-last",
      title: "重发最近一条热键",
      desc: "空闲时按下，把历史记录里最新一条发送成功的文本重新发送到当前前台窗口（用当前游戏适配与频道；正在说话/发送时按下不生效）。",
      help: "历史中没有发送成功的文本时按下无效果。",
      settingsKey: "resendLastHotkey",
      placeholder: "如 Alt+F6",
    },
    {
      id: "post-process-toggle",
      title: "文字处理开关热键",
      desc: "按下快速打开 / 关闭文字处理（屏蔽词与 AI 润色等一并启用/停用），对下一句识别生效；当前状态会短暂显示在悬浮窗。",
      help: "留空 = 关闭该热键，不会误触发。",
      settingsKey: "togglePostProcessingHotkey",
      placeholder: "如 Alt+F7",
    },
    {
      id: "post-process-cycle",
      title: "切换文字处理流程热键",
      desc: "按下切换到下一条文字处理流程（按声明顺序循环），切换到的新流程会短暂显示在悬浮窗；只有一条流程时不生效。",
      help: "留空 = 关闭该热键，不会误触发。",
      settingsKey: "cyclePostProcessingHotkey",
      placeholder: "如 Alt+F8",
    },
  ];

  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  /** 各行的当前草稿（初始化自配置）；空串 = 未设置 */
  let drafts = $state<Record<string, string>>({});
  let capturing = $state<Record<string, boolean>>({});
  let cleanups = $state<Record<string, (() => void) | null>>({});

  onMount(async () => {
    for (const section of NAMED_HOTKEY_SECTIONS) {
      drafts[section.id] = $settingsStore?.[section.settingsKey] ?? "";
    }
    hotkeyStatus = await getHotkeyStatus().catch(() => null);
  });

  function sectionOf(id: string): NamedHotkeySection {
    return NAMED_HOTKEY_SECTIONS.find((item) => item.id === id)!;
  }

  function statusOf(id: string) {
    return hotkeyStatus?.named?.find((item) => item.id === id);
  }

  /** 与录制键及其它具名热键做双向冲突预检；返回冲突方的展示名，无冲突返回 null。 */
  function findConflict(selfId: string, key: string): string | null {
    const recordKey = $settingsStore?.hotkey.key ?? "";
    if (recordKey && combosConflict(key, recordKey)) return "录制热键";
    for (const other of NAMED_HOTKEY_SECTIONS) {
      if (other.id === selfId) continue;
      const otherKey = $settingsStore?.[other.settingsKey] ?? "";
      if (otherKey && combosConflict(key, otherKey)) return other.title;
    }
    return null;
  }

  async function saveNamedHotkey(id: string) {
    const section = sectionOf(id);
    const key = (drafts[id] ?? "").trim();
    if (!key) {
      toast(false, `${section.title}不能为空`);
      return;
    }
    const conflict = findConflict(id, key);
    if (conflict) {
      toast(false, `${section.title}与${conflict}（${key}）冲突，请换一个`);
      return;
    }
    try {
      settingsStore.set(await updateSettings({ [section.settingsKey]: key }));
      drafts[id] = key;
      toast(true, `${section.title}已保存并生效：${key}`);
    } catch (e) {
      toast(false, `保存${section.title}失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  async function startCapture(id: string) {
    if (capturing[id]) return;
    capturing[id] = true;
    cleanups[id] = await captureHotkey((r) => {
      capturing[id] = false;
      cleanups[id] = null;
      if (r.kind === "combo") {
        drafts[id] = r.combo;
        void saveNamedHotkey(id);
      } else if (r.kind === "cancelled") {
        toast(false, "已取消录入");
      } else if (r.kind === "timeout") {
        toast(false, "录入超时，请重试");
      } else {
        toast(false, r.message);
      }
    });
  }

  async function clearNamedHotkey(id: string) {
    const section = sectionOf(id);
    drafts[id] = "";
    try {
      settingsStore.set(await updateSettings({ [section.settingsKey]: "" }));
      toast(true, `已关闭${section.title}`);
    } catch (e) {
      toast(false, `清除${section.title}失败：${errText(e)}`);
    } finally {
      hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
    }
  }

  onDestroy(() => {
    for (const cleanup of Object.values(cleanups)) cleanup?.();
  });
</script>

{#each NAMED_HOTKEY_SECTIONS as section, i (section.id)}
  {@const status = statusOf(section.id)}
  {@const draft = drafts[section.id] ?? ""}
  {@const isCapturing = capturing[section.id] === true}
  <section class="kotone-panel p-4 {i === 0 ? "" : "mt-3"}">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">{section.title}</h2>
    <p class="mt-1 text-[11px] leading-relaxed text-white/45">{section.desc}</p>
    <div class="mt-3 flex items-center gap-2">
      <input
        bind:value={drafts[section.id]}
        disabled={isCapturing}
        data-testid={`hotkey-input-${section.id}`}
        class="w-40 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
        placeholder={section.placeholder}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Enter" && !isCapturing) void saveNamedHotkey(section.id);
        }}
      />
      <button
        data-testid={`hotkey-capture-${section.id}`}
        class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {isCapturing
          ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
          : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
        disabled={isCapturing}
        onclick={() => void startCapture(section.id)}
      >
        {isCapturing ? "请按下键盘组合或鼠标侧键…（Esc 取消）" : "点击录入"}
      </button>
      {#if draft}
        <button
          data-testid={`hotkey-clear-${section.id}`}
          class="rounded-lg px-2.5 py-1.5 text-xs text-white/60 ring-1 ring-white/15 transition hover:bg-white/10 active:scale-95"
          onclick={() => void clearNamedHotkey(section.id)}
        >
          清除
        </button>
      {/if}
    </div>
    {#if status?.error}
      <p class="mt-2 text-[11px] leading-relaxed text-kotone-pink">{status.error}</p>
    {:else if status?.key}
      <p class="mt-2 text-[11px] text-white/40">当前生效：{status.key}</p>
    {:else if !draft}
      <p class="mt-2 text-[11px] text-white/40">未设置（默认关闭，不会误触发）</p>
    {/if}
    <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">{section.help}</p>
  </section>
{/each}