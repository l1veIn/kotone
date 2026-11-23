# ADR 032：往游戏聊天里发字，不被玩家的按键打断

状态：Accepted / 已确认（2026-11-22）。第一期、第三期已实现并合进 main（0.2.0-rc.3）；第二期（CS2）2026-11-23 玩家同意先加进内置配置（“同意cs2加入”），C1–C3 还没测：键位和延时是资料值，玩家在 CS2 里发现不对再改。真机验证大多等玩家，见末尾“实现情况”。玩家确认了两件事，见“玩家确认过的两件事”。

涉及游戏：英雄联盟、Dota 2、CS2。其他游戏跟着第 1 节一起受益。

## 背景

### 现在怎么发

所有游戏都走同一条键盘路径（`crates/kotone-core/src/inject.rs` 的 `send_sequence`，Windows 实现在 `crates/kotone-platform-windows/src/inject.rs`）：

```
开聊天框键按下 → 50ms → 松开 → 20ms → 文字（KEYEVENTF_UNICODE）→ 100ms → 发送键按下 → 50ms → 松开
```

英雄联盟和 Dota 2 的配置都是 Enter / Enter，整段约 220ms，分成至少 5 次 `SendInput`（两个 Enter 的按下和松开各一次，文字至少一次）。这套时序照的是 LeagueAkari，它主分支今天仍是同样的做法（见“依据”）。

### 为什么会失败

Windows 只保证**单次** `SendInput` 里的事件不被插队。各次之间的间隙里聊天框开着、键盘归聊天框，玩家这时按的键全进聊天框：

1. **进输入法**。字母键在中文输入法下开始组字；Kotone 最后的 Enter 被输入法拿去上屏，消息没发出，聊天框还开着，之后的按键继续进聊天框。这是玩家报的主要问题。
2. **消息里多出字符**。没开中文输入法时，按键变成字母混进消息。
3. **按键丢失**。这一下技能键被当成文字，没有生效。这一条在聊天框开着的时候无法避免。
4. **发错频道**（从代码推断，未实测）。窗口内玩家按下 Shift，开聊天框的 Enter 变成 Shift+Enter，即“所有人”频道。现在只在发送前等修饰键松开（`orchestrator.rs` 的 `wait_for_modifiers_up`），发送中不管。

## 三个游戏的分析

| | 英雄联盟 | Dota 2 | CS2 |
| --- | --- | --- | --- |
| 类型 | MOBA | MOBA | 第一人称射击 |
| 聊天键（默认） | Enter 队伍，Shift+Enter 所有人 | Enter 队伍，Shift+Enter 所有人 | U 队伍，Y 所有人，Enter 发送（未在游戏里核对） |
| 引擎读键盘的方式 | 未知；低层钩子吞键对它有效（见下） | Source 2 + SDL3，可切到原始输入 | 同 Dota 2（推断） |
| 绕开聊天框的官方通道 | 局内没有 | 控制台命令 `say` / `say_team` | 控制台命令 `say` / `say_team` |
| 聊天框开着 220ms 的代价 | 一次技能键 | 一次技能键 | 移动键失灵，代价最大 |
| Kotone 内置配置 | 有 | 有 | **没有** |
| 本机能否实测 | 能 | 能（已安装） | 不能（未安装） |
| 结论 | 第 1 节 | 第 1 节；可选第 3 节 | 第 2 节加配置；建议用第 3 节 |

### 英雄联盟

