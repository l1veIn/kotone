# ADR 022：会话、记忆与 SOUL——琴音有自己的工作区

- 状态：Accepted（2026-11-17）。用户确认：子会话可以发给自己的父会话；不做起名仪式；**全部放进 0.2.0**（以后再升级太麻烦）。用户修订：每个游戏的知识是“游戏记忆”，不是 Skill；Skill 是可以分发的通用技能包。
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
| `memory/games/<游戏配置 id>.md` | 这个游戏的记忆：界面在哪、按什么键、报点黑话、学到的规则（见第 4 节） | 琴音经 `memory` 工具；玩家可直接编辑 | 前台是这个游戏时注入 |
| `skills/<name>/SKILL.md` | 可以分发的通用技能包（见第 5 节） | 玩家导入；琴音经 `skill.manage`（先问） | 只放目录 |

- **注入方式照 Hermes**：会话开始时把这些文件冻结成快照放进系统提示；会话中途写入立即落盘、下一段会话才生效——前缀不变，缓存稳定。
- Kotone 的操作规则（工具怎么用、游戏规矩）仍在代码里（随工具一起变），不做成 `AGENTS.md`；`BOOTSTRAP.md` 首次运行仪式第一版不做（琴音有默认身份）。
- 现在游戏配置里的“陪玩说明”（`companionNotes`）迁进这个游戏的记忆文件。

### 3. 记忆：照 Hermes 的工具

- `memory {target: memory | user | game, game?: 游戏配置 id, operations: [{add | patch | remove, ...}]}`：整批原子生效；写满直接报错（让她自己整理），不悄悄丢旧条目；“学到的流程写进 Skill，不写进记忆”（Hermes 工具描述原文；在 Kotone 里，某个游戏的界面和规则写进这个游戏的记忆）。
- 写入前过注入扫描（照 Hermes `threat_patterns.py`，MIT），命中拒绝；**只有主会话能写**，子会话（里面全是陌生人的话）不能写——队友说的话种不进记忆。
- 每日笔记 `memory/YYYY-MM-DD.md`、压缩前 memory flush、历史会话搜索（Hermes `session_search`）放到以后。

### 4. 游戏记忆：琴音学会玩某个游戏（用户修订）

- 每个游戏一份 `memory/games/<游戏配置 id>.md`：这个游戏的界面在哪、按什么键、报点黑话、学到的规则、这个玩家在这个游戏里的习惯。它是**这个玩家自己的记忆**，不是用来分发的。
- 前台是某个游戏时，它和 `MEMORY.md` 一起在会话开始时注入（同样冻结快照；有上限，例如 4,000 字）。
- `game.look` / `game.act` 摸清一个界面、玩家教了她什么，她就用 `memory {target: game}` 记下来（ADR 019 提示词里“把新发现的规则记下来”终于有地方记了）。
- 游戏配置里现在的“陪玩说明”（`companionNotes`）迁成这个文件的开头。

### 5. Skill：可以分发的通用技能包（agentskills.io 格式）

- Skill 是**可以分享、可以安装的能力包**，和具体某个玩家无关：例如“同传”（怎么派子会话、怎么翻译报点）、“某游戏的日常任务一条龙”（说明 + 自动化 + 找图模板）。照 OpenClaw / Hermes：`skills/<name>/SKILL.md`（YAML frontmatter：name ≤ 64 字、description ≤ 1024 字），可以带附件（自动化、模板图片）。
- 系统提示里只放目录（name、description），全文按需：`skill.view {name, path?}`（Hermes）。
- 安装 / 导出：玩家在设置页导入一个 Skill 文件夹或压缩包，也能导出分享；外部 Agent 认同样的格式。
- 琴音可以把自己摸索出的通用做法整理成 Skill：`skill.manage {create | patch | delete | write_file | remove_file}`（Hermes），**先问玩家**；每次修改保留旧版本可撤销（OpenClaw）；导入和自建的 Skill 都过注入扫描。

### 6. 身份：名字、头像、唤醒词

- 设置页“琴音的身份”：名字、头像（图片 ≤ 2 MB，存进工作区）、唤醒词。**设置优先，没设的回退到 `IDENTITY.md`**（OpenClaw `set-identity` 的规则）。
- 名字出现在提示词、气泡、聊天窗口、导航栏“琴音陪玩”等处（现在写死在约 90 个文件里），统一改成读身份。
- 唤醒词：现在是两个预置拼音串（“琴音琴音” = `q ín y īn q ín y īn`）。改名后可以把“<新名字><新名字>”转成同样的拼音音节，并检查每个音节都在唤醒模型的词表里；准确度要实测，不行就退回预置的。

### 7. 安全

- 陌生人的内容（队友语音、游戏聊天）只进**工具受限的子会话**，不进能动手的主会话（OpenClaw：不可信内容交给不带工具的 reader）；子会话交回父会话的结果仍按不可信数据处理。
- 记忆、游戏记忆、SOUL、Skill 的写入只能由主会话做；SOUL、身份、Skill 先问玩家；全部过注入扫描。

