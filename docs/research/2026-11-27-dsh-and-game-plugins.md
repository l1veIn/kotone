# 兼容 DSH 插件、照知名游戏工具做插件：调研

日期：2026-11-27。状态：**调研完成**；插件标准的方向写进 [ADR 047](../adr/047-plugin-standard.md)（2026-11-27），扩充顺序和“对局里替玩家操作”的工具（第七节）还没定；没有改代码。相关：[ADR 038](../adr/038-kotone-and-apps.md)（插件 = MCP Apps）、[ADR 043](../adr/043-plugin-shape-and-pushes.md)（插件形态与推送；D11：第三方 MCP 插件只有 App 和工具，不能推送）、[ADR 045](../adr/045-automation-is-programs.md) / [ADR 046](../adr/046-programs-call-tools.md)（自动化的程序能 `push()`、能调工具）、[MCP 客户端调研](2026-11-24-mcp-client.md)、[英雄联盟对标](2026-11-25-lol-app-parity.md)。

玩家的话：

- “kotone现在有几个方向：尝试兼容dsh的插件，看看git上有哪些知名的游戏插件，参考它们做成MCP APPS，比如我们现在的英雄联盟插件就是参考的LeagueAkari”
- “思路有很多种，比如BetterGI适合做一层MCP封装。”

做法：三个子代理并行查 GitHub（`gh` API，星数、许可证、最近 push 都是 2026-11-27 当场取的），读 README 和关键源码，没有运行任何下载的代码。筛选照 Kotone 的规矩：数据只从游戏和客户端自己开放的接口、日志、网页接口、截图识别来，**不读内存、不注入、不改封包**；有封号风险的照做、写风险提示；插件只说事实，判断人、读局势归琴音。

## 一、结论

| 方向 | 结论 | 建议 |
| --- | --- | --- |
| 兼容 DSH 插件 | **做不到真兼容**：DSH 插件是跑在 DSH 自己 Node 进程里的 Cordis 插件，界面是注入 DSH 网页的 React 模块，接口约 120 个服务、100 个事件，还在 0.x 频繁变。所有能装 DSH 插件的桌面端都是把官方 DSH 整个嵌进去跑，没有一家重新实现 | 做两件小事：**“从 DSH 导入”**（只搬插件里的 MCP 服务器和技能，几天）；**反方向：Kotone 的插件也发成 DSH 插件**（英雄联盟先发，DSH 里英雄联盟几乎是空白） |
| 给现成工具包一层 MCP | **国内二游这批最合适**。头部工具大多已经留了给程序用的口子：绝区零一条龙自带完整 MCP；三月七有给 AI 用的 `m7a.exe`；MAA 有远程控制协议；阴阳师 OAS 有 HTTP；BetterGI 0.67 自带 MCP 骨架（只有健康和版本两个工具），启动靠命令行、完成靠 Webhook、状态读文件，**停止没有接口** | 现成工具管执行，Kotone 插件管页面、推送和米游社 / 森空岛数据（“两者结合”）。BetterGI 短期不改它代码，同时给上游提 PR 补 run / status / cancel |
| 自己做 MCP App（照英雄联盟） | 有**官方实时接口**的：CS2、Dota 2（Valve GSI）、火箭联盟（Stats API）；有**游戏日志**的：炉石、流放之路、塔科夫、Warframe、Minecraft、永劫无间；**和英雄联盟共用客户端**的：云顶之弈 | 云顶之弈、CS2（+ Dota 2 共用 GSI 接收）最先 |

GitHub 上游戏 MCP 服务器星数都只有 0–10（Minecraft 的 Mineflayer MCP 835 星是例外），带界面的游戏 MCP App 一个都没有（今天再搜只有一个井字棋玩具）。“把游戏伴侣工具做成 MCP App”这件事基本还没人做起来。

## 二、兼容 DSH 插件

依据 `deepseek-ai/deepseek-harness` @ `4b399c39`（2026-11-22，0.2.3-alpha.2，MIT，287,911★）和本地的 `dsh-game-communicate`。

### DSH 插件是什么

- 一个 npm 包，清单在 `package.json` 的 `dsh` 字段：`dsh.bundle.patch` 指向 `cordis.patch.yml`（往插件树里插哪几行），`dsh.client` 是浏览器那一半，`engines.dsh` 写兼容版本（`docs/user/develop/basic/publish.md`）。JS/TS，Node ≥ 24.9。
- 宿主那一半导出 `apply(ctx)`，所有扩展点都是 `ctx` 上的服务：工具、hooks、技能、MCP、系统提示词、模型适配器、子代理、定时任务、斜杠命令、界面插槽、主题（`docs/cookbook/extension-cookbook.md`）。
- 界面直接注入 DSH 网页的插槽，用宿主的 React，不在 iframe 里（`docs/subsystems/slots.md`）。**DSH 不渲染 MCP Apps**（源码里没有 `ui://`、`ext-apps`），面板和 `ui://` 对不上。
- 宿主代码在 DSH 进程里，权限完整，没有沙盒（`packages/boot/plugin-manager/README.md`）。
- 不稳定：0.2.x 的 18 个预发布版本里有 201 篇不兼容变更说明（`docs/upgrade-guide`）。
- 能脱离 DSH 用的只有两样：bundle 里的 `@deepseek-ai/dsh-mcp-client` 行（但常带 `!!js` 表达式和 `${credential:…}`，得跳过）和标准 `SKILL.md`。
- DSH 能被别的程序驱动：`dsh --profile acp`（ACP，只给自动化，不带界面）、`dsh --profile sdk`（stdio JSON-RPC）。DSH 自己不当 MCP 服务器。

