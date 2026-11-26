# ADR 044：默认以管理员身份运行；Kotone 打开的东西一律降回普通权限

状态：Accepted（2026-11-26，玩家定的：“可以，默认管理员权限吧”；细节由主会话照讨论记录定）。

依据：[2026-11-26 rc.9 试玩讨论记录](../research/2026-11-26-rc9-playtest-discussion.md)第一节。

## 背景

玩家试玩 rc.9：呼叫键（Ctrl+\`）在英雄联盟客户端或游戏前台时没反应；桌面、Chrome、QQ、Warp 前台时正常。

**原因**：Windows 的权限隔离（UIPI）。国服的 `League of Legends`、`LeagueClientUx`、`LeagueClient`、`RiotClientServices`、`wegame`、`tcls_core` 都是管理员权限（High），Kotone 是普通权限（Medium）。前台窗口权限比自己高时，普通权限程序的低层键盘钩子收不到按键，呼叫键、说话键、Esc 停下、快捷指令、一键喊话、鼠标侧键全都失效；往这些窗口发键（`SendInput`）也会被挡。

**证据**（讨论记录第一节）：日志里能用的那几次都有 `named hotkey fired: assistant`，在客户端或游戏前台按的一条都没有；玩家的配置里两段“要管理员权限”的提示都点了“不再提示”，那段提示只说“发文字会失效”，没说热键也会失效；对照实验里普通权限的 `RegisterHotKey` 在 `LeagueClientUx` 前台 7 次都收到（所以只解决呼叫键可以，但解决不了往游戏里打字）。

### 核对过的现状（2026-11-26，`1246c799`）

- 设置 `runAsAdminOnStart`、`adminPromptDismissed`、`autoAdminPromptDismissed`（`crates/kotone-core/src/settings.rs`），默认全是 `false`。
- 开着 `runAsAdminOnStart` 时，启动最早（Tauri 初始化之前，`startup.rs` 的 `auto_elevate_before_tauri_init`）用 `ShellExecuteExW "runas"` 以管理员身份重开自己，原来的进程退出；UAC 点“否”就照常以普通权限运行。`kotone.exe <子命令>`（命令行、MCP）从不提权。
- 开机自启是 HKCU 的 Run 键，启动的是普通权限，然后照上面那条弹 UAC。
- 两段提示：`ElevationPrompt.svelte`（“需要管理员权限 … 是否现在以管理员身份重启？”，有“不再提示”）和 `AutoAdminPrompt.svelte`（已经是管理员时问“启动时自动请求管理员权限？”）；发键被拦时 `kotone://elevation-needed` 弹 `blocked` 版本（不管“不再提示”）。
- **Kotone 启动的东西现在都继承 Kotone 的权限**，没有一处降权：
  - `shell.run`（`kotone-exec`，PowerShell，标准输入输出都接管道，放进关掉就杀的作业对象）；
  - 第三方 MCP 插件（`kotone-mcp` 的 stdio 服务器，同样接管道、进作业对象）；
  - 打开链接（`rundll32 url.dll,FileProtocolHandler`）、打开文件夹（`explorer.exe <目录>`）；
  - 启动游戏（`ShellExecuteW "open"`，exe、快捷方式、`steam://` 这类地址都有）；
  - `kotone-cli app launch`（`CreateProcessW`，命令行本身不提权，只有被管理员身份的 Kotone 调起时才会继承）。
- 故意以管理员身份启动的：上面的 `runas` 重开；游戏语音的 VB-CABLE 驱动安装程序（装驱动本来就要管理员）。
- 在线更新：`tauri-plugin-updater` 自己启动 NSIS 安装包；安装包是“装给当前用户”的，结束页和更新后重开都用 `nsis_tauri_utils::RunAsUser` 以普通权限打开 Kotone。
- 现有的权限判断只有 `TokenElevation`（`elevation.rs` 的 `is_elevated`、`is_process_elevated`），没有读完整性级别，也没有任何地方拿桌面外壳（explorer）的令牌。

## 参考了谁

