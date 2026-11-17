# 调研：琴音用麦克风替玩家说话（2026-11-17）

用户问：Kotone 已经有语音合成，能不能让琴音“用麦克风”说话，让 PUBG 之类游戏里的队友听到？本文是子代理调研开源社区后的结论，**还没有实现，也没有定方案**；末尾的问题等用户回答。来源链接都是调研时打开过的页面，标“未核实”的是没有找到可靠来源的说法。

## 结论

可以做。社区通用做法：**Kotone 把合成的语音播到虚拟声卡 VB-Cable 的“扬声器”端（CABLE Input），游戏 / 语音软件的麦克风选这张声卡的“麦克风”端（CABLE Output）**。玩家自己也要说话时，由 Kotone 把真麦克风和合成语音混成一路再送进去。第一版不自己写、也不打包驱动。

## 1. 技术方案

| 方案 | 驱动 / 管理员 / 签名 | 能否随软件分发 |
|---|---|---|
| VB-Cable（本机已装） | 内核驱动，安装要管理员，已正式签名 | 允许原样附带官方安装包，须注明来源 vb-cable.com 并说明是捐赠软件；A+B、C+D 版不能打包（[授权说明](https://vb-audio.com/Services/licensing.htm)） |
| VoiceMeeter | 内核驱动，自带混音 | Standard / Banana 同上，Potato 要付费（同上） |
| [VirtualDrivers/Virtual-Audio-Driver](https://github.com/VirtualDrivers/Virtual-Audio-Driver) | beta，要开测试签名模式 | MIT + MS-PL |
| [Scream](https://github.com/duncanthrax/scream) | 只有播放设备，不适合 | MS-PL |
| [SysVAD 示例](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample) | 只是写驱动的起点 | — |

- 自己发内核驱动门槛高：新驱动必须经微软 Hardware Dev Center 签名，要 EV 证书（[微软文档](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings)）。
- WASAPI loopback 只能录系统正在放的声音，造不出麦克风设备（推断）。

## 2. 可照抄的开源项目

- [TTS Voice Wizard](https://github.com/VRCWizard/TTS-Voice-Wizard)（MIT）：语音合成播到 VB-Cable，同时复制一份到耳机给自己听。
- [Soundux](https://github.com/Soundux/Soundux)（GPLv3）：Windows 上播到 VB-Cable，安装器自动装 VB-Cable。
- [InstanTTS](https://github.com/ZenitH-AT/InstanTTS)、[VoxBridge TTS](https://finickyspider.github.io/VoxBridge-TTS/getting-started/installation/)：同样的接法。
- 按键说话：Soundpad（闭源）的 [Auto Keys](https://www.leppsoft.com/soundpad/help/manual/tutorial/autokeys/)：开始播放时按住指定键，播完松开。
- 混音写法：cpal 的 [`feedback.rs`](https://github.com/RustAudio/cpal/blob/master/examples/feedback.rs) 示例（输入流 + 输出流 + 环形缓冲）。
- 延迟：VB-Cable 默认缓冲约 150 ms（[官方论坛](https://forum.vb-audio.com/viewtopic.php?p=473)）。

## 3. 游戏里的风险

- 反作弊：没找到 BattlEye / Vanguard / EAC 官方对虚拟声卡的明确说法（未核实）。测试签名模式的驱动大概率不被反作弊接受（推断，未核实）。
- 规则：PUBG 行为准则没点名音效板，但刷屏、冒犯会处罚（[PUBG 行为准则](https://pubg.com/en/clause/rules_of_conduct)）；Valorant 会录音审查举报（[公告](https://playvalorant.com/en-us/news/announcements/valorant-voice-evaluation-update/)）。
- 选麦克风：Valorant 有输入设备选项（[Riot 支持](https://support.riotgames.com/en-us/valorant/gameplay/valorant-voice-chat-troubleshooting)）；PUBG、CS2 据第三方文章用 Windows 默认通信设备（未核实）；LoL、KOOK、YY 没查。
- 回声：语音唤醒只能听真麦克风，绝不能听 CABLE Output，否则琴音会被自己唤醒。

## 4. 第一版的最小设计（建议）

1. 不打包驱动：检测有没有 “CABLE Input”，没有就引导玩家去官网装。
2. 用 cpal 在进程内混音：真麦克风 + 合成语音 → CABLE Input；合成语音另放一份到耳机。
3. 玩家设置：游戏 / Discord 的麦克风选 “CABLE Output”；只认系统默认设备的游戏改系统默认通信设备。要说清楚：**Kotone 没运行时队友听不到你**。
4. 按键说话：照 Soundpad Auto Keys，播放时按住玩家设的说话键，播完约 200 ms 后松开；或建议游戏里改成自由麦。
5. 默认关闭、按游戏开启；可选在开头加“AI 助手：”；限制频率防刷屏。

## 待用户确认

1. 玩家自己说话也要经过 Kotone（混麦），还是只送琴音的声音？
2. 让玩家自己装 VB-Cable，还是做一键安装（原样附带官方安装包并注明来源）？
3. 要不要自动按“按键说话”键？
4. 先在哪些游戏里实测？
5. 只送琴音的话，还是也包括音效？要不要强制“AI 助手”前缀？