## 实施步骤（0.2.0）

1. 会话：存储加“父会话 / 模型 / 工具范围”字段（旧对话照常读）；`session.spawn / send / list / history / stop`；完成事件回父会话；归属与级联清理；子会话禁用清单。
2. 工作区：`SOUL.md`、`IDENTITY.md`、`USER.md`、`MEMORY.md` 的读写与快照注入；`memory` 工具与注入扫描。
3. 游戏记忆（`companionNotes` 迁移）；Skill：目录、`skill.view`、`skill.manage`、导入 / 导出、版本保留。
4. 身份：设置页（名字、头像、唤醒词），名字从代码里抽出来。
5. ADR 021：`voice.listen.start / stop` 归属会话，“同传”用 `session.spawn` 实现。
6. 文档与第三方声明（OpenClaw、Hermes 的 MIT 声明）。

## 已确认（2026-11-17）

1. 子会话可以 `session.send` 给自己的父会话——可以。
2. 首次运行不做起名仪式——可以。
3. 放进 0.2.0（用户：以后发这些内容升级起来太麻烦）。

## 实施记录（2026-11-18，分支 `claude/kotone-agent`）

1. 会话：对话存储加 `parent`、`label`（旧对话照常读）；`session.spawn / send / list / history / stop` 在桌面端实现（`agent.rs`）。子会话：最多同时 5 个；每轮最多 8 步、90 秒；模型用 `agent.subagents.model`（空 = 和琴音同一个）；只能用 `CHILD_TOOLS` 里的工具；只能给父会话发消息；权限询问一律拒绝（子会话没人能回答）。子会话结束时由代码停掉它名下的监听。玩家在对话列表里选中子会话的记录时只能看，发消息会开新对话。
2. 工作区：`kotone-agent::workspace`（`<KOTONE_HOME>/agent/workspace`），`SOUL.md` / `IDENTITY.md` / `USER.md` / `MEMORY.md` / `memory/games/<id>.md` / `skills/<name>/SKILL.md`；`memory` 工具整批原子生效、有字数上限、写入前做注入扫描；快照在每段对话第一次用时冻结。
3. 游戏记忆：**没做 `companionNotes` 的迁移**——旧的游戏笔记照常读，这个游戏有了游戏记忆以后以游戏记忆为准。Skill：`skill.view`、`skill.manage`（新建、修改、加删文件、删除）、从文件夹导入；“导出”就是在资源管理器里打开 Skill 的文件夹拷走；每次修改前把旧的整个 Skill 文件夹备份一份（之前这里误写成“没做版本保留”，已更正）。内置 Skill：`kotone-config`、`skill-creator`（2026-11-18 补，见下）。
4. 身份：名字、头像在设置 `agent.identity`（设置优先，回退 `IDENTITY.md`，再回退“琴音”），气泡、侧栏、页标题、系统提示都用它。**唤醒词没有跟着名字改**：仍是预置的“琴音琴音 / 你好琴音”；按新名字生成拼音唤醒词需要实测准确度，留到以后。
5. ADR 021 见那边的实施记录。
6. 文档：CHANGELOG、THIRD_PARTY_NOTICES（OpenClaw、Hermes Agent 的 MIT 声明，wasapi-rs 等）。

测试：工作区与工具的单元测试；设置页 Playwright（改名、编辑 SOUL / 记忆 / 游戏记忆、删技能、头像超限）；真机（隔离实例，经 WebView2 调试端口只发 DOM 事件）：读 SOUL、改名写进配置、SOUL 写进文件、注入文本被拒、头像经 Tauri 传进工作区并显示、USER.md 保存、游戏记忆与技能列出。没验证：真实模型下琴音自己写记忆 / 技能的效果。

### 补充（2026-11-18，玩家试用后的反馈）

- 内置两个 Skill：`kotone-config`（只用一个 DeepSeek Key 配好全部功能；本地模型的做法）和 `skill-creator`（参照 Anthropic skill-creator 与 Hermes `skill_manage`）。第一次用时装进工作区，记在 `.bundled-skills.json`：玩家删了不再装回，玩家改过的不被新版本覆盖。系统提示要求做事前先看有没有对得上的 Skill；`skill.view` 不带名字 = 列出全部。
- 新工具 `connections.request_key`：聊天里弹出只给玩家看的输入框，Key 直接存进凭据库，模型只拿到“已保存”和连接 id（原来“Key 只能由玩家在连接页粘贴”的规矩不变，只是不用玩家自己找页面）。
- 真机：`scripts/manual-e2e/happy_path_e2e.py`（隔离实例 + 本地假 DeepSeek）——琴音读 `kotone-config`，把语音识别、文字识别换成 DeepSeek；之后点“呼叫琴音”说话，录音送到 DeepSeek 语音识别，她的回答用 DeepSeek 语音说出来。全部通过。没验证：真实 DeepSeek 模型会不会照着技能做（需要真 Key）。
