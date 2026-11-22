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
| [019 琴音在游戏里看屏幕、动鼠标](019-game-computer-use.md) | Accepted / 已采纳（设计，待实现） | 底层与自动化共用；新增 `game.look`（截图 + 编号文字元素）与 `game.act`（批量动作）；动作清单照抄 Anthropic，批量与安全照抄 dsh-computer-use-win，编号照抄 UFO² / OmniParser，游戏循环照抄 Cradle / GameVerse / UI-TARS |
| [020 替玩家在游戏语音里说话](020-voice-into-game-chat.md) | Accepted / 已采纳 | 虚拟麦克风 VB-CABLE（一键安装）、混音、改 Windows 默认麦克风；`voice_chat.*` 工具 |
| [021 听懂说外语的队友](021-hear-teammates.md) | Accepted / 已采纳 | 只录指定程序的声音、本地切句识别，交给“同传”子会话翻译 |
| [022 会话、记忆与 SOUL](022-sessions-memory-soul.md) | Accepted / 已采纳 | 子会话（照 OpenClaw）、工作区（SOUL / USER / MEMORY / 游戏记忆 / Skill，照 Hermes） |
| [023 界面转向陪玩 Agent](023-ui-toward-companion-agent.md) | Accepted / 已采纳 | 侧栏分“语音输入 / 游戏陪玩”，「琴音」页分聊天、角色、记忆、技能、设置 |
| [024 前缀缓存、上下文压缩与用量账本](024-prefix-cache-compaction-usage.md) | Accepted / 已采纳 | 照 DeepSeek Harness：系统提示与工具清单一段对话不变、会变的写进每轮附注、只追加；照 Goose 压缩（请求形状照 dsh，整理也命中缓存）；usage.jsonl 记 token、命中、费用 |
| [025 角色、自定义唤醒词与对局复盘](025-roles-wake-words-match-recap.md) | Accepted / 已采纳 | 角色卡照 Character Card V2；唤醒词照 sherpa-onnx text2token 转拼音并校验；游戏进程退出时自动复盘写进游戏记忆（陪玩特有，自己设计） |
| [026 不要通用读写执行；按 DeepSeek 最佳实践对齐](026-no-generic-rwx-deepseek-best-practices.md) | Accepted / 已采纳 | 琴音没有 read / write / bash（陪玩特有；第 1 条由 030 修订）；联网搜索用 DeepSeek 搜索与抓取接口做成 `web.search` / `web.fetch`；DeepSeek 连接用自己的客户端：`latest_reminder`、`title`、`safety_check`、最后一块报错、内容审核拒绝 |
| [027 0.1.x 到 0.2.0 过渡](027-0.2-upgrade-transition.md) | 候选方案，待验收和发布确认 | 旧用户优先保留语音输入，独立通道分桶，维护版升级前备份 |
| [028 按“游戏陪玩”重新设计交互流程和界面](028-game-companion-interaction.md) | Accepted / 已采纳 | 打开就是聊天页，一个 DeepSeek Key 打开全部功能；语音输入是子板块、不经过 Agent；“启动”只属于本地语音识别；「琴音 → 设置」并入「设置」；老用户可以一切照旧 |
| [029 像打电话一样和她说话：快速主会话 + 完整子代理](029-voice-call-and-background-tasks.md) | Accepted / 已确认，实现中 | 语音回合不思考、短回答、说话那一刻截屏、预生成等待短句、答完接着听 8 秒、按键打断、边生成边念；复杂的事交给玩家看不到的子代理（照 Codex / OpenCode / Goose），做完由主会话说一句 |
| [030 子代理可以执行命令、读写文件](030-subagent-shell-and-files.md) | Accepted / 已确认 | 主会话仍然没有；子代理有 `shell.run` / `file.*`，只能由玩家派；读过外面的东西（屏幕、聊天、网页、外部文件）就先问一句（照 Meta Agents Rule of Two）；删除类先问、模拟键鼠不让做（照 Codex execpolicy）；改文件先备份 |
| [031 内置浏览器与浏览器操作](031-built-in-browser.md) | Proposed / 方向已确认，0.2.0 之后 | 后台任务在 Kotone 隐藏的 WebView2 里操作网页：快照和点击照 dsh-ego-browser（调试协议、节点编号、不抢前台），安全外壳照 dsh-browser 和 xylt369 url-guard；程序内调用、不开端口、不驱动玩家的 Chrome；登录由玩家扫码；第一个用处是点歌 |
| [032 往游戏聊天里发字，不被玩家的按键打断](032-game-chat-send-reliability.md) | Accepted / 已确认，待实现 | 发送的约 220ms 里用已有的低层钩子吞掉玩家的按下，不替玩家补按任何键；新增 CS2 内置配置；Dota 2 / CS2 可由玩家打开“不开聊天框直接发”（写 `say_team` 到 cfg 再按一个绑定键，照 cs2-chat-translator）；英雄联盟局内没有官方写接口 |
| [033 呼叫就是通话：一直在听、开口就能打断、说“退下”挂断](033-call-mode.md) | Proposed / 已实现，待真机验证 | rc3 真人测试反馈：呼叫键 / 唤醒 = 接通通话，一直在听；回声消除照 orb（扬声器回采作参考 + 纯 Rust 的 sonora AEC3）；她说话时开口先压低、识别出真话再停（空的、“嗯”、她自己的回声不算）；“退下 / 安静”随时挂断，30 秒没人说话挂断；用嘴问过的确认之后整句说“可以 / 不要”也算回答；修订 ADR 029 第五节 |
