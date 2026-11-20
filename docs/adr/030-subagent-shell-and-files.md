# ADR 030：子代理可以执行命令、读写文件——读过外面的东西就先问一句

状态：Accepted / 已确认（2026-11-20）。修订 [ADR 026](026-no-generic-rwx-deepseek-best-practices.md) 第 1 条（主会话仍然没有，子代理有）。随 [ADR 029](029-voice-call-and-background-tasks.md) 第二期（子代理）实现。

## 背景

游戏周边有大量成熟的第三方工具，琴音能指挥它们，就不用自己从头做。例如：

- **BetterGI**（原神，GPL-3.0，约 1.8 万星）自带命令行：`startOneDragon [配置名]` 跑一条龙、`--startGroups <组名…>` 跑配置组、按 ID 运行已保存的计划（`BetterGenshinImpact/Helpers/CommandLineOptions.cs`）。Kotone 只调用它的命令行，不链接它的代码。
- **魔兽世界插件**：装在 `Interface/AddOns`，设置存在 `WTF`。

ADR 026 当时不给通用读写执行。现在有了“快速主会话 + 完整子代理”，那四条理由重新看：

| ADR 026 的理由 | 现在 |
| --- | --- |
| 太慢：游戏里要几秒内回答 | 不成立。子代理在后台跑，主会话照样快 |
| 注入：她读陌生人的聊天、屏幕、网页，又碰得到私密数据、能往外发 | 只成立一半。子代理只能由玩家派（源头干净），但任务要读的材料里还可能有“毒”。用第 3 节的规则管 |
| 绕过已有防护：前台判断、每个游戏的开关、Esc、每个工具的允许 / 询问 / 禁止 | 仍然成立。模拟键鼠的命令直接不让做，改用游戏工具（第 4 节）；Esc 结束子代理的命令（第 6 节） |
| 玩家判断不了 `Remove-Item …` 该不该允许 | 改成问“事”不问“命令”：“我照 CurseForge 上的说明装 DBM 插件，要我动手吗？” |

还讨论过“集成包”（Skill 加一份清单：固定程序路径、允许的命令前缀、能读写的目录、能下载的站点）。它把出事时的影响范围框得最死，但每个工具都要写一份，玩家认为复杂度太高，没有采用。

## 决定

### 1. 谁有、有什么

- **只有子代理有**。主会话没有：它要快，而且它直接读聊天和屏幕。经 MCP / 工具服务连进来的外部 Agent 也拿不到（它们有自己的命令行和权限体系，从这里拿就绕过了本 ADR 的规则）。
- 工具照 OpenCode（`shell` / `read` / `write` / `edit` / `glob` / `grep`）和 Claude Code（Bash / Read / Write / Edit / Glob / Grep）的形状：

| 工具 | 做什么 |
| --- | --- |
| `shell.run {command, timeoutMs?, cwd?}` | Windows PowerShell 5.1（系统自带，`-NoProfile -NonInteractive`）；默认 2 分钟、最长 10 分钟；输出截到约 3 万字（照 Claude Code） |
| `file.read {path, offset?, limit?}` | 读文本文件，带行号 |
| `file.write {path, content}` | 写整个文件 |
| `file.edit {path, old, new}` | `old` 必须恰好出现一次（和现有 `skill.patch` 一样） |
| `file.find {pattern, path?, contains?}` | 按文件名找，可以再按内容筛 |

- 默认工作目录是 Kotone 自己的文件区 `<KOTONE_HOME>/agent/files/`。
- 改 Kotone 文件区以外的文件前，先把原文件备份到 `<KOTONE_HOME>/agent/backups/<时间>/`（照 Skill 修改前备份，可撤销）。
- 设置里照旧可以给每个工具设允许 / 询问 / 禁止；默认“允许”，即按本 ADR 的规则决定什么时候问。

### 2. 源头：只有玩家能派

