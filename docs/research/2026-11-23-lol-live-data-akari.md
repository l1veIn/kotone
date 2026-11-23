# 2026-11-23 英雄联盟对局数据调研：LeagueAkari、Live Client Data API、规则（ADR 035 的依据）

起因：玩家想说“琴音，帮我查一下这把游戏选手们的战绩，告诉我对面谁是突破口”“帮我记录一下对面的召唤师技能。谁没闪？帮我发在聊天框”，并且“可以添加一系列英雄联盟的工具”。这里记 2026-11-23 用 `gh`（读 LeagueAkari `dev` 分支源码）和网上资料核对过的事实；没核实的标“未核实”。

## 一、三种数据来源

| 来源 | 是什么 | 什么时候有 | 怎么连 | 给什么 |
| --- | --- | --- | --- | --- |
| **Live Client Data API** | Riot 官方的局内只读接口，`https://127.0.0.1:2999/liveclientdata/…` | 只在对局里（大厅、选人、结算连不上）；加载界面期间返回不全，社区用 `gamestats.gameTime > 0` 当“进游戏了” | 自签证书（根证书 [riotgames.pem](https://static.developer.riotgames.com/docs/lol/riotgames.pem)），不用令牌 | 10 个人的英雄、召唤师技能、等级、KDA、补刀、装备、是否阵亡和复活倒计时、队伍、riotId；游戏时间、模式；事件（击杀、塔、龙、男爵、团灭） |
| **LCU**（客户端本地接口） | 客户端 `LeagueClientUx.exe` 的本地 REST + WebSocket，Riot 不正式支持但长期可用 | 客户端开着就有 | 从进程命令行读 `--app-port`、`--remoting-auth-token`，Basic `riot:<token>` | 当前对局阶段、对局成员的 puuid（局内）、战绩、段位、熟练度 |
| **SGP**（服务器网关） | Riot / 腾讯服务器上的接口，客户端自己也在用 | 一直 | 用 LCU 拿令牌，直接请求服务器（国服 `*.lol.qq.com:21019`） | 战绩（可按模式筛、一次给全 10 人）、正在进行的对局 |

### Live Client Data API 的细节（[Riot 文档](https://developer.riotgames.com/docs/lol)、[示例](https://static.developer.riotgames.com/docs/lol/liveclientdata_sample.json)）

- 接口：`allgamedata`、`activeplayer`、`activeplayername`、`activeplayerabilities`、`activeplayerrunes`、`playerlist`、`playerscores?riotId=`、`playersummonerspells?riotId=`、`playermainrunes?riotId=`、`playeritems?riotId=`、`eventdata`、`gamestats`；接口说明在 `/swagger/v3/openapi.json`。
- `playerlist` 每人：championName、rawChampionName、isBot、isDead、respawnTimer、items、level、position、runes、scores（kills、deaths、assists、creepScore、wardScore）、skinID、summonerName、riotId、riotIdGameName、riotIdTagLine、summonerSpells、team。
  - **召唤师技能只有名字和说明，没有冷却**；别人的符文只有基石和两系（看不到星界洞悉）；没有 puuid（未核实，示例里没有），要用 riotId 和 LCU 对上。
  - `position` 文档里有，但有 2026 年的国服项目说拿不到分路（[n.e.k.o 插件](https://github.com/2510765660/n.e.k.o_plugin_neko_lol)），不能依赖。
- 事件：GameStart、MinionsSpawning、FirstBrick、FirstBlood、TurretKilled、InhibKilled、DragonKill、HeraldKill、BaronKill、ChampionKill、Multikill、Ace、GameEnd。按 EventID 只处理新的。
- 没有限流说明，工具一般 2 秒一次、失败退到 5 秒。观战时 `activePlayer` 返回错误对象。
- **国服**：2026 年的国服项目都说能用（n.e.k.o 插件“国服（WeGame）与国际服通用”，[lol-mate-stats](https://github.com/cjq02/lol-mate-stats)），没找到被封的报告。这台电脑上还没实测（要开一局 curl 一下）。

### LCU 的细节（读 LeagueAkari 源码）

- **端口和令牌**：Akari 不读 lockfile，每 2 秒解析 `LeagueClientUx.exe` 的命令行（`--app-port`、`--remoting-auth-token`、`--app-pid`、`--rso_platform_id`、`--region`），默认用 `NtQueryInformationProcess(ProcessCommandLineInformation)` + `PROCESS_QUERY_LIMITED_INFORMATION`，**不需要管理员**（[ux-command-line-parser.ts](https://github.com/LeagueAkari/LeagueAkari/blob/dev/src/main/shards/league-client-ux/ux-command-line-parser.ts)）。这台电脑的国服目录 `C:\WeGameApps\英雄联盟\LeagueClient\lockfile` 是 0 字节、日期停在 2024-07-03，**国服很可能不写 lockfile**，和 Akari 的做法一致（客户端开着时再确认）。
- **证书**：Akari HTTP 和 WebSocket 都不校验证书（`rejectUnauthorized: false`），超时 17.5 秒，重试 2 次。
- **对局阶段**：订阅 WebSocket `[5,"OnJsonApiEvent"]`，看 `/lol-gameflow/v1/session` 的 phase：`ChampSelect` 选人；`GameStart / InProgress / WaitingForStats / PreEndOfGame / EndOfGame / Reconnect` 算局内（[computed-state.ts](https://github.com/LeagueAkari/LeagueAkari/blob/dev/src/main/shards/ongoing-game/computed-state.ts)）。
- **10 个人**：局内用 `gameflow.session.gameData.teamOne / teamTwo`（puuid、`teamParticipantId`、`selectedPosition`）和 `playerChampionSelections`（技能）。Akari 在 `GameStart` 时故意不读，`InProgress`（大概是加载界面，未核实）开始读。
- **选人阶段**：排位选人里 Riot 隐藏了名字（`nameVisibilityType === 'HIDDEN'`），Akari 用一个**不开源的预编译 `.node`** 把混淆过的 puuid 还原（远程开关 `ongoing-game.deobfuscation`）。这就是 Riot 禁止的“反匿名”，Kotone 不做。大乱斗选人看不到对面，最早也是加载界面。
- **战绩接口**：`/lol-match-history/v1/products/lol/{puuid}/matches?begIndex&endIndex`，再按局 `/lol-match-history/v1/games/{id}` 拿全部 10 人；`/lol-summoner/v2/summoners/puuid/{puuid}`、`/lol-ranked/v1/ranked-stats/{puuid}`、`/lol-champion-mastery/v1/{puuid}/champion-mastery`。
- **SGP**：战绩 `GET /match-history-query/v1/products/lol/player/{puuid}/SUMMARY?startIndex&count&tag=q_{queueId}`（`Bearer` = LCU `/entitlements/v1/token` 的 accessToken），其他用 `/lol-league-session/v1/league-session-token`；国服主机 `hn1-k8s-sgp.lol.qq.com:21019` 等（[builtin.ts](https://github.com/LeagueAkari/LeagueAkari/blob/dev/src/main/shards/akari-api/builtin.ts)）。Akari 默认优先 SGP（能按模式筛、一次给全），LCU 兜底；每人 50 局，并发 2。
- **已知问题**：腾讯服务器晚上常失败（[#531](https://github.com/LeagueAkari/LeagueAkari/issues/531)，艾欧尼亚、黑色玫瑰最多）；加速器让 SGP 超时（#328）；国服局内能看到别人设成隐藏的战绩（#450），Riot 服务器按设计隐藏。

## 二、LeagueAkari 怎么算“谁强谁弱”（[analysis/player](https://github.com/LeagueAkari/LeagueAkari/tree/dev/src/shared/data-adapter/analysis/player)）

只算匹配到的 PvP 对局，跳过重开局。

- **每局分（满分 17）**，“占比”= 自己占全队的份额 ÷ (1/队伍人数)：
  - KDA：`clamp(√max((K+A)/D − 2, 0) · 3/7, 0, 1)`
  - 胜负：最多 1
  - 伤害占比 1 → 2：最多 3；承伤占比 1 → 2：最多 2
  - 治疗（÷ 全队平均承伤）0.2 → 1.4：最多 2
  - 分均补刀 5 → 10：最多 2；经济占比 1 → 1.5：最多 2
  - 参团率 0.3 → 1：最多 2；视野占比 1 → 2：最多 2
- **汇总**：KDA 和胜率用平均值，其他项平均每局分；≥ 5 局平均 ≥ 6.5 标“表现出色”，≥ 8 局 ≥ 8 标“非常出色”。
- **标签**：连胜 / 连败（从最近一局往前数，≥ 3）；高胜率（≥ 16 局 ≥ 85%）；容易被抓（只算召唤师峡谷非打野，前 15 分钟被对面打野参与击杀的平均次数，> 2 很容易、≥ 1.5 容易、< 1 难抓）；击杀 / 伤害效率（击杀份额 ÷ 伤害份额，> 1.35 高、< 0.65 低）；闪现键位可疑（D、F 都放过闪）；隐私（PRIVATE）；遇到过；玩家自己打的标签。
- **开黑**：局内直接用 `teamParticipantId` 分组；否则把 10 个人的历史对局连成图，两两同队 ≥ 5 局（可设）算一组（`team-up-calc.ts`）。
- **队伍**：平均胜率、平均 KDA、平均分、分数离散度，`BSI = 平均分 / (1 + 变异系数)`。
- 没有“第一次玩这个英雄”标签（可以从近期这个英雄的局数算）。旧版的“上等马 / 下等马”排名已经没了。

其他可参考的算法（许可证没查，只参考思路）：

- [hh-lol-prophet](https://github.com/real-web-world/hh-lol-prophet/blob/master/计分方式.md)：每人最近 20 局单双排 / 灵活 / 大乱斗，5 小时内的局权重 80%；从 100 分起加减（一血、多杀、参团、经济、伤害、视野在本局排名、分均补刀）；≥ 180 通天代、150 小代、125 上等马、105 中等马、95 下等马、更低牛马；在选人聊天里发出来。
- [RiftStats](https://github.com/Enoch9x9/RiftStats)（国服、偏大乱斗，2026-11）：每局 0–100，分均伤害 35%（3500 满分）、伤害占比 30%（10% → 0、30% → 满）、伤害 / 经济 20%（1.2 → 3.6）、存活 15%；最近 20 局 S ≥ 85、A ≥ 70、B ≥ 55、C ≥ 40。大乱斗没有分路、补刀、视野，这套比 Akari 的更合适。
- Porofessor：加载界面查 10 人最近 30 天的 5v5 匹配 / 排位，标签有本命、连胜连败、第一次玩、绝活、不是主位置（阈值不公开，[FAQ](https://porofessor.gg/faq)）。[Mattofessor](https://github.com/mattcui951/mattofessor)（开源、有中文）：绝活、强势英雄、胜率、开黑、疑似小号；大乱斗去掉分路、补刀、死亡相关标签。

## 三、召唤师技能

### Akari 的计时器

[SummonerSpellsCdTimer.vue](https://github.com/LeagueAkari/LeagueAkari/blob/dev/src/renderer/src-cd-timer-window/components/SummonerSpellsCdTimer.vue)：**只能手点**（看到对面交了就点那个技能），冷却 = 技能冷却 × 100 / (100 + 模式急速)，大乱斗 / 海克斯大乱斗 70、无限火力 300，不算鞋子、符文、海克斯。可以发“{英雄} {技能} {m}分{s}秒时就绪”到聊天。Akari 只用 Live Client Data 的 `gamestats`（4 秒一次）和 `playerlist`（自己的复活时间，1 秒一次），不自动识别对面交闪。

### 冷却怎么算（2026 年）

- 冷却 = 基础 × 100 / (100 + 召唤师技能急速)。闪现基础 300 秒（[wiki](https://wiki.leagueoflegends.com/en-us/Flash)）。
- 急速来源：星界洞悉 18；明朗之靴 10；深红明朗 20（V26.01 从 10 改成 20，只在峡谷）；大乱斗全图 +70（闪现 176.5 秒，带明朗之靴 166.7 秒）。
- 海克斯大乱斗：符文关掉、没有虚弱（[Riot 支持页，2026-10-21](https://support.riotgames.com/en-us/league-of-legends/events/league-of-legends-aram-mayhem-game-mode)）；海克斯“闪现 2”再 +70（125 秒）、“闪闪发光”3 层共 120 秒、“闪光弹”死了重置闪现。Live Client Data 看不到海克斯（未核实），所以算不准。

### 规则：**冷却计时器现在被限制了**

- Riot 25.17 版本说明（2025-08-26，“Game Info Tracking Improvements”）：要“restrict cooldown-tracking tools from being provided”，同时在游戏里加了自己的做法——记分板或死亡回放里点 15 秒内看到过的召唤师技能，会在聊天里发“X 使用了闪现”（[说明](https://www.leagueoflegends.com/en-us/news/game-updates/patch-25-17-notes/)）。Akari 的免责声明也引用这一节。
- Overwolf 的规则直接禁止“追踪敌方召唤师技能冷却，或用计时器帮玩家追踪”，没有“手动输入可以”的例外（[Overwolf](https://dev.overwolf.com/ow-native/guides/game-compliance/riot-games)）；之前的大招计时禁令也写明“自动或手动”都不行。OP.GG 2025-08 下架了技能计时（[Hexgate](https://hexgate.app/blog/riot-third-party-apps/)）。
- 同类工具怎么记（规则变之前）：聊天命令 `/n ashe f`、手机网页按键打进聊天框（[PasteLikeDoinb](https://github.com/Knkajfw/PasteLikeDoinb)）；聊天格式 “mid1230 sup1504”“cho 12:30 flash”“Vayne Flash 12”。

## 四、其他规则和风险

- **加载界面查战绩：允许**。Riot 12.22 起“进加载界面就能看到名字”，查战绩的应用只是不能在排位选人里显示（[Riot 帖子](https://devtrackers.gg/leagueoflegends/p/749a443a-apps-that-reveal-player-names-or-stats-in-champ-select-need-to-remove-these-informations-starting-patch-12-22)）。
- **不能反匿名**选人阶段的玩家；**不能做 MMR / ELO 一类替代官方段位的排名**（[通用政策，2025-05-29](https://developer.riotgames.com/policies/general)）。Overwolf 还禁止“按当前局势指挥玩家做什么”的提示，所以“突破口”要说成信息（“X 最近五连败、这个英雄第一次玩”），不说成命令。
- **国服**：
  - LeagueAkari 2025-07（v1.3.7）起声明“不再对腾讯游戏服务器提供任何形式的支持”，“在腾讯服务器环境中使用本软件极有可能违反腾讯游戏相关用户协议、管理规范或技术保护措施”；代码里仍有国服 SGP 主机。
  - 封号报告：Akari #370（艾欧尼亚“检测到第三方软件加载”两次，维护者说封号“无法完全避免”）、#149、#116；国际服 #523（自动接受 / 自动选英雄后永久封禁）。这些和 Akari 的哪项功能有关不清楚（它还有全局键盘钩子、覆盖窗口、模拟输入）。
  - 禁言：Akari #373，在国服发战绩被禁言 24 小时；维护者说腾讯的聊天过滤可能连单个字都拦，不建议在国服自动发。
  - 国服的战绩工具 Seraphine 2025-03-09“由于不可抗力”归档（[仓库](https://github.com/Zzaphkiel/Seraphine)）。
  - 腾讯 2015 年的《插件公约》（[新浪](https://dj.sina.com.cn/article/fxifmki9522661.shtml)）：对局中影响和敌方交互的功能是红线，影响和队友交互的要腾讯 + Riot 审核，对局外的功能是绿色。2024–2026 年没找到新的公告（未核实）。
  - ACE 2024-12 起有开机前启动的模式，会“提示或拦截不兼容或影响游戏安全的程序”（[17173](https://news.17173.com/content/12152024/230906266.shtml)）；没找到它对读本地接口的程序怎么处理（未核实）。
- **国际服 Vanguard**：Riot 说 LCU 和局内接口照常能用（[Vanguard FAQ](https://www.riotgames.com/en/DevRel/vanguard-faq)）；2026-10-06 起禁止未知程序读游戏内存，官方接口不受影响（[Riot 支持页](https://support.riotgames.com/en-us/riot/performance/game-memory-access-removed-for-third-party-apps)）。

## 五、许可证

- **LeagueAkari**：现在的 `dev` 和 v1.5.x 是 MIT（© 2026 Hanxven）。中间 v1.3.x–v1.4.3（2024-11-08 到 2026-02-13）是 GPL-3.0，不要从这些版本拷。`resources/magic/*.node` 是没有源码的二进制，不碰。最新正式版 v1.5.1（2026-07-25），`dev` 最后合并 2026-11-08，约 4.8k 星。
- hh-lol-prophet、RiftStats、Mattofessor：许可证没查，只参考思路。

## 六、实测（2026-11-23 晚，国服训练模式，`scripts/manual-e2e/lol_probe.py`，只读）

- **不提权读到了客户端命令行**：探测脚本没有管理员权限，用 `NtQueryInformationProcess` 读到了 `LeagueClientUx.exe` 的 `--app-port` 和令牌；`rso_platform_id=TJ100`、`region=TENCENT`、`locale=zh_CN`。
- **阶段**：Lobby → ChampSelect → InProgress；进 InProgress 后 2999 先连不上，接着返回 404，约 4 秒后返回 200（训练模式加载快）。
- **局内接口能用**：一共 26 个路径，每个 GET 都读过一遍，**任何地方都没有冷却字段**（只有 `respawnTimer`）。`championName` 是中文称号（艾瑞莉娅返回的是“刀锋舞者”），`rawChampionName` 是 `game_character_displayname_Irelia`；召唤师技能是中文名（“闪现”“幽灵疾步”）；`position` 是 `NONE`。
- **战绩**：自己最近 20 局第一次请求 1452ms，之后 23ms；单局详情 19ms，10 个人都在；**海克斯大乱斗的队列 id 是 2400**；训练模式的队列 id 是 3140。

## 七、在这台电脑上还要实测的

1. 国服对局里 `https://127.0.0.1:2999/liveclientdata/allgamedata` 能不能连、加载界面返回什么、英雄名是不是中文。
2. 不提权读 `LeagueClientUx.exe` 的命令行（Kotone 在英雄联盟里通常已经以管理员运行，因为国服游戏要管理员）。
3. 国服 LCU 的 `gameflow/session` 在加载界面能不能拿到 10 个人的 puuid；查别人战绩（每人 20 局）要多久。
4. 海克斯大乱斗的队列 id（只在战绩里按模式筛时用）。
