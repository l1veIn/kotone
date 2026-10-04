# 架构决策记录

ADR 保留当时的上下文和选择，状态决定其适用性。当前架构与术语分别见 [architecture.md](../architecture.md) 和 [glossary.md](../glossary.md)。修订状态不表示原始方案从未实现；表示后续实现已改变其中部分约束。

| ADR | 状态 | 当前说明 |
| --- | --- | --- |
| [001 workspace 布局](001-workspace-crate-layout.md) | Amended / 已修订 | core 与实现层依赖方向保留；目录和 crate 数量已扩展 |
| [002 apps/crates 归位](002-apps-crates-monorepo-layout.md) | Accepted / 已采纳 | 目录原则仍适用 |
| [003 whisper sidecar](003-whisper-sidecar-engine.md) | Superseded / 已替代 | sidecar 已删除，当前使用 sherpa 配方与可选远程适配器 |
| [004 sherpa 流式](004-sherpa-onnx-streaming-engine.md) | Amended / 已修订 | sherpa 绑定保留，模型/默认 feature/引擎身份已变 |
| [005 评测](005-eval-recording-replay.md) | Amended / 已修订 | 录档默认关闭，目标与缓存遵循 ADR-011 |
| [006 交互模式](006-session-lifecycle-interaction-modes.md) | Amended / 已修订 | 只读预览保留；当前四预设含 solo |
| [007 VAD](007-vad-one-shot-mode.md) | Amended / 已修订 | 默认特性、内置模型和兜底已落地 |
| [008 聊天频道](008-chat-channels.md) | Accepted / 已采纳 | 频道属于游戏配置 |
| [009 后处理](009-post-processing-pipeline.md) | Amended / 已修订 | 连接、密钥、schema、在线处理与双文本已实现，共享AI层抽出 |
| [010 提示音](010-sound-feedback.md) | Accepted / 已采纳 | 已随 0.1.9 发布；与 TTS 不是同一能力 |
| [011 评测身份](011-eval-target-identity.md) | Accepted / 已采纳 | 明确模型与配置、源指纹和独占评测资源 |
| [012 提醒助手](012-reminder-assistant.md) | Accepted / 已采纳 | 独立入口、受限 AI 工具、本地调度、系统语音和弹幕 |
