# ADR 032 真机验证：玩家照着做的步骤

对应 [ADR 032](adr/032-game-chat-send-reliability.md) 的“真机验证清单”。这些项要真人按键或进游戏，Kotone 的自动测试做不了。每项做完在 ADR 末尾“实现情况”的表里把“等玩家”改成结果（通过 / 不通过 + 一句现象）。

## 准备

- 用 0.2.0-rc.3 安装包，或在仓库里 `pnpm dev`（`claude/adr-032` 合并后的 main）。
- **每次打开 Kotone 后的第一条消息没有挡键保护**（钩子还没见过 Kotone 自己的键）。每次重开 Kotone 后先随便发一条，再开始测。
- 看 Kotone 实际怎么发的：`%USERPROFILE%\.kotone\diagnostics\events.jsonl`，搜
  - `inject_gate`：发送窗口里吞了几个键（`swallowed=3 windowMs=259`）；
  - `console_chat_send`：走了控制台通道；
  - `console_chat_skipped`：声明了控制台通道但这次没走，`detail` 是原因（例如“游戏是在打开开关之前启动的”）。
- 只在训练模式、人机或自建房间里测（S7 除外）。

## 挡键（第一期）：L1–L3、M1、D1

“发送瞬间”= 说完话、松开语音键（或按下发送）之后马上；消息大约 0.2 秒内打完。

| 编号 | 在哪 | 怎么做 | 期望 / 要记的 |
| --- | --- | --- | --- |
| L1 | 英雄联盟训练模式，输入法切到微软拼音**中文**状态 | 用语音发一句，发送瞬间连按 Q W E R | 消息完整发出；没有输入法候选框；聊天框关上。日志里有 `inject_gate`，`swallowed` 大于 0 |
| L2 | 同上 | 发送瞬间按下 Shift 并按住 | 消息在**队伍**频道；松开 Shift 再按，Shift 组合键（例如 Shift+技能）正常 |
| L3 | 同上 | 发送瞬间按下一个技能键（例如 Q）并按住不放 | 不卡键；按住一会儿后技能被游戏认到（大约晚半秒，靠系统按键重复）；松开后一切正常 |
| M1 | 英雄联盟训练模式、Dota 2 人机 | 发送瞬间左键、右键点地面 | 只记录：消息有没有发不出去、聊天框有没有卡住。第一期不挡鼠标，所以在新版上测和“改之前”一样 |
| D1 | Dota 2 人机 | 同 L1–L3 | 同上。如果字母还是进了聊天框，在控制台输入 `cl_input_enable_raw_keyboard` 回车，记下它的值 |

## 控制台通道（第三期，Dota 2）：S1–S7

内置的 Dota 2 配置**还没打开**这一项（没实测过）。测试时先在自己的配置里加上：

1. 退出 Kotone 和 Dota 2。
2. 打开 `%USERPROFILE%\.kotone\profiles\dota2.json`，加一段 `consoleChat`，并给两个频道各加 `consoleCommand`：

   ```json
   "consoleChat": { "cfgDir": "game/dota/cfg" },
   "channels": [
     { "id": "team", "displayName": "队伍", "default": true, "consoleCommand": "say_team" },
     { "id": "all", "displayName": "所有人", "openChatKey": "Shift+Enter", "consoleCommand": "say" }
   ],
   ```

3. 打开 Kotone →「游戏列表」→ Dota 2 →「编辑」，会多出“不开聊天框，直接发到聊天”。下面那行应显示 `…\dota 2 beta\game\dota\cfg`。打开它（Dota 2 必须关着）。
4. 检查游戏目录：`game\dota\cfg\autoexec.cfg` 末尾多了一行 `bind "F23" "exec kotone_say" // Kotone <时间>`，`kotone_say.cfg` 是空文件；如果原来就有 `autoexec.cfg`，原文件备份在 `%USERPROFILE%\.kotone\agent\backups\<毫秒>\D\SteamLibrary\…\autoexec.cfg`。
5. 启动 Dota 2，进人机或演示模式。

