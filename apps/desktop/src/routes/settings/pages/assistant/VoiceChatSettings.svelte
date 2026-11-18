<script lang="ts">
  // 「琴音陪玩 → 设置 → 游戏语音」（ADR 020）：琴音用虚拟麦克风 VB-CABLE 替你在游戏语音里说话。
  // 开关等设置在 config.json `voiceChat`；安装、改默认麦克风、测试走 voice_chat_* 命令（与工具同一实现）。
  import { onMount } from "svelte";
  import { Download, LoaderCircle, Mic, RotateCcw, Volume2 } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { listAudioDevices, type AudioDevice, type VoiceChatConfig } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import {
    voiceChatDefaultMic, voiceChatInstall, voiceChatSay, voiceChatStatus, voiceChatTest, type VoiceChatStatus,
  } from "../../../../lib/voiceChat";

  const config = $derived<VoiceChatConfig>($settingsStore?.voiceChat ?? {
    enabled: false, includeMic: true, micDeviceId: "default", micGain: 100,
  });
  let status = $state<VoiceChatStatus | null>(null);
  let devices = $state<AudioDevice[]>([]);
  let busy = $state<"" | "install" | "mic" | "test" | "say">("");

  const gains = [
    { value: "50", label: "50%" }, { value: "75", label: "75%" }, { value: "100", label: "100%" },
    { value: "150", label: "150%" }, { value: "200", label: "200%" },
  ];
  const gainOptions = $derived(
    gains.some((g) => g.value === String(config.micGain)) ? gains : [...gains, { value: String(config.micGain), label: `${config.micGain}%` }],
  );
  const micOptions = $derived(
    devices
      .filter((d) => !d.name.startsWith("CABLE Output"))
      .map((d) => ({ value: d.id, label: d.name })),
  );

  async function refresh() {
    try {
      status = await voiceChatStatus(config.enabled);
    } catch (error) {
      toast(false, `读取游戏语音状态失败：${errText(error)}`);
    }
  }

  onMount(() => {
    void refresh();
    void listAudioDevices().then((list) => (devices = list)).catch(() => {});
    const timer = setInterval(() => void refresh(), 3000);
    return () => clearInterval(timer);
  });

  async function save(patch: Partial<VoiceChatConfig>, message: string) {
    await patchSettings({ voiceChat: { ...config, ...patch } }, message);
    setTimeout(() => void refresh(), 600);
  }

  async function run<T>(kind: typeof busy, work: () => Promise<T>, done: (value: T) => void) {
    busy = kind;
    try {
      done(await work());
    } catch (error) {
      toast(false, errText(error));
    } finally {
      busy = "";
      void refresh();
    }
  }

  const install = () => run("install", voiceChatInstall, (r) =>
    toast(r.installed, r.installed ? "VB-CABLE 已装好" : (r.note ?? "安装程序已运行，可能要重启电脑后才能用")));
  const useCable = (use: boolean) => run("mic", () => voiceChatDefaultMic(use), (r) =>
    toast(true, use ? `Windows 默认麦克风已改成 ${r.defaultMic}` : `已改回 ${r.defaultMic ?? "原来的麦克风"}`));
  const test = () => run("test", voiceChatTest, (r) => toast(r.ok, r.note));
  const say = () => run("say", () => voiceChatSay("这是游戏语音测试。"), () => toast(true, "已经说给队友了"));

  const stateText = $derived.by(() => {
    if (!status) return "读取中…";
    if (status.installing) return "正在安装 VB-CABLE…";
    if (!status.driverInstalled) return "还没有安装虚拟麦克风 VB-CABLE";
    if (!config.enabled) return "未开启";
    if (status.error) return `出错了：${status.error}`;
    return status.running ? "运行中：游戏里的麦克风选 CABLE Output，队友就能听到" : "正在打开…";
  });
</script>

