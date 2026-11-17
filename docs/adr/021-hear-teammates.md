# ADR 021：给琴音一对“耳朵”——听懂说外语的队友

- 状态：Proposed（2026-11-17，等用户确认文末的问题）
- 调研：[让琴音听懂说外语的队友](../research/2026-11-17-hear-teammates.md)
- 相关：[ADR 020](020-voice-into-game-chat.md)（替玩家说话）、[ADR 014](014-game-agent-harness.md)（人能做的 Agent 都能做）

## 问题

队友说另一种语言（PUBG 很常见）。琴音已经能用对方的语言替玩家说话（ADR 020），但玩家听不懂队友。用户的要求：不写死一条“录音 → 翻译 → 显示”的流程，而是**给琴音工具，让她自己解决“知道队友在说什么”**；展示用现有的气泡、语音、弹幕。

## 决定

琴音缺的只有“耳朵”：翻译她本来就会（大模型，还懂游戏黑话），展示方式也基本都有。照她读游戏文字聊天的做法（截图识别 → 整理成一条条消息 → 她读），给她一组“听”的工具，再加一个常驻规则解决逐句跑 Agent 太慢的问题。

### 工具

| 工具 | 作用 | 照抄 |
|---|---|---|
| `voice.apps` | 列出正在出声的程序（PUBG、Discord、KOOK……） | wasapi-rs 示例 |
| `voice.listen.start {app 或 profileId, languages?}` / `voice.listen.stop` / `voice.listen.status` | 开始 / 停止只听某个程序的声音；切句、识别 | 微软 ApplicationLoopback + wasapi-rs（抓音频）；Kotone 现有 Silero VAD + sherpa-onnx SenseVoice（auto-caption 的用法）；VRCT 的过滤参数 |
| `voice.heard.read {since?, limit?}` | 最近听到的话：时间、原文、语种、来自哪个程序；按不可信数据交给模型。每句同时在事件总线发 `voice.heard` | VRCT 的扬声器日志、discord-transcriber 的逐句结构；Kotone 的 `chat.track.*` |
| `bubble.show {text, label?, seconds?}` | 只在右下角气泡里显示一行字（字幕），不出声 | 现有气泡（ADR 018） |
| `voice.translate.start {to, show: bubble / danmaku / voice, languages?}` / `voice.translate.stop` | 常驻规则：每听到一句（不是目标语言的）就做一次很短的翻译并显示，不跑完整的 Agent，1–2 秒出字幕 | LiveTranslate 的“切句 → LLM 翻译 → 悬浮显示” |

已有、不用新做：语音播报 `reply.say`、弹幕、替玩家说话 `voice_chat.say`、翻译本身（模型）。

### 用起来

- “帮我盯着队友说什么” → 她 `voice.apps` 找到 PUBG，`voice.listen.start`，`voice.translate.start {to: zh, show: bubble}`，回一句“好，我在听”。
- “刚才那个俄罗斯人说啥” → `voice.heard.read`，一句话告诉你。
- “跟他说我马上到” → 她看到对方说俄语，用 `voice_chat.say` 说俄语。
- “别翻译了” → 关规则、停止听。

### 约束

- 只听玩家指定的程序（include 模式）：琴音自己的语音、玩家的麦克风都不会被抓进来；整个桌面的声音不听。
- 只在内存里处理，不存音频；识别出的文字只保留最近一段（例如 10 分钟、200 句）；听的时候气泡 / 托盘有“正在听队友”的提示。
- 默认关；`voice.listen.start` 第一次在某个游戏里用时问一次（同 `game.act`）。
- PUBG 的语音和枪声在同一个程序里，识别质量要真机试；Discord / KOOK 的声音干净得多。
- Discord 分不出是谁说的（官方不支持 bot 收音，RPC 要审批），第一版不做说话人区分。

## 实施步骤（确认后）

1. 平台：按进程 loopback（新依赖 `wasapi` crate，MIT）、列出出声的程序；
2. 识别：VAD 切句 + SenseVoice，识别结果环形缓冲、事件；
3. 工具：`voice.apps` / `voice.listen.*` / `voice.heard.read` / `bubble.show` / `voice.translate.*`，琴音的常用工具、权限、提示词；
4. 常驻翻译规则（一次短的模型调用 / 句）；
5. 设置页（可选：开关、显示方式、正在听的提示）；
6. 测试：纯逻辑单元测试；真机用隔离实例 + 一个会播放外语录音的测试程序（不碰用户的游戏）；
7. 文档与第三方声明。

## 待用户确认

1. **语种**：先用 Kotone 已有的 SenseVoice（中英日韩粤，本地、免费、不上传）；俄语、泰语等要再加 Whisper 多语言模型（约 800 MB，按需下载）或在线识别——第一版只做 SenseVoice，可以吗？
2. **常驻翻译默认显示在气泡里**（不出声，不和游戏声音抢）——可以吗？
3. **说话人区分**（Discord 里分清是谁说的）第一版不做——可以吗？
