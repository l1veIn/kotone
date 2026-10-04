# Sokuji ASR/TTS 对照与 Kotone 取舍

核实于 2026-10-04；来源为用户指定的 [kizuna-ai-lab/sokuji](https://github.com/kizuna-ai-lab/sokuji)，固定版本 [d2f8001beb6d8a7b34906667fccff4ee1a403431](https://github.com/kizuna-ai-lab/sokuji/tree/d2f8001beb6d8a7b34906667fccff4ee1a403431)。本次静态检查，没有安装依赖、下载模型或测量其性能。以下“适合借鉴”是结合 Kotone 提醒需求的设计判断，不是两个产品的性能排名。

## 当前实现观察

| 方面 | Sokuji 代码证据 | 对 Kotone 的意义 |
| --- | --- | --- |
| 阶段边界 | [localInference/adapter.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/providers/localInference/adapter.ts) 将 ASR、翻译、TTS 划为阶段，管理会话、顺序队列及过期结果；TTS 故障可保留文本输出 | 借鉴阶段接口、任务取消、顺序与文本降级，不把播报塞入识别器 |
| 轻接口与装配 | [engines.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/providers/localInference/engines.ts) 通过较小的 ASR/TTS 接口创建具体实现；[speech.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/providers/localInference/speech.ts) 管文本合成与播放数据 | Kotone 保持 `SttEngine`、`SpeechOutput`、`ReminderDelivery` 分离，装配留给桌面壳 |
| 浏览器本地 ASR | [AsrEngine.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/lib/local-inference/engine/AsrEngine.ts) 与 [StreamingAsrEngine.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/lib/local-inference/engine/StreamingAsrEngine.ts) 封装 worker 模型推理 | Kotone 已有 Rust/sherpa 运行时，无须为提醒复制一套浏览器识别链路 |
| 多种 TTS 后端 | [TtsEngine.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/lib/local-inference/engine/TtsEngine.ts) 按清单选择 sherpa-onnx、Piper+、Supertonic 或 Edge；管理 worker、模型缓存、请求与释放。Edge 分支是联网合成，不能因目录名 local-inference 就称全部离线 | 采用模型驱动端口，Kokoro INT8 复用已有原生 sherpa-onnx；不增加 WASM/WebGPU 栈，也不默认使用网络 Edge TTS |
| Native 路径 | [NativeAsrClient.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/lib/local-inference/native/NativeAsrClient.ts)、[NativeTtsClient.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/lib/local-inference/native/NativeTtsClient.ts) 通过本机 WebSocket 与原生服务通信，TTS 有请求 ID、流块回调和取消 | 借鉴请求身份和取消协议；不为 Windows 短提醒额外引入 Python 服务、端口治理或虚拟麦克风 |

当前代码里的 [IClient.ts](https://github.com/kizuna-ai-lab/sokuji/blob/d2f8001beb6d8a7b34906667fccff4ee1a403431/src/services/interfaces/IClient.ts) 明确标为 legacy；因此不把旧 ClientFactory 抽象作为新功能的主架构蓝本。

## 选型过程与当前默认

初版使用无额外模型的 SAPI；试听反馈认为音质不足后，曾将 Kokoro INT8 中文模型作为默认引擎进行开发和实测，约 147 MB、CPU 推理、复用现有原生运行时。最新产品决定恢复 Windows 系统语音为默认，无需下载 Kotone TTS 模型；Kokoro 保留为用户主动选择和手动下载的选项，已有明确配置不覆盖。历史测量保留在 [TTS 对照与实测](research-tts.md)。Windows SAPI 支持默认输出设备、已安装声音和异步播放；`ISpVoice::Speak` 支持清空待播内容。依据：[Microsoft Simple TTS Guide](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ee431810(v=vs.85))、[ISpVoice::Speak](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ee125024(v=vs.85))。

实现采用专用 COM 线程和有界队列，按普通文本播报，不解释 AI 文本中的 XML；开始采音时取消当前和排队播报，旧任务不能在取消后重新入队。声音列表来自系统，不承诺中文声音或统一音质。没有合适声音、设备故障或游戏独占全屏遮挡弹幕时，要保留真实失败状态和可见降级。

Kokoro 已作为同一 `SpeechOutput` 端口下的可选本地模型后端实现，包体、内存、加载延迟、取消与许可需分别核对。提醒对象是屏幕前的用户本人，不沿用翻译/语音转发产品向第三人输出的定位；语音入口依赖 Kotone 运行时启动，独立交互模式默认对讲机，文字入口和已保存提醒的调度不受语音启停影响。当前实现与约束见 [ADR-012](adr/012-reminder-assistant.md)，用户操作见 [助手手册](assistant.md)。
