# 2026-11-28 交接：Kotone 插件标准（ADR 047 第 1–4 期）

依据：[ADR 047](../adr/047-plugin-standard.md)（玩家 2026-11-27：“这两件事按你推荐的来”）和 [2026-11-27 调研](2026-11-27-dsh-and-game-plugins.md)。玩家把第 1–4 期交给一个自主会话、人离开电脑：允许推分支 `claude/plugin-standard`、手动跑远端 CI、在 `l1veIn` 下建一个公开仓库；不合并 main、不开 PR、不打 tag、不发版、不改版本号。主会话只做编排、核对、提交，代码由 7 个子代理写（标准 + 清单、CS2、原神、Steam 仓库、观察项、检查命令）。实现细节都记在 ADR 047 的“实现情况 / 实施记录”。

## 一、做了什么（玩家能看到的）

| 期 | 你能看到的 |
| --- | --- |
| 1 标准 + 清单 | 给作者看的 [`docs/plugin-standard.md`](../plugin-standard.md)（0.1 草案）。第三方插件的设置页多一节“Kotone 清单”（清单从哪来；给作者看的小字：不认识的字段、没用上的地方、标准比这一版新）；没有清单、也没有提示的插件不显示这一节。第三方插件也有“风险和许可”弹窗：原来设置页里的琥珀色“风险”框挪了进去 |
| 2 观察项 + 第三方事件 | 第三方插件设置页多一块“**事件**”：每个事件一行（说明、`<id>.<名字>`、出处：它给的 / Kotone 带的 / 你加的、“每 30 秒调一次 get_run_status，state 变成 success 时算”、状态、开关）；第一次打开要确认；能自己加、删自己加的；停了有“重试”，插件卡片上显示“事件“…”停了”。自动化的“当……的时候”能选到第三方事件（名字后标“第三方插件 <id>”）。粘贴绝区零一条龙的地址时用上 Kotone 带的预设、建议 id `zzz-onedragon`；模板“一条龙跑完推一行”。游戏中心多了“绝区零” |
| 3 两个内置插件 | 新卡片 **CS2**（默认开）：自动往 CS2 的 cfg 目录写 `gamestate_integration_kotone.cfg`（页面上能关、能看到写在哪）；这一局的比分、回合、我的数据；推送：下包倒数卡片、炸弹还剩 10 秒（一行 + 她说）、赛点、半场、比赛结束卡片。新卡片 **原神**（默认关，打开先弹“风险和许可”）：找到你的 BetterGI，列出一条龙配置、配置组、计划，能跑；跑完 / 出错一张卡片原地更新；Webhook 地址能复制或“一键填好”。游戏中心多了“原神” |
| 4 检查命令 + 老插件 | 没有界面变化：作者用的 `kotone-cli plugin check`；CI 里每次都跑的“老插件”测试 |
| 别的仓库 | **https://github.com/l1veIn/kotone-plugin-steam**（公开、MIT）：只读本机 Steam 的库、时长、成就，Rust 单个 exe（2.1 MB），不依赖 kotone 仓库；在 Kotone 里加 `mcp.json` 一行就能用，事件“玩了一个 Steam 游戏”“解锁了一个 Steam 成就” |

### 主会话替玩家定的（看得见的，不顺可以改）

- 清单放在服务器的 MCP 资源 `kotone://manifest` 里（不靠握手，新旧两版协议都拿得到）；你自己加的观察项写在 `<KOTONE_HOME>/plugins/<id>/manifest.json`。
- “事件”一块放在第三方插件的**设置页**里（有页面的插件，页面被它自己的 App 占满）。服务器给的、Kotone 带的观察项**默认关**，你打开并确认后才查；你自己加的加完即开。
- 观察项只在挂的游戏开着时查，**游戏退出后再查 3 分钟**；不挂游戏的插件（Steam）一直查，最快 60 秒一次。
- 第三方事件**只进自动化**：她看不到、不推送；要推由你的自动化 `push()`。
- 一条龙模板选了就是开着的；推的是固定的两句“绝区零一条龙跑完了”“绝区零一条龙出错了，去看看”，不拼它说的原话。
- CS2：默认开、cfg 默认写，**没有风险弹窗**（Valve 官方接口）；赛点只在第一次到赛点时说一次，只算竞技 / 优先 / 搭档；下包卡片自己倒数，不每秒刷新；默认端口 23730。地图用中文名（荒漠迷城……）。
- 原神：**默认关**；跑任务要原神配置的“允许自动化输入”开着；**BetterGI 开着时只能交计划**（它只认转发来的计划，一条龙和配置组按钮变灰并说明）；停止如实说“没有接口”，并写出 BetterGI 里的停止热键，不杀进程；“一键填好”只在 BetterGI 没开时可用，先备份。
- 原神、绝区零的游戏配置是新的内置配置：聊天键、延时是资料值，没在装了游戏的机器上核对；删了就不再装回（同 CS2）。
- Steam 插件：不放封面图（要联网），用首字母方块；成就名字跟 Steam 客户端的语言。

