<script lang="ts">
  import { updateSettings } from "../../../../lib/ipc";
  import { settingsStore } from "../../../../lib/stores/ui";
  import HotkeyField from "../../../../lib/components/HotkeyField.svelte";

  interface NamedHotkeySection {
    id: string;
    title: string;
    desc: string;
    help: string;
    settingsKey:
      | "channelCycleHotkey"
      | "resendLastHotkey"
      | "togglePostProcessingHotkey"
      | "cyclePostProcessingHotkey"
      | "runtimeToggleHotkey";
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
    {
      id: "runtime-toggle",
      title: "启动 / 停止 Kotone 热键",
      desc: "按下等同于标题栏与首页的「启动 / 停止 Kotone」按钮：停止时按下启动（加载模型需要几秒，设置页会显示「启动中」），运行中按下停止并注销其他热键。",
      help: "与其它热键不同，这个热键在 Kotone 停止后依然有效——它正是用来在游戏里把 Kotone 拉起来的；留空 = 关闭。保存后立即生效、不需要重启；若按下无反应，先确认没有其它软件（录屏、游戏悬浮层、输入法）占用同一组合，并看本行是否显示「当前生效」。",
      settingsKey: "runtimeToggleHotkey",
      placeholder: "如 Alt+F10",
    },
  ];

</script>

{#each NAMED_HOTKEY_SECTIONS as section, i (section.id)}
  <section class="kotone-panel p-4 {i === 0 ? "" : "mt-3"}">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">{section.title}</h2>
    <p class="mt-1 text-[11px] leading-relaxed text-white/45">{section.desc}</p>
    <HotkeyField id={section.id} title={section.title}
      value={$settingsStore?.[section.settingsKey] ?? ""} placeholder={section.placeholder}
      onSave={async (key) => { settingsStore.set(await updateSettings({ [section.settingsKey]: key })); }} />
    <p class="mt-1.5 text-[10px] leading-relaxed text-white/35">{section.help}</p>
  </section>
{/each}