| 编号 | 怎么做 | 期望 / 要记的 |
| --- | --- | --- |
| S1 | 启动后直接用语音发一句（不手动 `exec`） | 消息出现在队伍聊天里，聊天框**没有**打开；日志里是 `console_chat_send` |
| S2 | 不重启游戏，连着发三句不同的话 | 每次发出的是新内容，不重复上一句 |
| S3 | 发中文；再发一个带表情的（用下面的命令行最方便：`"<Kotone 安装目录>\kotone.exe" send --text "好的😀" --profile dota2 --delay-ms 5000`，5 秒内切回 Dota 2） | 显示正确，没有乱码 |
| S4 | 同样用命令行发：`--text "测试; 分号 // 双斜杠 \ 反斜杠 ' 单引号 % 百分号"` | 记下实际显示成什么：分号后面有没有被截掉、`//` 后面有没有丢、反斜杠和 `%` 是否原样 |
| S5 | 发一句很长的话（一百多字）；再连着快速发三四句 | 记下游戏的长度上限（截断在哪）和发言冷却时的表现（有没有丢、有没有提示） |
| S6 | ① 控制台输入 `cl_input_enable_raw_keyboard 1`，发一句；再 `cl_input_enable_raw_keyboard 0`，发一句。② 按住 Shift（或 Ctrl）时触发 F23：在终端里运行 `"<Kotone 安装目录>\kotone.exe" send --text "按住Shift测试" --profile dota2 --delay-ms 5000`，5 秒内切回 Dota 2 并按住 Shift 不放 | ① 两种设置下都能发出。② 按住 Shift / Ctrl 时也能发出（命令行发送不等修饰键松开，正好测这个）。如果按住时发不出，记下来——这决定以后控制台通道要不要继续等修饰键松开 |
| S7 | 在**匹配**对局（不是自建房间）里发一条队伍消息 | `say_team` 没有被限制。这一项进真实对局，封号风险请自己判断，不想测可以跳过 |

测完关掉开关（Dota 2 关着时）：`autoexec.cfg` 里那一行和 `kotone_say.cfg` 应该都没了。顺便看一下：再开 Dota 2 时按 F23 是否还会触发什么（Source 引擎有时会把绑定存进游戏自己的按键配置；`kotone_say.cfg` 已删除，所以最多在控制台报一句找不到文件）。

## CS2：C1–C4（以及以后的 S1–S7）

CS2 的内置配置在分支 `claude/adr-032-cs2`，**没有合并进 main，也没进 rc.3**（ADR：没人实测之前不发布）。装了 CS2 之后：

1. `git checkout claude/adr-032-cs2`，`pnpm dev`（或让我打一个带 CS2 的包）。
2. 「游戏列表」里会多出 Counter-Strike 2（U 队伍、Y 所有人、Enter 发送）。

| 编号 | 怎么做 | 期望 / 要记的 |
| --- | --- | --- |
| C1 | 在 CS2 的设置 → 键位里看聊天键 | 队伍聊天是 U、所有人是 Y；不一样就记下实际的键 |
| C2 | 人机里用语音发一句中文（队伍频道），再切到所有人发一句 | 文字正确、频道正确 |
| C3 | 同 L1–L3 | 同 L1–L3 |
| C4 | 按住 Shift（静步）或 Ctrl（蹲）时用语音发送 | 记录结果：Kotone 会等修饰键松开，最多 1 秒，等不到就不发（提示“发送前 Ctrl、Alt、Shift 或 Win 仍未松开”） |

CS2 的控制台通道照 Dota 2 的 S1–S7 测，只是配置里写 `"consoleChat": { "cfgDir": "game/csgo/cfg" }`，频道同样加 `consoleCommand`。

## 测完之后

把结果告诉我（或直接改 ADR 末尾的表）。S1–S7 通过后，内置的 Dota 2 配置才加 `consoleChat`；C1–C3 通过后 CS2 分支才合并发布。
