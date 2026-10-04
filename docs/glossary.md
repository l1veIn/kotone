# Kotone 术语表

这是产品文案、代码评审和架构说明的权威术语表。代码标识保持原样；中文表述优先使用下表。模型能力清单以 `kotone-stt::model` 为准，配置字段以 Rust 定义为准。

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 运行时 / Runtime | `RuntimePhase`, `RuntimeStatus` | 应用级 stopped → starting → running → stopping 生命周期；包含模型预热、热键和资源释放。不是一句话的识别状态。 |
| 识别会话 / Session | `Orchestrator`, `OrchestratorState`, `SttSession` | 一次采音、识别、处理和交付；`OrchestratorState` 表示业务阶段，`SttSession` 是识别端口实例。取消后的迟到结果不能继续交付。 |
| 交互模式 / Interaction mode | `InteractionMode` | 决定怎样开始、结束以及直接发送或预览。包括对讲机、录音笔、说一句就走、独奏模式。`HotkeyMode` 仅描述按住/点按，不等于完整交互模式。 |
| 识别引擎 / STT engine | `SttEngine`, `EngineRegistry`, `engineId` | 执行识别的适配器/运行循环，例如 `sherpa-streaming`、`sherpa-offline`。不是模型家族；旧 `sherpa-onnx-*` ID 只是兼容别名。 |
| 模型 / Model | `ModelInfo`, `MultiFileModel`, `modelId`, `ModelRecipe` | 具体权重或在线服务条目及其加载配方。X-ASR、SenseVoice、FunASR-Nano 属模型；多个模型可以共用同一引擎。 |
| 流式与本地 / Streaming and local | `ModelIo`, `EngineCapabilities` | `ModelIo::Offline` 表示非流式，`EngineCapabilities.offline` 表示无需远程服务。两者不得混用。“在线 transducer”常指流式解码，不代表上传音频。 |
| 游戏配置 / Game profile | `GameProfile`, `ChatChannel` | 前台程序匹配、聊天打开/发送按键、频道、前缀和热词等游戏适配数据。不是 AI 连接，也不是全局设置。 |
| 连接 / Connection | `Connection`, `connectionId`, `ResolvedConnection` | 可复用的服务地址、协议、模型与凭据引用。凭据由独立存储管理，不嵌入 pipeline、profile 导出或诊断包。 |
| 后处理流水线 / Post-processing pipeline | `PostProcessPipeline`, `PipelineStepConfig`, `TextProcessor` | 有序处理步骤；每步消费上一步输出。处理器是能力定义，步骤是带配置和失败策略的实例。`required` 阻止失败交付；`best-effort` 保留上一文本继续。 |
| 识别原文与待交付文本 / Recognized and ready text | `RecognizedText`, `ReadyText` | 原文来自 STT；待交付文本经过后处理后进入只读预览/注入。处理失败重试处理阶段，发送失败重试现成文本。评测使用识别原文。 |
| 评测目标 / Evaluation target | `EvalTarget` | `engineId + modelId + sessionConfig` 的明确快照。目标指纹区分模型与配置，源指纹区分录音与人工标注；旧无身份结果不能当作当前缓存。 |
| 语音活动检测 / Voice activity detection | `Vad`, `VadConfig` | 识别语音区间、静音判停和音频裁剪；不是语音转文字模型。`vadSilenceMs` 是交互判停等待，`vad.*` 是检测器内部参数。 |

## 提醒能力

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 提醒助手 / Reminder assistant | `assistant`, `voice_reminder`, `danmaku_reminder` | 面向屏幕前用户本人，通过独立热键或文字入口让 AI 提出受限的提醒计划；不向第三人喊话或注入游戏。语音入口依赖运行时启动，文字入口不依赖。 |
| 助手交互模式 / Assistant interaction mode | `AssistantInteractionMode`, `assistant.interactionMode` | 与游戏输入独立的对讲机、录音笔、说一句就走三种模式，默认对讲机，热键默认未绑定；录音笔确认提交给助手，其余模式按各自结束规则提交，不走游戏发送出口。旧助手独奏配置兼容为对讲机。 |
| 提醒 / Reminder | `ReminderIntent`, `Reminder`, `ReminderScheduler` | 一次性的内容、到期时间、输出方式和状态；输出采用简短正文，不附称呼或开场白。本机保存，应用未退出时调度，停止语音运行时不停止既有提醒；不等于系统级闹钟。 |
| 语音播报 / Text to speech (TTS) | `SpeechOutput`, `SpeechRequest`, `SpeechVoice` | 文本转成可听语音，默认 Windows SAPI 声音，无需下载 Kotone TTS 模型；Kokoro 中文模型可选、手动下载。与 STT、提示音分开。 |
| 漂浮弹幕 / Danmaku | `DeliveryKind::Danmaku`, `DanmakuPresentation`, `ReminderDelivery` | 到期时从右向左滑动的单行桌面文字，可带小表情前缀；窗口透明、无底板、点击穿透。速度以 CSS 像素/秒定义，按实际条宽计算时长；不是聊天室消息，不能保证覆盖独占全屏游戏。 |

## 四种交互模式

下表描述游戏语音输入的四种模式；助手仅复用前三种的采集规则，提交对象是提醒助手。助手录音笔预览后确认创建请求，其余两种模式结束后提交。

| 产品名称 | 配置值 | 开始 | 结束与交付 |
| --- | --- | --- | --- |
| 对讲机 | `push-to-talk` | 按住热键 | 松手结束，转写/处理成功后直接发送 |
| 录音笔 | `dictation` | 点按热键 | 再点停止，显示只读预览，再确认发送 |
| 说一句就走 | `one-shot` | 点按热键 | VAD 静音判停后发送；热键可强制结束 |
| 独奏模式 | `solo` | 点按热键 | 每段判停后发送并继续监听，再点停止 |

文档中的“确认发送”不意味着可编辑预览。首字延迟也不等于用户等待时间：评测的全速回放不模拟真实说话节奏，模型预热不包含在采音后的计时里。