- `agent.start` / `agent.followup` 只在**玩家亲口说或亲手打字**的那一轮可用。自动化触发、提醒到点、子代理做完后唤醒主会话说一句的那一轮，都不能派子代理。
- 交给子代理的任务附上玩家这一轮的**原话**，由运行时从对话里取，不经模型转述。子代理以原话为准，主会话写的任务说明只是参考。

### 3. 读过外面的东西就先问一句（照 Meta “Agents Rule of Two”）

Meta 的规则：一个代理在一次任务里，下面三样最多同时占两样——读不可信的内容；碰私密数据或系统；改东西或往外发。三样都要占时，留一个人确认。命令行一个就占了后两样，所以：

- **什么算“外面的东西”**：
  - 结果标了不可信的工具（屏幕、文字识别、游戏聊天、联网搜索和抓取；现有 `untrusted` 标记，`crates/kotone-agent/src/catalog.rs`）；
  - `file.read` 读 Kotone 文件区以外的文件；
  - `shell.run` 里读文件内容的命令（`Get-Content`、`gc`、`cat`、`type`、`Select-String` 等，按命令前缀判断）。
  - 其余命令的输出（列目录、查进程、退出码）不算，否则第二条命令就要问。提示词要求读文件用 `file.read`（照 Claude Code：读文件用 Read，不用 cat）。
- **算谁的**：这个子代理自己拿到的结果，加上派它的那一轮主会话（从玩家这句话开始）拿到的结果。
- **读过之后**：下一次 `shell.run`、`file.write`、`file.edit` 之前，子代理用一句话说要做什么，经主会话问玩家（语音时开口问，打字时是确认卡片，卡片里能展开看具体命令）。玩家同意后，这个任务剩下的步骤不再因为这条规则问；玩家下一次追加任务时重新算。
- **没读过**：直接做。

判断全在代码里，不靠模型自觉。

### 4. 先问的命令、不让做的命令（照 Codex execpolicy）

照 Codex `execpolicy` 的前缀规则写法（`prefix_rule(pattern, decision, justification, match, not_match)`）：规则随程序发，每条带正反例，例子当单元测试跑。

- **永远先问**（不管读没读过外面的东西）：删除（`Remove-Item`、`del`、`rm`、`rd`、`rmdir`）、改注册表、格式化或分区、关机重启、改执行策略、改用户账户、停系统服务。问的时候说清删什么、多少个文件。
- **不让做**，并告诉模型改用什么：模拟键鼠（`SendKeys`、`keybd_event`、`SendInput`、`mouse_event`）→ 用 `game.act` / `game_chat.send`，它们有前台判断、每个游戏的开关和 Esc。

按字面前缀匹配，挡的是模型的习惯和失误，挡不住刻意绕过；刻意绕过来自注入，由第 3 节管。

### 5. 游戏和第三方工具

- **游戏目录**：写进 Kotone 认识的游戏的安装目录（游戏配置里有程序路径）时，每个游戏第一次先问，并说一句封号风险（反作弊会检查游戏文件）。游戏正在运行时不写它的目录（魔兽世界退出时会覆盖 `WTF` 里的设置），直接报错“游戏开着，退出后再改”。
- **第三方工具**（BetterGI 等）：不额外拦。提示词要求第一次启动某个工具前说一句它是第三方工具、用不用由玩家决定。

### 6. 停下

Esc、“停下”、卡片上的停止：结束这个子代理正在运行的命令和它们的子进程（Windows Job Object）。玩家要它启动并一直开着的程序（例如 BetterGI）不算，要停得明说。

## 玩家看到的

- “写个脚本，每天早上 4 点用 BetterGI 跑一条龙。”——不问，做完主会话说一句“建好了，每天 4 点跑”。
- “帮我装一下 DBM 插件。”——子代理搜过网页，动手前主会话问：“我从 CurseForge 下载 DBM，装到魔兽的插件目录，要我动手吗？”
- “把插件目录里没用的都清了。”——删除前问：“要删掉 12 个插件文件夹，可以吗？”

## 已知风险