### 生态

- 插件索引（awesome-dsh-plugin，25,759★，`dsh-plugin-catalog/index.json`）当场下载：**7,076 个**。界面 / 主题 / 会话 / 用量四类约 35%，天生绑在 DSH 网页上；描述里提到 MCP 的约 3.6%。
- 游戏相关的都很小，抽查 16 个：只有 1 个是纯技能，其余都用 `ctx.tools.register` 注册原生工具，大多带 React 面板，带不过来。

| 仓库 | ★ | 做什么 | 能搬过来吗 |
| --- | --- | --- | --- |
| [ZSeven-W/dsh-game-studio](https://github.com/ZSeven-W/dsh-game-studio) | 974 | 一句话做小游戏 | 同时发成 Claude Code 插件带 MCP（双包） |
| [SAOS-3624/dsh-see-now](https://github.com/SAOS-3624/dsh-see-now) | 785 | 实时看屏幕挑帧 | 否（原生工具） |
| [gychen-NJU/dsh-game-daily](https://github.com/gychen-NJU/dsh-game-daily) | 72 | 原神 / 崩 3 / 星铁日常：排班，驱动 BetterGI、一条龙、三月七，跑完汇报 | 否，但**是第三节最好的对标** |
| [taptap/instant-games-open-mcp](https://github.com/taptap/instant-games-open-mcp) | 66 | TapTap 小游戏 | 是：本体是 MCP 服务器 |
| [trueRISCOacnt/maa-dsh-skill](https://github.com/trueRISCOacnt/maa-dsh-skill) | 5 | MAA（调 maa-cli） | 只是技能（要 shell） |
| [jinghuashuiyue2004/dsh-plugin-lol-companion](https://github.com/jinghuashuiyue2004/dsh-plugin-lol-companion)、[NormalAorii/dsh-plugin-lol-mayhem](https://github.com/NormalAorii/dsh-plugin-lol-mayhem) | 1、1 | 英雄联盟陪玩、海斗查询 | 否 |
| [TindalosKorone/dsh-cheatengine](https://github.com/TindalosKorone/dsh-cheatengine) | 16 | 接 Cheat Engine 改内存 | 红线，不碰 |

### Kotone 的几种做法

| 做法 | 要做多少 | 玩家得到什么 | 问题 |
| --- | --- | --- | --- |
| (a) 带一个 DSH 当后台进程 | 中到大：带 Node 24 + pnpm + DSH，Rust 写 ACP 客户端，一直追 0.x | 只能把事交给 DSH 里的另一个 Agent，没有页面、推送、语音 | 第三方代码权限完整；Kotone 默认提权，必须降权；和 ADR 026 冲突 |
| (b) 自己实现 DSH 插件接口 | 不可行（接口太大还在变，还得内嵌 Node 和 React 插槽） | 永远是部分兼容 | 同上 |
| (c) **从 DSH 导入** | 小：`crates/kotone-remote-plugin/src/import.rs` 已经能从 Claude Desktop / VS Code 导入，加一个来源；读 profile 里的 `dsh-mcp-client` 行，跳过 `!!js`，Key 迁进凭据库；技能目录照搬 | 用过 DSH 的玩家能带过来 MCP 服务器和技能；面板、主题、桌宠带不过来 | 走现有的第三方 MCP 询问规则；要 shell 的技能只能给后台（ADR 030） |
| (d) **Kotone 插件也发成 DSH 插件** | 小：照 `dsh-game-communicate`（`dsh-mcp-client` 行 + 技能 + 设置面板），每个插件一个薄包 | 借 DSH 的生态拿曝光；DSH 里英雄联盟方向几乎空白 | 名字里不要冒用 “DeepSeek Harness” |

建议 (d) 为主、(c) 为辅，(a)(b) 不做。(d) 正是 ADR 038 和 MCP 客户端调研第五节定过的“英雄联盟插件对外发布”，多套一层 DSH 的壳。

## 三、给现成工具包一层 MCP（国内二游为主）

### 各工具的口子

| 工具 | ★ / 许可证 | 现有口子 | 不改它的代码能包吗 | 已有封装 |
| --- | --- | --- | --- | --- |
| [BetterGI](https://github.com/babalae/better-genshin-impact)（原神） | 18,359 / GPL-3.0 | 命令行、本机 HTTP / WebSocket 日志 / MCP（2 个工具）、Webhook、配置和运行记录文件、JS 脚本仓库（详见下） | 能，**停止除外** | [bettergi-mcp](https://github.com/gychen-NJU/bettergi-mcp)（0★，MIT，29 个工具，底下是 PowerShell 脚本） |
| [March7thAssistant](https://github.com/moesnow/March7thAssistant)（星铁） | 13,006 / GPL-3.0 | [`m7a.exe`](https://github.com/moesnow/March7thAssistant/blob/main/assets/agent/skills/march7th-assistant/SKILL.md)：`--status`、`--list --json`、`<任务> --detach --json`、`--follow`、`--control current pause/resume/stop/skip`，退出码各有含义，运行记录 JSON + 失败截图，随包带给 AI 的 Skill | **能，最省事** | [hsr-assistant-driver](https://github.com/hsr-assistant/hsr-assistant-driver)（5★，AGPL） |
| [ZenlessZoneZero-OneDragon](https://github.com/OneDragon-Anything/ZenlessZoneZero-OneDragon)（绝区零） | 8,185 / GPL-3.0 | **自带 MCP**（`127.0.0.1:23001/mcp`，22 个工具：一条龙、单项、状态、停止、截图、点击、按键……）和同样能力的 HTTP（[mcp.md](https://github.com/OneDragon-Anything/ZenlessZoneZero-OneDragon/blob/main/docs/develop/zzz/backend/mcp.md)）；事件推送在它的路线图上 | **直接连** | 自带 |
| [MAA](https://github.com/MaaAssistantArknights/MaaAssistantArknights)（明日方舟） | 24,809 / AGPL-3.0 | MaaCore 多语言接口、[maa-cli](https://github.com/MaaAssistantArknights/maa-cli)、[远程控制协议](https://github.com/MaaAssistantArknights/MaaAssistantArknights/blob/dev-v2/docs/zh-cn/protocol/remote-control-schema.md)（MAA 轮询 getTask、回报 reportStatus） | 能：Kotone 在本机开两个端点当“控制端” | [maa-arknights-mcp](https://github.com/Ichigo0820/maa-arknights-mcp)、[arknights-mcp](https://github.com/Yum0Rin/arknights-mcp)（都 0★） |
| [OnmyojiAutoScript](https://github.com/nearaura/OnmyojiAutoScript)（阴阳师） | 422 / GPL-3.0 | FastAPI：`/{script}/start`、`/stop`、`/state`、`/log`、改配置、`/ws/{script}` | 能 | 没有 |
| [StarRailCopilot](https://github.com/LmeSzinc/StarRailCopilot) / [ALAS](https://github.com/LmeSzinc/AzurLaneAutoScript) | 4,215 / 9,836，GPL-3.0 | PyWebIO 网页（不是 REST）、配置 JSON、日志、onepush 通知 | 勉强：起停进程、改配置、读日志、onepush 的 webhook 推完成 | — |
| [ok-wuthering-waves](https://github.com/ok-oldking/ok-wuthering-waves)（鸣潮） | 8,726 / AGPL-3.0 | 命令行 `-t <任务> -e`；ok-script 的 MCP（开发用，`run_code` 能跑任意 Python，不接） | 能（命令行） | — |
| [Path of Building](https://github.com/PathOfBuildingCommunity/PathOfBuilding)（流放之路） | 5,554 / MIT | `HeadlessWrapper.lua` 无界面跑计算 | 能 | [pob-mcp](https://github.com/ianderse/pob-mcp)（63★） |
| [ArchiSteamFarm](https://github.com/JustArchiNET/ArchiSteamFarm)（Steam） | 13,844 / Apache-2.0 | IPC REST（`127.0.0.1:1242`，Swagger） | 能 | — |
| [Mineflayer MCP](https://github.com/yuniko-software/minecraft-mcp-server)（Minecraft） | 835 / Apache-2.0 | 本来就是 MCP | 直接用 | 自带 |
| tosu（osu!）、OverlayPlugin（FF14）、HunterPie（怪猎）、HDT 的 HearthMirror（炉石） | — | 口子很好包 | **不接**：数据来自读内存，包它等于间接读内存 | — |

GPL / AGPL 的工具只隔着进程调（命令行、HTTP、MCP），不链接、不抄代码，和 ADR 030 对 BetterGI 的处理一样。

### BetterGI 细看（0.67.0，默认分支 `refactor/20260930`）

- **命令行**（[CommandLineOptions.cs](https://github.com/babalae/better-genshin-impact/blob/refactor/20260930/BetterGenshinImpact/Helpers/CommandLineOptions.cs)）：`startOneDragon [配置名]`、`--startGroups <组名…>`、`--pulonia-run <计划ID>`（新任务系统 Pulonia 里存好的计划）、`--instance`。源码里已经开着时再启动，参数经命名管道转给正在跑的那个（`InstanceService.DispatchActivation`），但 bettergi-mcp 的作者说会被吃掉——**要真机验证**。
- **本机外部访问服务**（[设计文档](https://github.com/babalae/better-genshin-impact/blob/refactor/20260930/Docs/design/external-access.md)，0.67 已带，默认关，端口 30648，只听本机）：`/api/v1/health`、`/version`（Bearer 令牌）；`/ws/v1/logs` 实时日志；`/mcp` 只有 `get_bgi_health`、`get_bgi_version`。文档原话“首个版本……不提供任务启动、停止等业务控制能力”。
- **Webhook**：一条龙、配置组、秘境、地脉、任务出错、JS 自定义等事件（[NotificationEvent.cs](https://github.com/babalae/better-genshin-impact/blob/refactor/20260930/BetterGenshinImpact/Service/Notification/Model/Enum/NotificationEvent.cs)），可带截图。
- **文件**：`User/config.json`、`User/ScriptGroup/`、`User/OneDragon/`、`User/KeyMouseScript`（键鼠录制）、`User/Pulonia/`（计划、`state.json` 里排队和正在跑的、`history/`）、`log/`。BetterGI 开着时不要写它的配置（会被它回写覆盖）。

不改代码，包一层能做：启动（命令行）、查状态（读 Pulonia 的 `state.json` / `history/` 或听日志）、列计划和脚本（读文件）、完成事件（Webhook 指向 Kotone，带截图）。**停止只能结束进程**。

给上游的最小 PR：[`IPuloniaTaskService`](https://github.com/babalae/better-genshin-impact/blob/refactor/20260930/BetterGenshinImpact/Pulonia/Services/IPuloniaTaskService.cs) 里 `EnqueueAsync`、`ListRunsAsync`、`GetRunAsync`、`CancelAsync`、`RunChanged` 都有了，在它的外部访问服务里接出 `list_plans`、`run_plan`、`list_runs`、`get_run`、`cancel_run` 和 `/ws/v1/runs`，估计几百行。它的分支正在大重构（比 main 领先 178 个提交），先开 issue 问维护者。

### 和 Kotone 规则的接法

- 第三方 MCP 插件（比如直接连绝区零一条龙）照 ADR 043 D11 **不能推送**。两种接法：
  1. **内置的游戏插件去驱动现成工具**（`plugins/genshin` 调 BetterGI 的命令行、收它的 Webhook）：是内置插件，能有页面和推送，米游社数据也放这里。原神、星铁、明日方舟走这条。
  2. **直接连它的 MCP + 一个自动化模板**：自动化的程序能调工具（ADR 046）、能 `push()`（ADR 045），模板轮询 `get_run_status`、跑完推一行。不用改 D11，零代码就能试，适合绝区零先验证。
- 驱动这些工具等于模拟键鼠：放在游戏的“允许自动化输入”开关后面；工具里底层的点击、按键（一条龙的 `click_game`、`key_tap`）不给她。
- 米游社（原神 / 星铁 / 绝区零的树脂、开拓力、电量、派遣）一次扫码，代码共用（像英雄联盟和云顶共用客户端代码），仍然一个游戏一个插件。参考 [mihoyo-mcp](https://github.com/AI1379/mihoyo-mcp)（扫码登录、便笺、告警去重）。抽卡记录读游戏网页缓存里的 authkey，不用代理抓包。
- 三月七的规矩照它的 SKILL.md：免 UAC 和首次免责声明只能玩家本人点；默认 `--after-finish None`（不然跑完会关机）。

## 四、自己做 MCP App 的游戏

| 游戏 | 对标 | 数据 | 插件长什么样（页面 / 推送 / 工具） | 要注意 |
| --- | --- | --- | --- | --- |
| **云顶之弈** | [TFT-OCR-BOT](https://github.com/jfd02/TFT-OCR-BOT)、[JinChanChanTool](https://github.com/XJYdemons/JinChanChanTool)（都 GPL，只学功能）；OP.GG 官方 MCP 的云顶工具 | LCU（和英雄联盟同一个客户端）、截图识别、OP.GG、CommunityDragon | 战绩、阵容 / 海克斯 / 装备；推海克斯三选一评级（和海斗三选一同一条路）、阶段提醒（2-1 / 3-2 / 4-2）；`tft.augment`、`tft.comps`、`tft.history` | 局内接口在云顶里能不能用**未核实**；Riot 云顶政策不许对局中显示“这一局的聚合统计”（如 8 个人最近名次）→ 照做就写进风险说明；LCU 代码抽成共用的 |
| **CS2** | [CS Demo Manager](https://github.com/akiver/cs-demo-manager)（2,042★ MIT，有命令行，支持 5E / FACEIT）、[demoparser](https://github.com/LaihoE/demoparser)（748★ MIT，Rust） | **官方 GSI**：游戏按 cfg 把 JSON POST 到本机；demo 录像 | 这一局（比分、回合、经济、击杀）、最近比赛和录像分析；推下包倒数 40 秒（剩 10 秒语音）、赛点、半场、结束卡片；`cs2.state`、`cs2.recent_matches`、`cs2.analyze_demo` | 一键写 `gamestate_integration_kotone.cfg`；自己打时 GSI 只给自己的数据，**做不了开局 10 人战绩**；国服（完美）cfg 路径未核实 |
| **Dota 2** | [OpenDota](https://github.com/odota/core)（MIT）、[frontys/dota2-mcp](https://github.com/frontys/dota2-mcp)（2★，GSI + OpenDota，计时清单可参考） | 官方 GSI、OpenDota | 这一局、战绩；推符文、莲花、昼夜、肉山复活窗口、不朽盾过期（数字随版本变，放带版本号的数据表） | 和 CS2 共用 GSI 接收；Valve 有意挡住选人时的玩家资料，不做对面开局战绩；dota2-mcp 的“小号雷达”是判断人，归琴音 |
| **炉石传说** | [HDT](https://github.com/HearthSim/Hearthstone-Deck-Tracker)（4,937★，**没有开源许可**，只学功能）、[python-hearthstone](https://github.com/HearthSim/python-hearthstone)（MIT） | 游戏日志 `Power.log`（改 `log.config` 打开） | 记牌器（剩余牌库、对手出过的牌）、套牌胜率；推对局开始 / 结束；`hs.state`、`hs.deck_remaining` | HDT 的收藏、竞技场选牌靠读内存，不做或改截图识别；国服日志路径未核实 |
| **流放之路 1/2** | PoB（包 MCP）、[Awakened PoE Trade](https://github.com/SnosMe/awakened-poe-trade)（MIT） | 游戏自带 Ctrl+C 复制物品文字、官方交易站、`Client.txt` 日志 | 查价、构筑计算、练级指南；推进新区域的练级提示、交易私聊、死亡 | APT 不支持腾讯国服，国服接口先核实 |
| **Warframe** | [WFInfo](https://github.com/WFCD/WFinfo)（Apache-2.0，README 说 DE 员工确认不封号） | `EE.log` 出现奖励 → 截图 → OCR → warframe.market | 推遗物四选一价格（和三选一同构）、虚空商人到了、平原入夜 | OCR 换成 Kotone 本地的 |
| **Steam**（平台） | [Watt Toolkit](https://github.com/BeyondDimension/SteamTools)、[Playnite](https://github.com/JosefNemec/Playnite)、ASF | 官方 Web API、本机库文件、`steam://` | 库、好友在玩什么、成就；推好友开始玩、成就解锁 | 国内访问 Steam API 可能不通 |
| 逃离塔科夫 | [TarkovMonitor](https://github.com/the-hideout/TarkovMonitor)、[tarkov.dev](https://github.com/the-hideout/tarkov-dev) | 游戏日志、截图文件名里的坐标 | 推匹配成功、开局、任务完成；物价 / 任务 / 地图 | — |
| 永劫无间 | [黑金古刀](https://github.com/ViewSuSu/BlackGoldAncientSword)（20★ MIT，“永劫版 Akari”） | 游戏写的 `Player.log`、官方战绩（微信扫码，非公开） | 对局状态、队友战绩 | 对标很小，战绩接口不公开 |
| 火箭联盟 | 官方 [Stats API](https://www.rocketleague.com/en/developer/stats-api)（v2.72 加了 WebSocket） | 官方本机接口 | 进球、扑救、结束卡片 | 技术最干净，国内玩家少 |
| Minecraft | Mineflayer MCP、HMCL | `logs/latest.log` | 推死亡、进度、聊天；“她进游戏陪你玩” | 国内多数玩网易版，不是同一个客户端 |

做不好的（官方不给、只能截图识别、或者只有赛后数据）：无畏契约（没有官方接口，Riot 正打击秒选和揭示隐藏名字，2026-09 一波封约 1 万人）、Apex、守望先锋、三角洲行动、PUBG（只有赛后）、命运 2（只有网页接口）、暗黑 4（截图识别）、魔兽世界、艾尔登法环。

## 五、红线以外的（不做）

| 做法 | 例子 | 替代 |
| --- | --- | --- |
| 读内存 | tosu / gosumemory（osu!）、ACT FF14 插件 / OverlayPlugin / cactbot、HunterPie、HDT 的 HearthMirror、Firestone 的 MindVision、deadlock-api-ingest | osu! 用官方 API v2 只做赛后；FF14 用 Lodestone、Universalis、截图识别；炉石用日志 |
| 注入 / 模组 / 改内存 | 胡桃 Remastered 的 UnlockerIsland、genshin-fps-unlock、BakkesMod、Dalamud、艾尔登法环帧率解锁、d4lf 的读屏 DLL | 火箭联盟改用官方 Stats API；其余没有替代 |
| 改封包 / 中间人代理 | Deceive（LoL / 无畏契约隐身）、抽卡导出的代理模式、ACT 抓包 | 抽卡改读 authkey |
| 外挂 | CS2-DMA、三角洲雷达和 OBS 锁、PUBG 地图挂、dsh-cheatengine | 不做 |

另外：搜结果里有大量刷星的外挂 / 恶意仓库（如 `mym4s4s1/dota-2-overlay` 194★），不当对标。Riot 从 2026-10-06 起默认拦未知第三方程序读 LoL / 云顶 / 无畏契约的内存（[公告](https://support.riotgames.com/en-us/riot/performance/game-memory-access-removed-for-third-party-apps)），Blitz、Porofessor 靠内存的功能会没，对只用接口的 Kotone 反而是机会。

## 六、建议的顺序

第一批（先把三条路各走通一次）：

1. **绝区零一条龙**：零代码，在现有 MCP 客户端里加 `http://127.0.0.1:23001/mcp`，再给一个“跑完推一行”的自动化模板。验证“外部游戏工具”这条路。
2. **云顶之弈**：和英雄联盟插件复用最多，国服玩家多，开源对标不成熟。第一步核实局内接口在云顶里能不能用。
3. **CS2**：官方 GSI，和英雄联盟插件同构；Dota 2 跟着共用 GSI 接收。
4. **DSH**：英雄联盟插件发一个 DSH 薄包；导入入口加“从 DSH 导入”。

第二批：原神（BetterGI 命令行 + Webhook + 文件，同时提上游 PR；米游社便笺）→ 星铁（`m7a.exe`）→ 明日方舟（MAA 远程控制 + 森空岛）→ 炉石（日志）→ Steam。

候补：流放之路（PoB 包 MCP 有现成例子，国服接口先核实）、Warframe、永劫无间、鸣潮。

## 七、要玩家定的

1. **对局里替玩家操作的**：压枪宏（[Soldier76](https://github.com/kiccer/Soldier76)，2,124★）、云顶 / 金铲铲自动拿牌、三角洲交易行抢购。不读内存，但在对局里替玩家动手、影响公平。建议不做，只推事实提示。
2. 第一批的顺序（上面第六节是建议）。

## 八、没核实的（要真机或再查）

- BetterGI 已经开着时命令行参数会不会转给它；要不要管理员权限；有没有默认的停止热键。
- 局内接口（Live Client Data）在云顶里能不能用。
- CS2 国服（完美世界）的 GSI cfg 路径和行为。
- 炉石、流放之路、Warframe 国服的日志路径；流放之路国服交易接口。
- 森空岛的具体字段；OAS 的端口；胡桃工具箱原仓库下架的原因。
- 插件市场“1–3 万个插件”的说法（当场下载的索引是 7,076 个）。

## 九、补：Coopanion / Cortico（玩家 2026-11-27 问到）

[Pal-AI-Lab/Coopanion](https://github.com/Pal-AI-Lab/Coopanion)：3,894★，2026-09-23 建，今天还在推；Electron 桌宠“Coo”（贴着屏幕底边走、按键说话、记忆、截图点鼠标、OBS 叠加、Steam 版）。应用 AGPL-3.0（0.1.10 及以前是 MIT），要签 CLA；底下的框架 [Pal-AI-Lab/Cortico](https://github.com/Pal-AI-Lab/Cortico)（385★）是 **MIT**。国内社区很活跃（QQ 群已经开到 6 群）。它和 Kotone 最像：都是“桌面陪伴 Agent + 插件”，它偏桌宠和生活，Kotone 偏游戏。

### 它的插件叫 World，形状和 Kotone 的插件几乎一样

Cortico 的 World（[docs/worlds.md](https://github.com/Pal-AI-Lab/Cortico/blob/main/docs/worlds.md)）= **工具**（`tools()`）+ **事件**（`pushEvent`，带投递时机：抢占 / 打断 / 清出 / 分批 / 搭车）+ **环境提示词**（`ENV_PROMPT.md`）+ **页面**（`webUi()`：状态灯、徽标、按 schema 画的配置、面板）。工具名带自家前缀、在清单里自描述工具和事件，和 Kotone 的 `plugin.toml` 是一个思路。

它还**专门写了给别的宿主挂 World 的兼容等级**（[docs/world-compatibility.md](https://github.com/Pal-AI-Lab/Cortico/blob/main/docs/world-compatibility.md)）：L1 只要工具和环境提示词；L2 加事件流（外部事件能唤醒一轮）；L3 加投递时机；L4 加输出流等实时能力。官方的英雄联盟以外大多数游戏 World 要 L2–L3。

和 DSH 一样的限制：World 是 npm 包，JS 在 Cortico 的 Node 进程里跑，权限完整；面板是贡献方自己打包的 JS（`mount(ctx)`），塞进 Cortico 的网页，不是 iframe，也不是 MCP Apps。所以 Kotone 要挂 World，得带一个 Node 子进程当“World 宿主”，面板带不过来（状态灯、配置这类按声明画的可以画）。

### 扩展市场里的游戏 World（[官方索引](https://github.com/Pal-AI-Lab/cortico-extensions) `catalog.json`，当场下载：406 个 World）

| World | 下载量 | 许可证 | 数据 | 备注 |
| --- | --- | --- | --- | --- |
| [炉石陪打](https://github.com/fateSSD/cortico-world-hearthstone) | 9,013 | MIT | `Power.log`（第一次替玩家打开） | 13 个工具、9 个事件；复盘、记最近几十局；判断写在 World 里（和我们的边界不同） |
| Minecraft（内置） | 9,126 | MIT | Mineflayer | 13 个工具 |
| [欧卡 / 美卡副驾](https://github.com/minekooooo/euro-truck2-coopanion) | 6,251 | MIT | 官方遥测（一键装 `scs-telemetry.dll`） | 31 个事件：事故、超速、罚款、收费站、过境 |
| 僵尸毁灭工程陪玩 | 5,321 | MIT | 未看 | 天黑催回家、被围提醒 |
| 战争雷霆 | 4,395 | MIT | 本机 8111 遥测 | 战局播报、对局翻译 |
| [刀塔战况](https://github.com/YearDouble/cortico-world-dota2-gsi) | 2,269 | MIT | 官方 GSI | 26 个事件；按固定规则提醒敌方全员消失 / 抱团 / 绕后 / 进肉山坑、塔和兵营、肉山、买活。**小地图那部分要核实是不是只用了自己能看到的** |
| [Steam](https://github.com/Unnori/cortico-world-steam) | 2,268 | MIT | 本机 Steam 记录 + 价格 | 在玩什么、时长、成就、史低 |
| 杀戮尖塔 1/2、植物大战僵尸、恶魔轮盘、胡闹厨房 2、星露谷（SMAPI）、雀魂、文明六、泰坦陨落 2、MSFS（SimConnect） | 0.5k–3.3k | 多为 MIT | 各不相同 | 很多是“她自己玩”；植物大战僵尸原版等可能读内存，没看 |
| 英雄联盟（cortico-world-lol） | 561 | **CC-BY-NC-4.0** | LCU + Live Client Data | 7 个工具、9 个事件；建于 2026-11-24 |
| 三国杀陪玩搭子、三角洲改枪台（只读公开网页）、小黑盒、KOOK 语音、MuMu 模拟器（adb） | 0.7k–6.8k | MIT / AGPL | 读屏、网页、adb | — |

另有一个 [cortico-world-dsh-bridge](https://www.npmjs.com/package/cortico-world-dsh-bridge)（795 下载）：在 Coo 里当“DSH 迷你宿主”，把 DSH 插件放进独立子进程，只取它们的工具。说明“只兼容 DSH 插件的工具”这一层有人做成了；它模拟了多少 `ctx`、能跑多少插件没核实（第二节的 (b) 仍然判断为全量不可行，“只要工具”的小子集可以再看）。

### 对 Kotone 意味着什么

1. **游戏插件的参考**：这批游戏 World 大多是 MIT，比 LeagueAkari（GPL）好用：规则和代码都能照着写（写明出处）。第四节漏掉的几个有官方遥测的游戏要补上：欧卡 / 美卡（SCS 遥测）、战争雷霆（8111）、MSFS（SimConnect）。下载量也说明了需求：炉石 9k、欧卡 6k、刀塔 2k，英雄联盟才 561（刚出三天）。
2. **兼容 World 比兼容 DSH 更顺**：它的契约本来就是“工具 + 事件 + 页面”，还写了兼容等级。做法是带一个 Node 子进程实现 L2（工具 + 事件），World 的工具变成 Kotone 的工具、事件给她。但有三处冲突：第三方 JS 权限完整（同 DSH）；第三方事件进她的上下文、能唤醒她，ADR 043 D11 和 MCP 客户端调研第 4.4 节现在都不给第三方；面板带不过来。
3. **反方向更便宜**：照 `dsh-game-communicate`，写一个薄 World 接 Kotone 的 MCP，把英雄联盟插件发到它的市场，借它的玩家群。
4. 它是最近的同类：桌宠、记忆、按键说话、截图点鼠标、OBS 叠加它都有，而且在 Steam 上发；Kotone 的不同在游戏本身（插件页面、推送、游戏里打字、语音陪玩）。

### 它缺的是自动化这一层（玩家 2026-11-27：“kotone多一个自动化，更偏向游戏治理”）

Cortico 里事件只有一个去处：模型。World 推事件 → 按投递时机进上下文 → 模型决定说什么、调什么工具。它对“事件怎么喂给模型”做得很细（五档投递时机；投递时才生成正文的 `pushDeferred`；投递时整批挑选的 `pushCandidate`；撤回、提级；`ephemeral` 用完即删）。但没有“不经模型的程序层”：刀塔 World 的“固定规则提醒”规则写在 World 里，出口按契约仍是模型（每条提醒一次模型调用、几秒延迟）；炉石 World 把“该解场还是该压”的判断也写进了 World。

Kotone 多的是：事件先给自动化的程序（毫秒级、不花 token、同样的输入同样的输出、玩家能看能关），程序再调函数（说一句、推送、发到游戏聊天、调工具、问她）；她只是消费者之一。

Coo 要补上这一层，光做一个“自动化 World”不够，按 `WorldHost`（`src/core/types.ts`，2026-11-27）要先在 Core 补口子：

- **调别的 World 的工具**：`WorldHost` 没有，工具只有模型按名字调。程序里的 `tool()`（ADR 046）做不到。
- **订阅别的 World 的事件**：只能读事件库（`store.range` / `grep`，轮询）或 `drainPendingEvents` 把待投递的事件取走（取走了模型就看不到）。能凑，不干净；要一个投递前的旁路。
- **不经模型的出口**：气泡归桌宠 World，`pet_say` 是给模型的工具（`packages/cortico-world-desktop-pet/README.md`），别的 World 没有“说一句固定的话 / 推一条”的标准路。
- **游戏输入的闸门**：替玩家按键时的输入锁、前台检查、Esc 停（ADR 032）；Coo 的 computer use 是模型驱动的。

补了这些，自动化的运行时、编辑页、模板倒很适合做成一个 World（Home Assistant 的样子：引擎和口子在核心，集成是插件；Kotone 也是这么定的，自动化在本体）。

反过来值得 Kotone 学的：**它的投递时机**。尤其 `preempt`（新信息让还没说出口的回答作废、带着新事件重答）在游戏里很有用：她正要说“对面打野在上路”，下一条事件说打野已经阵亡。我们现在有“她忙时压住、空了再记”（`when_free.rs`），没有“作废重答”。

## 十、插件扩充的几种做法（汇总，2026-11-27）

| 做法 | 例子 | 玩家拿到什么 | 成本 | 卡在哪 |
| --- | --- | --- | --- | --- |
| 1. 自己做（内置插件，照知名工具） | 英雄联盟 ← LeagueAkari；云顶、CS2 / 刀塔（GSI）、炉石（日志）、Steam；Cortico 的 MIT World 可以直接参考 | 页面、推送、工具、事件，全套 | 高：每个游戏一套，游戏更新要跟 | 每个都要真机验证 |
| 2. 内置插件驱动现成工具 | 原神 BetterGI、星铁 `m7a.exe`、明日方舟 MAA、阴阳师 OAS | 全套；执行交给那个工具 | 中 | 玩家要另装；它改版我们跟着改；GPL 只能隔着进程调；BetterGI 停止要上游 PR |
| 3. 直连第三方 MCP 服务器（已经能做） | 绝区零一条龙、Mineflayer、pob-mcp、OP.GG | 工具（它带 MCP App 的话还有页面） | 零代码 | 没有事件、不能推送（D11）；质量参差 |
| 4. 导入别的生态 | DSH（MCP 行 + 技能；或“只取工具”的迷你宿主）；Cortico World（Node 子进程实现 L2） | 以工具为主，面板带不过来 | 导入小；宿主中 | 第三方 JS 权限完整；要带 Node；World 的事件要进她的上下文 |
| 5. 自动化模板（玩家 / 社区写） | “一条龙跑完推一行”“进加载播报”；程序 `tool()` + `push()`（ADR 045 / 046） | 推送规则、把几个插件串起来 | 低 | 前提是插件有事件和工具 |
| 6. 反向发布（扩大影响，不扩充本体） | 英雄联盟插件单独发成 MCP 服务器 + App、DSH 薄包、Cortico 薄 World | 别的宿主的玩家用上我们的插件 | 低 | — |

底下有一个共同的问题：**第三方插件的事件**。3 和 4 缺的都是它。建议用 MCP 的 Events 扩展（SEP-3415，已经定稿，见 MCP 客户端调研第二节），而且第三方事件**只进自动化**、不直接进她的上下文：玩家在“当……的时候”里能选第三方插件的事件，要不要推、推什么由玩家的程序决定。这样 D11（推送不给第三方）不用改，“事件先过自动化”这层治理也守住了。

**给第三方 MCP 包一层“壳”，把读的工具变成事件**（玩家 2026-11-27：“大部分MCP本身获取信息的工具，所以是不是可以二次封装一个MCP壳？用来透传工具和产生事件”）。这是成熟的“轮询触发”：Home Assistant 的 REST / command_line 传感器（定时取值，状态变了就是一件事）和 Zapier / n8n 的轮询触发（定时取列表，按 id 去重，新条目才算）。Kotone 里不用另起进程：远程插件（`crates/kotone-remote-plugin`）本来就是这层壳，工具已经以 `<id>.<原名>` 透传；缺的只是事件那一半——给第三方的宿主里 `game_event` 现在是空函数（`apps/desktop/src-tauri/src/remote_plugins.rs`）。做法：

- **观察项** = 一个只读工具 + 参数 + 间隔 + 怎么比较 + 事件名，登记进它的清单 `[[events]]`，自动化里就能选。两种比较：**状态变了**（取结果里一个字段，变了才发，带旧值新值，可以写“变成某值时”）和**新条目**（结果是列表，按 id 去重，第一次只记底、不发）。
- 只在相关游戏开着时轮询（游戏中心知道）；间隔有下限，网页接口更长；失败退避，连续失败停下并显示在插件卡片上。
- 只轮询只读工具：注解不可信，第一次照现有规则问玩家；事件内容按不可信处理。
- 常见的服务器我们出预设（绝区零一条龙：`get_run_status` → 跑完 / 出错），玩家也能在插件页自己加。上游以后支持 MCP Events（SEP-3415）就改成直接订阅，事件名不变，自动化不用改。
- 限制：只对返回结构化结果（`structuredContent`）的工具，或我们写了预设的工具开放（纯文字里常带时间戳，每次都“变了”）；轮询是秒级的，局内实时的事还是插件自己做（GSI、日志）；重的“读”（截图分析）不轮询；网页接口轮询可能触发风控（米游社）。

建议的组合：先 3 + 5 验证（绝区零一条龙 + 模板），顺带 4 里最小的“从 DSH 导入”；主线是 1（云顶、CS2 / 刀塔）和 2（原神、星铁、明日方舟）；之后做“第三方事件进自动化”，再考虑兼容 Cortico World；6 随时可做。

调研时下载的资料在会话临时目录（`scratchpad\dsh`：DSH 源码、`catalog.json`、`game_hits.txt`；`scratchpad\games-cn\bettergi`：BetterGI 稀疏克隆；`scratchpad\games-global\readmes`；`scratchpad\coop\catalog.json`：Cortico 扩展索引），重启后可能被清掉；需要的结论都已经写进本文。
