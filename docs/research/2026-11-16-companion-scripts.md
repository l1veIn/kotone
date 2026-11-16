# 自动化（原“陪玩脚本”）：实现与验证记录（2026-11-16 起）

2026-11-16 起用户可见处统一叫“自动化”：工具 `automation.run / list / stop`、事件 `automation.finished`、设置 `automationStopHotkey`、托盘「停止所有自动化」。下文第一、二阶段的记录保留当时的名字（`script.*`、`scriptStopHotkey`），是改名前的实测。

设计见[演进计划 §12](../product-evolution-game-tools.md#12-自动化大模型写程序沙盒执行2026-11-16-定方向)。分支 `claude/companion-scripts`，从 `main`（`90113d8`）开出。

## 第一阶段：语言实验（`crates/kotone-script`）

**结论：采用 Luau**（`mlua` 0.10，`luau` + `vendored` 特性，Luau 0.663 随构建一起编译）。

- **能构建**：在本机 MSVC 工具链下直接编译通过，不新增运行时 DLL。
- **沙盒**：只加载 string / table / math / utf8 标准库，移除 `loadstring`、`require`、`getfenv` / `setfenv`、`collectgarbage`、`print`，再开启 Luau `sandbox` 模式（全局只读、脚本独立环境）。测试确认 `io.open`、`os.execute`、`loadstring`、`require`、`getfenv`、`dofile` 都不可用；`string.rep('x', 1e9)` 被 16 MB 内存上限拦下。脚本给内置函数重新赋值只会在它自己的环境里遮蔽，不影响宿主函数，每次运行也是新 VM。
- **限时与防死循环**：Luau 的中断回调在函数调用与循环回边处触发，用来检查取消、总时长与执行步数。`while true do end` 被步数上限终止；持续按键的脚本被总时长或按键次数上限终止。
- **数据不是指令**：`ocr()` 返回不透明对象，只提供 `:has(文字)` 与 `:count()`，`tostring` 与字段访问都拿不到原文，无法把屏幕上的字传给 `type_text` / `say`。
- **两个目标场景**（假宿主、虚拟时钟）：`every(1000, function() key("Q") end, { for_ms = 10000 })` 恰好按 10 次、间隔 1000 ms；“左上角变黑按回车”在变黑后连续 3 帧（每 200 ms）满足时只按一次回车。
- **其他护栏**：两次按键至少间隔 50 ms（过快时自动等待）；游戏不在前台时按键暂停、回到前台继续；取消立即生效；参数不合法、调用不存在的函数、语法错误都如实报告；`check_syntax` 用于大模型写完后先校验。
- **为什么不是 Rhai**：Rhai 纯 Rust、沙盒更省事，但大模型对它远不如对 Lua 熟悉；Luau 的沙盒与中断能力已经覆盖需要的护栏，因此不再试 Rhai。
- 验证：`cargo test -p kotone-script`（10 个用例），`cargo clippy -p kotone-script --all-targets` 无警告。
- **未验证**：真实按键注入与截图（第二阶段接入）；在真实大模型手里写出脚本的成功率。

## 第二阶段：桌面执行器与真机测试

### 实现

- `apps/desktop/src-tauri/src/scripts.rs`：每个脚本一个线程；`DesktopHost` 把白名单函数接到真实能力——按键 / 打字走 `SendInput`（新增 `inject::key_down` / `key_up`），看屏幕用 GDI 截启动时的游戏客户区（不在前台时先暂停等待），`ocr()` 用陪玩选定的 OCR 模型，`say` / `toast` 复用提醒的交付路径。按住的键记账，脚本结束时全部松开；游戏窗口关闭则结束脚本。同时最多 3 个。
- 停止：应用级**停止脚本热键** `scriptStopHotkey`（默认 `Ctrl+Shift+F12`，设置页「高级 → 热键」可改）、托盘「停止所有脚本」、工具 `script.stop`、Kotone 退出。
- 工具 `script.run / list / stop` 与事件 `script.finished`（为了真机测试提前开放；原计划在第五阶段）。
- 构建：Luau 的 C++ 必须用 `/MT`，与 sherpa-onnx 的 static-MT 一致（`.cargo/config.toml` `[env]`），否则桌面端链接失败（LNK2005 / LNK2038）。

### 真机测试（2026-11-16，本机 Windows 11）

隔离环境：`KOTONE_HOME` 与 `WEBVIEW2_USER_DATA_FOLDER` 在 `target/script-e2e/`；调试构建（嵌入前端）复制到 `%LOCALAPPDATA%\Temp\kotone-script-e2e\` 运行（见下面的“完整性级别”）。目标“游戏”是一个 PowerShell WinForms 测试窗口：记录收到的每个按键按下 / 松开与焦点变化，左上角有可变黑的面板，中间有 40 号字的“找到对局”，可按命令把焦点切给另一个窗口再切回。脚本经 `script.run` 启动，结果从 `script.finished` 事件读取。

| # | 场景 | 结果 |
| --- | --- | --- |
| 1 | `every(500, key("Q"), {times=6})` | 窗口收到 6 次 Q，间隔 499–501 ms，每次按住约 50 ms |
| 2 | 左上角 `brightness` 低于初值 30%、连续 3 帧后按回车 | 面板变黑后 491 ms 收到一次回车 |
| 3 | 每 300 ms 按 W 共 12 次，中途焦点切走 1.5 s | 切走期间两个窗口都没收到 W；切回后**修复前**连续补按 7 次（间隔约 50 ms），修复后按原节拍继续，共 12 次 |
| 4 | `key_down("S")` 后等待，1.5 s 后 `script.stop` | 停止后 3 ms 收到 S 松开 |
| 5 | 停止脚本热键 | 已在低层钩子上注册（日志 `app hotkey script-stop registered on llhook: Ctrl+Shift+F12`）；**未能自动验证按下效果**：钩子按设计忽略合成输入，测试只能发合成按键 |
| 6 | 第 4 个并发脚本 / 语法错误 | 分别返回 `BUSY` 与带位置的 `INVALID_ARGUMENTS`；`script.stop` 一次停止 3 个 |
| 7 | 脚本里 `toast` + `say` | 弹幕在屏幕上滚动（截图确认）；语音返回成功（未人工听）。`say` 排在弹幕后面，等弹幕滚完才开始，整段约 21 s——与提醒相同的输出队列 |
| 8 | `ocr(区域):has("找到对局")` 时按 F | 下载并选用 `ppocrv5-ch-mobile`（`models.download {category: ocr}`，1.5 s）后：**修复前**裁剪区域里一句话碎成 4 个乱码片段、判断为假；修复后读出完整一行，按了 F；对屏幕上没有的字判断为假 |
| 9 | 按住 D 时 Kotone 退出 | 退出时脚本被停止，14 ms 后 D 松开 |
| 10 | 运行中关闭游戏窗口 | 脚本结束，`status=stopped`、`message=游戏窗口已关闭` |

### 真机测试发现并修复的问题

1. **恢复前台后连续补按**（场景 3）：`every` 原先按固定节拍补上暂停期间错过的拍子。改为不补拍：错过的拍子直接跳过，从当前时刻按原间隔继续；单元测试已覆盖。
2. **裁剪后的大字 OCR 碎裂**（场景 8）：检测器按 RapidOCR 的规则把短边放大到 736；560×192 的裁剪图被放大 3.8 倍，65 px 的字变成约 250 px，检测器看不全，一句话碎成 4 段。改为**最多放大 2 倍**（`kotone-ocr` `local.rs` `MAX_UPSCALE`；缩小不受影响）。在同一张裁剪图上 2 倍、3 倍都能完整读出；重新下载全部 15 个模型文件（哈希仍一致）、用 17 个渲染样例复跑多语种验证：2 倍时英文 0.95→0.98～1.00、法文 0.98→1.00、泰文 0.98→1.00、印地文 0.92→0.95，希腊文 1.00→0.94（一个三字母小词读错），其余不变；3 倍时总体略差。全部 13 个包的验证从 125 s 降到 27 s。**这改变了所有本地 OCR 的预处理**（包括陪玩只识别聊天框的裁剪），真实游戏截图上的影响未验证。
3. **`:has()` 只在单行内找**：识别器常把一句话切成几段，改为同时在按阅读顺序拼接、去掉空白后的全文里找；单元测试已覆盖。

### 环境发现（不是产品问题）

- **完整性级别**：本仓库根目录带低完整性标签（继承到所有子目录），从这里启动的 `kotone.exe` 是 Low IL，向普通窗口发送的模拟输入会被 UIPI 静默丢弃（`SendInput` 仍返回成功）。最初 Q 全部“发送成功”却没有到达窗口，就是这个原因；把 exe 复制到仓库外运行后正常。已写入[开发手册](../development.md)。
- **输入法**：测试窗口在中文输入法下，字母键的按下会被输入法吞掉（物理按键同样如此）；测试窗口关闭了输入法。游戏通常自己关闭输入法；若游戏里输入法处于中文状态，脚本按的字母键同样会被输入法拦下。

### 其他验证

`cargo test --workspace`（695 个通过）、`cargo clippy --workspace --all-targets -D warnings`（顺带修了 main 上已有的两处：`OcrBackend::id` 的 getter 命名检查、TTS 清单函数参数过多；以及本阶段的两处）、`pnpm check`、Playwright 全量 58 个。

### 未验证

- 停止脚本热键的物理按下、托盘菜单「停止所有脚本」的点击（只能人工操作）。
- 真实游戏（含反作弊、独占全屏、游戏内输入法状态）里的按键与截图。
- 语音的实际听感；2 倍放大上限对真实游戏聊天截图的影响。

## 第三阶段：统一调度器（2026-11-16，ADR 013）

### 实现

- `kotone-core::automation`：`AutomationScheduler` 由提醒调度器推广而来，管全部执行记录（`Run`）：动作是播报（原提醒）或程序；等待中的执行存在 `~/.kotone/automation-runs.json`，重启后仍在；崩溃时“运行中”的记录恢复为失败；到期超过 120 秒记为错过；到点时已有 3 个程序在跑则这次失败，不排队。播报沿用原来的单条投递管道；程序另有一个 250 ms 的分发循环，不会排在长播报后面。
- `kotone-core::reminders` 只剩提醒意图、提醒视图（界面与助手接口不变）、投递端口与只读的旧格式；第一次启动时把 `reminders.json` 里未到期的提醒导入（记下已导入，旧文件不改）。
- 桌面端 `automation.rs` 改为调度器的 `ProgramLauncher`：启动前确认安排时记下的目标窗口还在，结束时回报 `finish_program`；运行记录不再存在内存里。
- 工具：`automation.run` 加 `delaySeconds` / `at`；`automation.list` 改名 `automation.runs`（含提醒）；不带 `id` 的 `automation.stop` 停止全部程序并取消等待中的程序，保留等待中的提醒；`automation.finished` 对每条进入终态的执行（含提醒）发布一次。

### 真机测试（隔离 `KOTONE_HOME`，exe 复制到仓库外，WinForms 测试窗口）

| # | 场景 | 结果 |
| --- | --- | --- |
| 1 | 隔离目录里预置旧 `reminders.json`（1 条未到期、1 条已过期、1 条已投递）后首次启动 | 只导入未到期的那条；旧文件哈希不变；再次启动不重复导入 |
| 2 | `delaySeconds: 4`，每 300 ms 按 Q 共 3 次 | 返回 `scheduled`；到期后 176 ms 收到第一次 Q（分发循环 250 ms），间隔 289 / 300 ms，记录 `finished`、`keys=3` |
| 3 | 一个运行中的程序 + 一个 60 秒后的程序 + 一条提醒，调用不带 `id` 的 `automation.stop` | `stopped=2`：两个程序都 `cancelled`（原因“工具 automation.stop”），提醒仍在等待；每条一次 `automation.finished`，不含源码 |
| 4 | 安排 3 秒后按 R，随即关闭测试窗口 | 到期时 `failed`：“游戏窗口已关闭（…），自动化没有启动”，没有按键 |
| 5 | 一个 5 分钟后的程序 + 一个按住 S 的运行中程序，`app.quit` 后重启 | 等待中的仍在；运行中的记为 `failed`（“…不会自动重新运行”）。**发现问题**：退出时 S 没有松开，见下 |
| 6 | 重启后按 `id` 取消等待中的程序 | `stopped=1`，`cancelled` |
| 7 | 修复后：按住 D 时 `app.quit` | 18 ms 后 D 松开，记录 `cancelled`（“Kotone 退出”） |
| 8 | 把导入的旧提醒改为 15 秒后到期再启动 | 按时投递（弹幕），记录 `finished`，发布 `automation.finished` |

### 真机测试发现并修复的问题

- **退出时按住的键没有松开**（场景 5）：退出处理只发出取消信号就返回，进程随即结束，程序线程来不及松键、写记录。第二阶段的场景 9 通过，是碰巧赢了时序。改为退出时等待各程序线程松开按住的键、写完结束记录（最多 1.5 秒）；线程写完记录后才从运行表移除。

### 其他验证

`cargo test --workspace`（704 个通过，含 7 个新的调度器测试与迁过来的提醒测试）、`cargo clippy --workspace --all-targets -D warnings`、`cargo fmt --check`、`pnpm check`、Playwright 全量 59 个。

### 未验证

- 弹幕与语音的实际显示 / 听感（只看了执行记录）；界面上的提醒列表在新调度器下的显示（数据接口没变，未打开界面看）。
- Kotone 崩溃（而不是正常退出）时按住的键：进程被杀时没有机会松键，系统里这个键会保持按下状态，直到用户再按一次。

## 第四阶段：定义与入参（2026-11-16）

### 实现

- `kotone-core::automation::library`：已保存的自动化（`~/.kotone/automations.json`，最多 200 个）：名字、说明、入参声明、动作（程序 / 播报）、创建者、时间。入参类型：数字（范围、整数）、文字（长度、不能换行）、布尔、选项、按键、区域、颜色；没有缺省值即必填。`validate_args` 在运行前校验并补上缺省值；保存时校验声明本身（参数名、范围、缺省值）。
- `kotone-script::run_with_args`：入参在程序里是只读全局表 `args`；区域就是 `{x, y, w, h}` 表，可直接交给 `brightness` / `ocr`。
- 调度器新增 `schedule_speak`（运行播报类自动化）；执行记录里的程序保存源码、入参与来源定义的快照。
- 工具：`automation.save / get / list / delete`；`automation.run` 可传 `id + args`；事件 `automation.saved` / `automation.deleted`。原 `automation.list` 已在第三阶段改名为 `automation.runs`，`automation.list` 现在列定义。

### 真机测试（同一隔离环境与测试窗口）

| # | 场景 | 结果 |
| --- | --- | --- |
| 1 | 保存带 4 个参数（按键、间隔、次数、区域）的程序 | 成功，`createdBy=agent`，未给 `label` 的参数补成参数名 |
| 2 | 保存语法错误的程序 / 缺省区域超出窗口的声明 | 都是 `INVALID_ARGUMENTS`，错误说明具体到参数 |
| 3 | 保存播报类自动化；`list` 与 `get` | `list` 不含源码，`get` 含源码 |
| 4 | 运行时缺必填参数、按键名不合法、多给参数、`source` 带 `args` | 都是 `INVALID_ARGUMENTS`，没有执行记录 |
| 5 | `args {times:3, key:"E", interval:200}` | 区域亮：按了 3 次 E，间隔 196 / 199 ms；执行记录带 `automationId` 与补全缺省值后的入参 |
| 6 | 把区域变黑后再运行 | 没有按键（程序走了 `log("dark")` 分支） |
| 7 | 运行播报类自动化（2 秒后） | `scheduled`，不需要目标窗口，到点开始投递 |
| 8 | 安排 4 秒后运行，随即删除定义 | 删除后再按 id 运行报“找不到自动化”；已安排的那次照常按了 2 次 H |
| 9 | 退出后看 `automations.json` | 剩下的定义还在，编号不复用 |

### 其他验证

`cargo test --workspace`（709 个通过）、`cargo clippy --workspace --all-targets -D warnings`、`cargo fmt --check`、`pnpm check`、Playwright 全量。

### 未验证与已知不足

- 播报类自动化的执行记录没有记下来源定义（`automationId` 只在程序动作里）；界面阶段若要按定义筛选执行记录再补。
- 第 7 项在投递中途退出了 Kotone，那条记录按规则在下次启动时记为失败；没有等它投递完成。
