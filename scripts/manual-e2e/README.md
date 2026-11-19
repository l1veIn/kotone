# 手动真机测试工具（自动化 / 内置 Agent）

2026-11-16 自动化各阶段真机测试用过的脚本，结果见 [自动化实现记录](../../docs/research/2026-11-16-companion-scripts.md)。只在隔离环境里用：不碰真实的 `~/.kotone`、凭据库、已安装的 Kotone 与 WebView2 用户数据目录。

| 文件 | 作用 |
| --- | --- |
| `target_window.ps1 -Dir <目录>` | WinForms 测试窗口“KotoneScriptTarget”：把按键、鼠标（按窗口比例的位置）、滚轮、焦点变化逐行写进 `<目录>/events.log`；左上角可变黑的面板、40 号字“找到对局”、橙色方块（中心蓝点）、“接受”标签。往 `<目录>/control.txt` 写 `dark` / `light` / `fg` / `blur <ms>` / `close` 控制它；启动完成写 `ready.txt` |
| `fg.ps1 [-Title]` | 把测试窗口切到前台（先敲一下 Alt 绕过前台锁定，再 `SetForegroundWindow`） |
| `uia.ps1` | 用 Windows UI 自动化操作 Kotone 设置窗口（WebView2 把无障碍树暴露给 UIA）：列出元素、点按钮、给输入框填值。经 `kotone.py ui` 调用，中文名用 base64 传，避免控制台编码问题 |
| `computer_use_e2e.py` | `game.look` / `game.act`（ADR 019）的真机测试：坐标点击、按编号点击、按键 / 组合键 / 滚轮 / 按住、相对移动、旧编号被拒、玩家按 Esc 停下（Esc 不传给游戏、别的键不停）并松开按住的键、琴音自己的 Esc 照常、被别的窗口盖住不点、窗口尺寸变了作废。隔离目录 `target/cu-e2e/`；`init` 写隔离配置，`setup-ocr` 在隔离目录里下载并启用文字识别模型。**前台不是测试窗口、也不是终端时直接退出**（2026-11-17 测试开始时前台是用户的英雄联盟对局，`fg.ps1` 的 Alt 进了游戏） |
| `kotone.py` | 对隔离实例调用工具（`call` / `runs` / `wait`）和 UI 自动化（`ui`） |
| `real_deepseek.py` | 用玩家真实的 DeepSeek Key 测试：从 Windows 凭据库（`kotone` / `connection:<id>`）只读进内存，经 `tools call … --args-file -` 的标准输入交给隔离实例（它有自己的凭据空间），**不打印、不写文件、不进命令行**；测完 `delete_connection` 删掉。拿不到时设环境变量 `KOTONE_REAL_DEEPSEEK_KEY` |
| `search_real.py` | 真实 DeepSeek：联网搜索（ADR 026）。三种思考强度各开一段对话问英雄联盟版本、海克斯大乱斗强势英雄、寒冰的强化，再在游戏内问一句蒙多出装；逐轮打印用时、搜了几次、token、费用和回答开头；检查 DeepSeek 起了标题、内容审核没拦、账本里有联网搜索 |
| `cache_real.py` | 真实 DeepSeek：聊天 5 轮 + 游戏内 4 次（同一段对话），逐个请求打印输入 / 命中 / 命中率 / 费用；再把压缩阈值调到刚好超过，看整理那一次和之后的命中率，以及整理后还记不记得早先的事 |
| `speech_preview_real.py` | 真实 DeepSeek 语音的「试听语音」，用 WASAPI 回环量扬声器的音量（`PLAYER_LIKE=1` 用玩家自己的语音设置） |
| `happy_path_real.py` | 真实 DeepSeek 的 happy path：只给 DeepSeek Key → 琴音读《Kotone 配置》技能把识别、文字识别换成 DeepSeek → 呼叫琴音说一句（VB-CABLE）→ DeepSeek 识别、DeepSeek 语音回答 → 测试窗口用 DeepSeek OCR 认字（切前台前先确认前台不是游戏） |
| `sessions_real.py` | 真实模型的同传子会话：派子会话、听 python.exe、翻译成弹幕、“停止同传”连同监听一起停（SenseVoice 模型用 `target/sess-e2e` 里下好的） |
| `voice_chat_real.py` | 真实 DeepSeek 的“跟队友说话”：先应玩家一声，再从虚拟麦克风说出来；录 CABLE Output（队友那边）和扬声器回环（玩家这边）的音量。不混玩家自己的麦克风 |
| `recap_real.py` | 对局复盘：把改名的 PING.EXE 当成“游戏”，游戏内说几句，结束进程，看聊天里的复盘和这个游戏的记忆（`KOTONE_RECAP_MIN_SECONDS=20`） |
| `roles_real.py` | 换角色：琴音 → 大肥鱼 → Claude大小姐 → 琴音，名字、唤醒词（从 VB-CABLE 喊）和应答的声音都跟着换 |
| `wake_eval.py` | 唤醒词实测（离线）：SAPI 和 DeepSeek 两种声音合成各个唤醒词，加几分钟不含唤醒词的普通说话，喂给和桌面端同一个唤醒检测器，统计喊中率和误唤醒 |