1. 玩家同意之后，带毒的材料还在子代理的上下文里，可能让它多做一点事。和在 Claude Code 里点“允许”是同一级别。
2. 只看派它的那一轮主会话：更早读过的聊天、屏幕如果被模型写进任务，标记不到。缓解：任务附玩家原话。
3. 用命令读文件的写法很多，按前缀判断会漏。
4. Rule of Two 不管模型自己的失误（Meta 原文也这么说）。删除类先问、改文件先备份兜一部分；命令行直接改文件不在备份范围里。
5. 没有沙盒：命令以玩家的身份运行，能读玩家能读的一切。以后要给开发者更放开的模式时，照 Codex `windows-sandbox-rs`（独立沙盒用户、受限令牌、按工作区设文件权限、WFP 断网）。
6. 反作弊：调用第三方工具、改游戏文件的封号风险由玩家承担，琴音只负责说清楚。

## 实现情况

2026-11-20，分支 `claude/voice-call`，随 ADR 029 第二期。

- **执行引擎**：crate `kotone-exec`（`crates/kotone-exec/`）。规则表在 `src/policy.rs` 的 `RULES`，每条带正反例，单测逐条验证；`check()` 判断直接做 / 先问 / 不让做，`run_tool()` 执行，`Taint` 记“读过外面的东西”。
- **接线**：
  - 五个工具由 `kotone_agent::catalog::exec_entries` 从 `kotone_exec::tool_specs()` 进清单（分组 `computer`，设置页叫“电脑（命令和文件）”，默认规则“允许”），来源是 `Source::Exec`。
  - 工具不在 `MAIN_TOOLS` 里，主会话拿不到；也不在工具服务里，所以 MCP、`kotone.exe tools` 和外部 Agent 都拿不到。kotone-agent 把 kotone-exec 转出为 `kotone_agent::exec`。
  - 端口把调用交给 `BuiltinHost::exec`，桌面端在 `agent.rs` 的 `exec_tool` 里执行。判断流程在 `kotone_agent::subagents::run_exec`：规则 → 经主会话问 → 执行 → 记下读到的外面的东西。
  - 玩家把某个工具设成“每次询问”时，桌面端照样先问一句，而且只问一遍；设成“禁止”或开着只读模式时，不交给桌面。
  - `ExecConfig` 的游戏目录取游戏配置里“启动路径”是 `.exe` 的所在文件夹，用进程名判断游戏开没开着。
- **“读过外面的东西”存在哪、从哪来**：存在后台任务的登记里（`Subagent.taint` / `taintApproved`）。来源有三处：
  1. 后台任务里结果标了不可信的非执行工具，以及收到的别人说的话（同传听到的句子）和附带数据。
  2. 执行工具自己报的 `taints`。执行工具的结果虽然也按不可信包给模型，但标记只认 `taints`，否则第二条命令就要问。
  3. 派它的那一轮主会话：从玩家这句话开始拿到的不可信结果，或者玩家这句话本身附带了截图 / 数据。

  玩家追加任务（`agent.followup`）时，“同意过”清掉，读过的标记留着，再加上新那一轮主会话读过的。卡片上点“继续”只清“同意过”。
- **先问的入口**：后台任务的确认（ADR 029 第三节）。语音时主会话开口问，打字时在卡片上点；问句就是 `check()` 给的那句。
- **和上面写的不一样的地方**：
  - 写进 Kotone 自己的文件区不因读过外面的东西而问：不改变玩家电脑上的东西，真要执行时 `shell.run` 照样会问。
  - Kotone 自己的设置、记忆、技能、对话（`<KOTONE_HOME>` 里文件区以外）一律不能直接写，要用 `settings.update`、`memory`、`skill.manage`，否则会绕过它们写入前的检查。
  - `Invoke-Expression` / `iex` 也在“永远先问”名单里，因为 `iwr … | iex` 是最常见的注入写法。
  - “本段对话都允许”对这些询问不适用，按“允许这一次”处理，删除这类操作每次都问。
