# 手动真机测试工具（自动化 / 内置 Agent）

2026-11-16 自动化各阶段真机测试用过的脚本，结果见 [自动化实现记录](../../docs/research/2026-11-16-companion-scripts.md)。只在隔离环境里用：不碰真实的 `~/.kotone`、凭据库、已安装的 Kotone 与 WebView2 用户数据目录。

| 文件 | 作用 |
| --- | --- |
| `target_window.ps1 -Dir <目录>` | WinForms 测试窗口“KotoneScriptTarget”：把按键、鼠标（按窗口比例的位置）、滚轮、焦点变化逐行写进 `<目录>/events.log`；左上角可变黑的面板、40 号字“找到对局”、橙色方块（中心蓝点）、“接受”标签。往 `<目录>/control.txt` 写 `dark` / `light` / `fg` / `blur <ms>` / `close` 控制它；启动完成写 `ready.txt` |
| `target_window.ps1 -Dir <目录> -Chat` | 同一个测试窗口的聊天模式（ADR 032）：按下时多记 Shift 状态和 `dwExtraInfo`（Kotone 的键是 `4B544E49`），不压掉字符消息，逐个 UTF-16 单元记 `CHAR` |
| `input_gate_e2e.py [exe]` | 发送窗口内挡键（ADR 032 决定 1）的真机测试：自己起隔离实例（`target/gate-e2e/`，`--background`）和聊天模式的测试窗口，用 `companion.type` 发消息，同时用不带标记的 `SendInput` 当“玩家”按键。对照：启动后第一条（不受保护）里字母混进了消息；之后发送期间连按 Q W E R，消息和原文完全一致；Kotone 第一个 Enter 之后按下并按住 Shift，两个 Enter 都不带 Shift、窗口没收到 Shift 的按下、最后一个 Enter 之后 Kotone 没再发任何键；过程日志里有 `inject_gate`。exe 默认是 `target/gate-e2e/bin/kotone.exe`（`target/debug/kotone.exe` 的副本，免得被当成玩家在跑 `pnpm dev`）。有游戏进程或前台不是终端 / 测试窗口时直接退出 |
| `voice_input_real.py [--exe 路径]` | 语音输入（热键 → 麦克风 → 识别 → 文字处理 → 打进聊天框）挪进插件前的行为基线，挪之前、挪之后各跑一遍。**会往测试窗口里打字，跑的时候别碰键盘鼠标（约 3 分钟）**。隔离实例 `target/voice-input-real/`（`--background`，exe 是 `target/debug/kotone.exe` 的副本，模型目录只读指向 `~/.kotone/models` 里的流式 x-asr），`hotkeyBackend: "register"` + 语音输入键 F22：低层钩子不认模拟按键，RegisterHotKey 认，所以 `SendInput` 按 F22 就是“玩家按了语音输入键”；`zh-game-3s.wav` 放进 VB-CABLE。查：对讲机按住说、松开直接发，测试窗口收到的正好是记录里的字（回车开框、回车发送）；录音笔预览时一个字都不打，`confirm_send`（悬浮窗“发送”）后才发；屏蔽词流程换成“打野→JG”的词库后发出去的字被换掉；每次都记进隔离目录的记录（sent），`history.list` 列得出；经调试端口订阅 `kotone://state`：listening → transcribing → processing →（preview →）sending → success → idle，悬浮窗出现又收起；她的 `game_chat.send`（脚本里的假模型调它，不用 Key）、外部经 `tools call game_chat.send` 和工具服务的 `companion.type` 打进测试窗口；声明了控制台通道的配置在没有游戏时退回键盘（真控制台通道要真游戏，SKIP）；关掉语音输入插件以后 `game_chat.send` 还在工具服务里（本体的，不带 `plugin`），外部调用和她（新对话）照样打进测试窗口，执行记录的发起方分别是 `tool-service` 和 `agent:<对话 id>`（ADR 039 D2）；`~/.kotone` 的配置、记录、模型文件没变。有游戏进程（含 Steam 客户端）或前台不是终端就不开始；每次要打字前把测试窗口切到前台并确认，会话中前台换了就停掉语音输入再退出 |
| `fg.ps1 [-Title]` | 把测试窗口切到前台（先敲一下 Alt 绕过前台锁定，再 `SetForegroundWindow`） |
| `uia.ps1` | 用 Windows UI 自动化操作 Kotone 设置窗口（WebView2 把无障碍树暴露给 UIA）：列出元素、点按钮、给输入框填值。经 `kotone.py ui` 调用，中文名用 base64 传，避免控制台编码问题 |
| `computer_use_e2e.py` | `game.look` / `game.act`（ADR 019）的真机测试：坐标点击、按编号点击、按键 / 组合键 / 滚轮 / 按住、相对移动、旧编号被拒、玩家按 Esc 停下（Esc 不传给游戏、别的键不停）并松开按住的键、琴音自己的 Esc 照常、被别的窗口盖住不点、窗口尺寸变了作废。隔离目录 `target/cu-e2e/`；`init` 写隔离配置，`setup-ocr` 在隔离目录里下载并启用文字识别模型。**前台不是测试窗口、也不是终端时直接退出**（2026-11-17 测试开始时前台是用户的英雄联盟对局，`fg.ps1` 的 Alt 进了游戏） |
| `kotone.py` | 对隔离实例调用工具（`call` / `runs` / `wait`）和 UI 自动化（`ui`） |
| `real_deepseek.py` | 用玩家真实的 DeepSeek Key 测试：从 Windows 凭据库（`kotone` / `connection:<id>`）只读进内存，经 `tools call … --args-file -` 的标准输入交给隔离实例（它有自己的凭据空间），**不打印、不写文件、不进命令行**；测完 `delete_connection` 删掉。拿不到时设环境变量 `KOTONE_REAL_DEEPSEEK_KEY` |
| `search_real.py` | 真实 DeepSeek：联网搜索（ADR 026）。三种思考强度各开一段对话问英雄联盟版本、海克斯大乱斗强势英雄、寒冰的强化，再在游戏内问一句蒙多出装；逐轮打印用时、搜了几次、token、费用和回答开头；检查 DeepSeek 起了标题、内容审核没拦、账本里有联网搜索 |
| `cache_real.py` | 真实 DeepSeek：聊天 5 轮 + 游戏内 4 次（同一段对话），逐个请求打印输入 / 命中 / 命中率 / 费用；再把压缩阈值调到刚好超过，看整理那一次和之后的命中率，以及整理后还记不记得早先的事 |
| `speech_preview_real.py` | 真实 DeepSeek 语音的「试听语音」，用 WASAPI 回环量扬声器的音量（`PLAYER_LIKE=1` 用玩家自己的语音设置） |
| `happy_path_real.py` | 真实 DeepSeek 的 happy path：只给 DeepSeek Key → 琴音读《Kotone 配置》技能把识别、文字识别换成 DeepSeek → 呼叫琴音说一句（VB-CABLE）→ DeepSeek 识别、DeepSeek 语音回答 → 测试窗口用 DeepSeek OCR 认字（切前台前先确认前台不是游戏） |
| `interpreter_e2e.py` + `mock_interpreter.py` | 同传（ADR 021）作为后台任务（ADR 029 第二期），假 AI、不花钱：经 WebView2 调试端口打字“帮我盯着队友”→ 主会话 `agent.start` 交给后台 → 后台开始听 python.exe → 另一个 Python 进程从扬声器放两遍英语句子（音量 0.3）→ 后台收到识别出的句子、用弹幕显示译文、做完第一轮仍在听 → “停止同传”用 `agent.stop` 连同监听一起停。SenseVoice 第一次运行时下载进隔离目录 `target/interp-e2e`。前台是游戏就停 |
| `voice_chat_real.py` | 真实 DeepSeek 的“跟队友说话”：先应玩家一声，再从虚拟麦克风说出来；录 CABLE Output（队友那边）和扬声器回环（玩家这边）的音量。不混玩家自己的麦克风 |
| `recap_real.py` | 对局复盘：把改名的 PING.EXE 当成“游戏”，游戏内说几句，结束进程，看聊天里的复盘和这个游戏的记忆（`KOTONE_RECAP_MIN_SECONDS=20`） |
| `roles_real.py` | 换角色：琴音 → 大肥鱼 → Claude大小姐 → 琴音，名字、唤醒词（从 VB-CABLE 喊）和应答的声音都跟着换 |
| `copilot_flow_real.py [new] [keep] [key]` | 真实 DeepSeek：ADR 028 的交互流程。新用户引导（一个 Key 换五项、呼叫键、聊天预填“你能做什么？”并打印她的回答）、在线识别不点启动就就绪、换成本地识别立即启动、聊天页呼叫按钮先启动再听、换回在线；0.1.x 配置“继续使用语音输入法”什么都不改、重开落在语音输入；0.1.x 配置填 Key 不动语音识别。经 WebView2 调试端口操作，不发系统按键；结束时连同卡住时删不掉的 Key 副本一起清理 |
| `voice_turn_real.py` | 真实 DeepSeek 的语音回合计时（ADR 029 第三期）：`--background` 启动、经 WebView2 调试端口点聊天页的呼叫按钮，玩家的话用 SAPI 合成后放进 VB-CABLE。呼叫 → 简单回答；不按键在接着听的 8 秒里接着说；搜版本（工具前一句 + 回答两路）；长回答开口 1.5 秒后再点呼叫打断（记录只留说出口的部分 +“（被打断）”、不接着听）；重启后等待短句从 `cache/tts` 读回、不再花钱。打印 `voice-timing` 表和每轮的语音合成费用。会出声（音量 8；`KOTONE_TEST_SPEECH_VOLUME=0` 不出声：流式那一路照样放、增益为 0，预先合成的短句跳过；`KOTONE_TEST_EXE` 用 `target/debug` 以外的一份 exe，跑的时候不挡别的编译），开始前和每句之前看前台，是游戏就停。2026-11-25 起认 ADR 033 的通话：“呼叫 →”的回合先挂断还开着的那一通，“接着听”看通话还开着 |
| `background_task_real.py` | 真实 DeepSeek 的“交给后台”（ADR 029 第二期）：`--background` 启动、经 WebView2 调试端口打字“查一下英雄联盟当前版本更新了什么，整理成三条”。查：主会话用 `agent.start` 交出去、4 秒窗口内返回 running、这一轮很快结束；卡片登记在主会话上、后台记录不进对话列表；后台做完自己交回（`agent-report`）并唤醒主会话写一句；卡片能看后台的步骤；后台的费用记到派它出去的那一轮。不出声、不发系统按键，前台是游戏就停。`--exec-only` 只跑执行命令、读写文件（ADR 030）三例：写 notes.txt 不问；搜过网页后照着跑命令先问、卡片上“允许”；删除先问、“拒绝”后文件还在，她说没删 |
| `kotone_issue_real.py` | 开发者自用技能 `scripts/dev-skills/kotone-issue` 的真机测试（复用 `background_task_real.py` 的启动和打字）：打字“帮我给 Kotone 提个 issue：…”。查：她读技能、交给后台；后台一条命令查齐版本、日志和 GitHub 上的同类 issue，草稿写进文件区，**没提交**；她把草稿给玩家看并问要不要提交，写草稿时一次都不用问；说“可以，提交吧”后正好建一个带 bug / enhancement 标签的 issue，她说出编号；草稿和 issue 里没有用户目录和 Key。用本机的 `gh` 登录，会在公开仓库建一个测试 issue，最后一定关掉。`--draft-only` 只写草稿、不建 issue |
| `music_real.py` | 放歌（ADR 034）的真机测试：真 app、真网易云、真 DeepSeek，隔离 KOTONE_HOME（网易云登录存在它自己的凭据服务里），音乐音量 12。没登录（默认，`--background`）：搜《孤勇者》几秒内有结果、陈奕迅原唱标成 VIP；音乐页显示扫码卡片、出真二维码、取消能收起；没登录点放歌提示扫码（D5）；打字“我要听孤勇者”她直接用 `music`（不交给后台）并请玩家扫码。`--login --reuse`：复用玩家在不开调试端口的隔离实例（`target/music-manual`）里手动登录过的网易云（开着调试端口时官网登录会被判“设备环境异常”），然后每日推荐、歌单、放 / 暂停 / 继续 / 这是什么歌、非 VIP 跳过原唱并说明，最后退出登录 |
| `lol_probe.py [--watch 秒]`（离线看三个工具会说什么：`cargo run -p kotone-plugin-lol --example render_probe -- <探测目录> <游戏资料目录>`） | 英雄联盟本机接口（ADR 035）的只读探测：不提权读 `LeagueClientUx.exe` 的命令行拿端口（令牌只在内存里）、对局阶段、这局 10 个人的隐私设置和战绩（每人要多久）、自己最近 20 局、局内接口（2999）列出的每个 GET 都读一遍找冷却字段。`--watch` 记录阶段变化和局内接口什么时候能连（看加载界面）。只发 GET、不按键；结果写到 `target/lol-probe/<时间>/`（里面有玩家名和 puuid，不提交） |
| `lol_tools_real.py [--probe 目录] [--gamedata 目录]` | 英雄联盟三个工具（ADR 035）的真模型测试，**不用开游戏**：一个 127.0.0.1 上的 HTTPS 小服务把 `lol_probe.py` 录好的一局（默认 `target/lol-probe/20261123-210745`，一局真人海斗）放出来，既当客户端接口（随机令牌、阶段 InProgress、这局的 10 个人和每人的战绩），又当局内接口（游戏时间接着录的时候往下走）。用 `KOTONE_LOL_LCU=<端口>:<令牌>`（不找客户端进程）和 `KOTONE_LOL_LIVE=https://127.0.0.1:<端口>`（只认本机地址）指过去，不会碰玩家真的客户端。其余同 `music_real.py`：`--background`、隔离 KOTONE_HOME（`target/lol-tools-real`）、真 DeepSeek、禁掉会按键的工具、经调试端口打字。查：后台查完 10 个人的战绩；“查一下这把的战绩，对面谁是突破口”用 `lol.scout`，结果里写了“英雄是随机的”、没有“没玩过”，她不贴“牛马”一类标签；“对面丽桑卓闪了”用 `lol.spell` 并说几点好；“谁没闪”说出丽桑卓；“发出去”她去调 `game_chat.send`（这里被禁掉），内容有丽桑卓和闪。录的数据里有真实玩家名，只在 `target/` 里，打印的东西只在本机看。有游戏在跑或在前台就不跑 |
| `game_events_real.py [--probe 目录] [--gamedata 目录]` | 游戏中心和按游戏事件运行的自动化（ADR 036 一期）的真机测试，**不用开游戏**：沿用 `lol_tools_real.py` 的假客户端，但按时间线放一局：大厅 → 8 秒选英雄 → 16 秒 InProgress（局内接口 404，加载）→ 22 秒有局内数据 → 40 秒对面拿了火龙 → 60 秒结算 → 70 秒大厅。先打开内置模板“英雄联盟进加载界面播报突破口”，再存一个“对面拿了{dragon_name}”的播报（`on` = 对面拿龙）。查：游戏中心的阶段（menu → pre_match → loading → in_match）；当前游戏配置自动切到英雄联盟（D7）；进加载界面时模板把话交给她，她用 `lol.scout` 并开口说；拿龙时播报“对面拿了火龙”（事件数据填进参数）；`game.state` 里有最近的事；结算时 `game.match_ended` 触发复盘（`KOTONE_RECAP_MIN_SECONDS=20`）。真 DeepSeek、隔离目录 `target/game-events-real`、不发任何系统按键，玩家对局时也能跑 |
| `auto_card_real.py [--probe 目录] [--gamedata 目录]` | 自动卡片和消息提示（ADR 039 D3、D4）的真机测试，**不用开游戏**：沿用 `game_events_real.py` 的假客户端和时间线，但不开任何自动化、复盘关掉，所以这局里自己发生的只有自动卡片。先打字“你好”让她答一句（这段对话就是卡片要去的地方）。查：选英雄时没有卡片；进加载后一张“开局战绩”（`ui://kotone/lol/match`）接在对话末尾、后面跟着给她的那句（来源 `auto-card`、按数据附上），前面的消息一个字节没变；消息提示的小窗出来了（置顶、`WS_EX_NOACTIVATE`）、前台窗口没变，只截小窗那一块到 `target/plugin-shots/round7/message-popup-real.png`（`KOTONE_POPUP_CAPTURABLE=1` 让它能被截到，平时不进截图）；前台是全屏程序时小窗按设计不弹，只报告；结算后一张“这局总结”（`ui://kotone/lol/game`，带 `justEnded`）；一局正好两张；发卡片前后 `usage.jsonl` 一条没多；再打字问“刚才英雄联盟发来的卡片说了什么”，她答得出来。真 DeepSeek（两次对话）、隔离目录 `target/auto-card-real`、不发系统按键、不点小窗。拿 `target/tmp/real.lock` |
| `background_voice_real.py` | 真实 DeepSeek 的“交给后台”语音路径（ADR 029 第二期 + ADR 030），沿用 `voice_turn_real.py` 的做法（`--background`、调试端口点呼叫、SAPI 句子放进 VB-CABLE、音量 8）：用嘴派写自动化的活，她当场一句、后台做完她自己开口说结果；用嘴让后台删文件，删除前她开口问、脚本用嘴答“可以”、删掉并开口说结果。打印她每句语音回答的字数和这一轮的费用。前台是游戏就停 |
| `slow_tool_voice_real.py` | 主会话里慢的工具转到后台（ADR 029 §一，MCP 方案第 1.5 期），沿用 `background_voice_real.py` 的做法和环境变量：调试版的测试钩子 `KOTONE_TEST_SLOW_TOOL=web.search:5000` 让搜索先等 5 秒，用嘴问“帮我搜一下英雄联盟现在是哪个版本”。查：3 秒时这一步转到后台（工具结果 `background: true`）、Kotone 念“交给我，好了告诉你”、这一轮在那一步结束（没再叫模型）、通话一直开着；搜索做完结果自己交回、只交回一次，她用 `speak` 说出版本；聊天里那一行变成后台任务卡片（“联网搜索 · 完成”），登记里一件 `tool = web.search` 的事。打印说完→出声、转走→做完→交回→她开口的时间。拿 `target/tmp/real.lock`，前台是游戏就停 |
| `wake_eval.py` | 唤醒词实测（离线）：SAPI 和 DeepSeek 两种声音合成各个唤醒词，加几分钟不含唤醒词的普通说话，喂给和桌面端同一个唤醒检测器，统计喊中率和误唤醒 |
| `mcp_names_real.py [--exe 路径] [--mcp 路径] [--keep]` | `kotone.exe mcp` 把带下划线的工具名还原对（MCP 方案第 0 期）：隔离实例（`target/mcp-names-real/`，`--background`，开工具服务和游戏语音插件，`includeMic: false`，exe 先复制到 `bin/`），经 stdio 走一遍真实的 MCP 对话：`initialize` → `tools/list`（Kotone 里名字带下划线的工具都以原来的 MCP 名列出）→ `tools/call kotone_voice_chat_status`（只读）→ 不存在的名字报错并说下一步；再开一个会话，不先列工具直接调。不调会打字、出声、改默认麦克风、装驱动的工具（脚本里拦着），不发系统按键；有游戏在跑或在前台就不跑；跑完删掉隔离目录。`--mcp` 选 MCP 那一端的 exe（默认和 `--exe` 同一个；只重编了命令行时用 `target/debug/kotone-cli.exe`，代码相同） |
| `mcp_app_resources_real.py [--exe 路径] [--offline] [--keep]` | `kotone.exe mcp` 提供英雄联盟的 MCP App（MCP 方案第 3、5 期，ADR 039 D5）：隔离实例（`target/mcp-app-real/`，`--background`，exe 先复制到 `bin/`），经 stdio：`initialize`（`resources` 和 `io.modelcontextprotocol/ui`）→ `resources/list`（做成 App 的五页：这局、战绩、单局详情、英雄、强度榜，CSP 声明 CommunityDragon；设置是宿主的页面，不列）→ 五页各 `resources/read` 一次（整页 HTML、页面标记；没打包时是占位页，只报告）→ 不是 App 的页面（设置页）-32002 → `tools/list` 的 `_meta.ui`（`kotone_lol_scout`、`player`、`game`、`champion`、`tier` 连到自己的页面，`kotone_lol_champions` 只给页面、不连设置页）→ 不带 `--offline` 时调一次 `kotone_lol_tier`（向腾讯 101 发一个公开请求，网络不通记 SKIP）。不发系统按键、不出声，不调操作客户端和打字的工具；有游戏在跑或在前台就不跑 |
| `mcp_app_real.py [--probe 目录] [--gamedata 目录] [--keep]` | 英雄联盟的 MCP App 在真 Kotone（WebView2）里显示（MCP 方案第 3 期）：先 `pnpm -C apps/desktop build:apps`、`build:web`，再 `cargo build -p kotone-tauri --features tauri/custom-protocol`；隔离实例（`target/mcp-app-real-ui/`，exe 先复制到 `bin/`，窗口会出现），经 WebView2 的调试端口（Playwright connectOverCDP，不发系统按键）：接一个假客户端（`lol_tools_real.py` 的录像回放，`KOTONE_LOL_LCU` / `KOTONE_LOL_LIVE`）；插件 → 英雄联盟整页是 App、从“这局”开始，是回环代理上的 iframe、握手完成、显示录像里的 10 个人；从 App 和沙盒代理页调 Kotone 的命令不通（含一个往 `kotone.log` 写探针的命令，对照主窗口）；不带密钥 404；再用 `ui.open` 在右侧栏单独打开战绩、单局详情、英雄、强度榜（聊天卡片、“这局总结”自动卡片走的是同一条路），每页不是“页面没有”、握手完成、内层是 `/app/lol/<页面>`、有数据（英雄、强度榜要向腾讯 101 发公开请求，网络不通只报告）；截图在 `target/plugin-shots/round7/real-*.png`。拿 `target/tmp/real.lock`；有游戏在跑或在前台就不跑 |
| `mcp_app_external_real.py [--exe 路径] [--fresh] [--keep] [--probe 目录]` | 英雄联盟的 MCP App 在别的 MCP 宿主里能用（MCP 方案第 5 期，交接第十一节第 15、16 条）：宿主是 ext-apps 的参考宿主 `examples/basic-host`（和规范一样的 AppBridge + 两层 iframe 沙盒，对 Kotone 一无所知），无头 Chrome 打开它；它经脚本里的小桥（Streamable HTTP ↔ stdio，消息原样转，会打字、出声、操作客户端的工具在桥上直接拒绝）连 `kotone.exe mcp`，后面是隔离实例（`target/mcp-app-external/`，`--background`，exe 先复制到 `bin/`）和 `lol_tools_real.py` 的假客户端。在 basic-host 里调 `kotone_lol_tier {mode: rift, lane: 中}`（向腾讯 101 发一个公开请求）和 `kotone_lol_scout {}`（假客户端），等 App 在沙盒里画出来，再点卡片上的“打开”。查：服务器说明带英雄联盟的风险提示（腾讯 / 拳头、LeagueAkari #523、操作客户端默认关）；两个工具连着自己的 App、basic-host 读到了资源（`text/html;profile=mcp-app`，CSP 声明 CommunityDragon）；强度榜和这局都画出来、有数据；强度榜显示的是调用要的模式（basic-host 不带 `toolInfo`，App 要等推来的结果，不能自己用缺省参数盖掉）；英雄头像从 CommunityDragon 来；“打开”请宿主换成 fullscreen、整页画出 10 个人；被拒的工具一个没调。截图在 `target/plugin-shots/round7/basic-host-*.png`（录像里有真玩家名，只留在本机）。前置：Node 20+ 和 npm（没有 `target/research/ext-apps` 时还要 git，浅克隆）、联网（第一次 npm install、腾讯 101、CommunityDragon）、本机 Chrome、先 `pnpm -C apps/desktop build:apps` 再 `cargo build -p kotone-tauri`、一份 `lol_probe.py` 的录像；basic-host 复制到 `target/research/basic-host-run`（ext-apps 锁 2.1.0）装一次、之后复用（`--fresh` 重装），8080 / 8081 端口要空着。不发系统按键、不出声。拿 `target/tmp/real.lock` 和 `target/tmp/pw.lock`（各等最多 30 分钟）；有游戏在跑或在前台就不跑 |
| `mcp_servers_real.py [--keep]` | 第三方 MCP 服务器当插件（MCP 方案第 2 期）：本机用 `npx` 起官方示例服务器 `@modelcontextprotocol/server-everything` 两次——一次 stdio（由 Kotone 起）、一次 Streamable HTTP（脚本起，`PORT` 随机）。隔离实例（`target/mcp-servers-real/`，`--background`，exe 先复制到 `bin/`），经 WebView2 调试端口调宿主命令：`mcp_servers_preview`（粘贴 Claude Desktop 的写法 / 一个裸的 `{"url"}`）→ `mcp_server_add`；两个都连上、工具名 `<id>.<工具>`、卡片上有服务器自己的名字和图标；`kotone.exe tools call system.capabilities` 里没有它们、外部调用被拒；她（脚本里的假模型）调 `everything.echo`、`everything-http.echo`，结果进对话，下一次发给模型时包在不可信数据块里（“含不可信内容”）；插件页面那条路（`plugin_call`）能调；关开关断开、`mcp_server_remove` 删干净（它的“总是允许”也清掉）、手改配置以后“总是允许”和信任都不留。审查后加的（2026-11-25）：开始前 `mcp.json` 里写一个 `lol` 键（和内置英雄联盟同名），它不算插件、`mcp_servers_status` 和插件页上说原因，内置英雄联盟照常（App 资源走内置的路、外部 Agent 不被当第三方拒），删掉这一项只动 `mcp.json`、英雄联盟的开关还在。权限：隔离配置里给这两个工具一条“允许”（= 玩家选了“总是允许这个工具”），询问规则本身是 `kotone-remote-plugin` 的单元测试。不发系统按键、不出声、不用 Key；第一次要联网下载 npm 包。拿 `target/tmp/real.lock`；有游戏在跑或在前台就不跑 |
| `mcp_remote_app_real.py [--keep]` | 别人的 MCP App 在真 Kotone（WebView2）里显示（MCP 方案第 2 期界面）：先 `pnpm -C apps/desktop build:web`、`cargo build -p kotone-tauri --features tauri/custom-protocol`；隔离实例（`target/mcp-remote-app-real/`，exe 先复制到 `bin/`，窗口会出现），经 WebView2 调试端口点界面（不发系统按键）：插件 → 加 MCP 服务器 → 粘贴 ext-apps 的官方示例 `@modelcontextprotocol/server-basic-vanillajs --stdio` → 信任提示（命令、包名、风险）→ 信任并添加 → 卡片在“第三方插件”里、连上了；点卡片：它的 App 在回环代理上的 iframe 里、握手完成、CSP 从 `default-src 'none'` 起；点 App 里的按钮，App 用服务器上的名字（`get-time`）调工具、经 Kotone 换成 `basic.get-time`、显示出结果；从 App 里调不了 Kotone 的命令。截图在 `target/plugin-shots/round7/mcp-remote-*.png`。第一次要联网下载 npm 包。拿 `target/tmp/real.lock`；有游戏在跑或在前台就不跑 |
| `background_focus_real.py [--normal \| --open] [--exe 路径] [--keep]` | `--background` 启动不抢焦点（2026-11-25）：隔离实例（`target/background-focus-real/`，exe 先复制到 `bin/`）。默认：记下前台窗口 → `--background` 启动 → 每 50 毫秒看一次前台，直到工具服务能连上再看 8 秒：前台从没变成 Kotone（含 WebView2 进程）的窗口、还是原来那个，设置窗口藏着、不是它线程的活动窗口。`--normal`：不带 `--background` 启动，设置窗口出现、成为前台、键盘焦点在页面（WebView）里。`--open`：先跑默认那些，再经 WebView2 调试端口调 `auto_card_open`（点消息提示；和托盘同一个 `show_and_focus`），查同样三条（要嵌入前端的构建）。不发按键、不点鼠标；`--normal` / `--open` 会把 Kotone 窗口拉到前面，只在前台是终端时跑、跑完关掉。拿 `target/tmp/real.lock`；有游戏在跑、游戏或 Kotone 在前台就不跑 |

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