- **局内没有可用的官方写接口。** LCU 的 `lol-chat` 会话接口只管房间和选人阶段。接口表里另有 `POST /lol-game-client-chat/v1/party-messages?message=` 和 `instant-messages`，从名字看是开黑小队和私聊，不是队伍频道，对局中的行为没验证，不采用。队伍和所有人频道走游戏服务器。
- **低层钩子吞键对它有效。** 提交 `2fd7c2eb` 的试玩记录：呼叫键 `` ` `` 被 Kotone 的钩子吞掉后，游戏确实收不到“只攻击英雄”。所以第 1 节的做法在英雄联盟里有直接证据。
- 国服反作弊是 ACE。第 1 节只用 Kotone 已经在用的两个接口（低层键盘钩子、`SendInput`），不新增接口。

### Dota 2

本机安装目录 `D:\SteamLibrary\steamapps\common\dota 2 beta`，2026-11-22 直接在游戏文件里查字符串得到：

- `game/dota/bin/win64/client.dll` 里有 `say_team`、`messagemode`、`messagemode2`：控制台聊天命令在当前版本存在。
- `game/bin/win64/inputsystem.dll` 的按键名表里有 `F13`…`F24`：这些键可以绑定。
- `inputsystem.dll` 里有控制台变量 `cl_input_enable_raw_keyboard` 和 SDL 的 `SDL_WINDOWS_RAW_KEYBOARD`：键盘可以走原始输入（Raw Input）。默认值没查到。
- `game/bin/win64` 和 `game/dota/bin/win64` 下的 DLL 里都没有 `netconport`：Source 1 的 TCP 控制台端口在这里应该不存在。
- `game/dota/cfg/` 存在，里面目前没有 `autoexec.cfg`。

原始输入对第 1 节的影响：网上对“低层钩子吞掉的键，原始输入还收不收得到”有两种说法（有实测说收不到，也有相反的说法）。第 1 节只吞按下、不补发，所以即使某个游戏吞不住，结果也只是退回现状，不会变得更糟。是否吞得住列进真机验证。

### CS2

本机没装，以下都来自资料，**没有实测**。

- 和 Dota 2 同是 Source 2，`say` / `say_team` 可以写进绑定，例如 `bind l "say_team ..."`，是社区长期通用的做法。
- 成熟先例：[cs2-chat-translator](https://github.com/MeckeDev/cs2-chat-translator) 就是“把 `say_team "文字"` 写进 `cfg/chat_reader.cfg`，再模拟按一个绑了 `exec chat_reader.cfg` 的键”。这个做法依赖 `exec` 每次都重新读文件（验证 S2）。它写完后等 150ms 再按键，引号用反斜杠转义。
- Valve 2024-08-19 的规则针对的是“一个键触发多个移动或开火动作”（急停、跳投绑定），处罚是踢出对局。聊天绑定不在其列。对“外部程序改 cfg 再模拟按键”没有官方表态。
- 射击游戏里聊天框开 220ms 的代价比 MOBA 大：这段时间移动键不归游戏。所以 CS2 最该用不开聊天框的通道。
- CS2 里 Shift（静步）和 Ctrl（蹲）经常一直按着。键盘路径发送前要等修饰键松开（`wait_for_modifiers_up`，最多等 1 秒，等不到就不发），所以在 CS2 里键盘路径会经常发不出去（验证 C4）。这是 CS2 该用控制台通道的第二个理由。

## 决定

### 1. 键盘路径：发送窗口内挡住玩家的按下（所有游戏）

照 AutoHotkey 的思路（发送期间不让玩家的键混进来），但**不补发**：窗口内玩家的按下直接吞掉，松开照常放行，Kotone 不替玩家按任何键。玩家的理由（2026-11-22）：游戏的变化是实时的，这一秒能放的技能下一秒可能就不能放了，晚到的按键可能是错的。

规则：

1. **什么时候挡**：`WindowsInjector::send` 发第一个键之前开始，最后一个键松开后再留 30ms 结束。只有走键盘时序的发送才挡；`game.act`、脚本里单次打字、热键都不挡。
2. **在钩子里的位置**：`hotkey_ll.rs` 的 `hook_proc` 里，现有处理（Esc 停止、探测、热键匹配）先走；它们没吞的事件才轮到这条规则。热键行为不变。
3. **挡什么**：窗口内的键盘**按下**，且不是 Kotone 自己发的。松开一律放行。鼠标不挡。
4. **怎么认自己发的**：`LLKHF_INJECTED` 且 `dwExtraInfo == KOTONE_INPUT_TAG`。其他程序注入的键（改键软件、宏）按玩家的键处理。
5. **不补任何键**：修饰键也一样。窗口内按下的 Shift / Ctrl / Alt / Win 同样被吞掉、不补发，玩家松开重按（或按住不动等系统自动重复）后才生效。
6. **硬上限 600ms**：从开始算起超过 600ms，钩子自己停止吞键（比较时间戳即可，不依赖发送线程）。玩家把延时调得很长时，超出的部分和现在一样不受保护。
7. **认不出自己的键就不挡**：钩子没在运行、或者钩子从没见过带 Kotone 标记的键时不启用（每次启动后的第一条消息因此不受保护）。启用后如果某次发送里一个自己的键都没认出来，停用到下次启动并写日志。原因：有的输入过滤软件会清掉注入标记，那时挡键会把 Kotone 自己的键也吞掉。
8. **记录**：窗口内吞过键时，在过程日志记一条 `inject_gate`（吞了几个、窗口多长），用来看这件事实际发生得多不多。

效果对照：

| 情况 | 现在 | 改后 |
| --- | --- | --- |
| 窗口内按字母键，开着中文输入法 | 组字，消息发不出，聊天框卡开 | 消息完整发出 |
| 窗口内按字母键，没开输入法 | 消息里多出字母 | 消息完整发出 |
| 这一下技能键 | 丢失 | 丢失（不变） |
| 窗口内按下 Shift | 可能发到所有人频道 | 仍在原频道；这次 Shift 按下丢失 |
| 窗口内按下并一直按着的普通键 | 当成文字 | 窗口结束后靠系统的按键自动重复被游戏认到，晚多久取决于系统的按键重复延迟（默认约 0.5 秒）；游戏认不认由验证 L3 确认 |

发送前等修饰键松开的逻辑（`wait_for_modifiers_up`）保留：它管的是窗口开始前就按着的修饰键。

不补发的理由和代价，见“不做”第 1 条和“已知风险”第 3 条。

### 2. CS2 内置配置

新增 `crates/kotone-core/resources/profiles/cs2.json`：进程 `cs2.exe`；频道“队伍”开聊天框键 `U`、“所有人”`Y`，发送键 `Enter`；延时先照 Dota 2（20 / 20 / 100）；配一份 CS2 热词（地图点位、武器、战术用语）。照 `builtin_dota2` 的方式接入（`profile.rs`），包括“玩家删过就不再恢复”的标记。

键位和延时都是资料值，要在装了 CS2 的机器上核对后才能当默认发布。

### 3. Source 2 控制台通道：不开聊天框直接发（Dota 2、CS2，玩家自己开）

做法照 cs2-chat-translator：

```
把 say_team "文字" 写进 <游戏>/cfg/kotone_say.cfg → 模拟按一下 F23（游戏里绑了 exec kotone_say）
```

聊天框不打开，键盘始终归游戏；没有输入法的事；玩家同时按什么都不影响。

规则：

1. **游戏配置声明它支持**：`GameProfile` 新增可选的 `consoleChat { cfgDir, triggerKey }`（`cfgDir` 相对安装目录：Dota 2 是 `game/dota/cfg`，CS2 是 `game/csgo/cfg`；`triggerKey` 默认 `F23`），频道新增可选的 `consoleCommand`（队伍 `say_team`，所有人 `say`）。**只有实测通过的游戏，内置配置才带这一段。**
2. **开关就是 `autoexec.cfg` 里的一行**，不另存设置：
   `bind "F23" "exec kotone_say" // Kotone <写入时间>`
   游戏配置页上一个开关。打开时写这一行（没有 `autoexec.cfg` 就新建，追加在文件末尾），并建一个空的 `kotone_say.cfg`；关闭时删掉这一行和 `kotone_say.cfg`。只在游戏没运行时改，改之前把原文件备份到 `<KOTONE_HOME>/agent/backups/<时间>/`（照 ADR 030）。界面上写清楚：会在游戏目录里写这两个文件、封号风险由玩家自己判断、重启游戏后生效。