| 照什么 | 来自 |
| --- | --- |
| 游戏工具以管理员身份运行、每次启动点一次 UAC | 国服英雄联盟客户端、WeGame 本身就是这样（玩家：“可以，默认管理员权限吧”） |
| 以管理员身份运行的程序用普通权限启动别的程序：拿桌面外壳（`GetShellWindow` 的进程，也就是 explorer）当新进程的父进程（`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`），新进程就继承外壳的普通权限令牌 | [PowerToys](https://github.com/microsoft/PowerToys)（MIT）的 `run_non_elevated`（`src/common/utils/elevation.h`）：PowerToys 可以设成“总是以管理员身份运行”，它打开的东西照样是普通权限；Raymond Chen 的 [How can I launch an unelevated process from my elevated process, redux](https://devblogs.microsoft.com/oldnewthing/20190425-00/?p=102443)（讲的就是换父进程） |
| 链接、文件夹、快捷方式、`steam://` 这类要交给外壳打开的：让外壳自己去 `ShellExecute` | Raymond Chen 的 [How can I launch an unelevated process from my elevated process and vice versa?](https://devblogs.microsoft.com/oldnewthing/20131118-00/?p=2643)（经桌面的 `IShellDispatch2::ShellExecute`）。（原来这里还写了安装包用的 `nsis_tauri_utils::RunAsUser` 是同一件事：实现时核对，它做的是“复制资源管理器的令牌 + `CreateProcessWithTokenW`”，不是这个，见实施记录） |

实现时照这两个参考写，代码里注明出处和许可（原则 7）。

## 决定

### D1 默认以管理员身份运行

- `runAsAdminOnStart` 新装默认打开（`Settings::default()` 改成 `true`；没写这个键的旧配置也按新默认）。
- **第一次先解释再弹 UAC**：加一个新字段记“已经用新说法问过”（例如 `adminPromptRevision`，原则 19 只加不改）。没问过的（新装的、以前点过“不再提示”的、以前就开着的，一律）：启动时先不自动重开，在 Kotone 窗口里用一句话说明原因，再弹 UAC。建议的说法（照“界面用语 v2”，实现时定稿）：“英雄联盟这类游戏是以管理员身份运行的，Kotone 也要以管理员身份运行，呼叫键、说话键和往游戏里打字在游戏里才管用。接下来 Windows 会问你一次。”两个按钮：「好」（记下问过、打开 `runAsAdminOnStart`、以管理员身份重开）和「先不用」（记下问过、关掉 `runAsAdminOnStart`）。
- **问过以后**：照 `runAsAdminOnStart` 直接弹 UAC（包括开机自启），不再解释。
- **UAC 点了“否”**：照常以普通权限运行。等发现前台游戏是管理员权限时（现有的 `is_process_elevated`），说清楚哪些用不了：呼叫键、说话键、Esc 停下、快捷指令、一键喊话、往游戏里打字；给一个“以管理员身份重开”。每次启动最多说一次。说法里不再只提“发文字”。
- 两段旧提示（`ElevationPrompt` 的“以后不再提示”、`AutoAdminPrompt`）由上面的一个解释框和一个“用不了什么”的提示代替；“设置 → 系统”里的“启动时以管理员身份运行”开关留着（玩家随时能关）。
- **有多深远**：默认值和一个新字段，双向门（随时能改回默认关）。
- **什么事实会让我们改主意**：管理员身份运行时 WebView2、浏览器、MCP App 出了普通权限下没有的毛病；杀毒软件因为它报毒；玩家嫌每次点 UAC 烦（那时看 D3 的长期办法）。

### D2 必须一起做：Kotone 打开的东西一律用普通权限运行

Kotone 以管理员身份运行时，她跑的命令、第三方 MCP 服务器、打开的网页都不能跟着变成管理员：模型决定的命令和别人的程序拿到管理员，出了事就是整台机器的事。

- 本体只留**一个**启动入口（`kotone-platform-windows` 里一个模块），两种用法：
  1. **要接管道、要放进作业对象的**（`shell.run`、第三方 MCP 的 stdio 服务器）：照 PowerToys，以桌面外壳为父进程创建，标准输入输出照旧接到 Kotone，照旧放进关掉就杀的作业对象。
  2. **交给外壳打开的**（链接、文件夹、启动游戏、`kotone-cli app launch` 被 Kotone 调起时）：照 Raymond Chen（2013）的做法，让外壳去打开。（`kotone-cli app launch` 实现时改走第 1 种：它要进程号，见实施记录。）
- Kotone 本来就是普通权限时，两种都照原来的做法直接启动（行为不变）。
- **降不下来时**（找不到桌面外壳，例如 explorer 没在跑）：她的命令和第三方 MCP 服务器**不启动**，返回具名的原因（“没法用普通权限启动：……”），不偷偷以管理员身份跑；链接、文件夹、游戏同样不打开，说明原因。
- **故意保持管理员的**只有两处：`runas` 重开自己、VB-CABLE 驱动安装程序。
- 在线更新的安装包由 `tauri-plugin-updater` 自己启动，跟着 Kotone 的权限；安装包验过签名、装给当前用户，结束后用 `RunAsUser` 以普通权限打开 Kotone（再弹一次 UAC）。不改插件，写进已知情况。
- 测试：单元测试测“普通权限时照旧直接启动”和管道、作业对象；真机测试要以管理员身份启动 Kotone（要点 UAC），检查 `shell.run` 跑出来的 `whoami /groups` 是 Medium、第三方 MCP 服务器进程、打开的浏览器、启动的游戏都是普通权限——这一项列进要玩家本人做的清单。
- **有多深远**：双向门（一个模块）。
- **什么事实会让我们改主意**：某个杀毒软件把“换父进程”当成可疑行为拦了；那时只留“交给外壳打开”这一种，管道的用另一个办法（例如经外壳启动一个普通权限的小中转进程）。

### D3 不做静默提权

不做“最高权限计划任务”（每次不弹 UAC 就拿到管理员）：Kotone 可以装在普通程序也能写的目录里（玩家装在 `D:\琴音`），静默提权等于让任何程序换掉 `kotone.exe` 就能不弹窗拿到管理员；杀毒软件也会报；维护面也会变大。

长期的路：代码签名 + 装进 Program Files + `uiAccess`。那样不要管理员也能听键、往游戏里打字，UAC 也不用弹了。这条以后单独做 ADR。

### D4 只给呼叫键加 `RegisterHotKey` 先不做

它能让呼叫键在管理员游戏前台时管用，但解决不了往游戏里打字（核心功能，ADR 039 D2），也解决不了说话键、Esc、快捷指令。默认管理员一条规矩全解决，比补一半简单。

## 已知的代价

- 管理员身份运行时，从普通权限的资源管理器往 Kotone 窗口里拖文件会被 Windows 挡掉（UIPI 也管拖放）。
- 每次启动（包括开机自启）点一次 UAC。

## 不做

- 不做静默提权（D3）。
- 不读游戏内存、不注入（红线不变）。

## 实施记录

### 实施记录（默认管理员、打开的东西降回普通权限，2026-11-26）

**玩家看到的**：

- 装好以后第一次打开（以前点过“不再提示”的、以前就开着的也一样）：Kotone 先不弹 UAC，窗口里出一个框“以管理员身份运行 Kotone”：“英雄联盟这类游戏是以管理员身份运行的，Kotone 也要以管理员身份运行，呼叫键、热键和往游戏里打字在游戏里才管用。每次打开 Kotone，Windows 都会问你一次。”下面一行小字“随时可以在「设置 → 系统」里关掉”。「好」= 记下问过、打开开关、马上以管理员身份重开（弹 UAC；已经是管理员就不重开）；「先不用」= 记下问过、关掉开关。框盖在新手引导上面（先决定要不要重开，免得引导做到一半被重开打断）。窗口藏着时（`--background`），等它显示出来框就在那里。
- 问过以后：照“启动时以管理员身份运行”直接弹 UAC，包括开机自启，不再说明。
- UAC 点了“否”（或者开关关着）：照常以普通权限运行。管理员身份的游戏（游戏配置里的进程，加上插件登记了要往里打字的窗口，也就是英雄联盟客户端）到前面来时，窗口里出“游戏在前面时有些功能用不了”：“前面的游戏是以管理员身份运行的，Kotone 不是。游戏在前面时，呼叫键、语音输入热键、停下热键和 Esc、快捷指令、一键喊话都收不到，往游戏里打字也会被挡。以管理员身份重开 Kotone 就都能用。”按钮「以管理员身份重开」「知道了」。往游戏里打字真的被拦下时开头换成“刚才往游戏里打字被 Windows 挡掉了：……”。每次启动最多说一次（两种合起来一次）。
- 「设置 → 系统」的开关改叫“启动时以管理员身份运行”，说明写了默认开、关掉的后果、每次打开问一次、Kotone 打开的东西照样是普通权限；拨一下也算问过。语音输入页的横幅改成“游戏是以管理员身份运行的，Kotone 不是 / 游戏在前面时，呼叫键和热键都收不到，往游戏里打字也会被挡……”，按钮“以管理员身份重启”。
- 管理员身份的 Kotone 开着时再双击图标（普通权限）：不弹 UAC，直接把已经开着的窗口唤出来，也不会再起第二份。
- Kotone 是管理员身份时：她跑的命令（`shell.run`）、第三方 MCP 服务器、打开的网页和文件夹、启动的游戏、`kotone-cli app launch` 起的进程都是普通权限（父进程是资源管理器）。降不下来（资源管理器没在运行）时不启动，报“没法用普通权限启动：找不到桌面（资源管理器没在运行）”。

**怎么做的**：

- 设置（原则 19，只加不改）：`runAsAdminOnStart` 默认改成开（`#[serde(default = "default_true")]`，没写这个键的旧配置也按新默认）；新字段 `adminPromptRevision: u32`（默认 0）。现在的说法是第 1 版（`elevation::ADMIN_PROMPT_REVISION`）。`adminPromptDismissed` / `autoAdminPromptDismissed` 留着只读不写。`settings_guide` 里 `adminPromptRevision` 记为界面提示状态，`runAsAdminOnStart` 的说明照新说法。
- 启动（`startup.rs`，Tauri 初始化之前）：`should_auto_elevate(开关, 版本, 已是管理员, 防循环标记)`，版本比第 1 版小就不重开。前面先看单实例插件的互斥锁：已经有一个在跑就不弹 UAC；打不开它的锁（它是管理员身份、这次是普通权限）时单实例插件会以为自己是第一个、再起一份，所以照插件 2.4.3 的格式（`{identifier}-sim/-sic/-siw`、`WM_COPYDATA` 1542、`工作目录|参数…`）自己转交然后退出；管理员身份的那份启动时对插件的消息窗口 `ChangeWindowMessageFilterEx(WM_COPYDATA, MSGFLT_ALLOW)`，普通权限发得进来。单元测试钉住 identifier 和插件版本，插件升级时提醒回来核对。
- 隔离实例（设了 `KOTONE_HOME`：开发、测试、外部 Agent 的独立实例）不参与默认管理员，和它们不参与单实例一样：不弹 UAC、不说明（`get_elevation_status` 报版本 0）、不看前台。不然 `scripts/manual-e2e` 的真机脚本每个都会被说明框挡住（它们的配置只写了 `adminPromptDismissed: true`）。
- “用不了什么”（`compatibility.rs`）：普通权限时每 2 秒看一次前台：前台进程是管理员身份（`is_process_elevated`）而且是游戏的窗口，发 `kotone://elevation-needed`（载荷 `{reason: "foreground"}`）；打字被拦时照旧从 `inject` 的监听发（`{reason: "blocked"}`）。只发一次，`get_elevation_status` 多了 `notice`（这次启动已经发现的那一次）和 `adminPromptRevision`，窗口晚打开也补得上。界面：`AdminExplainDialog.svelte`、`ElevationNotice.svelte` 代替 `ElevationPrompt.svelte`、`AutoAdminPrompt.svelte`（删了），逻辑在 `Settings.svelte`，不新开窗口。
- 降权启动只有一个入口 `crates/kotone-platform-windows/src/unelevated.rs`（`needed()` = Kotone 是管理员身份；普通权限时调用方照原来的做法直接启动）：
  - **要接管道、放进作业对象的**（`kotone-exec` 的 `shell.run`、`kotone-mcp` 的 stdio 服务器）：`spawn_piped_in_job(PipedCommand)`。照 PowerToys `run_non_elevated`（MIT，`src/common/utils/elevation.h`：`GetShellWindow` → 进程 → `PROCESS_CREATE_PROCESS` → `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`）和 Raymond Chen “How can I launch an unelevated process from my elevated process, redux”（https://devblogs.microsoft.com/oldnewthing/20190425-00/?p=102443）。换了父进程以后可继承的句柄从新父进程继承，所以三根管道的子进程那一头先 `DuplicateHandle` 进资源管理器（可继承），`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` 只继承这三个，`STARTUPINFO` 填它们在资源管理器里的值，`CreateProcessW` 之后马上在那边关掉（不关的话读不到结尾）。挂起创建 → 放进关掉就杀的作业对象 → 恢复，`CREATE_NO_WINDOW`，环境变量照旧（MCP 先清空再给基线 + 配置的，`shell.run` 继承 Kotone 的）。命令行、`.cmd` / `.bat` 经 `cmd.exe` 的转义照抄标准库 `library/std/src/sys/args/windows.rs`（MIT/Apache-2.0；标准库建不了换父进程的进程）。等退出用 `RegisterWaitForSingleObject`，不占 tokio 线程；标准输入输出还是 tokio 的 `ChildStdin` / `ChildStdout`（`from_std`），两个调用方几乎没改。
  - 没选 `CreateProcessWithTokenW` + 资源管理器的令牌：它经 Secondary Logon 服务建进程，文档列的创建标志里没有 `CREATE_NO_WINDOW`（PowerShell 会弹黑窗口），那个服务被关掉时也用不了。
  - **交给外壳打开的**（`open_external` 的网页、`open_in_file_manager` 的文件夹、`game_window::launch` 的程序 / 快捷方式 / `steam://`）：`open_via_desktop`，照 Raymond Chen “How can I launch an unelevated process from my elevated process and vice versa?”（2013-11-18，https://devblogs.microsoft.com/oldnewthing/20131118-00/?p=2643）：`IShellWindows::FindWindowSW(桌面)` → `SID_STopLevelBrowser` → `IShellView` → `IShellFolderViewDual::Application` → `IShellDispatch2::ShellExecute`，在单独的 STA 线程里调。交给桌面以后拿不到“打不开”的结果，所以先查路径在不在；先 `AllowSetForegroundWindow(资源管理器)`，打开的窗口才能到前面。
  - **`kotone-cli app launch`**（调用方是管理员身份时）：要 PID 和退出码判断单实例转交，所以用换父进程那条路（不接管道、不继承句柄、`DETACHED_PROCESS`），不交给外壳。另外 `app launch` 碰到起来的 kotone.exe 正常退出（0）而工具服务还没好时接着等：默认管理员以后它多半是以管理员身份重开了自己、在等玩家点 UAC（以前直接报“启动后退出”）。
  - 故意以管理员身份启动的只剩 `runas` 重开自己和 VB-CABLE 安装程序，没动。在线更新照 ADR 不改插件（安装包跟着 Kotone 的权限，结束后 `RunAsUser` 以普通权限打开）。
- KOTONE_HOME 里的文件（查过，没改代码）：管理员身份新建的文件所有者是 Administrators 组，但访问靠的是继承下来的“当前用户：完全控制”（用户目录的 ACL），完整性级别不打标签（中及以上建的文件没有显式标签，按中处理），所以 UAC 点了“否”以后普通权限的 Kotone 照样读写。工具服务的私有目录 / 描述文件 DACL 只给当前用户 SID 和 SYSTEM，读的一方 `verify_private_file` 本来就认 Administrators 当所有者，所以普通权限的 `kotone.exe tools call` / `kotone.exe mcp` 连管理员身份的 Kotone 照旧能连（走本机回环 HTTP，不受 UIPI 管）。没把进程令牌的默认所有者改成用户：那样管理员身份建的内核对象普通权限也能改 DACL，是降低安全性。例外：数据放在只有 `CREATOR OWNER` 授权的目录（某些非系统盘的自建目录）时，管理员身份建的子目录普通权限可能写不进——默认的 `~/.kotone` 不是这样。
- 参考链接核对：ADR “参考了谁”里 “redux” 那篇讲的是换父进程（2019-04-25，上面的链接），让桌面 `ShellExecute` 的是 2013-11-18 那篇；nsis-tauri-utils 的 `RunAsUser`（`crates/nsis-process/src/lib.rs`，照 Chromium `base/win/elevation_util.cc`）现在做的是“复制资源管理器的令牌 + `CreateProcessWithTokenW`”，不是 `IShellDispatch2`，那一格要改。出处写进了 `THIRD_PARTY_NOTICES.md`（新的一节）。
- 叫法：照界面用语 v2，“说话键”写成“语音输入热键”（呼叫键是唯一简称“键”的），“Esc 停下”写成“停下热键和 Esc”。

**测试**：

- 单元测试（`kotone-platform-windows`）：`should_auto_elevate` 真值表（版本 × 开关 × 已是管理员 × 防循环标记，版本 0 时一律不重开）；自己的完整性级别和 `is_elevated` 一致；互斥锁在不在 / 打不打得开；转交：一个不显示的窗口收到 `WM_COPYDATA`（1542、`…\0`）。`unelevated`：命令行和批处理转义和标准库一致、环境变量块排序 / 同名后者算、程序路径解析；普通权限时照旧直接启动（父进程是自己）；**强制走换父进程**（测试开关，不用管理员）：管道进出、退出码 7、环境变量、工作目录、父进程是资源管理器、完整性级别 Medium；结束作业对象连同孙进程（ping）一起结束、丢掉也结束；`.cmd` 带空格、`%`、`&` 的参数原样到达；桌面打不开（假 PID）时不启动、错误以“没法用普通权限启动”开头，找不到程序还是 NotFound；脱离启动的父进程是资源管理器、Medium；让桌面打开：起一个没窗口的 `wscript //B //E:JScript`（只睡几秒），父进程是资源管理器、Medium（不弹窗、不抢焦点；没有桌面的 CI 上这些都跳过）。
- `kotone-exec`：`shell.run` 强制走换父进程：中文输出、退出码、`whoami /groups` 里是 `S-1-16-8192`（Medium）没有 `S-1-16-12288`。`kotone-mcp`：新的 `tests/stdio_unelevated.rs`：服务器握手、调工具，父进程是资源管理器，服务器和它起的进程都是 Medium，关掉以后都结束。另外设上 `KOTONE_SPAWN_UNELEVATED=1` 把 `kotone-exec` 37 个、`kotone-mcp` 全部（含 `tests/stdio.rs` 10 个：超时结束子进程、正常结束留下 `Start-Process` 起的程序、`.cmd` 包装的参数、环境变量基线、启动崩溃说原因、找不到程序）整套换这条路跑了一遍，全过。
- `kotone-core`：新字段默认 0、开关默认开；旧配置（`runAsAdminOnStart: false` + 两个“不再提示”）读出来版本 0、开关照旧关；没写开关的按新默认开。`kotone-tauri`：单实例插件的名字和版本钉住。
- Playwright `apps/desktop/e2e/elevation.spec.ts` 10 个：没问过时弹说明、「先不用」写 `{adminPromptRevision: 1, runAsAdminOnStart: false}` 不重开、「好」写开 + 重开一次、已是管理员「好」不重开、说明盖在新手引导上面点完接着引导、问过了不再弹、自己拨开关也算问过、浏览器预览里不弹；“用不了什么”的两种开头和六样东西、「以管理员身份重开」、已是管理员不说；语音输入页的横幅。terminology.spec 过；整套 Playwright 218 过、2 个没过的是别人新加的 `push.spec.ts`（import 了另一个测试文件导致 `test.use()` 不一致），和这次无关。

**还没测**（要玩家本人在真机上做，点不了 UAC）：

1. 第一次打开（新装或现在这份配置）：先出说明框、没有 UAC；点「好」→ UAC → 以管理员身份重开；任务管理器“详细信息”加“提升的”一列，kotone.exe 是“是”。以后再打开、开机自启：直接 UAC，不再说明。
2. UAC 点“否”：照常打开；把英雄联盟客户端切到前面，等几秒回到 Kotone 窗口，看到“游戏在前面时有些功能用不了”，点「以管理员身份重开」能重开。
3. 管理员身份开着时：让她跑 `whoami /groups`，结果里是 `Mandatory Label\Medium Mandatory Level`（不是 High）；加一个第三方 MCP 服务器（例如 npx 起的）、让她打开一个网页、在聊天里点一个链接、打开一个文件夹（识别历史 / 模型目录）、从游戏列表启动英雄联盟（WeGame），在任务管理器里看这些进程“提升的”都是“否”（WeGame / 游戏自己要管理员时由 Windows 再问一次）。打开的浏览器 / 文件夹窗口能到前面来。
4. 管理员身份开着、窗口收到托盘里时，再双击桌面图标：不弹 UAC，窗口出来，任务管理器里只有一份 Kotone。
5. 管理员身份运行时英雄联盟客户端和游戏在前面按呼叫键（Ctrl+\`）能接通；语音输入热键、停下热键、快捷指令、往游戏里打字都能用（讨论记录第十一节那条复测）。
6. 管理员身份运行过一次以后，把开关关掉、以普通权限打开：设置、聊天记录、模型都还在、能改；外部 Agent（`kotone.exe tools call app.status`、`kotone.exe mcp`）普通权限连管理员身份的 Kotone 连得上。
7. 已知的代价照 ADR：管理员身份时从资源管理器往 Kotone 窗口拖文件会被挡；外部 Agent 用 `kotone-cli app launch` 起 Kotone 时也会弹 UAC（要玩家点）。
