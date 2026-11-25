# ADR 041：发到游戏——入口和通用发法归本体，每个游戏的发法归游戏插件

状态：Accepted（方向，2026-11-25，玩家：“发到游戏这部分没问题”）/ 第 1 期已实现（2026-11-25，真机脚本待跑，见“实施记录”）/ 第 2 期已实现（2026-11-25，真机脚本 C1–C8 和真客户端待跑，见“实施记录”）/ 第 3 期已实现（2026-11-25，一键喊话：真机脚本不按键的部分过了，按键的和真客户端待跑，见“实施记录”）。细化 [ADR 039](039-desktop-companion-and-game-apps.md) D2 里“往游戏聊天里打字归本体”那一半（语音识别那一半不变）。

## 背景

玩家（2026-11-25）：

> “我突然觉得，‘发送到游戏’这件事，本质上是‘Enter——Unicode——Enter’，现在归于本体。实际上，我觉得这件事应该归于插件？lol 插件有自己的‘发送到游戏’的方式，甚至可以配置一键喊话，LeagueAkari 就是这样做的。以后会有 Dota2、CS2 等等，发送到游戏的方式可能都不一样，所以这件个工具的边界应该是在插件自治范围内？我觉得这个划分很清晰。但是让我犹豫的是，我们有‘语音输入法’这个插件。这里如果发送到游戏归属其他插件，那么语音输入法这个插件如何使用这些工具呢？我想到一个方案，利用自动化中转。语音输入识别完成之后，触发一个事件把识别结果丢给自动化，然后自动化调用发送到游戏？这样会不会过于复杂？”

核对过的现状（[共享资源评估](../research/2026-11-25-shared-resources-audit.md)“往游戏里输入”，只读代码）：