3. **每次发送时判断走哪条路**。同时满足才走控制台通道，否则走第 1 节的键盘路径：
   - 找得到安装目录（用 `games.rs` 的 Steam 检测结果）；
   - `autoexec.cfg` 里有 Kotone 那一行；
   - 游戏进程的启动时间晚于那一行的写入时间（否则绑定还没生效）；
   - 前台窗口是这个游戏。
4. **发送**：整理文字 → 写 `kotone_say.cfg`（UTF-8，无 BOM，一行 `say_team "…"` 或 `say "…"`）→ 按一下触发键（按住 50ms）→ 约 1 秒后把文件清空（免得玩家误触触发键时重发上一条）。
5. **整理文字**：换行换成空格；英文双引号 `"` 换成单引号 `'`；首尾去空白；空消息不发。分号、`//`、反斜杠是否需要处理，实测后定。
6. **触发键**：默认 `F23`（`F24` 是 Kotone 探测钩子用的键）。`inject.rs` 的 `vk_from_name` 目前只认到 `F12`，要加上 `F13`–`F24`。
7. **走控制台通道时不等修饰键松开**，前提是验证 S6 确认按住 Shift / Ctrl 时绑定照样触发。否则保留等待。
8. **`kotone_say.cfg` 是唯一在游戏运行时写的文件**，只由 Kotone 自己拥有。这是 ADR 030“游戏运行时不写它的目录”的一处例外，理由：那条规则防的是子代理乱改游戏文件，这里是玩家自己打开的功能，写的是 Kotone 自己的文件。

