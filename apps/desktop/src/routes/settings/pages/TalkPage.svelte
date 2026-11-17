<script lang="ts">
  /*
   * 「交流」（ADR 023）：你怎么叫琴音、她怎么回答你、怎么替你跟队友说话。
   * 原来散在「通用」（声音与弹幕）和「琴音陪玩 → 设置」（陪玩热键、快捷指令、唤醒、气泡、游戏语音）里，
   * 搬过来按方向分组；设置项和它们的测试标识不变。
   */
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import CompanionInputSettings from "./assistant/CompanionInputSettings.svelte";
  import WakeSettings from "./assistant/WakeSettings.svelte";
  import BubbleSettings from "./assistant/BubbleSettings.svelte";
  import OutputSettings from "./assistant/OutputSettings.svelte";
  import VoiceChatSettings from "./assistant/VoiceChatSettings.svelte";
</script>

{#snippet group(title: string, description: string)}
  <div class="pt-2">
    <h2 class="m-0 text-[15px] font-semibold text-foreground">{title}</h2>
    <p class="mt-1 mb-0 text-xs leading-relaxed text-muted-foreground">{description}</p>
  </div>
{/snippet}

<SettingsPage title="交流" description="你怎么叫琴音、她怎么回答你，以及怎么替你跟队友说话。这些也可以直接跟琴音说，让她帮你设。" testid="talk-page">
  {@render group("你 → 琴音", "在游戏里叫她：陪玩热键、快捷指令、语音唤醒。")}
  <CompanionInputSettings />
  <WakeSettings />
  {@render group("琴音 → 你", "她怎么回答你：右下角的对话气泡、语音和弹幕。")}
  <BubbleSettings />
  <OutputSettings />
  {@render group("琴音 → 队友", "用虚拟麦克风替你在游戏语音里说话。")}
  <VoiceChatSettings />
</SettingsPage>