- 发到游戏聊天的路有 `game_chat.send`、语音输入识别完以后、旧的 `companion.type`、自动化的 `builtin-chat-repeat`（自带一份“回车—打字—回车”，写死回车，对 CS2 是错的）、`game.act`、控制台发话、剪贴板粘贴、调试命令，各自的前台检查都不一样。
- 游戏特有的知识都在本体：各游戏的聊天键、频道、时序，Source 2 控制台的全部细节（`console_chat.rs`），为英雄联盟低帧率调的按键时长。
- 打字之间没有锁：语音输入只锁自己，`game_chat.send` 不加锁，输入闸门按层数计数、放行所有 Kotone 的按键——两边同时打字按键会交错（没实测）。`game.act` 和按键程序互斥，但都不和打字互斥；ADR 038 / 039 说的“共用一把输入锁”代码里没有。
- `game_chat.send` 不等修饰键松开（语音输入等）：按住 Shift 时英雄联盟、Dota 2 里会变成全体（没实测）；用的是“当前选中”的游戏配置，不是前台检测到的；没有频道参数；不接受换行，英雄联盟的提示词却让琴音“一行一条”发战绩。
- 英雄联盟插件不用客户端的聊天接口。LeagueAkari 按阶段选路：选人 / 大厅走 `POST /lol-chat/v1/conversations/{id}/messages`（多行用换行连起来一次发）；对局里先确认客户端在前台，再逐行“回车—打字—回车”，行间隔可设（[send-executor.ts](https://github.com/LeagueAkari/LeagueAkari/blob/main/src/main/shards/in-game-send/send-executor.ts)）。CS2 用 Y / U 加回车，或控制台 `say` / `say_team`。

## 参考了谁

- **系统输入法框架**：输入法只管把文字交出去，交给有焦点的程序，怎么接收是那个程序的事。语音输入法本来就是输入法，游戏就是那个输入框。
- **VS Code 的语言 provider**：本体定义“补全”这件事，每种语言的扩展提供实现，编辑器按当前文件的语言去问对应的扩展。
- **LeagueAkari**：按游戏阶段选发法（上面）。

## 为什么不用自动化中转

玩家提的“识别完发事件 → 自动化 → 发到游戏”能做出来，但不用：

- 事件是广播，“发生了一件事”，谁想听谁听，没人回话；发到游戏是请求，要**恰好一个**接手的、要回答发没发成、发到哪个频道。
- 基础功能会依赖一条玩家能改能删的自动化：删了它，语音输入就悄悄坏了。
- 多绕一道，慢，难查；团战里说一句话等不起。

“识别完了”照样可以作为事件广播，给想知道的（识别记录、统计）。

## 决定

**D1 一个入口。** “把这段话发到当前游戏的聊天”只有一个入口：`game_chat.send`，名字不变，加可选的频道（队伍 / 全体，缺省队伍），接受多行。用它的有：语音输入法、琴音、插件的命令（例如英雄联盟的“一键喊话”，按 [ADR 040](040-hotkeys-belong-to-core.md) 绑键）、自动化、外部 Agent。旧的 `companion.type` 和 `builtin-chat-repeat` 自带的那份打字改成调这个入口（原则 15：只登记一次）。

**D2 通用发法在本体。** “开聊天框 → 逐字 Unicode → 发送”，用游戏配置里的聊天键、频道和时序（照旧在游戏列表页改）。没有插件提供发法的游戏（例如玩家自己加的游戏）用它；提供发法的插件关掉时也退回它。

**D3 每个游戏的发法由游戏插件提供。** 游戏插件可以在 `plugin.toml` 里声明“我给游戏 `<id>` 提供发到聊天”，实现一个“发这段话（频道）→ 结果”的方法。宿主按**前台检测到的游戏**挑：有插件提供就用插件的，没有就用通用发法。

- 英雄联盟插件：选人 / 大厅走客户端聊天接口（不按键）。~~对局里用本体的打字原语，按英雄联盟的规矩（频道、逐行、行间隔）。~~ **对局里不归插件，照旧是本体的通用发法**（2026-11-25 第 2 期改，主会话定的）：英雄联盟对局里的规矩（Enter / Shift+Enter、20/20/100 毫秒、一行一条、行间 65 毫秒）本来就在游戏配置里（ADR 039 D2：聊天键、频道和时序留在游戏配置），和 LeagueAkari 对局里的做法一样，第 1 期已经做完了；再经插件调一次，就是同一件事在本体和插件各管一份（原则 2），还得给插件开一个打字的权力（原则 11）。
- “前台检测到的游戏”（第 2 期补）：前台是游戏的窗口——游戏配置里的进程，**或者游戏插件登记的、属于这个游戏的别的窗口**（英雄联盟客户端 `LeagueClientUx.exe`：它不在游戏配置的进程里，配置的进程名还管“游戏在不在跑”、覆盖层、她能不能操作，不能把客户端加进去）。插件登记的是窗口，不是“现在接不接”：客户端窗口归插件、游戏窗口归本体，静态分好，没有“先问插件、不接再按键”的判断链。
- 控制台发话（Source 2）暂时留在本体、不再扩展，有 CS2 / Dota 2 插件时搬过去。

**D4 打字原语和一把输入锁在本体。** 模拟按键、输入闸门是全进程只有一份的东西（ADR 032、038），插件要按键只经宿主。打字（所有发话的人）、`game.act`、按键程序共用**一把**输入锁：后来的排队等，不交错；等太久就返回失败，不丢不重发（ADR 032：不替玩家重按）。

**D5 入口里统一做的检查**（第 2 期补：这几条是**按键的发法**统一做的；插件的发法不按键，不拿输入锁、不等修饰键、不切窗口，几行可以连成一条发）。等修饰键松开（照语音输入现在的做法，有上限）；目标游戏在前台（按前台检测，不按“当前选中”的配置）；多行逐行发、行间有间隔。语音输入“回到开始收音时的窗口”照旧由它自己决定（回不回、回哪个），但**把要回去的窗口交给入口，入口拿到输入锁以后再切回去发**（2026-11-25 修订，原来写的是“回去以后再交给入口”，为什么改见“实施记录”）。

**D6 结果说清楚。** 入口返回走了哪条路（客户端聊天接口 / 按键 / 控制台）、哪个频道、发成没有、没发成的原因，给琴音和外部 Agent 看。

**D7 第三方先不给。** “提供某个游戏的发法”先只给内置插件（单向门，[软件哲学](../philosophy.md)原则 11）；以后第三方 MCP 服务器可以用 `_meta` 声明。

## 不做

- 自动化中转（上面）。
- 和琴音说话时自动不转给队友（游戏语音）。玩家：“实际上用户知道怎么区分，现在游戏玩家玩游戏时候很多都是‘按键说话’，我觉得先别加太多复杂度。”

## 分期

| 期 | 做什么 | 玩家看到的 | 怎么验 |
| --- | --- | --- | --- |
| 1 | 入口加固：一把输入锁、等修饰键、按前台检测的游戏、频道参数、多行、结果说清楚；`companion.type`、`builtin-chat-repeat` 改调入口 | 按住 Shift 不会发到全体；两边同时发不交错 | 假注入器的单元测试（交错、修饰键、前台）；测试窗口在前台的真机脚本 |
| 2 | “提供发法”的插件契约；英雄联盟插件实现（选人 / 大厅走客户端聊天接口；对局里照旧是通用发法，见 D3 的修改） | 选人时琴音发的话不再按键 | 假客户端的测试；`lol_tools_real.py`；真客户端留给玩家 |
| 3 | 英雄联盟“一键喊话”命令（依赖 ADR 040 第 2 期） | 一个键把对面技能 / 战绩发出去 | Playwright；真机脚本 |

## 实施记录

- **2026-11-25 第 1 期第一段（复现，没改产品代码）**：
  - 共享资源评估 #3（两边同时打字交错）复现了：`crates/kotone-core/tests/orchestrator.rs` 的 `adr041_voice_input_and_game_chat_send_do_not_interleave`（先标 `#[ignore]`，手动跑 `cargo test -p kotone-core --test orchestrator -- adr041 --ignored`）——语音输入发送时另一路照 `game_chat.send` 的做法调同一个注入器，流水是 `open:A, open:B, text:A, text:B, send:A, send:B`。
  - #2（按住 Shift 发到全体）只核对了代码（`game_chat.send` 一路不读修饰键，只有语音输入等），真机没跑成（玩家在用机器，前台不是终端）。真机复现 / 回归脚本 `scripts/manual-e2e/send_to_game_e2e.py`：按住 Shift 后松开、一直按住、两条同时发、两行、`channel: "all"`，第 1 期之前五项都应失败。
  - 清点出来 ADR 没列的一条：旧陪玩流程的“帮忙打字”（`companion.rs` 的 `handle_assistant` / `run`）也直接调注入器、不核对前台；`companion.type` 也不核对前台。
- **2026-11-25 第 1 期第二段（实现）**。主会话审过设计后定的（下面的“定了”都是那次定的）：
  - **入口和锁放哪**：入口 `kotone-core/src/send_to_game.rs`（`SendToGame` 端口、本体的实现 `GameChat`、`pick_game`、`pick_channel`、`split_lines`），锁的类型 `kotone-core/src/input_lock.rs`；**实例只有桌面端建一份**（`lib.rs` → `SharedState.game_chat`，`apps/desktop/src-tauri/src/game_chat.rs` 是桌面端那一面），交给编排器（`Orchestrator::with_send_to_game`，语音输入经 `orch.send_to_game()` 用它）、`game_chat.send`、`companion.type`、旧陪玩流程、自动化、`game.act`。不做成 `static`，插件拿不到。编排器的 `injector()` / `focus()` 删了，语音输入拿不到注入器。`Host` 这一期不加方法（原则 14：第一个经 `Host` 用的是第 3 期的“一键喊话”）。
  - **锁的粒度**：一条发言（全部行）/ 一次 `game.act`（整批，先拿锁再查有没有程序在跑，结尾再看一眼之前放）/ 按键程序的每一下按键、打字、点击；松开键、松开鼠标、结束时全部松开不拿锁。先来后到排队，最多 5 秒（正常最长的占用者是另一条 10 行的发言约 3.1 秒加它等修饰键），轮不到就失败、说谁占着。没照 LeagueAkari（新的一条取消旧的，`send-executor.ts`）：它只有一个发话人，我们有玩家的语音输入和她，取消一半会留下一个开着、打了半句的聊天框。ADR 019 的“`game.act` 和按键程序不同时跑”照旧。
  - **每行**：等修饰键松开（每行最多 1 秒：行与行之间挡键窗口是关的，玩家这时按下的 Shift 会让下一行变成 Shift+Enter）→ 前台还是同一个窗口 → 发（一行一个 ADR 032 的挡键窗口，窗口有 600 毫秒硬上限）→ 行间 65 毫秒（LeagueAkari `InGameSendSettings.sendInterval = 65`）。最多 10 行、每行 200 字，空行丢掉。
  - **挑配置**：前台程序对得上某个具体游戏配置就用它（选中英雄联盟、前台是 Dota 2 时按 Dota 2 的键发），否则用当前选中的；替玩家发言的（她、外部 Agent、自动化）照旧的规则拦（前台是桌面、Kotone 自己、或者选中的是具体游戏而前台不是它）；玩家自己的语音输入不拦（玩家拍板过“前台是什么窗就往哪发”）。
  - **频道**：`game_chat.send` 加可选 `channel: "team" | "all"`，缺省 = 游戏配置的默认频道（内置的都是队伍）。没有要的频道时**发给更多人不退、发给更少人可以退**：要“所有人”而没有 → 不发；要“队伍”而没有 → 默认频道（默认频道本身是“所有人”时也不发）。语音输入照旧用自己记的 (游戏, 频道)，挑中的游戏不是那个游戏时退回默认频道。
  - **结果**：以前的 `{typed, sent, profile, receipt, runId}` 照旧（`sent` 照旧是“按了发送键”，多行时要每行都发了才是 true），加上 `route: keys | console, game, gameName, channel, channelName, lines, sentLines, waitedMs, stopped?: {code, message}`（只加不改，单向门；第二段提交时漏了 `typed` / `profile`，随后补上，`voice_input_real.py` 6b 查 `typed`）。一行都没发出去 → 工具返回错误（话说清楚原因和下一步）；发了一部分 → 照实说哪几行没发。不重试、不补发。
  - **截止时间（没照设计的建议，主会话改的）**：设计里说“工具服务 5 秒先超时、字在后台发完，写进说明让调用方调长 `timeoutMs`”。改成：入口收一个截止时间（工具服务调的时候是它自己按 `timeoutMs` 算的期限减 300 毫秒余量，经新加的 `AppHost::invoke_until` 传进来；她和语音输入用入口的默认 8 秒），排队等锁、逐行发都看它：等锁等到截止还没轮到就返回 `busy`，一个键都没按；开始一行之前估计这一行（按键时长、配置里的延时、打字、挡键尾巴、余量）来不及在截止前发完就停，返回已发几行、`stopped` 写明 `timeUp`。理由：“报超时、其实发了”时外部 Agent 很可能重发，队友就收到两遍，违反 D6“结果说清楚”和 ADR 032“不替玩家重按”的精神。剩下唯一说不准的情况是系统调用被安全软件挂住（截止后 2 秒看门狗，话里说“聊天框里可能已经有一部分字”）。`ToolSpec` 照旧不加期限字段。
  - **D5 改了写法**：语音输入以前先切回开始收音时的窗口、再交给入口。她正在打多行时，语音输入这一切，她这一行剩下的键会打进新的前台窗口（锁管得住按键交错，管不住“前台被换了”）。所以改成入口拿到锁以后再切：先等修饰键（等的时候取消了就不切）→ 切回去 → 读前台挑配置 → 发。
  - **`companion.type` 的语义**：看了它的说明和实现，它就是“往前台游戏的聊天里发一句”（按游戏配置的开聊天框键、输入方式、发送键，`send=false` 只打进聊天框），所以改调入口（发话人记成外部 Agent），参数不扩（只收一行，多行请用 `game_chat.send`），结果在 `{typed, sent, profile, receipt}` 上只加字段；**行为变化**：以前什么前台都打，现在和 `game_chat.send` 一样核对前台。不写执行记录（以前也不写）。
  - **旧陪玩流程的“帮忙打字”**（`companion.rs` 的 `handle_assistant` / `run`）：经入口（发话人记成她）。
  - **自动化**：脚本加 `chat(文字, {channel})`，经入口；`builtin-chat-repeat` 换成它、去掉 `open_key` 参数（聊天键归游戏配置，CS2 写死的回车不对）。内置自动化只在第一次安装，老用户手里那份程序一字没改过的，`ensure_builtin` 换成新的（玩家改过的参数缺省值保留），改过的不动。降级到旧版时，新的那份能读，运行时报“没有 chat 这个函数”。
  - **删掉的**：桌面端 `companion::type_text`、`foreground_refusal`（规则搬进 `pick_game`，测试一起搬）；语音输入的 `wait_for_modifiers_up`、焦点恢复和注入调用、`resolve_send_strategy`（测试搬进入口）；没人调用的 `simulate_send`（前后端、脚本、测试都 grep 过，悬浮条的重试用的是 `confirm_send`）。
  - **边界测试**（原则 16）：`crates/kotone-plugin-api/tests/boundaries.rs` 的 `plugins_never_press_keys_themselves`：插件源码里不出现打字 / 按键原语和注入器。
  - **不在这把锁里**：命令行的 `kotone send` / `kotone listen`（另一个进程，没有钩子，调试用）。控制台通道（评估 #12）在进程内被锁串起来了，但游戏读 `kotone_say.cfg` 的延迟有没有短于 65 毫秒的行间隔没测（ADR 032 S2、S5）。
  - **测试**：入口和锁的单元测试（假注入器 / 假前台 / 假时钟：交错、修饰键、行间按下 Shift、前台挑配置和拦截、频道、多行、截止时间、取消、切回窗口在锁里、锁被占着时 `busy`）；第一段的复现测试改成经入口、去掉 `ignore`，转绿。真机脚本 `send_to_game_e2e.py` 还没跑（要玩家让出前台）；修之前的程序另存在 `target/prefix-exes/kotone-pre-adr041.exe`，用来补跑“修之前”五项都失败。
- **2026-11-25 第 2 期：主会话定的**（设计见子代理的 `adr041-phase2-design.md`；玩家交代过拿不准的照文档和软件哲学自己定。下面标“主会话定的，玩家可以改”的是玩家看得到、感觉得到的）：
  - **对局里不归插件**（改了 D3 那一句，理由写在 D3）；`Host` 这一期不加打字的方法，插件拿到的权力不变。
  - **按插件登记的窗口挑**：清单 `[[send_to_game]]`（`game`、`windows`、`needs`）。前台（语音输入：开始收音的那个窗口，窗口没了就看现在的前台）的程序在某个插件登记的窗口里 → 交给插件；否则第 1 期的通用发法一字不改。不按游戏中心挑（它知道英雄联盟在选英雄，不知道玩家在哪个窗口：玩家在 Discord 里说的话不该被客户端截走），也不问插件“这次接不接”。
  - **插件接手以后失败了也不退回按键**：客户端没在期限内回话时话可能已经发出去了，再按一遍队友就收到两遍（ADR 032、原则 4）；往客户端窗口里按键效果不可预料（焦点在按钮上时 Enter 等于点它）；一条请求只走一条路，`route` 才说得清楚（D6）。**插件或它要的开关关着时**：替玩家发言的（她、外部 Agent、自动化）不发、说开哪个开关（`providerOff`）；只有玩家自己的语音输入退回按键（玩家拍板过“前台是什么窗就往哪发”）。
  - **插件的发法不拿输入锁、不等修饰键、不切窗口**：输入锁排的是往游戏里按键的人（术语表），客户端接口不按键；拿锁只会让 `game.act`、按键程序白等，还会在锁里做网络请求（原则 20）。
  - **结果只加不改（单向门）**：`route` 加 `"client"`（游戏客户端自己的聊天接口，不按键）；加 `plugin`（提供发法的插件 id，走插件时才有）；原因代码加 `noChatHere`、`clientUnavailable`、`clientRejected`、`clientNoReply`、`providerOff`、`noDraftHere`（插件那一组是封闭的，名字不带哪个游戏，`client` 指“游戏客户端”；`noSuchChannel`、`timeUp`、`cancelled` 沿用第 1 期的）。回执：“已经经客户端的聊天接口发到…（n/n 行，连成一条）；只证明客户端收下了，不证明队友已经看到”。
  - **“在客户端里发聊天”单独一个开关、默认开**（主会话定的，玩家可以改）：不并进“读客户端数据”（它写着只读），不并进“操作客户端”（默认关，管自动接受、自动选英雄这类替玩家做决定的事，风险出处是 LeagueAkari #523）。发聊天只在有人要发时发，和玩家自己打字是一类（原则 3：读本地数据默认开、自动操作默认关）。风险提示写出处：国服用户协议、[#480](https://github.com/LeagueAkari/LeagueAkari/issues/480)（模板里的词被屏蔽）、[#524](https://github.com/LeagueAkari/LeagueAkari/issues/524)（WeGame 去掉了能改内容的一键喊话）、Riot 说客户端接口不正式支持第三方使用（[developer.riotgames.com/docs/lol](https://developer.riotgames.com/docs/lol)）。
  - **她发话时 Kotone 窗口在前台 → 照旧说“先切到游戏”**（主会话定的，玩家可以改）：不做 LeagueAkari 那种不管前台直接经客户端发；要做是多一条规则（“前台不是游戏时，有不按键的发法就用它”），以后几个游戏都有这种发法时还得排先后。
  - **语音输入的频道停在“所有人”时进了选英雄 → 不发、说清楚**（主会话定的，玩家可以改）：和第 1 期“要所有人而没有就不发”同一条。
  - **关了“帮忙打字后自动发送”的玩家在选英雄 / 房间里 → 不发、说清楚**（`noDraftHere`，主会话定的，玩家可以改）：客户端接口只能整条发出去，没法只打进聊天框让玩家确认；退回按键也不行（上面）。
  - **结算界面的聊天这期不发**：ADR 的范围是选人 / 大厅；结算的聊天两边都看得到。
  - **第三方（D7）结构上拿不到**：宿主只从内置插件表里建发法；第三方的清单由 `kotone-remote-plugin` 合成，`[[send_to_game]]` 永远是空的。以后开放时的 `_meta` 写法这期不定。
- **2026-11-25 第 2 期第一段（入口挑发法，core）**：
  - `kotone-core/src/send_to_game/provider.rs`（新）：清单那一项 `SendToGameSpec`、交给插件的 `ChatSend`（整理好的行、算好的频道 id、截止时间、取消）、插件回的 `ChatSent` / `ChatStop`（`StopCode` 那一组）、入口看到的端口 `ChatProvider`（`plugin`、`spec`、`off`：None = 开着，Some = 为什么关着、`send`）。类型在 core（入口在这里，core 不依赖插件契约，原则 13），插件契约重新导出（第二段）。
  - `GameChat::with_providers`；`send` 在整理完文字、拿锁之前挑发法（上面的规则），`send_via_provider`：`send = false` → `noDraftHere`；频道 id 由 `requested_channel` 算（缺省 = 游戏配置的默认频道，语音输入记的频道属于别的游戏 → 默认频道并标 `channelFellBack`；插件那边只有“有就发、没有就不发”）；插件回 `Ok` 却一行都没发当作没发（`clientRejected`）。`ChatOutcome` 加 `plugin`，`SendRoute` 加 `Client`（`inject.rs`），`ChatFailure` 加 `Provider` / `ProviderOff` / `NoDraftHere`。`FocusBackend` 加 `window_info`（默认 None，Windows 实现读窗口的进程号）：看语音输入开始收音的那个窗口归不归插件。桌面端 `game_chat.rs` 的回执加客户端那种。这一段还没有插件登记发法，行为不变。
  - **测试**（`send_to_game/tests.rs`，假发法登记 `LeagueClientUx.exe`）：客户端在前台 → 交给插件一次、0 个键、`route = client`、`plugin = lol`；锁被别人占着、修饰键一直按着、带着 `restore` 也马上发（没拿锁、没等、没切窗口）；开始收音的窗口是客户端而前台已是游戏 → 插件，反过来 → 按键，窗口没了 → 看前台；游戏窗口、Kotone 自己的窗口不归插件；插件关着 → 她 / 外部 Agent / 自动化 `providerOff`、0 个键，玩家的语音输入照旧按键；插件回 `clientNoReply` → 错误、0 个键、只问一次；发了一部分照实说；`send = false` → `noDraftHere`；频道算好再交（缺省 team、all 原样、语音输入记的 Dota 2 频道退回 team）；取消了不问插件。把挑发法那一步关掉，这 7 个新测试失败、原来的 29 个照过。
- **2026-11-25 第 2 期第二段（契约 + 英雄联盟的发法）**：
  - **契约**（`kotone-plugin-api`）：`Manifest.send_to_game`（`[[send_to_game]]`，和 `[[commands]]` 并排、同一种写法：类型在 core，契约 `pub use`，检查在 `Manifest::check`：给的游戏要在 `games` 里、一个游戏一项、`windows` 写成 `程序名.exe` 不带路径、`needs` 是自己的开关）；`Plugin::send_to_game(host, &ChatSend) -> Result<ChatSent, ChatStop>`，默认拒绝。重新导出 `ChatSend` / `ChatSent` / `ChatStop` / `StopCode` / `SendToGameSpec` / `CancelToken`。第三方的清单由 `kotone-remote-plugin` 合成，这一项永远是空的（测试 `third_party_plugins_never_provide_send_to_game`）。
  - **英雄联盟**（`plugins/lol/src/chat.rs`，新）：清单登记 `LeagueClientUx.exe`（不登记 `League of Legends.exe`），`needs = ["client", "chat"]`；新数据开关 `chat`“在客户端里发聊天”，默认开，风险提示带出处（见上面“主会话定的”）。发的时候现读对局阶段（`ChampSelect` → 选英雄的聊天；`Lobby` / `Matchmaking` / `ReadyCheck` → 房间的聊天；对局里、结算、首页 → `noChatHere`，各说一句为什么），读选英雄 / 房间的数据看是不是自定义对局（自定义 → 两边都看得到 = `all`，否则 `team`），要的频道和这里一样才发（“发给更多人不退”），现查会话（选英雄认 `championSelect` 而且 id 含 `lol-champ-select`，房间认 `customGame`；id 有 `/`、`?`、`#`、空白的不拼进路径），几行用换行连成一条，`POST /lol-chat/v1/conversations/{id}/messages` 一次，请求体只有 `body`、`type: chat`。每次读最多 2 秒、不超过离截止还剩的时间；离截止不到 300 毫秒就不读不发（`timeUp`）。`Lcu` 加 `get_until` / `post_until`（这一次的期限；POST 只看状态）和 `LolError::Timeout`。错误：连不上 / 读的时候没回话 → `clientUnavailable`；POST 404 → `noChatHere`（聊天已经没了）；别的非 2xx → `clientRejected`（带状态码和客户端的话）；POST 期限到了或连接中途断了 → `clientNoReply`（“可能已经发出去了，别再发一遍”）。只发一次。日志只记地方、行数和原因代码，不记文字。照 LeagueAkari 的做法（`dev` 分支提交 `4f8981f8` 的 `in-game-send/send-executor.ts`、`league-client/lc-state/index.ts`、`ongoing-game/computed-state.ts`），只学做法、不抄代码（ADR 039 D5）。给琴音的那段规则加了半句：选英雄、房间里也用 `game_chat.send`，只有队伍频道。
  - **测试**：`chat::tests` 11 个，本机假客户端走真 HTTP（查 Basic 认证、记下每个请求）：选英雄一次发完、路径里的 id 原样、请求体是换行连起来的一条、读的是阶段 / 选英雄数据 / 会话三样；房间、排队、确认都发到房间；自定义房间和自定义选英雄要队伍不发、要所有人发；选英雄要所有人不发；首页 / 对局里 / 结算不发、只读了阶段；只有不认的选英雄会话 → 没连上；客户端回 404 / 500 只发一次；POST 卡住 3 秒、期限 0.9 秒 → 1.2 秒内返回 `clientNoReply`、只发一次；期限不到 300 毫秒一个请求都不发；取消了不发；没人听的端口 → `clientUnavailable`（Windows 上连没人听的端口要重试约 2 秒，读的 2 秒先到，报的是“没回话”）。`kotone-plugin-api` 的 `send_to_game_is_checked_against_the_manifest`；`plugins/lol` 的 `manifest_and_tools_agree` 钉住只登记客户端、`needs`、`chat` 默认开且风险提示带 #480；Playwright `lol-control.spec.ts` 加“在客户端里发聊天”默认开、风险提示里两个 GitHub 链接和 Riot 的出处、关掉写进设置。这一段宿主还没把插件接进入口（第三段），玩家还看不到变化。
- **2026-11-25 第 2 期第三段（桌面端接上 + 文档）**：
  - **桌面端**（`apps/desktop/src-tauri/src/game_chat.rs`）：`providers(app)` 把内置插件表（`plugins::registry()`）里每一项 `[[send_to_game]]` 包成入口的 `ChatProvider`（`PluginChat`）：`off()` 现看设置，话和游戏中心的事共用 `Manifest::needs_off`（从 `event_off` 抽出来，`kotone-plugin-api/src/events.rs`）；`send()` 把插件的异步方法交给运行时、用 channel 等结果（不 `block_on`：哪个线程调都不会卡住运行时），插件过了截止时间 0.5 秒还没回就说“可能已经发出去了”（比入口的看门狗短）。只从内置插件表建，第三方插件（`remote_plugins`）拿不到（D7）。`lib.rs` 建入口时 `with_providers(game_chat::providers(app.handle()))`。测试 `plugins_register_their_own_windows_and_never_a_game_window`：窗口不重复、不是任何内置游戏配置的进程（第 2 期还没有按键的发法；加 `keys` 时删掉这一条）、英雄联盟登记了客户端。
  - **语音输入**（`plugins/voice-input/src/send.rs`）：插件的发法没发出去时代码是 `CLIENT_CHAT_FAILED`（原来会落到 `INJECTION_FAILED`，诊断包里看着像打字失败）。
  - **`game_chat.send` 的说明**（`kotone-agent/src/body.rs`）：英雄联盟客户端在前台时（选英雄、房间里）经客户端的聊天接口发、不按键、几行连成一条、只有队伍频道（自定义对局两边都看得到）、不能只打进聊天框；`route` 的三个值。
  - **文档**：[工具服务手册](../tool-service.md)“发到游戏的结果”一节（`route`、`plugin`、`channelName`、回执、全部原因代码，第 2 期加的六个）；[术语表](../glossary.md)加“发法 / Chat provider”；`plugins/lol/README.md`（第二段）；CHANGELOG。
  - **真机脚本** `scripts/manual-e2e/lol_tools_real.py` 加 C 部分（假客户端加会话列表、房间、记下 POST、阶段可切；配置选中英雄联盟，走按键那条路时前台必须是游戏）：C0（不动前台、不发系统输入，只在终端 / 资源管理器 / VS Code 在前台时跑）前台不是客户端 → 被拒、假客户端没收到；C1–C8 要 `--chat-front`，**要玩家让出前台**（把 `powershell.exe` 复制成 `LeagueClientUx.exe` 跑测试窗口、`fg.ps1` 拿到前面会按一下 Alt）。`--chat-only` 不要 DeepSeek Key。
  - **测试**：`cargo test -p kotone-tauri --lib` 159 个通过（新加 `plugins_register_their_own_windows_and_never_a_game_window`）；`kotone-agent` 106、`kotone-plugin-api` 20 + 边界 4 + 事件边界 3、语音输入 9 个通过；clippy（`kotone-tauri`、`kotone-agent`、`kotone-plugin-api`、语音输入）干净。真机 `lol_tools_real.py --chat-only`（新编的 `target/debug/kotone.exe`，隔离实例，不发系统输入、不动前台）C0 2/2：假客户端在选英雄、前台是终端（warp.exe）→ `game_chat.send` 被拒（“前台是 warp.exe，不是「League of Legends」”）、假客户端没收到消息。
  - **还没测**：C1–C8（要玩家让出前台：`python -X utf8 -I scripts/manual-e2e/lol_tools_real.py --chat-front`，加 `--chat-only` 不跑真模型的 C8）；真客户端（客户端窗口的前台进程名、换行在选英雄聊天里怎么显示、10 行 × 200 字收不收、自定义对局和斗魂竞技场的选英雄聊天谁看得到、国服屏蔽词的表现）；玩家自己的语音输入在客户端里（要麦克风或 VB-CABLE）。
- **2026-11-25 第 3 期：英雄联盟一键喊话**（和 [ADR 040](040-hotkeys-belong-to-core.md) 第 3 期一起发版；设计见 ADR 040 第 3 期设计第 4 节，主会话定的 P6：先做四条，都发队伍频道，不带默认键）：
  - **`Host::send_to_game(text, channel)`（给插件的新权力，单向门）**（`kotone-plugin-api`）：插件把几行交给本体**唯一的发话入口**，和 `game_chat.send` 同一个入口（桌面端 `game_chat::send_to_game_chat`：前台挑游戏和发法、频道、一把输入锁、等修饰键、不补发、执行记录都在那里），返回和 `game_chat.send` 一样的结果。插件拿到的只是“经入口发一段话”，自己不按键（边界测试 `plugins_never_press_keys_themselves` 照旧过）。默认实现一律拒绝，第三方插件的 `RemoteHost` 不实现（D7）；桌面端 `DesktopHost` 只给内置插件、插件开着、**有人在调它的工具**时（她 → `Speaker::Companion`，外部 Agent → `Agent`，热键 → `Automation`，照调它的人的前台规矩；插件自己的后台、页面调的、入口回头调插件的发法时都不能发：`plugins::speaker_of`）。执行记录的发起方是 `plugin:<插件 id>`。
  - **“停下”停掉正在发的几行**（设计第 4 节第 6 条要先核对的）：核对下来以前停不掉（`stop_everything` 不碰入口里的发送）。现在经 `send_to_game_chat` 发的（她、外部 Agent、热键、一键喊话）登记自己的取消，“停下”一起取消，停在下一行之前（已经按下去的不撤回、不补发）。所以不另加 LeagueAkari 那种“取消发送”键。玩家自己的语音输入、自动化各有自己的取消，不在这里。
  - **`lol.shout`**（`plugins/lol/src/watcher/shout.rs`，新）：`{what: flash | records | premade, side?: enemy | ally | all, channel?: team | all, preview?}`，按这局已有的数据现生成几行（不调 AI、不现查），经 `Host::send_to_game` 发；`preview: true` 只返回会发的几行。副作用 `input`、`records-run`（和 `game_chat.send` 一样，默认允许，绑热键时不问），`needs = ["client"]`，只给她（和热键、外部 Agent），页面调不到。
    - **闪现那份数据从哪来**：英雄联盟的接口里**没有**召唤师技能冷却（局内接口只有带了什么技能），只有玩家报过的——玩家说“对面阿狸闪了”，她调 `lol.spell` 记下（按游戏时间、模式和装备算好几点好），记在插件内存里的 `SpellTracker`，这局结束清掉。“喊对面闪现”每个还没好的一行“亚索 闪 14:32好”（和 `lol.spell` 返回的 `chat` 同一个写法，先好的在前），再加一行局内接口里的“对面没带闪：阿狸”。带没带闪按不随客户端语言变的 `rawDisplayName`（`GeneratedTip_SummonerSpell_SummonerFlash_DisplayName`；局内接口没有技能 id）认，`lol.match` 和一键喊话用同一个函数 `LivePlayer::carries_flash`；字段缺、认不出时不说“没带闪”（以前按显示的名字“闪现”认，英文客户端会把每个人都当成没带闪）。**没有记录时**：有人没带闪就只喊那一行；都带着闪、谁也没记过就不发，说“没记到对面谁交了闪（英雄联盟的接口里没有冷却，要你先说“对面亚索闪了”她才记得住），对面也都带着闪：这次没发”（热键那边照“没成”说给玩家）。
    - **战绩**：后台进加载界面就查好的那一份（`lol.scout` 同一份）：段位、近 20 局胜负、KDA、连胜连败、常玩 / 没玩过这个英雄，一人一行，“对面 亚索：……”；还有人没查完就不发（“还差 N 个人，过几秒再按”），团战等不起现查。选英雄时只有我方（对面看不到是谁）。
    - **开黑**：同一份数据里的开黑分组，一组一行，对面在前；看不出开黑不发。
    - **不发玩家名**：只写英雄；选英雄时还没选的用分路，再不行用“几楼”（排位选英雄时名字可能是藏着的）。
    - 没东西可喊、读客户端数据关着、不在对局里、入口不发（前台不是游戏……）都返回错误，热键那边算“没成”，用一句话告诉玩家为什么。
  - **四条预设**（`plugins/lol/plugin.toml` 的 `[[commands]]`，ADR 040 D1 的第二种命令“调自己的工具，带固定参数”第一次落地）：喊对面闪现（`flash`/`enemy`）、喊对面战绩（`records`/`enemy`）、喊我方战绩（`records`/`ally`）、喊开黑情况（`premade`/`all`），都是 `channel = "team"`、不带键。清单检查（`Manifest::check`）：只能调自己的工具、按一下、不听、`args` 是表；`check_tools`：调的是自己给她的工具。热键表里列在“英雄联盟”下面（名字在插件的 messages 里，`plugin_lol_command_<id>_name`），玩家录一个键就生效，存法照旧 `{key, command: "lol.shout-enemy-flash"}`。按下时走自己加的“调一个工具”同一条路（`hotkey_run::press_command`）：随应用登记、不用语音识别、照 `lol.shout` 现在的权限再判一次、同一条没做完再按不叠加、成了弹幕一行、没成说原因、`hotkeys.status` 记 `lastRun`。能不能按由宿主照工具目录和权限算（`Sources::custom` 同一个判法）：“读客户端数据”关着写“要先在左侧栏「插件 → 英雄联盟」打开“读客户端数据””、不登记；插件关着不登记不占键；权限里设成禁止 / 只读模式写原因；**设成每次问的写“热键上没法每次问，去她的「权限」里改成允许”**（插件预设没有确认这一步：确认要存在条目里，预设的条目只有键；实施时定的）。
  - **照 LeagueAkari**（`dev` 分支提交 `4f8981f8` 的 `src/main/shards/in-game-send/`：`preset-controller.ts` 预设按下时从这局已经读好的数据现生成几行再发，“我方 / 对面 / 全部”说的是谁的情况不是频道；`send-executor.ts` 按阶段选路、发的时候新的取消旧的、单独一个取消键），只学做法、不抄代码（ADR 039 D5）。不一样的地方：键归本体的热键表（插件不登记键，ADR 040）；同一条没发完再按不叠加（不是取消旧的，免得队友收到半截）；停下用本体的“停下”；多了 LeagueAkari 没有的“喊对面闪现”；不做它的“打野偏好”（要逐局时间线）和“自定义模板”（一段代码生成文字：要做也是自动化）。
  - **给她的规则**加半句：“把对面闪现 / 战绩 / 开黑发出去”用 `lol.shout`（一次调用、措辞固定、不发玩家名）。
  - **测试**：`plugins/lol` 5 个新的（`watcher/shout.rs`）：参数；假局内接口的一局里闪现只喊报过的、已经好了的不喊、没记过只喊没带闪、都带着闪不发并说怎么让她记；假客户端的近 20 局一人一行、不带玩家名、隐藏战绩、还在查的不发；开黑一组一行、没选英雄的用分路和几楼；工具经假宿主走一遍（交给宿主的是生成好的几行和预设的频道、只看不交、宿主不发原因照实交回、读客户端数据关着和不在对局里什么都不交），清单测试钉住四条预设和文案。`kotone-plugin-api`：调工具的命令只调自己的、给她的工具（调 `settings.update`、别的插件的、听的、按住的、`args` 不是表、只写 `args` 都读不进来）；没实现的宿主拒绝 `send_to_game`。`kotone-core`：插件调工具的命令列在表里、照宿主判、随应用登记、要问的写原因不登记、插件关着不登记。桌面端 4 个：四条预设当成“调一个工具”看（默认能按、读客户端数据关着说打开哪个开关、插件关着、每次问、禁止），绑了键随应用登记、不交给“听一句”的插件，插件发话照调它的人（页面调的不能发，`lol.shout` 只给她），“停下”取消正在发的。Playwright `hotkeys-page.spec.ts`：表里多了英雄联盟那四行（名字、来源、没设键、没有恢复默认），搜“英雄联盟”只剩四行，录 Alt+F1 存进热键表、当前生效，读客户端数据关着写要打开它、不登记。真机 `lol_tools_real.py` 加 S 部分（不发系统输入，`--chat-only` 就跑）：S1 表里有四行；S2 没记录时不发并说怎么记、`lol.spell` 丽桑卓以后第一行就是它的 `chat`；S3 对面战绩 5 行、不带录像里任何一个玩家名，开黑分组；S4 终端在前台时真发被入口拒绝（“请先切到游戏”）、假客户端没收到；S5 F19 绑喊对面闪现，这个测试里 `lol.shout` 是禁止的 → 不登记、原因写权限。新编的 `target/debug/kotone.exe` 跑 `--chat-only`：11/11 过（S 9 条、C0 2 条）。
  - **还没测**：真的按下预设热键（应用级热键走低级钩子，`SendInput` 进不去，要人按）；对局里经按键发几行（要游戏在前台）、选英雄里经客户端发（C1–C8 那样要玩家让出前台）；真客户端里的样子（国服屏蔽词、10 行连成一条在选英雄聊天里怎么显示）。