已知限制：按键发出后，Kotone 不知道游戏有没有真的发出消息（键盘路径也一样）。如果玩家自己的配置把绑定清掉了，消息会无声地丢失。开启后界面要提示玩家发一条测试消息确认。

## 不做

1. **不替玩家补按键（玩家确认，2026-11-22）。** AutoHotkey 的做法是把窗口内玩家的键存起来、发完再补发。不采用，原因依次是：游戏的变化是实时的，晚 0.2 秒到的技能键可能已经是错的；补发的键是模拟输入，等于 Kotone 在对局里替玩家按键，这是反作弊更敏感的一类行为（ADR 019 对“在对局里模拟操作”一直很谨慎）；万一某个游戏钩子吞不住，补发会造成一个键生效两次，只吞不补的话吞不住也只是退回现状。修饰键同样不补。
2. **不用 `BlockInput`。** 实际要管理员权限；直接丢弃玩家输入，包括松开，会造成卡键。
3. **不把整段合并成一次 `SendInput`。** 现有延时是为低帧率下 Enter 可靠而加的；而且有其他低层钩子在场时，单批也可能被插队。
4. **不用 `PostMessage` 直接投字符。** 开聊天框的键仍得是真实按键，聊天框照样开着，根因没变。
5. **不靠切换输入法解决。** 第 1 节之后玩家的键到不了输入法。
6. **不装驱动，不把 Kotone 做成输入法。** 要装驱动或把 DLL 载入游戏进程，越过“不注入”的红线。
7. **不用 `-netconport` / VConsole。** 前者在 Source 2 里不存在；后者要用工具模式启动游戏。
8. **不用 LCU 的 `party-messages`。** 频道语义不同，行为未验证。
9. **暂不挡鼠标。** 鼠标移动挡不了（光标会冻住）；鼠标点击会不会打断聊天框要先实测（见验证 M1），确实会再单独设计。
10. **英雄联盟房间、选人阶段走 LCU 聊天**是另一件事，不在本 ADR 里。

## 玩家确认过的两件事

2026-11-22：

1. **窗口内玩家的按键直接丢弃，Kotone 不替玩家补按任何键。** 理由见“不做”第 1 条。
2. **接受第 3 节往游戏目录写文件。** 默认不开，玩家在每个游戏的配置页自己打开；写什么、什么时候写，按决定 3 的第 2 条和第 8 条。

