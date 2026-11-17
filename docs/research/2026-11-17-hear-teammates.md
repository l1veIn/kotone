# 调研：让琴音听懂说外语的队友（2026-11-17）

用户问：琴音能替我用别的语言说话了，那怎么听懂队友说的话？并要求换个角度——**给琴音什么工具，让她自己解决“知道队友在说什么”**，展示用现有的气泡、语音、弹幕。以下是子代理读公开网页与仓库的结论，没在本机跑过；标“未验证”的是推断或二手说法。设计见 [ADR 021](../adr/021-hear-teammates.md)。

## 现成项目

| 项目 | 许可 | 怎么抓对方的声音 | 识别 | 翻译 | 显示 |
|---|---|---|---|---|---|
| [VRCT](https://github.com/misyaguziya/VRCT) | MIT（附例外） | 整个输出设备的 WASAPI loopback | faster-whisper / 云端 | NLLB、m2m100、DeepL、LLM | VR 悬浮层、VRChat 聊天框 |
| [LiveCaptions-Translator](https://github.com/SakiRinn/LiveCaptions-Translator) | Apache-2.0 | 不碰音频，读 Windows 11 实时字幕窗口的文字 | Windows 实时字幕 | Google、DeepL、LLM | 悬浮窗 |
| [LiveTranslate](https://github.com/TheDeathDragon/LiveTranslate) | MIT | 系统 loopback，可混入麦克风 | faster-whisper、SenseVoice、FunASR-Nano | LLM 流式 | 悬浮窗 |
| [auto-caption](https://github.com/HiMeditator/auto-caption) | MIT | 系统音频 / 麦克风 | **sherpa-onnx SenseVoice**、Vosk、云端 | LLM、Google | 字幕窗 |
| [ARIA](https://github.com/sayksii/aria) | GPL-3.0 | 系统音频 | sherpa-onnx + faster-whisper | Google、Bing、NLLB | 双层悬浮 |
| [OBS LocalVocal](https://github.com/locaal-ai/obs-localvocal) | GPL-2.0 | OBS 音频源 | whisper.cpp | 多种 | OBS 文本 |
| [discord-transcriber](https://github.com/dtinth/discord-transcriber) | ISC | Discord bot 收每人一路音频 | Silero VAD + 云端 | — | 频道消息 |

GPL / AGPL 的只参考思路，不拷代码。商业产品（CaptionsRush、Seagull 等）闭源。

## 要点

- **只抓某个程序的声音**：微软 ApplicationLoopback 示例（`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`，include / exclude 进程树），Windows 10 build 20348 起，Win11 都行（[文档](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)）。Rust 有 [wasapi-rs](https://github.com/HEnquist/wasapi-rs) 的 `new_application_loopback_client(pid, include_tree)`（示例 `record_application.rs`）；cpal 只支持整设备 loopback。只抓游戏 / Discord 时，琴音自己的语音天然不会被抓进来。
- **识别**：Kotone 已有的 sherpa-onnx SenseVoice（中英日韩粤）有先例（auto-caption、ARIA）；更多语种多推荐 faster-whisper large-v3-turbo（约 800 MB）。过滤参数可参考 VRCT（能量阈值、句间 3 秒、Whisper 的 `no_speech_prob` 等）。**枪声、音乐下的公开评测一个也没找到**（未验证）。PUBG 的语音和音效在同一个进程里，按进程抓也分不开（推断）。
- **Discord**：bot 收音不是官方支持的功能，且 2026-03 起语音强制端到端加密；用 RPC 的 `SPEAKING_START/STOP` 对齐说话人需要 Discord 审批。按进程抓 `Discord.exe` 最省事，但分不出是谁说的。
- **风险**：没找到 BattlEye / Krafton 对音频 loopback 的官方条款；按进程 loopback 不注入游戏（未验证是否被认可）。隐私：只在内存里处理、不存音频、界面提示“正在听”。
- **延迟**：没有公开实测，厂商说 1–2 秒对报点偏慢（未验证）。
