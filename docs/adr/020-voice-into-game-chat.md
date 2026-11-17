# ADR 020：琴音用虚拟麦克风替玩家在游戏语音里说话

- 状态：Accepted（2026-11-17）。用户确认方向，按调研的默认做法实现，并补充两条要求：**VB-CABLE 要自动安装**；**声卡相关的配置也要能交给 Agent**。
- 调研：[琴音用麦克风替你说话](../research/2026-11-17-voice-into-game-chat.md)
- 相关：[ADR 014](014-game-agent-harness.md)（三层：基础能力 → 自动化 → Agent，人能做的 Agent 都能做）、[ADR 017](017-voice-wake.md)（语音唤醒）

## 问题

玩家说“琴音，跟队友说一声我去拿龙了”，琴音的语音要让**队友**听到，也就是要进游戏的语音聊天。游戏语音只读麦克风，所以要一个“虚拟麦克风”：Kotone 往里放声音，游戏把它当麦克风读。

## 决定（每一部分照抄谁）

| 部分 | 照抄 | 做法 |
|---|---|---|
| 虚拟麦克风 | VB-CABLE（VB-Audio，捐赠软件）；Soundux 的安装脚本（`VBCABLE_Setup_x64.exe -h -i`） | 不自己写驱动。没装时**自动安装**：从官网下载 `VBCABLE_Driver_Pack46.zip`（固定 SHA-256），解压到 Kotone 目录，以管理员身份静默运行官方安装程序（Windows 弹一次授权）。Kotone 的安装包里不带它；安装前界面写明它是 VB-Audio 的捐赠软件、来自 vb-cable.com |
| 声音怎么进去 | TTS Voice Wizard（MIT）：合成的语音播到 CABLE Input，同时自己听一份 | “说给队友”的话：合成一次，送进虚拟麦克风；玩家照常从自己的耳机里听到同一句 |
| 玩家自己的声音 | cpal `examples/feedback.rs`：输入流 → 环形缓冲 → 输出流 | 开着“游戏语音”时，真麦克风的声音实时转进 CABLE Input，和琴音的声音混成一路（琴音说话时把玩家的声音压低一半，避免叠在一起听不清） |
| 游戏选麦克风 | AudioDeviceCmdlets / EarTrumpet 用的 `IPolicyConfig::SetDefaultEndpoint` | 很多游戏（据调研 PUBG、CS2）只用 Windows 的默认录音 / 默认通信设备。提供“把 Windows 默认麦克风设成 CABLE Output”和“改回原来的”，先记下原来的设备 |
| 按键说话 | 不做（第一版） | 建议游戏里改成自由麦；以后可照 Soundpad Auto Keys：播的时候按住说话键 |

## 交给 Agent（为 Agent 而生）

人在设置页能做的，琴音和外部 Agent 都能做：

- `voice_chat.status`：VB-CABLE 装没装、两个设备在不在、开没开、Windows 默认麦克风现在是哪个、转发在不在运行、当前语音引擎能不能送进游戏；
- `voice_chat.install`：自动下载并安装 VB-CABLE（会弹管理员授权，**先问玩家**）；
- `voice_chat.default_mic`：`{use: "cable" | "restore"}` 把 Windows 默认麦克风设成 CABLE Output，或改回原来的（改系统设置，**先问玩家**）；
- `voice_chat.say`：把一句话说给队友听（**先问玩家**，可以在权限里改成直接执行）；
- `voice_chat.test`：往虚拟麦克风放一段测试音，同时从 CABLE Output 录回来，确认通了；
- 开关、混不混真麦克风、用哪个真麦克风：设置项 `voiceChat.*`，经 `settings.update`。

## 约束

- 默认关；开着时才转发麦克风。**Kotone 没运行时，游戏里选了 CABLE Output 的话队友听不到你**——界面写明。
- 语音唤醒、语音输入永远不该听 CABLE Output（否则琴音会被自己唤醒）；设备选成它时提示。
- 系统语音（Windows SAPI）直接出声、拿不到声音数据，不能送进游戏；用 DeepSeek / Kokoro / 在线引擎时可以。
- 只送“说给队友”的话，平时的回答、提醒不送。

## 实施步骤

1. 平台：VB-CABLE 检测、静默安装（下载、校验、解压、提权运行并等待）、Windows 默认录音设备读 / 写；
2. 平台：转发与混音（真麦克风 + 琴音的语音 → CABLE Input）；
3. 语音合成：一句话除了自己播，还能交给转发器；
4. 设置项、设置说明、工具、权限、琴音的提示词；
5. 设置页；
6. 测试：纯逻辑单元测试；真机只测转发和测试音（这台电脑已装 VB-CABLE），**不改用户的 Windows 默认设备、不重装驱动**；
7. 文档与第三方声明。