## 已知风险

1. **原始输入**：Dota 2 / CS2 开着原始键盘输入时，钩子能不能吞住按键没有定论。吞不住时第 1 节对它们无效（退回现状），不会更糟。
2. **输入过滤软件**清掉注入标记时，第 1 节自动停用（决定 1 第 7 条）。
3. **窗口内按下的修饰键也丢失**：玩家在窗口内按下 Alt 并按住、窗口结束后紧接着按 Q，游戏看到的是单独的 Q（例如自我施法变成普通施法）。只有在发消息的那约 220ms 里起手组合键才会遇到。玩家已确认不补按键，这一条作为已知代价保留。
4. **按住的键恢复慢**：窗口内按下并按住的普通键要等系统自动重复才被游戏认到。在 CS2 里这意味着移动键约半秒不响应，所以 CS2 建议用第 3 节。
5. **Valve 没有对“外部程序写 cfg 并模拟按键”表态。** cfg 和绑定是游戏认可的自定义方式，社区工具在用，但风险由玩家承担，界面上要说清楚。
6. **CS2 的一切都没实测。**

## 实现计划

分三期，每期一个或几个提交，每个提交都能单独编译。

### 第一期：发送窗口内挡键（第 1 节）

- `crates/kotone-platform-windows/src/`：
  - 新文件 `input_gate.rs`：状态和判定。判定逻辑写成不碰 Win32 的纯函数（输入：按下 / 松开、是不是自己的、现在的时间；输出：吞 / 放行），方便单测。
  - `hotkey_ll.rs` 的 `hook_proc`：在现有处理之后、`CallNextHookEx` 之前接入。钩子里只做原子变量读写，不加锁等待，不写日志。
  - `inject.rs` 的 `WindowsInjector::send`：包住 `send_sequence`，用 RAII 保证任何出口（成功、失败、取消）都结束窗口；结束后记 `inject_gate`。除了发送时序本身的键，这一期不新增任何 `SendInput`。
- 单测（纯函数）：窗口内按下被吞、松开放行；自己的键放行；别的程序注入的键被吞；超过 600ms 不再吞；修饰键的按下同样被吞、松开放行；没见过自己的键时不启用；一次发送里没认出自己的键后停用。
- 真机自动测试（隔离实例 + `scripts/manual-e2e/` 的测试窗口，不进游戏）：发送期间用不带标记的 `SendInput` 连续注入字母键，检查测试窗口里的文字和发出的消息完全一致；再测一次窗口内按下并按住 Shift：消息仍在默认频道，窗口结束后 Kotone 没有发出任何多余的键。
- `CHANGELOG.md` 加一条。

验收：单测和真机自动测试通过；验证清单 L1–L3 由玩家在训练模式确认。

### 第二期：CS2 内置配置（第 2 节）

- `cs2.json`、`profile.rs` 的 `builtin_cs2` 与 `ensure_builtin` 接入、对应单测（照 Dota 2 的写法）。
- 验收：验证清单 C1–C3。原定没人实测之前不发布；2026-11-23 玩家决定先加上，C1–C3 留给玩到 CS2 时核对。

### 第三期：Source 2 控制台通道（第 3 节）

- `kotone-core`：`GameProfile.console_chat`、`ProfileChannel.console_command`，`ChannelStrategy` 带上命令；`orchestrator.rs` 的 `resolve_send_strategy` 一带把命令放进发送用的配置。其他直接调 `injector.send` 的地方（`companion.rs`、`runtime_cmd.rs`、`kotone-cli`）一并核对。
- `kotone-platform-windows`：新文件 `console_chat.rs`（找 cfg 目录、读写那一行绑定、判断是否生效、整理文字、写 `kotone_say.cfg`）；`inject.rs` 的 `vk_from_name` 加 `F13`–`F24`；`WindowsInjector::send` 里按决定 3 第 3 条选路。
- 桌面端：`GamePage.svelte` 的游戏配置里加开关和说明；对应的 Tauri 命令（开、关、查状态）。
- 单测：整理文字；绑定行的增删（已有 `autoexec.cfg`、没有、重复开、玩家改过文件）；四个条件各缺一个时退回键盘路径。
- 先做 Dota 2（本机能测），验证清单 S1–S7 通过后内置配置才加 `consoleChat`；CS2 同理。