## 二、怎么验的

- **每个提交单独验过**：在 `target/tmp/wt-verify`（独立工作区、自己的 `CARGO_TARGET_DIR`）里把那一刻的索引做成临时提交，`clippy --workspace --all-targets --locked -D warnings` + svelte-check；CS2 和原神那两个提交共享登记文件，按内容拆开（CS2 的提交里没有原神的行，`Cargo.lock` 在工作区里单独生成），各自跑了相关 crate 的测试；第 2、4 期跑了整个工作区的测试。

| 提交 | 内容 | 单独验 |
| --- | --- | --- |
| f7730f35 | 第 1 期：标准文档、读清单 | clippy ✓、svelte-check 0 |
| 1eda9b50 | 第 3 期：CS2 | clippy ✓、svelte-check 0、kotone-tauri / cs2 / plugin-api / core 测试全过 |
| d7bef850 | 第 3 期：原神（顺手删 CS2 一条没用到的文字） | clippy ✓、svelte-check 0、相关测试全过、Playwright genshin + cs2 + terminology 过 |
| 38612438 | 第 2 期：观察项 | clippy ✓、svelte-check 0、fmt ✓、**工作区测试 2033 过 0 失败** |
| 63e01371 | 第 4 期：检查命令 + 老插件 | clippy ✓、fmt ✓、**工作区测试 2039 过 0 失败**（含 `old_plugins_still_work`） |
| （本提交） | ADR 047 实现情况、交接文档 | 只有文档 |

- **最后在本地照 CI 跑了一遍**（rustc 1.100.0，和 CI 一样）：fmt ✓、clippy `-D warnings` ✓、`test --workspace --locked` 2039 过 0 失败、`pnpm check` ✓（svelte-check + 英雄联盟 App）、**Playwright 286 过**、`pnpm audit` / `cargo audit` ✓、`release:verify` / `release:test` ✓。
- **远端 CI**（`gh workflow run ci.yml --ref claude/plugin-standard`）：1eda9b50 那次红（CS2 一条没用到的文字 → 下一个提交修了；另有 kotone-mcp `a_crash_at_startup_says_why` 一次 “pipe is being closed”、悬浮窗一个用例重试才过，都不是这一轮改的，下一次就绿了）；d7bef850 绿；38612438 绿；最后一次见第五节上面的结论。
- **真机脚本**（隔离的 `KOTONE_HOME`、假服务器 / 假数据、不发键）：

| 脚本 | 结果 | 测了什么 |
| --- | --- | --- |
| `scripts/manual-e2e/cs2_gsi_real.py` | 39/39 | 端口被占换端口并重写 cfg；令牌不对 403；阶段 菜单 → 对局前 → 对局中 → 结束；下包卡片原地更新成“炸了”、30.2 秒时推“还剩 10 秒”；半场、赛点、结束卡片；`cs2.state` / `cs2.rounds`；关掉写 cfg 只删我们的文件 |
| `scripts/manual-e2e/genshin_bgi_real.py` | 10/10 | 假 BetterGI 安装 + 假 exe：降权启动、参数恰好是 `startOneDragon 默认配置`；Webhook 开始 / 结束 → 卡片原地更新、`genshin.run_finished` 触发自动化；令牌不对 404；关掉“允许自动化输入”就不启动；停止回“没有接口” |
| `scripts/manual-e2e/remote_watch_real.py` | 27/27 | 假一条龙（随机端口）：用上预设、没打开不查、游戏没开等着、第一次记底、跑完 → 自动化用模板推一行、`game.state` 和对话文件里都没有这件事、游戏退出后宽限期里的“出错了”照样到 |
| `scripts/manual-e2e/bgi_copy_probe.py` | 只做了只读部分 | 玩家的 BetterGI 和原神都没在跑；exe 清单是 `asInvoker`，但 BetterGI 0.67 一起来会以管理员身份重开自己，这台机器管理员免提示——副本会悄悄变成管理员、结束不了，所以**没启动副本** |

- Steam 仓库：`cargo test` 31 过、clippy 干净；集成测试用两版协议连真 exe；在 ext-apps 的 basic-host 里用无头 Chrome 显示页面（亮 / 暗）；用 Kotone 自己的 `kotone-remote-plugin` 连上它，清单 `source: server`、没有不认识的字段；它自己的 GitHub CI 绿。`kotone-cli plugin check` 对它：✓ 13 ! 0 ✗ 0。
- BetterGI：只读看了 `D:\BetterGI`（`D:\software\BetterGI` 是指向它的链接），版本 0.67.0；没改里面任何文件，配置和日志没进仓库，测试样例都是合成的。

