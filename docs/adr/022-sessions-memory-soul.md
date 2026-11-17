# ADR 022：会话、记忆与 SOUL——琴音有自己的工作区

- 状态：Proposed（2026-11-17，等用户确认文末的问题）
- 调研：[OpenClaw 与 Hermes Agent](../research/2026-11-17-openclaw-hermes.md)（两者都是 MIT）
- 修订：[ADR 015](015-agent-scope-and-stack.md) 第 1 节的“子 Agent：不做”“Skill：以后做”
- 被依赖：[ADR 021](021-hear-teammates.md)（听队友）

## 问题

讨论“听懂队友”时发现，问题不在缺一个翻译工具，而在**上下文结构**：琴音只有一个主会话，所有入口都进它；持续不断的事（听队友）既不该挤进主会话，也不该变成工具里藏着的另一个大模型。同时琴音已经能“学会玩某个游戏”，却没有地方记下学到的东西；她的人格、名字、头像写死在代码里。用户的判断：Kotone 越来越像 OpenClaw、Hermes，那就照它们的做法，**工具要少**（`session.send` 能解决很多事），并加上记忆和 SOUL。

## 决定

### 1. 会话：照 OpenClaw，工具越少越好

| 工具 | 照抄 | 说明 |
|---|---|---|
| `session.spawn {task, label?, model?, thinking?, context?: fork / isolated, tools?}` | OpenClaw `sessions_spawn` | 建一个子会话并把 `task` 作为第一条消息发给它。`model` / `thinking` 缺省用设置 `agent.subagents.model` / `.thinking`（快模型、低思考）；`fork` = 继承主会话上下文（命中同一份缓存），`isolated` = 全新；`tools` = 这个子会话能用的工具（只能在下面的禁用清单之外收窄） |
| `session.send {id, text}` | OpenClaw `sessions_send` | 往一个会话发一条消息（主会话 → 子会话；子会话 → **只能发给自己的父会话**，见下） |
| `session.list` / `session.history {id}` | OpenClaw | 看有哪些会话、读某个会话的记录（人在对话列表里能做的，她也能做） |
| `session.stop {id}` | OpenClaw `subagents` 取消、`/stop` | 结束一个会话，**级联**停掉它的子会话和它名下的一切 |

- **子会话汇报**：子会话这一轮做完，结果作为一条“完成”系统消息交还父会话（OpenClaw announce；Kotone 现有的“自动化完成回到对话”同一条路：父会话在回答就插话，没在回答就只记下不调用模型）。
- **和 OpenClaw / Hermes 的一处不同**：它们禁止子 agent 主动发消息（`sessions_send` / `send_message`），因为它们的子 agent 做完就结束。Kotone 的“同传”这类子会话是**长期运行**的，中途要能叫琴音（队友在叫你、问你、求救），所以允许子会话 `session.send` **只发给自己的父会话**，不能发给别的会话。
- **子会话禁用的工具**（照 Hermes `DELEGATE_BLOCKED_TOOLS` + OpenClaw 硬禁清单，配置也放不开）：`session.spawn`（深度 1）、写记忆、写 Skill、改 SOUL / 身份、`reply.ask`（问玩家）、`voice_chat.say`（对外说话）、`game.act` 与自动化（动手）、改设置。子会话只加载自己的 `task`，不加载 SOUL、记忆（OpenClaw minimal 模式、Hermes 子 agent 不加载 SOUL）。
- **归属与清理**：长期运行的工具（例如 `voice.listen`）归启动它的会话所有；会话被 `session.stop`、出错退出、Kotone 退出时，代码自动停掉它名下的一切，不依赖模型记得去停。停止热键、语音“停下”也停掉全部子会话。子会话最多 5 个，空闲 60 分钟归档（OpenClaw 默认值）。子会话作为对话保存，在对话列表里标出，玩家能点开看。

### 2. 工作区与 SOUL：照 OpenClaw 的文件名

`<KOTONE_HOME>/agent/workspace/`：

| 文件 | 内容 | 谁写 | 注入 |
|---|---|---|---|
| `SOUL.md` | 人格、语气、边界（现在写死在 `prompt.rs` 的“温暖、轻快、简短”搬到这里作为默认） | 玩家；琴音可以改，但**先问玩家、改了告诉玩家** | 主会话 |
| `IDENTITY.md` | Name、Emoji、Avatar（OpenClaw 字段），加 Kotone 自己的 Wake（唤醒词） | 玩家（设置页）；琴音可改名（先问） | 主会话 |
| `USER.md` | 关于玩家的稳定事实与偏好（≤ 1,375 字，Hermes） | 琴音经 `memory` 工具；玩家可直接编辑 | 主会话 |
| `MEMORY.md` | 精选长期事实（≤ 2,200 字，Hermes） | 同上 | 主会话 |
| `skills/<name>/SKILL.md` | 见第 4 节 | 琴音经 `skill.manage`；玩家可编辑 / 导入 | 只放目录 |