<SettingsSection title="游戏语音" description="替你跟队友说话：她的声音和你的麦克风一起送进虚拟麦克风 CABLE Output，游戏或语音软件的麦克风选它就行。也可以直接让她帮你设。" id="voice-chat-settings">
  <div data-testid="voice-chat-settings" class="contents">
    <SettingsRow label="虚拟麦克风" description={status?.attribution ?? ""}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <span class="text-xs text-muted-foreground" data-testid="voice-chat-state">{stateText}</span>
          {#if status && !status.driverInstalled}
            <Button size="sm" data-testid="voice-chat-install" disabled={busy !== ""} onclick={() => void install()}>
              {#if busy === "install"}<LoaderCircle class="size-3.5 animate-spin" />{:else}<Download class="size-3.5" />{/if}自动安装
            </Button>
          {/if}
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="开启游戏语音" description="开着时把你的麦克风实时转进虚拟麦克风；Kotone 没运行时队友听不到你，记得在游戏里改回原来的麦克风。">
      {#snippet control()}
        <Switch data-testid="voice-chat-enabled" disabled={!status?.driverInstalled}
          bind:checked={() => config.enabled, (checked) => void save({ enabled: checked }, checked ? "游戏语音已开启" : "游戏语音已关闭")} aria-label="开启游戏语音" />
      {/snippet}
    </SettingsRow>
    <SettingsRow label="混入我的麦克风" description="关掉后虚拟麦克风里只有她的声音。她说话时你的声音自动压低一半。">
      {#snippet control()}
        <Switch data-testid="voice-chat-include-mic" bind:checked={() => config.includeMic, (checked) => void save({ includeMic: checked }, "已保存")} aria-label="混入我的麦克风" />
      {/snippet}
    </SettingsRow>
    {#if config.includeMic}
      <SettingsRow label="我的麦克风" description="不能选 CABLE Output。">
        {#snippet control()}
          <div class="flex items-center gap-2">
            <div class="w-56 max-w-full">
              <SelectField ariaLabel="我的麦克风" testid="voice-chat-mic" value={config.micDeviceId || "default"} options={micOptions}
                onchange={(id) => void save({ micDeviceId: id }, "麦克风已保存")} />
            </div>
            <div class="w-24">
              <SelectField ariaLabel="我的麦克风音量" value={String(config.micGain)} options={gainOptions}
                onchange={(v) => void save({ micGain: Number(v) }, "音量已保存")} />
            </div>
          </div>
        {/snippet}
      </SettingsRow>
    {/if}
    <SettingsRow label="Windows 默认麦克风" description={`有些游戏（例如 PUBG）只用 Windows 的默认麦克风，不能在游戏里选。现在是：${status?.windowsDefaultMic?.name ?? "未知"}。改了以后 Discord、会议软件也会用它，用完记得改回。`}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          {#if status?.windowsDefaultIsCable}
            <Button size="sm" variant="outline" data-testid="voice-chat-restore-mic" disabled={busy !== "" || !status?.canRestoreDefaultMic} onclick={() => void useCable(false)}>
              <RotateCcw class="size-3.5" />改回原来的
            </Button>
          {:else}
            <Button size="sm" variant="outline" data-testid="voice-chat-use-cable" disabled={busy !== "" || !status?.driverInstalled} onclick={() => void useCable(true)}>
              <Mic class="size-3.5" />设成 CABLE Output
            </Button>
          {/if}
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="试一下" description={status && !status.ttsCanSpeak ? "当前语音是系统语音，送不进游戏：在「通用 → 语音」换成 DeepSeek、Kokoro 或在线语音。" : "测试音：放 1 秒“嘀”再从 CABLE Output 录回来，确认通了；说一句：对队友说一句测试的话。在游戏语音里时队友会听到。"}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <Button size="sm" variant="outline" data-testid="voice-chat-test" disabled={busy !== "" || !status?.running} onclick={() => void test()}>
            {#if busy === "test"}<LoaderCircle class="size-3.5 animate-spin" />{/if}测试音
          </Button>
          <Button size="sm" variant="outline" data-testid="voice-chat-say" disabled={busy !== "" || !status?.running || !status?.ttsCanSpeak} onclick={() => void say()}>
            <Volume2 class="size-3.5" />说一句
          </Button>
        </div>
      {/snippet}
    </SettingsRow>
  </div>
</SettingsSection>