## 准备

```sh
# 1. 嵌入前端的调试构建
cd apps/desktop && pnpm build:web && cd ../..
cargo build -p kotone-tauri --features tauri/custom-protocol
# 2. 复制到仓库外（只在仓库带“低完整性”标签时需要）：带这个标签时，从仓库里启动的 exe 以低完整性运行，
#    发出的模拟输入会被系统静默丢弃。检查：icacls <仓库> 显示 Low Mandatory Level 即是；
#    2026-11-16 已改为 Medium（icacls <仓库> /setintegritylevel (OI)(CI)M），Codex 的沙盒可能会再加回来。
mkdir -p "$LOCALAPPDATA/Temp/kotone-automation-e2e" && cp target/debug/kotone.exe "$LOCALAPPDATA/Temp/kotone-automation-e2e/"
# 3. 隔离目录：开启工具服务、跳过首启向导
mkdir -p target/automation-e2e/home target/automation-e2e/webview target/automation-e2e/target
echo '{"toolService":{"enabled":true},"ui":{"firstRunCompleted":true}}' > target/automation-e2e/home/config.json
```

用后台方式启动（Claude Code 里用 `run_in_background`，不要用 `&`）：

```sh
KOTONE_HOME='<仓库>\target\automation-e2e\home' WEBVIEW2_USER_DATA_FOLDER='<仓库>\target\automation-e2e\webview' \
  "$LOCALAPPDATA/Temp/kotone-automation-e2e/kotone.exe"
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/manual-e2e/target_window.ps1 -Dir '<仓库>\target\automation-e2e\target'
```

## 例子

```sh
python -X utf8 -I scripts/manual-e2e/kotone.py ui invoke "暂不"          # 首次启动的“需要管理员权限”弹窗
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/manual-e2e/fg.ps1
python -X utf8 -I scripts/manual-e2e/kotone.py call automation.run '{"source":"click(0.5, 0.5)"}'
python -X utf8 -I scripts/manual-e2e/kotone.py runs
python -X utf8 -I scripts/manual-e2e/kotone.py ui invoke "自动化"
python -X utf8 -I scripts/manual-e2e/kotone.py ui names
python -X utf8 -I scripts/manual-e2e/kotone.py call app.quit
```

## 已知限制

- Kotone 的热键走低层键盘钩子，钩子按设计忽略模拟按键：**热键只能人工按**。
- 中文输入法会吞掉字母键的按下；测试窗口关闭了输入法。
- 隔离实例的设置窗口启动后会抢前台，后台启动的测试窗口拿不回前台，用 `fg.ps1`。
- UIA 按名字找元素：同名的文字和输入框都会匹配，`setvalue` 只挑输入框，`invoke` 只挑按钮；自定义下拉框（`Select` 组件）还没有封装。