### 实现时的纪律

- 真机发任何模拟输入前，先看前台窗口；前台是玩家的对局就停下等。
- 游戏在运行时不跑 cargo 构建和测试。
- 游戏内测试只在训练模式、人机或自建房间做。

## 真机验证清单

钩子与键盘路径（玩家用真实按键）：

| 编号 | 游戏 | 做什么 | 期望 |
| --- | --- | --- | --- |
| L1 | 英雄联盟训练模式，微软拼音中文状态 | 发送瞬间连按 QWER | 消息完整发出，没有组字，聊天框关闭 |
| L2 | 同上 | 发送瞬间按下并按住 Shift | 消息在队伍频道；Shift 松开重按后组合键正常 |
| L3 | 同上 | 发送瞬间按下并按住一个技能键 | 不卡键；松开后一切正常 |
| M1 | 英雄联盟、Dota 2 | **改之前**，发送瞬间左键、右键点地面 | 记录现在会不会失败，决定要不要管鼠标 |
| D1 | Dota 2 人机 | 同 L1–L3 | 同上。若按键仍进了聊天框，说明原始输入下吞不住，记录 `cl_input_enable_raw_keyboard` 的值 |
| C1 | CS2 | 核对默认聊天键是不是 U / Y | 与 `cs2.json` 一致 |
| C2 | CS2 人机 | 用键盘路径发中文 | 文字正确、频道正确 |
| C3 | CS2 人机 | 同 L1–L3 | 同上 |
| C4 | CS2 人机 | 按住 Shift 或 Ctrl 时用键盘路径发送 | 记录结果：现有逻辑会等修饰键松开，最多 1 秒，等不到就不发 |

控制台通道（Dota 2 先测，CS2 另测）：

| 编号 | 做什么 | 期望 |
| --- | --- | --- |
| S1 | 写好 `autoexec.cfg` 后启动游戏 | 绑定自动生效，不用手动 `exec` |
| S2 | 不重启游戏，连续改写 `kotone_say.cfg` 并按触发键 | 每次发出的是新内容 |
| S3 | 发中文、表情符号 | 显示正确 |
| S4 | 文字里带 `;`、`//`、`\`、`'`、`%` | 确定哪些要处理，补进“整理文字” |
| S5 | 发很长的一句、连续快速发几条 | 记录游戏的长度上限和发言冷却的表现 |
| S6 | 模拟按 F23 | 游戏认得到（原始键盘输入开、关各测一次）；按住 Shift 或 Ctrl 时也触发 |
| S7 | 在匹配对局里（非自建房间）发一条队伍消息 | `say_team` 没有被限制 |

## 依据

