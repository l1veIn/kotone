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
| 自动化 / Automation | `automation.*`、`kotone-script` | 替用户持续或稍后做事：**定义**（名字、入参、动作）+ **触发**（立即 / 延迟 / 指定时刻 / 热键）+ **执行记录**。动作是“播报”或“程序”（沙盒 Luau）。提醒是“延迟 + 播报”的自动化（ADR-013）。用户可见处不叫“脚本”。 |
| 执行记录 / Automation run | `automation.runs` | 一次执行：入参、目标窗口、到期时间、状态（等待中 / 运行中 / 完成 / 失败 / 取消 / 错过）、日志。最多执行一次，不自动重试，错过不补。 |
| 漂浮弹幕 / Danmaku | `DeliveryKind::Danmaku`, `DanmakuPresentation`, `ReminderDelivery` | 到期时从右向左滑动的单行桌面文字，可带小表情前缀；窗口透明、无底板、点击穿透。速度以 CSS 像素/秒定义，按实际条宽计算时长；不是聊天室消息，不能保证覆盖独占全屏游戏。 |

## 本机工具服务

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 工具服务 / Tool service | `kotone-tools`, `ToolService`, `toolService.enabled` | 桌面实例托管、默认关闭的 loopback HTTP 服务，供 Agent 调用确定性的本机工具；独立于 ASR 运行时。不是 MCP/DSH 插件，也不自行规划任务。 |
| 描述文件 / Descriptor | `Descriptor`, `tool-service-s<会话>.json` | 当前用户、当前 Windows 会话私有的发现文件，含协议版本、实例 ID、PID、地址与会话凭证；客户端校验权限、会话与进程身份后才连接。 |
| 能力清单 / Capabilities | `system.capabilities`, `GET /v1/capabilities` | 本实例真实实现的工具及其输入/输出 schema、就绪状态与限制；路线图工具不在其中。 |
| 窗口引用 / Window reference | `windowId`, `WindowIdentity` | 不透明 ID，绑定窗口句柄、PID 与进程创建时间，仅在发放它的实例内有效；捕获前重新校验。 |
| 前台可见客户区捕获 / Foreground visible client capture | `gdi-screen-foreground-client` | 目标为前台时复制其客户区内屏幕已合成的物理像素；覆盖其上的窗口会被一并捕获，不支持后台/最小化/隐藏窗口，独占全屏可能得到黑图。 |
| 产物 / Artifact | `artifactId`, `ArtifactStore` | 不可变的内存 PNG 图像及元数据（SHA-256、尺寸、TTL、父/根产物、到根图的变换）；裁剪生成新产物，原图不变。 |
| 帧 / Frame | `frameId` | 一次采集的标识；同一捕获的裁剪/预处理链共享，不同帧的坐标不可混用。 |
| 预处理 / Preprocess | `image.preprocess`, `preset`, `operations` | 显式的缩放/灰度/反色/自动对比度/阈值操作，生成新产物并记录实际比例；`preset=none` 不处理。不是 OCR 的强制前置步骤。 |
| 根变换 / Root transform | `rootTransform` | `rootPoint = offset + scale × point`：把任意派生产物（含放大后再裁剪）中的像素映射回原始捕获的物理像素。 |
| OCR 后端 / OCR backend | `OcrBackend`, `local-ppocrv5-korean`, `deepseek-ocr` | 可插拔的文字识别实现，显式选择、互不降级；本地不外发，在线只上传所选图像且需用户指定连接。 |
| 文字观测 / Observation | `observations[]`, `granularity` | OCR 返回的一段原文及其几何：`line` 行框、`block` 整块框（块内行无坐标）、`page`/无几何；置信度可空，不是语义正确率。 |
| 跟踪会话 / Tracking session | `chat.track.*`, `session` | 调用方命名的聊天跟踪状态：消息、事件、帧记录；有容量与空闲 TTL。 |
| 消息 ID 与修订 / Message ID, revision | `messageId`, `revision` | 一个可视文本行在代际内的稳定身份；OCR 改字保持 ID 并 revision+1；同文重复发送可以是新 ID。 |
| 代际 / Generation | `generation`, `chat.track.reset` | 新对局、重连、换 ROI 等边界；重置后清空消息与事件，旧游标失效。 |
| 游标 / Cursor | `cursor`, `newSince` | 绑定会话与代际的事件位置，只用于读取其后的新增与修订；`visible` 读取与游标无关。 |
| 事件总线 / Event bus | `EventBus`, `event.read`, `GET /v1/events` | 工具服务内 Kotone → Agent 的有界事件流（`user.request`、`agent.*`、`runtime.phase`、`app.*`）；游标 `ev1.<实例>.<序号>` 只在本实例有效，失效报错而不跳过。与聊天跟踪的游标是两回事。 |
| 用户请求 / User request | `user.request`, `AgentRequests`, `areq_…` | 用户经 Kotone（`assistant.route=agent` 时的助手热键语音或文字入口）发给外部 Agent 的原文；pending → claimed → replied / expired，十分钟有效。原文对 Kotone 是数据。 |
| 助手去向 / Assistant route | `AssistantRoute`, `assistant.route` | 助手入口把请求交给谁：`reminders`（默认，Kotone 自己的 AI 连接规划提醒）或 `agent`（发布 `user.request`）。 |
| 领取 / Claim | `agent.request.claim`, `consumer` | Agent 处理用户请求前声明归属；同一领取者幂等，其他领取者被拒绝，保证多个 Agent 在线时一条请求只处理一次。 |
| Agent 回复 / Agent reply | `agent.reply`, `deliveryId` | Agent 经 Kotone 对用户说话（语音/弹幕），复用提醒的交付路径；默认受理即返回，交付结果以 `agent.reply.delivered/failed` 事件给出。 |
| 桌面宿主 / App host | `AppHost`, `DesktopHost` | 工具服务调用应用级能力（启停运行时、退出、交付回复、助手提交）的端口；`kotone-tools` 不依赖 Tauri。 |
| 后台启动 / Background launch | `--background`, `kotone-cli app launch` | 只驻留托盘、不打开设置窗口、不抢焦点的启动方式；进程不继承调用方句柄。 |

## 四种交互模式

下表描述游戏语音输入的四种模式；助手仅复用前三种的采集规则，提交对象是提醒助手。助手录音笔预览后确认创建请求，其余两种模式结束后提交。

| 产品名称 | 配置值 | 开始 | 结束与交付 |
| --- | --- | --- | --- |
| 对讲机 | `push-to-talk` | 按住热键 | 松手结束，转写/处理成功后直接发送 |
| 录音笔 | `dictation` | 点按热键 | 再点停止，显示只读预览，再确认发送 |
| 说一句就走 | `one-shot` | 点按热键 | VAD 静音判停后发送；热键可强制结束 |
| 独奏模式 | `solo` | 点按热键 | 每段判停后发送并继续监听，再点停止 |

文档中的“确认发送”不意味着可编辑预览。首字延迟也不等于用户等待时间：评测的全速回放不模拟真实说话节奏，模型预热不包含在采音后的计时里。