- **注入方式照 Hermes**：会话开始时把这些文件冻结成快照放进系统提示；会话中途写入立即落盘、下一段会话才生效——前缀不变，缓存稳定。
- Kotone 的操作规则（工具怎么用、游戏规矩）仍在代码里（随工具一起变），不做成 `AGENTS.md`；`BOOTSTRAP.md` 首次运行仪式第一版不做（琴音有默认身份）。
- 现在游戏配置里的“陪玩说明”（`companionNotes`）迁成这个游戏的 Skill。

### 3. 记忆：照 Hermes 的工具

- `memory {target: memory | user, operations: [{add | patch | remove, ...}]}`：整批原子生效；写满直接报错（让她自己整理），不悄悄丢旧条目；“学到的流程写进 Skill，不写进记忆”（Hermes 工具描述原文）。
- 写入前过注入扫描（照 Hermes `threat_patterns.py`，MIT），命中拒绝；**只有主会话能写**，子会话（里面全是陌生人的话）不能写——队友说的话种不进记忆。
- 每日笔记 `memory/YYYY-MM-DD.md`、压缩前 memory flush、历史会话搜索（Hermes `session_search`）放到第二期。

### 4. Skill：琴音学会玩某个游戏（agentskills.io 格式）

- `skills/<name>/SKILL.md`（YAML frontmatter：name ≤ 64 字、description ≤ 1024 字；Kotone 加 `games: [进程名]` 用来在前台是这个游戏时提示她），可以带附件（找图模板、自动化）。
- 系统提示里只放目录（name、description），全文按需：`skill.view {name, path?}`（Hermes）。前台是某个游戏时，提示里标出对应的 Skill。
- 琴音自建 / 修改：`skill.manage {create | patch | delete | write_file | remove_file}`（Hermes），**先问玩家**；每次修改保留旧版本可撤销（OpenClaw）；自建的 Skill 也过注入扫描（Hermes 默认关，我们默认开）。
- `game.look` / `game.act` 摸清一个界面、玩家教了她什么，她就提议写进这个游戏的 Skill（ADR 019 提示词里“把新发现的规则记下来”终于有地方记了）。

### 5. 身份：名字、头像、唤醒词

- 设置页“琴音的身份”：名字、头像（图片 ≤ 2 MB，存进工作区）、唤醒词。**设置优先，没设的回退到 `IDENTITY.md`**（OpenClaw `set-identity` 的规则）。
- 名字出现在提示词、气泡、聊天窗口、导航栏“琴音陪玩”等处（现在写死在约 90 个文件里），统一改成读身份。
- 唤醒词：现在是两个预置拼音串（“琴音琴音” = `q ín y īn q ín y īn`）。改名后可以把“<新名字><新名字>”转成同样的拼音音节，并检查每个音节都在唤醒模型的词表里；准确度要实测，不行就退回预置的。

### 6. 安全

- 陌生人的内容（队友语音、游戏聊天）只进**工具受限的子会话**，不进能动手的主会话（OpenClaw：不可信内容交给不带工具的 reader）；子会话交回父会话的结果仍按不可信数据处理。
- 记忆、SOUL、Skill 的写入只能由主会话做、都先问玩家、都过注入扫描。

## 实施步骤（确认后，建议放在 0.2.0 之后的版本）

1. 会话：存储加“父会话 / 模型 / 工具范围”字段（旧对话照常读）；`session.spawn / send / list / history / stop`；完成事件回父会话；归属与级联清理；子会话禁用清单。
2. 工作区：`SOUL.md`、`IDENTITY.md`、`USER.md`、`MEMORY.md` 的读写与快照注入；`memory` 工具与注入扫描。
3. Skill：目录、`skill.view`、`skill.manage`、版本保留；`companionNotes` 迁移。
4. 身份：设置页（名字、头像、唤醒词），名字从代码里抽出来。
5. ADR 021：`voice.listen.start / stop` 归属会话，“同传”用 `session.spawn` 实现。
6. 文档与第三方声明（OpenClaw、Hermes 的 MIT 声明）。

## 待用户确认

1. **子会话可以 `session.send` 给自己的父会话**（OpenClaw / Hermes 都禁止，Kotone 为长期运行的“同传”放开这一条）——可以吗？
2. **首次运行不做“给琴音起名”的仪式**（OpenClaw 的 `BOOTSTRAP.md`），琴音带默认身份，想改去设置页或直接跟她说——可以吗？
3. **放在 0.2.0 之后的版本做**（会话、记忆、SOUL、Skill、身份、听队友都在这一批）——可以吗？
