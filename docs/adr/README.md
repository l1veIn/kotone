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
| [012 提醒助手](012-reminder-assistant.md) | Amended / 已修订 | 独立入口、受限 AI 工具、系统语音和弹幕保留；调度并入自动化（ADR-013） |
| [013 自动化领域模型](013-automation-domain-model.md) | Accepted / 已采纳 | 定义 / 触发 / 执行记录三层，一个调度器，提醒 = 延迟 + 播报；最多一次、错过不补 |
| [014 游戏场景 Agent 运行框架](014-game-agent-harness.md) | Accepted / 已采纳 | 基础能力 → 自动化 → Agent 三层；内置 Agent（聊天窗口 + 游戏内语音），权限为允许 / 询问 / 禁止；取代陪玩“一次决策”设计 |
| [015 内置 Agent 的完成度与技术栈](015-agent-scope-and-stack.md) | Accepted / 已采纳 | 第一版对标 pi 的精简完整度、游戏相关层做深；Rust 单进程，不带 Node；模型通信层用 rig-core，循环自己写；边界用 MCP / SKILL.md / ACP 式事件 |
| [016 前端引入 shadcn-svelte](016-ui-foundation-shadcn.md) | Accepted / 已采纳 | 分三步：底子 → 「琴音」聊天页（复用 RambleDesk 的聊天实现）→ 其他页面逐页迁移；悬浮窗与弹幕窗不改 |
| [017 语音唤醒与执行中的语音接话](017-voice-wake.md) | Accepted / 已采纳 | sherpa-onnx 关键词检测（“琴音琴音”），模型按需下载；唤醒后与陪玩热键同一条收音与琴音流程；应答“嗯？/我在”，玩家开口即打断；1.5 秒先接一句；默认关 |
| [018 琴音的对话气泡](018-companion-bubble.md) | Accepted / 已采纳 | 语音和琴音说话时用屏幕角落的独立气泡窗（头像 + 状态 + 一句话），不再借用语音输入的悬浮窗；整窗鼠标穿透、不抢焦点；回答默认不走弹幕 |