## 用真实 DeepSeek 测试的规矩（2026-11-18）

- Key 只在内存里（`real_deepseek.py`），测完删掉隔离实例里的连接。
- 真机脚本一律禁止会按键的工具（`game_chat.send`、`game.act`、`automation.run`、`companion.type` 设为 deny）：
  2026-11-18 `voice_chat_real.py` 第一次跑时，琴音把“跟队友说我去拿龙了”理解成打字，字和回车进了前台（桌面）。
  之后 Kotone 自己也加了检查：前台不是当前游戏就不打字。
- 会出声的脚本（试听、跟队友说话、同传）开始前看一眼前台，玩家在游戏里就不要跑。

## 跨版本过渡

配置与游戏数据往返测试只编译探针、读写工作树 `target/upgrade-e2e/` 下的临时副本。它不运行安装器、不录音、不按键、不读取系统凭据。运行方式：

```powershell
python -X utf8 scripts/manual-e2e/upgrade_transition.py --old <0.1.13工作树> --maintenance <0.1.14维护工作树> --new <0.2.0工作树> --diagnostic <诊断包.zip>
```

可多次传 `--diagnostic`。完整的匿名样本用 `--sample` 指向含 `config.json`（可附 `profiles/`、`reminders.json`）的目录。诊断包只用于重建有限设置，不能替代完整样本。

在专用 Windows 虚拟机或测试账号，可以用前台测试文本框、已安装的虚拟输入和 Mock 识别器验证输入链。先切到桌面或终端，由脚本确认前台才发测试按键：

```powershell
python -X utf8 scripts/manual-e2e/upgrade_transition_native.py --exe <target目录里本地构建的0.2.0-kotone.exe>
```

此项验证注册热键后能把模拟识别文字输入测试框，以及相同前台收到 OEM3。真实语音模型、原安装包覆盖与物理低层钩子仍按下面的步骤测。

同机测启动时间与常驻内存时，用支持 `KOTONE_HOME` 的 0.1.13 隔离构建和 0.2.0 本地构建。要求 Node.js 24 和 Python psutil，安装器参数只用于读取包体大小。

```powershell
python scripts/manual-e2e/measure_upgrade_startup.py --baseline <隔离0.1.13的kotone.exe> --candidate <本地0.2.0的kotone.exe> --baseline-installer <隔离0.1.13的NSIS包> --candidate-installer <本地0.2.0的NSIS包>
```

发布前在专用 Windows 虚拟机或测试账号补做实际安装，不要在玩家账号启动 0.1.13 原包。这个版本不支持 `KOTONE_HOME`。

1. 核对 0.1.13 官方包和候选包的 SHA-256。安装 0.1.13，在测试账号导入匿名配置，核对录音键、已配置的英雄联盟适配、模型与待触发提醒。只用已确认前台的测试窗口，避免在真实对局内第一次尝试。
2. 经测试用的 0.1.14 更新源引导到 0.2.0，检查用户确认安装前的快照、安装包验签以及停量后不能开始新安装。备份内的相对路径、目录和文件哈希与复制前的应用数据比对。排除备份目录本身及另有位置的系统凭据。
3. 首开 0.2.0，点“继续只用语音输入法”，对比旧设置。按原录音热键说一句，检查文字到测试聊天框。在原来没有绑定该键的样本上，用物理键按 `~` 并检查前台窗口收到。检查待触发提醒按原时间和交付方式出现。再用另一份副本走“认识琴音”，检查只在选键后注册呼叫。
4. 从 0.1.14 的「关于」或 0.2.0 的完整更新日志取得 0.1.13，0.1.14 使用发布时核对过的测试包。退出后保留当前数据副本，再手动卸载与重装。若刚播过旧提醒，避开原到期点后两分钟的旧调度宽限。分别验证 0.1.14 的降音量偏好与 Dota 2 删除标记、0.1.13 的旧配置和语音输入。每次结果记录二进制哈希、测试样本编号、读写差异和实际检测到的失败。禁止使用玩家的正式安装目录。