- **“读过外面的东西”不算的**：Kotone 自己给语音这句话附上的“玩家开口时的前台窗口”说明（窗口标题、程序名，`speech_screen::is_turn_note`）。屏幕上的文字要经 `screen.at_speech` 读，那个算。真机发现：算上它的话，所有用嘴派的活执行命令前都要问，打字派的却不用。已知缺口：窗口标题本身可能是别人写的（例如网页标题）。
- **真机**（真实 DeepSeek，2026-11-20，隔离目录，不发系统按键，音量 0 / 8）：

  打字（`scripts/manual-e2e/background_task_real.py --exec-only`，同一段对话），第三次运行 16 / 16：

  | 说的 | 问了几次 | 后台用的工具 | 用时 | 费用 | 她说的 |
  | --- | --- | --- | --- | --- | --- |
  | 在你的文件夹里写一个 notes.txt，内容是今天的日期 | 0 | shell.run、file.write、file.read | 11 秒 | ¥0.056 | 写好了，一行 2026-11-20…… |
  | 搜一下 PowerShell 怎么列出文件夹大小，然后照着在你的文件夹里跑一下 | 1（“我要列出琴音文件文件夹里各子文件夹的大小（只读），要我动手吗？”）→ 允许 | web.search、shell.run ×2 | 14 秒 | ¥0.025 | 跑完了：我的文件夹里没有子文件夹，只有那个 notes.txt…… |
  | 把你文件夹里的 notes.txt 删掉 | 1（删除）→ 拒绝 | shell.run | 10 秒 | ¥0.049 | 刚才那次删除被拒绝了，notes.txt 还在原地没动…… |

  三次运行里发现并修掉的：
  1. 主会话有一次先在聊天里自己问“确定删吗”才交出去，玩家同意的话后台还会再问一遍。现在主会话的提示词写明删东西、跑命令直接交给后台，后台动手前会问。
  2. 模型把路径放进变量（`$p = …; Remove-Item $p`），问句念成“这一步会删除：$p”。现在变量不列出，改说“这一步会删除文件或文件夹”。
  3. 第一次运行时脚本自己判断完成的方式不对，没有接住 4 秒内就做完的情况。

  第二次运行的 E1 由后台在 4 秒内做完、当场回答。读过网页后，连只读的 `Get-ChildItem` 也要先问一次；问一次后这件事剩下的命令不再问，这是 ADR 写的“每轮一次”。

  语音（`scripts/manual-e2e/background_voice_real.py`）：用嘴说“把你文件夹里的 notes.txt 删掉”，后台删除前要确认，主会话开口问，用嘴回答“可以”后删掉并开口说结果，8 / 8。修了一个和接着听抢时间的问题（见 ADR 029 实现情况）。
- **单测**：
  - kotone-exec：36 个。
  - kotone-agent：5 个流程，分别是文件区里写入不问；读外部文件后改文件先问、拒绝不做、同意后这一轮不再问、追加任务后再问；删除每次都问、“本段对话都允许”不跨次；模拟键鼠直接拒绝；玩家设“每次询问”时也问。另有清单和端口的 3 个：执行工具只给后台、不在工具服务里；玩家规则；没有桌面时不可用。
  - kotone-ai：登记字段的兼容。

## 参照

- Meta AI，《Agents Rule of Two: A Practical Approach to AI Agent Security》，2025-10-31，https://ai.meta.com/blog/practical-ai-agent-security/ ；Simon Willison 的介绍，https://simonwillison.net/2025/Nov/2/new-prompt-injection-papers/
- OpenAI Codex CLI（Apache-2.0）`codex-rs/execpolicy/README.md`（前缀规则、`forbidden` 附替代建议、正反例当测试）、`codex-rs/windows-sandbox-rs/`。
- OpenCode（MIT）`packages/core/src/tool/plugin/{shell,read,write,edit,glob,grep}.ts`。
- Claude Code 的 Bash / Read / Write / Edit / Glob / Grep（超时默认 2 分钟、最长 10 分钟；读文件用 Read 不用 cat）。
- BetterGI（GPL-3.0）`BetterGenshinImpact/Helpers/CommandLineOptions.cs`。
