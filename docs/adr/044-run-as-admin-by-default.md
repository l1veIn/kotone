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
| 以管理员身份运行的程序用普通权限启动别的程序：拿桌面外壳（`GetShellWindow` 的进程，也就是 explorer）当新进程的父进程（`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`），新进程就继承外壳的普通权限令牌 | [PowerToys](https://github.com/microsoft/PowerToys)（MIT）的 `run_non_elevated`（`src/common/utils/elevation.h`）：PowerToys 可以设成“总是以管理员身份运行”，它打开的东西照样是普通权限 |
| 链接、文件夹、快捷方式、`steam://` 这类要交给外壳打开的：让外壳自己去 `ShellExecute` | Raymond Chen 在 The Old New Thing 上的 “How can I launch an unelevated process from my elevated process, redux”（经桌面的 `IShellDispatch2::ShellExecute`；链接实现时核对后补上）；我们安装包用的 `nsis_tauri_utils::RunAsUser`（[tauri-apps/nsis-tauri-utils](https://github.com/tauri-apps/nsis-tauri-utils)）做的是同一件事 |

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
  2. **交给外壳打开的**（链接、文件夹、启动游戏、`kotone-cli app launch` 被 Kotone 调起时）：照 Raymond Chen / `RunAsUser`，让外壳去打开。
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

（做完以后写在这里。）