## 三、没验的 / 已知的

- **没碰真游戏**：CS2（自己打时有没有 `round.bomb` 和倒数、`map.round` 什么时候加、优先是不是报成 competitive、搭档加时规则、半场的 `intermission`；国服完美的路径）、原神 + 真 BetterGI（UAC、Webhook、跑完时原神还开着没有、开着时交计划）、真一条龙、真 Steam（成就缓存的格式最没把握，`LastPlayed` 多快写到磁盘）。
- **MCP Events（SEP-3415）没接**：rmcp 3.11 没有现成实现，照 ADR 不自己造；观察项先补上。
- **BetterGI 会自我提权**：Kotone 降权启动它也没用，默认 UAC 设置下每次弹一次。要么上游加参数，要么接受。
- 原神事件只在原神开着时进自动化（游戏中心的规则）：一条龙设成“跑完关游戏 / 关机”时可能丢。
- 观察项的记底只在内存里：Kotone 重开后第一次查又是记底（不会补发重开前错过的）。
- 第三方插件“风险和许可”看过只记一次：作者后来加了或改了 `risk`，不会再自己弹。
- Steam 插件没有 Release：玩家要自己 `cargo build --release`（ADR 不让这一轮发版）。
- kotone-cli 的 `rust-version` 还写着 1.77.2，实际要更新的（桌面端本来就这样），没动。
- 米游社没做。

## 四、要玩家本人测的（编号）

1. **CS2**：打一局竞技，看 cfg 自动写上（页面上能看到路径）、下包卡片倒数、“还剩 10 秒”、赛点、半场、结束卡片对不对；国服（完美）的话看 cfg 写没写对地方。
2. **原神 / BetterGI**：插件页打开原神，看它找没找到 `D:\BetterGI`、列出的一条龙配置和配置组对不对；“一键填好”Webhook（BetterGI 关着时）；跑一次真一条龙，看 UAC 弹不弹、卡片“正在启动 → 正在跑 → 跑完了”、跑完时原神还开着没有。
3. **BetterGI 开着时交一个计划**，看能不能转过去。
4. **以管理员身份跑** `python -X utf8 -I scripts/manual-e2e/bgi_copy_probe.py`：验副本的 `/api/v1/health`、日志 WebSocket、单实例转发、起停（脚本只运行 `target/bgi-copy` 里的副本，只传无害参数，只结束自己启动的进程）。
5. **绝区零一条龙**（装了的话）：插件里加 `http://127.0.0.1:23001/mcp`，打开两个事件、用模板建自动化，跑一次，看跑完 / 出错各推一行；顺便看绝区零的窗口标题和聊天键。
6. **Steam 插件**：`git clone https://github.com/l1veIn/kotone-plugin-steam && cargo build --release`，`mcp.json` 加一行，看库、时长、成就对不对，页面在 Kotone 插件页里的样子；开一个游戏，看“玩了一个 Steam 游戏”多久之后发生。
7. **原神、绝区零的聊天键**（游戏配置里的资料值）。
8. 看一眼第一节“主会话替玩家定的”，不顺的告诉我。

## 五、下一步

- **1.0 还差什么**（D6）：三种做法各有一个插件**真机**跑通——自己做（CS2，第 1 项）、驱动现成工具（原神，第 2–4 项；BetterGI 自我提权要先有说法）、远程 MCP + 观察项（一条龙，第 5 项）；别的仓库的插件（Steam）已经只靠外层协议跑通（第 6 项真机看一下）。标准文档、检查命令、老插件测试三样都有了。满足后冻结 1.0，老插件测试里加一份 1.0 的最小插件。
- **BetterGI 上游提议**（留给你决定要不要提；源码 0.67.0 核对过）：① 在本机外部访问服务里接出 `run_plan` / `list_runs` / `cancel_run`（`IPuloniaTaskService` 已经有）——有了停止才有接口；② 开着的实例也执行转发来的 `startOneDragon`、`--startGroups`；③ 一个关掉启动时自我提权的参数（如 `--no-elevate`），方便被别的工具以普通权限拉起。上游分支在大重构，建议先开 issue 问维护者。
- **要不要合并到 main**：这一轮没合并。分支在 main（4df7f309）之后 6 个提交，本地 CI 全绿。建议你测完第 1–2 项（CS2、原神）再合；合了以后出 rc 时 CHANGELOG 里加这一轮的条目（这一轮没碰 CHANGELOG 的版本节）。
- 之后可以做：MCP Events 等 rmcp 支持了改成订阅；英雄联盟插件按外层发成独立的 MCP 服务器（D1 说它也算外层）；云顶之弈（D6 里“自己做”的另一个候选）；Dota 2 跟 CS2 共用 GSI 接收。
