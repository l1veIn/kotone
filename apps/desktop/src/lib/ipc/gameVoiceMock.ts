/**
 * dev:web 里的游戏语音插件（ADR 020、038）：照桌面端 plugins/game-voice 回答 voice_chat.* 工具。
 * 驱动和 Windows 默认麦克风是模拟的（预览开始时没装 VB-CABLE）；自己的麦克风、音量读设置 `plugins.game-voice.settings`。
 */
import type { PluginToolResult } from "../plugins";
import { getSettings } from "./browserMock";

const preview = {
  installed: false,
  defaultIsCable: false,
  restorable: false,
};

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// i18n-ignore-start：浏览器预览的示例数据（真实应用里来自后端或玩家自己的设备），不是界面文字
const CABLE_INPUT = "CABLE Input (VB-Audio Virtual Cable)";
const CABLE_OUTPUT = "CABLE Output (VB-Audio Virtual Cable)";
const USB_MIC = "麦克风 (USB Audio)";
const ATTRIBUTION = "VB-CABLE 是 VB-Audio 的捐赠软件（https://vb-cable.com），安装即表示同意它的许可；欢迎到官网捐助作者。";
const TEST_OK = "通了：游戏或语音软件的麦克风选 CABLE Output，队友就能听到";
const NOT_INSTALLED = "还没有安装 VB-CABLE";
// i18n-ignore-end

async function status(enabled: boolean): Promise<Record<string, unknown>> {
  const settings = (await getSettings()).plugins?.["game-voice"]?.settings ?? {};
  const mic = preview.defaultIsCable ? CABLE_OUTPUT : USB_MIC;
  return {
    driverInstalled: preview.installed,
    installing: false,
    devices: preview.installed ? { cableInput: CABLE_INPUT, cableOutput: CABLE_OUTPUT } : { cableInput: null, cableOutput: null },
    enabled,
    includeMic: settings.includeMic !== false,
    micDeviceId: typeof settings.micDeviceId === "string" ? settings.micDeviceId : "default",
    micGain: typeof settings.micGain === "number" ? settings.micGain : 100,
    running: enabled && preview.installed,
    error: enabled && !preview.installed ? NOT_INSTALLED : null,
    windowsDefaultMic: { id: "mic", name: mic },
    windowsCommunicationsMic: { id: "mic", name: mic },
    windowsDefaultIsCable: preview.defaultIsCable,
    canRestoreDefaultMic: preview.restorable,
    ttsCanSpeak: true,
    attribution: ATTRIBUTION,
  };
}

/** 插件开着时的工具（插件开关由 pluginsMock 先看）。 */
export async function gameVoiceCall(tool: string, args: Record<string, unknown>): Promise<PluginToolResult> {
  switch (tool) {
    case "voice_chat.status":
      return { content: await status(true) };
    case "voice_chat.install":
      await wait(400);
      preview.installed = true;
      return { content: { installed: true, needsReboot: false } };
    case "voice_chat.default_mic": {
      const useCable = args.use === "cable";
      preview.defaultIsCable = useCable;
      preview.restorable = useCable;
      return { content: useCable ? { defaultMic: CABLE_OUTPUT, restorable: true } : { defaultMic: USB_MIC, restored: true } };
    }
    case "voice_chat.test":
      await wait(300);
      return { content: { ok: true, peak: 0.3, note: TEST_OK } };
    case "voice_chat.say":
      return { content: { sent: true, durationMs: 1200 } };
    default:
      return { content: { status: "error", text: tool } };
  }
}