- LeagueAkari 局内发送（2026-11-22 读主分支）：[send-executor.ts](https://github.com/LeagueAkari/LeagueAkari/blob/main/src/main/shards/in-game-send/send-executor.ts)，仍是 Enter → Unicode → Enter，房间和选人阶段走 LCU。
- `SendInput` 只保证单次调用内不被插队：[Microsoft Learn](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-sendinput)。
- AutoHotkey：发送期间缓冲玩家输入，[Send](https://www.autohotkey.com/docs/v1/lib/Send.htm)；`BlockInput` 可能需要管理员、会丢弃输入并造成卡键，[BlockInput](https://www.autohotkey.com/docs/v1/lib/BlockInput.htm)；有其他键盘钩子时 `SendInput` 可能被插队，[论坛](https://autohotkey.com/boards/viewtopic.php?p=562246)。
- LCU 接口表：[openapi.json](https://www.mingweisamuel.com/lcu-schema/lcu/openapi.json)（`/lol-game-client-chat/v1/party-messages`、`instant-messages` 只有 POST）。
- 钩子与原始输入的两种说法：[GameDev.net](https://gamedev.net/forums/topic/550264-dinput-and-global-key-hooks-vistawin7/)（实测吞得住）、[AltSnap PR #773](https://github.com/RamonUnch/AltSnap/pull/773)（认为两者无关）。
- CS2 写 cfg 再按键的先例：[cs2-chat-translator](https://github.com/MeckeDev/cs2-chat-translator)（`bin/cs2-chat-translator.js` 的 `writeChatCfg` / `pressBindKey`）。
- Valve 2024-08-19 的输入自动化规则：[Dust2.us](https://www.dust2.us/news/51933/cs2-8-19-update-null-binds-snap-tap-and-jump-binds-banned-in-latest-update)、[PC Gamer](https://pcgamer.com/games/fps/valve-says-weve-decided-to-draw-a-clear-line-and-bans-snap-tap-keyboard-automation-from-counter-strike-2)。
- `say_team` 绑定写法：[CS2](https://csdb.gg/zh-cn/command/say-team/)、[Dota 2](https://lis-skins.com/blog/how-to-bind-a-phrase-in-dota-2/)。
- Dota 2 游戏文件里的字符串：2026-11-22 在本机安装目录用 `strings` / `grep` 查得，见“三个游戏的分析”。
- 英雄联盟里钩子吞键有效：提交 `2fd7c2eb` 的试玩记录。

## 实现情况（2026-11-22）

### 提交

| 期 | 提交 | 分支 | 内容 |
| --- | --- | --- | --- |
| 第一期 | `4a380fe9` | `claude/adr-032` → main | `input_gate.rs`、钩子接入、`WindowsInjector::send` 开关窗口、真机自动测试 `scripts/manual-e2e/input_gate_e2e.py`、测试窗口的 `-Chat` 模式 |
| 第三期 | `d9841055` | `claude/adr-032` → main | `consoleChat` / `consoleCommand`、`console_chat.rs`、选路、`F13`–`F24`、游戏配置页开关、Tauri 命令 `console_chat_status` / `console_chat_set` |
| 第二期 | `2660d8ee` | `claude/adr-032-cs2`（**没合并**） | `cs2.json`、`builtin_cs2`、删除标记 `.cs2-deleted` |

本节和玩家验证步骤 [docs/playtest-adr-032.md](../playtest-adr-032.md) 是之后的文档提交。

### 测试结果

- `cargo test`：kotone-core、kotone-platform-windows、kotone-tauri、kotone-cli 全部通过（新增：`input_gate` 10 个、`console_chat` 11 个 + 1 个真机、profile 3 个、orchestrator 1 个、`vk_from_name` 1 个；CS2 分支另加 2 个）。
- `cargo clippy --all-targets -- -D warnings`：上面四个 crate 通过。`rustfmt` 只对自己改的文件跑。
- 前端：`svelte-check` 0 错误 0 警告；Playwright `e2e/game-profile.spec.ts` 5/5（新增“控制台开关只在声明了的配置里出现”）。
- 真机自动测试（第一期，隔离实例 `target/gate-e2e/` + 测试窗口，没进游戏），跑了两遍都 PASS：
  - 对照（启动后第一条，不受保护）：发送期间用不带标记的 `SendInput` 连按 Q W E R，消息变成 `rqwerqwe对面打野来下路了，小心 rgankqwerqwerqwe`——说明干扰确实落在发送窗口里；
  - 受保护的发送，同样连按：消息与原文完全一致，窗口外的 53 个字母照常到达测试窗口；`inject_gate` 记 `swallowed=29 windowMs=259`；
  - Kotone 第一个 Enter 之后 7ms 按下并按住 Shift 一秒：两个 Enter 都不带 Shift、消息在队伍频道、测试窗口没收到 Shift 的按下、最后一个 Enter 之后没有任何带 Kotone 标记的键（只有 Enter 和文字）。
- 真机（第三期的本机部分）：测试窗口在前台时，`send` 把 `say_team "推中 '一起' 走"` 写进文件、带 Kotone 标记的 F23 到达测试窗口、约 1 秒后文件清空。**没进游戏**：游戏读不读这个文件、绑定是否生效，都在 S1–S7。

### 和方案不一样、或方案没写到的地方

1. **决定 1 第 7 条“一个自己的键都没认出来就停用”**：只在这次发送里 Kotone 的 `SendInput` 确实交出过键盘事件时才算。发送前就失败（例如前台是管理员程序，`SendInput` 前就报错）不会误停用。
2. **并发发送**：两次发送重叠时窗口按计数，最后一个结束才关；30ms 尾巴只在交出过键时等。
3. **控制台通道多一个走键盘的条件**：发送键为空（陪玩“只打进聊天框、不发送”，`send=false`）时不走控制台，因为 `say` 会直接发出去。ADR 没覆盖这种调用。
4. **“前台是这个游戏”**：除了进程名，还要求前台程序在找到的 Steam 安装目录里（多份安装时不写错目录）；拿不到程序路径时走键盘。
5. **怎么找安装目录**：只扫 Steam 库（`games::steam_games`），用现有的 `find_match`（先比进程名，再比名字）。Steam 的检测结果没有进程名，实际按名字对上“Dota 2”；CS2 的内置配置名字因此叫 “Counter-Strike 2”。
6. **配置校验**（防止导入的配置包借这个功能写别的命令、写到别处）：`consoleCommand` 只能是 `say_team` / `say`；`cfgDir` 必须是相对路径、只有普通路径段、最多 8 段、最后一段是 `cfg`；`triggerKey` 只能是 F13–F23（F24 留给探测）。
7. **发送用配置怎么带命令**：`GameProfile::for_channel` 把选中的频道在发送用配置里标成默认频道，注入器取默认频道的命令。陪玩打字、调试发送、`kotone send` 直接用原配置，即默认频道（队伍）。
8. **那一行的写入时间**写成本地时间带时区：`2026-11-22 01:23:45 +08:00`。玩家改坏了时间就当没生效（走键盘）；绑的键被改过也走键盘，设置页会提示“关掉再打开可以修好”。
9. **关闭开关**只删那一行和 `kotone_say.cfg`；`autoexec.cfg` 本身保留，哪怕是 Kotone 新建、删完变空的。
10. **决定 3 第 5 条**：分号、`//`、反斜杠没处理（等 S4）。**第 7 条**：S6 没验证，所以走控制台通道时仍然等修饰键松开（orchestrator 没改）。
11. **内置配置都没带 `consoleChat`**（S1–S7 没做）。游戏配置页的开关只在声明了这一段的配置里出现；测试时玩家手动加到自己的 `dota2.json`（步骤见验证文档）。
12. **第二期**：ADR 说“没人能实测之前不发布这一期”，所以 CS2 留在 `claude/adr-032-cs2`，没进 main 和 rc.3。任务里的“CS2 的控制台通道配置”没写进内置配置（没实测不带 `consoleChat`），测试用的配置片段写在验证文档里。

### 验证清单的状态

| 编号 | 状态 |
| --- | --- |
| 第一期真机自动测试（测试窗口 + 不带标记的 `SendInput`） | 通过 |
| 第三期本机部分（写文件 → 按触发键 → 清空） | 通过 |
| L1 L2 L3 | 等玩家 |
| M1 | 等玩家（第一期不挡鼠标，新版上测和改之前一样） |
| D1 | 等玩家 |
| S1 S2 S3 S4 S5 S6 S7 | 等玩家（要先在自己的 `dota2.json` 里加 `consoleChat`） |
| C1 C2 C3 C4 | 等玩家（要装 CS2，用 `claude/adr-032-cs2` 分支） |

照着做的步骤：[docs/playtest-adr-032.md](../playtest-adr-032.md)。
