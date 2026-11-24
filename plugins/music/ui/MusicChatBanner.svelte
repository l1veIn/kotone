<script lang="ts">
  // 聊天输入框上面的登录卡片（ADR 034 D5；放歌插件给宿主的 `chatBanner`，插件开着时才有）：她要放歌而你没登录网易云、
  // 登录窗口开着、过期了、出错了时出现——她不自己弹窗，免得游戏里抢焦点。
  import type { PluginChatBannerProps } from "$lib/pluginUi";
  import MusicLoginCard from "./MusicLoginCard.svelte";
  import { loginPending, musicStore } from "./music";

  let { plugin }: PluginChatBannerProps = $props();
</script>

{#if loginPending($musicStore)}<div class="contents" data-plugin={plugin.manifest.id}><MusicLoginCard compact /></div>{/if}
