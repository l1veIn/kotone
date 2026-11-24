# 2026-11-25 英雄联盟 App 对标调研：LeagueAkari、AramHex

起因：[交接第十一节](2026-11-23-session-handoff.md)第一组第 2 条、[ADR 039](../adr/039-desktop-companion-and-game-apps.md) D5、D6、D3。玩家原话：

> “英雄联盟插件太简陋了……需要把这个插件当作一个独立的 MCP apps 来做，完成度对标应该是 leagueAkari 和 aramHex，使用 Claude、Codex 之类的也可以使用它。界面上，应该有自己独特的风格和审美，app 的可用性上也要对标这些成熟的项目，在能探测到游戏生命周期之后，什么时间给用户什么提示，给用户展示什么信息，都可以参考这些成熟的项目。这些项目里怎么展示英雄头像、玩家头像，怎么展示海克斯 logo 都需要复刻。”

拆成五条：功能对等、时机对等、素材对等、视觉原创、对外可用。这份文档回答前三条，并列出和我们现在的差距；视觉由 `docs/design/lol-app.md` 定，对外可用是第 12 条的事。

**怎么查的**（2026-11-25）：

- 用 `gh` 读 LeagueAkari 默认分支 `dev` 的源码和中文文案：窗口什么时候弹、页面放了什么、图从哪取；
- 读 AramHex（czgreat/aramhex）的源码、文档和更新记录；
- OP.GG Desktop、Blitz、U.GG 只看官网和商店页的公开说明；
- 素材地址都用 `curl` 实际请求过；
- 国服客户端的数据用 `target/lol-probe/`（2026-11-23 录的）对照。

没核实的标“未核实”。

**许可证**：

- **LeagueAkari**：GitHub 接口显示 `dev` 的 LICENSE 是 MIT（© 2026 Hanxven）。LICENSE 的提交历史：2024-11-08 改成 GPL-3.0（`a11ccc1a`），2026-02-13 改回 MIT（`9d3d52fa`）。第十一节写的是 GPL-3.0，照 ADR 039 D5 按更严的一边做：**只学功能、布局和时机，一行代码、文案、美术都不抄**。本文里的功能描述都是自己的话。
- **AramHex**：czgreat/aramhex 是原作，代码 MIT。第十一节写的 saon3r/aramhex 是它的分叉（MIT），做的是英文、欧服版：比原作多 95 个提交、少 12 个，换成 Windows 自带的文字识别，去掉了远程控制和统计。功能以原作为准。
- **aramhex-data**：CC BY-NC 4.0，要署名、不能商用。

---

## 一、两个项目各有什么

### 1. LeagueAkari：主窗口 4 页、4 个小窗、设置

Akari 连客户端的本机接口（LCU），一直开着，自己也有托盘。

**README 写的支持范围**：

- 不支持腾讯的服务器；
- 不用管理员；
- 有些快捷键、叠在游戏上的窗口要管理员。

| 页面 / 窗口 | 什么时候出现 | 有什么 |
| --- | --- | --- |
| 主窗口 · **战绩**（玩家标签页，可以开很多个） | 手动；打开时默认是自己 | 头部：召唤师头像、等级、名字#编号、隐藏战绩标记、自己打的标签。段位：单双排、灵活的段位图、胜点、胜负、历史最高。<br>汇总：近期胜率、KDA、常用英雄、分路。英雄熟练度、挑战、最近一起玩的人、遇到过的对局。<br>战绩列表（每页数量可调）：按模式筛；高级筛选能按英雄、装备、海克斯、符文、技能、分路、队友组合，还能连续往前翻页收集。<br>每一局一张卡片，展开后分“总览 / 详尽表格 / 符文 / 事件 / 构建 / 时间线”；能下载、看录像。<br>搜人：模糊、名字#编号精确、puuid；最近访问、置顶、好友列表；观战 |
| 主窗口 · **对局** | 选人、加载、对局中、结算都有内容。可设置“进选人或开局时自动切过来”，默认关。另一个设置“在房间里就查”默认开 | 两队（斗魂竞技场是多队），每个人一张固定宽度的卡片，见下一小节。<br>队伍表头：颜色点、队名、几项队伍标签（平均胜率、平均 KDA、几黑、胜率明显高或低的队）。<br>排序：按楼层、分路、开黑、胜率、KDA、综合分。<br>每人查 50 局战绩、20 局详情，同时查 2 个人 |
| 主窗口 · **自动操作** | 手动设置 | 自动接受、自动点赞、自动回房间、自动匹配、自动重连、自动处理邀请、预约邀请好友。<br>自动选 / 禁英雄、自动交换、按英雄自动配符文和召唤师技能。<br>自动回复、锁离线、登录时改签名和展示段位 |
| 主窗口 · **工具集** | 手动 | 往游戏聊天发预设文字（固定文字、模板、打野、开黑、评分，按快捷键发）。<br>房间工具、领奖励、战利品、好友工具、状态和签名、改展示段位、观战、修客户端窗口 |
| **小窗**（辅助窗口） | 默认开。房间、排队、确认时自动弹；选人时也弹，但“英雄数据”窗开着时它让位；其他阶段自动藏 | 房间和排队：自动匹配、自动接受的开关和倒计时（能取消、拒绝、手动接受）。<br>选人：备选席（大乱斗，带每个英雄的平衡性调整：伤害、承伤、治疗……的加减）、重随、选禁动作的时间线、自动选禁计划、秒退、换皮肤 |
| **英雄数据**窗 | 默认开，选人时自动弹 | 数据源 OP.GG 或腾讯 101（自动换）。<br>强度榜；英雄详情：胜率、选取率、禁用率、克制、出装（出门、鞋、核心、棱彩）、符文、加点、召唤师技能、搭配、海斗的海克斯。<br>一键把符文、召唤师技能、装备方案写进客户端（可设成自动）。<br>“选人助手”页：本局的英雄和选禁状态。<br>海斗总览：英雄排行、海克斯排行、英雄组合 |
| **对局浮窗**（叠在游戏上） | 默认关。打开后，在游戏里按住快捷键才出现（游戏在前台时），松开就藏；鼠标能点 | 和“对局”页同一套内容 |
| **技能计时**窗（叠在游戏上） | 默认关。打开后，在支持的模式里一进对局就自动出现 | 手点记对面的召唤师技能（倒计时或正计时），能发进聊天 |
| 侧栏、托盘、通知 | 一直 | 连了哪个客户端；复活倒计时，复活时提示一句；连不上客户端时提示怎么办；检测到直播软件时建议开主播模式（隐藏名字） |
| **设置** | 手动 | 语言、主题（多套配色）、数据源（腾讯 / 拳头服务器或客户端）、主播模式。<br>“对局”页：显示哪些标签、查多少局、开黑判定（两人同队 ≥ 5 局）、房间阶段要不要查、自动切页。<br>每个小窗的开关、自动弹出、透明度、快捷键、复位。<br>存的玩家和标签；首次启动的免责声明 |

#### “对局”页：每个人一张卡片放了什么（信息密度照这个学）

卡片固定宽、两队并排。从上到下：

1. **头部一行**：
   - 英雄头像（约 42 px 圆形、细边框），右下角压一块召唤师等级小牌；
   - 名字（粗体）加灰色的 `#编号`，开黑的人名字用小队的颜色；
   - 右边一个菜单：给这个人打标签。
2. **段位一行**：单双排、灵活各一个段位小图（约 14 px）加短文字；没定级的写“未定级”。鼠标移上去看完整段位表。
3. **数字一行**：
   - 近期胜率，带局数。≥ 53% 绿，≤ 47% 红；斗魂竞技场显示第一名率；
   - KDA，和全场比算四分位，高的绿、低的红；
   - 分路偏好。
4. **标签区**：
   - 开黑小队；
   - 连胜、连败；
   - 单杀；
   - 好抓、难抓：峡谷非打野，前 15 分钟被对面打野参与击杀的次数；
   - 表现出色；
   - 伤害、承伤、经济占全队的份额；
   - 分均补刀、伤转率（伤害 ÷ 金币）、视野分；
   - 发问号次数；
   - 击杀 / 伤害比：拿头多还是打工多；
   - 自己、遇到过、上局队友 / 对手、打过标签的；
   - 隐藏战绩、综合分、闪现键位不固定（D、F 都放过闪）。

   每个标签都能在设置里关掉，鼠标移上去有一句解释。
5. **常用英雄**：一排 20 px 英雄小头像；边框颜色表示胜率高低，熟练度高的加一颗星。鼠标移上去看这个英雄的近期战绩。
6. **最近战绩列表**（滚动）：每行约 34 px，按胜负着底色。行里有：
   - 英雄 24 px；
   - 模式名；
   - 日期；
   - 胜 / 负；
   - K/D/A。

   点一行在弹窗里看那局详情。
7. **开黑标记**：同一小队的人卡片描同一种颜色的边，右上角一个小色块；颜色按字母 A、B、C… 分配。

#### 战绩页里每一局（卡片总览）放了什么

- 英雄头像约 44 px 圆角方：赢蓝边、输红边，右下角压分路小图标；斗魂第一名头上一顶皇冠。
- 两个召唤师技能（20 px，竖排）、基石符文和副系（20 px，竖排）、海克斯（20 px，3 个一排）。
- K/D/A、伤害（带全队占比的条）、补刀和分均。
- 6 件装备加饰品（20 px）。
- 右边两队 10 个人的英雄小头像（16 px）和名字，点名字跳到那个人。

### 2. AramHex：托盘、主窗口 3 页、3 种叠在游戏上的东西

**只截屏识别**：

- 不连客户端和局内接口；
- 不读内存、不注入、不模拟按键；
- 游戏要设成无边框或窗口化。

只支持海克斯大乱斗。国服优先，繁中客户端直接能用，日服、美服、欧服在设置里选语言。

| 页面 / 浮层 | 什么时候出现 | 有什么 |
| --- | --- | --- |
| 托盘 | 一直；关主窗口时提示“还在后台” | 打开、退出、开机自启 |
| 主窗口 · **出装与加点** | 手动；选英雄时认出新英雄会自动切到这个英雄 | 搜英雄；分段（娱乐 / 中分 / 前 5%，没有分段数据时用全体，写明口径）；官方出装、鞋、出门装、技能加点，按胜率排序；拿到“命运之武器”一类给装备的海克斯时的装备参考 |
| 主窗口 · **海克斯选择** | 三选一时自动更新 | 三张卡的判断和依据；“重新识别”“重新校准卡片位置” |
| 主窗口 · **设置** | 手动 | 浮窗位置（左上 / 右上）、标签放卡片上方还是下方、显示不显示胜率、自动更新数据、开机自启、匿名统计；查自己的水平（输入名字#编号，向 aramhex.com 查区内前百分之几，有次数限制）；检查更新 |
| **选英雄标注**（盖在客户端上，点击穿透） | 选英雄界面认出来就贴 | 我方 5 人和顶部备选席每个头像旁边贴：评级字母（按国服胜率排名换算：前 10% S、30% A、60% B、85% C、其余 D，按评级着色）+ 胜率 + “第几 / 共几”。<br>备选席里比我当前英雄胜率高的标绿，开局“选择你的英雄”那 2–3 张卡里胜率最高的描金框 |
| **三选一标签**（盖在三张海克斯卡中间那块，点击穿透） | 认出三选一（或四选一）就贴，卡片没了就收 | 每张卡标题上方（或下方，可设）一个深色圆角块：<br>第一行判断，大字、着色：首选（绿）/ 可选（蓝）/ 可考虑刷新（红）/ 未知 / 无数据 / 识别中…；<br>第二行“+分数 · 胜率”；<br>第三行依据：“金色胜率第 3/40 · S 级 · 样本偏少”，新海克斯写测试服的局数；<br>给装备的海克斯另画一块装备提示（装备图标、名字、胜率）。<br>最好的那张边框粗一点。游戏切到后台时藏起来 |
| **角上的按钮和详情**（游戏右上或左上角） | 进对局（含加载）就显示，第一次选完海克斯 0.5 秒后藏（最多 5 分钟）；之后鼠标移到那个角附近才出现（48 px 按钮，外面多 24 px 感应区），移开 1.5 秒后藏 | 点开：英雄、推荐出装、加点、识别状态，鼠标离开 5 秒收起；背景能切透明 / 半透明 / 不透明。点完把焦点还给游戏 |

AramHex 的显示性能经验（Kotone 第 3 步已照做）：

- 浮层只盖要盖的那一块；
- 不盖满屏，也不跨屏；
- 识别线程用“低于正常”的优先级。

### 3. OP.GG Desktop、Blitz、U.GG：只看公开说明

| | OP.GG Desktop（官网游戏页，已读） | Blitz（官网 403，只看了搜索摘要，未核实） | U.GG（官网 403，只看了商店页和搜索摘要，未核实） |
| --- | --- | --- | --- |
| 选人 | 英雄分析（胜率、选取、禁用、克制、加点、符文、出装）、海克斯层级 | 选禁建议 | 看双方已选英雄的数据；按分路、对位自动导入符文和出装 |
| 加载 | 每个人的最近战绩，双方 AD / AP 比例 | 加载界面侦察：队友、对手、对位 | 没提 |
| 对局中 | 打野路线、野怪和水晶复活计时、出装、加点、实时对比、金币装备对比、治疗装统计、真眼数、海克斯层级；还有用语音控制麦克风和跟 AI 说话 | 海斗海克斯层级（选海克斯时）、实时表现对比、装备价值对比 | 加点、出装、对局中侦察 |
| 结算 | 没提 | 赛后回看（未核实是否英雄联盟） | 赛后分析：时间线、Carry 分；每局胜点 |

Porofessor 的主打也是加载界面查 10 个人，见 [2026-11-23 调研](2026-11-23-lol-live-data-akari.md)。**结论：加载界面是这类工具公认的“看对面”时机；选人是“看自己和队友、拿出装”的时机；赛后各家都弱。**

---

## 二、生命周期时机表

### 1. 对标项目在每个阶段做什么

阶段名是客户端接口 `/lol-gameflow/v1/gameflow-phase` 的值。Kotone 的游戏中心把它们归成 menu / queue / pre_match / loading / in_match / post_match（ADR 036）。

| 阶段 | LeagueAkari | AramHex | OP.GG / Blitz / U.GG |
| --- | --- | --- | --- |
| **客户端打开** | 自动连上（读命令行，不用管理员）；读不到时提示办法；主窗口默认打开自己的战绩 | 不连客户端；托盘常驻，靠截图判断在哪个界面 | 登录后载入自己的资料 |
| **大厅**（`Lobby`） | 小窗自动弹：自动匹配、自动接受的状态；默认查房间里所有人，“对局”页已经有队友 | 无 | — |
| **排队、确认**（`Matchmaking` / `ReadyCheck`） | 小窗：排队计时；自动接受倒计时（能取消、拒绝、手动接受） | 无 | — |
| **选人**（`ChampSelect`） | 英雄数据窗自动弹（我的英雄的出装符文，可一键或自动写进客户端；本局英雄）；“对局”页显示看得到名字的人（排位时 Akari 用不开源的组件还原隐藏名字）；按设置自动切到“对局”页；给打过标签的人在选人聊天里发提醒；自动选 / 禁 | 认出我方和备选席的英雄，在头像旁贴评级和胜率；备选席比我高的标绿；开局英雄卡最高的描金框；第一次认出新英雄时自动展开出装加点 | 自动导入符文出装；看双方已选英雄；海克斯层级 |
| **加载**（`GameStart` / `InProgress`，游戏时间还是 0） | “对局”页换成 10 个人，分好开黑小队；对局浮窗准备好；小窗藏起来 | 角上按钮一直显示，让玩家知道在哪 | OP.GG：每人最近战绩、双方 AD / AP；Blitz：加载侦察；Porofessor：查 10 人 |
| **对局中**（`InProgress`） | 按住快捷键看对局浮窗；技能计时窗自动出现（打开了的话），手点记、能发聊天；复活倒计时，复活时提示；按快捷键发预设文字 | 每次三选一给每张卡贴判断；角上按钮按需出现；游戏切到后台全藏 | 计时类、出装、加点、对比、海克斯层级 |
| **结算**（`WaitingForStats` / `PreEndOfGame` / `EndOfGame`） | “对局”页留着刚才那局；把其他 9 人记成“遇到过”；自动点赞；自动回房间、自动再排 | 浮层收起 | U.GG：赛后分析、胜点 |
| **客户端关闭** | 断开、小窗藏；界面关了但客户端还在时试着重连 | 继续在托盘 | — |

### 2. Kotone 现在和建议

| 阶段 | Kotone 现在（`plugins/lol`） | 建议今晚 | 以后 |
| --- | --- | --- | --- |
| 客户端打开 | 每 2 秒读一次阶段，报给游戏中心；界面没变化 | 不弹东西（照 AramHex 安静） | 插件页显示“已连上客户端” |
| 大厅 | 无 | 无 | 房间里就查队友（可选，Akari 默认开） |
| 排队、确认 | 自动接受（“操作客户端”开关，默认关） | 不变 | 自动匹配等，同一个开关下 |
| 选人 | 覆盖层：我方英雄的胜率和名次（只有文字）；自动选 / 禁；“这局”页说“进了加载界面才查得到” | “这局”页：我方看得到名字的人的战绩 + 双方英雄 + 分路 + 召唤师技能（差距表 G1）；覆盖层加评级（G8，有空再做）。**不发自动卡片、不弹消息提示**（见第五节） | 选人辅助：出装、符文导入 |
| 加载 | 后台查 10 个人（同时 2 个，每人 20 局）；“这局”页能看（只有文字）；琴音能答 | “这局”页带图（G11）；10 人查完（或进加载 20 秒）发一张“开局战绩”自动卡片 + 消息提示（游戏独占全屏时不弹，ADR 039 D4） | 对局中按快捷键看“这局” |
| 对局中 | 海斗三选一贴分数；琴音按你说的记技能；局内事件进游戏中心 | 三选一标签加“首选 / 可选 / 可考虑刷新”（G24，有空再做）；不发卡片 | 技能计时浮层、角上按钮 |
| 结算 | 技能记录清掉；“这局”页还留着上一局；游戏中心 `post_match` 触发复盘（叫模型，写进游戏记忆，界面上看不到） | 拿这局的结果，“单局详情”页加“刚结束”的头部（G29）；发一张“这局总结”自动卡片 + 消息提示 | 记下遇到过的人、自动点赞 / 回房间 |
| 客户端关闭 | 清状态 | 不变 | — |

---

## 三、素材：怎么展示、从哪取

### 1. 每类素材

“客户端开着”一列是本机客户端接口的路径，要带客户端的端口和令牌。

“客户端没开 / 别的宿主”一列：CommunityDragon 下面简写成 `CD/…`，完整地址是 `https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/…`。表里每条地址都在 2026-11-25 实际请求过，返回 200。

| 素材 | 对标怎么展示 → 建议 | id 从哪来 | 客户端开着 | 客户端没开 / 别的宿主 |
| --- | --- | --- | --- | --- |
| **英雄方形头像** | Akari：<br>· “对局”卡 42 px 圆形 + 等级小牌；<br>· 战绩 44 px 圆角方，胜蓝负红边，角上分路；<br>· 小处 16–24 px。<br>原图四周有一圈暗边，Akari 放大到 112% 裁掉。<br>**建议**：统一圆角方（和 Akari 的圆形区分开），行内 32–36 px、小处 20–24 px；同样裁暗边；角上可叠等级、分路、“我” | `championId`：选人 session、gameflow session、战绩、单局详情。<br>局内接口只给名字，要按 `rawChampionName` 换 id | `/lol-game-data/assets/v1/champion-icons/{id}.png`（128×128，约 25 KB） | `CD/v1/champion-icons/{id}.png`。<br>腾讯 `game.gtimg.cn/images/lol/act/img/champion/{英文名}.png`（120×120；客户端没连上时 Akari 用它） |
| **召唤师头像** | Akari 只在战绩页头部用大头像 + 等级；“对局”卡里放英雄头像，不放召唤师头像。<br>**建议**：战绩页头部 56 px **圆形**（只有这里用圆形），底部压等级；“这局”的行里不放 | `profileIconId`：`current-summoner`、按 puuid 查的召唤师、gameflow session 的 `teamOne/teamTwo[]`。<br>战绩里是 `participantIdentities[].player.profileIcon` | `/lol-game-data/assets/v1/profile-icons/{id}.jpg`（300×300 JPG） | `CD/v1/profile-icons/{id}.jpg`。录到的 22 个国服头像 id 全都有 |
| **段位** | Akari 把自己一套段位小图打进安装包，14 px + 文字；战绩页用大图。<br>**建议**：不带图进安装包。用客户端的小徽章 SVG，14–16 px + 短文字（“钻石 II · 45 点”）；拿不到图就显示带段位色的文字。<br>大图约 160 KB，列表里别用 | `/lol-ranked/v1/ranked-stats/{puuid}` 的 `queueMap.RANKED_SOLO_5x5`、`RANKED_FLEX_SR`：`tier`、`division`、`leaguePoints`、`wins`、`losses`、`highestTier` | 小徽章 `/fe/lol-static-assets/images/ranked-mini-crests/{段位小写}.svg`；<br>大图 `/fe/lol-shared-components/{段位小写}.png`。<br>这两条路径取自别的开源项目，**没在国服客户端实测** | `https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-static-assets/global/default/images/ranked-mini-crests/{段位}.svg`，0.9–2.3 KB，有 `unranked`。<br>大图在 `rcp-fe-lol-shared-components/global/default/{段位}.png` |
| **海克斯** | Akari：20 px 方块，1 px 边框按稀有度着色（银灰、金黄、棱彩紫色渐变），深色底（图标是透明线稿）；鼠标移上去看名字、稀有度、说明（中文说明来自腾讯）。<br>AramHex 在游戏里不画图标（卡片上本来就有），只在详情里放。<br>**建议**：行内 20–24 px，英雄页的列表 40 px；稀有度边框的颜色由设计文档定，名字一直放在 `title` 里 | 战绩 `playerAugment1–6`。<br>`cherry-augments.json`：`augmentSmallIconPath`（64×64）、`augmentLargeIconPath`（256×256）、`rarity`（`kSilver` / `kGold` / `kPrismatic` / `kEventChoice`） | 例：`/lol-game-data/assets/ASSETS/UX/Kiwi/Augments/Icons/QuestBangBang_small.png`。<br>海斗的在 `UX/Kiwi/`，和斗魂共用的在 `UX/Cherry/` | `CD/assets/ux/kiwi/augments/icons/questbangbang_small.png`（路径小写）。<br>腾讯 `game.gtimg.cn/images/lol/act/img/js/kiwi/kiwi_augments.json`：241 条，带 `small_Icon` / `large_Icon` 和中文名，Akari、AramHex 都用 |
| **装备** | Akari：20 px、小圆角，6 格 + 饰品，空格是灰块；鼠标移上去看名字、价格、合成 | 战绩 `item0–6`；局内接口 `items[].itemID` | `items.json` 的 `iconPath`，例：`/lol-game-data/assets/ASSETS/Items/Icons2D/1001_Class_T1_BootsofSpeed.png`（64×64） | `CD/assets/items/icons2d/1001_class_t1_bootsofspeed.png`；腾讯 `items.js` |
| **召唤师技能** | Akari：20 px，两枚竖排，放在英雄头像右边 | `spell1Id` / `spell2Id`：战绩、选人 session、gameflow 的 `playerChampionSelections` | `summoner-spells.json` 的 `iconPath`，例：`/lol-game-data/assets/DATA/Spells/Icons2D/Summoner_flash.png` | `CD/data/spells/icons2d/summoner_flash.png`；腾讯 `summonerskill_list.js` |
| **英雄技能**（Q W E R、被动） | 加点顺序一排 24 px，角上写键位 | `/lol-game-data/assets/v1/champions/{id}.json` 的 `spells[].abilityIconPath`、`passive.abilityIconPath` | 例：`/lol-game-data/assets/ASSETS/Characters/Ahri/HUD/Icons2D/Icons_Ahri_Q.png` | `CD/assets/characters/ahri/hud/icons2d/icons_ahri_q.png`；腾讯 `js/hero/{id}.js` |
| **符文** | Akari 战绩：基石 20 px + 副系 20 px，竖排在技能旁边 | 战绩 `perk0`（基石）、`perkPrimaryStyle`、`perkSubStyle` | `perks.json` / `perkstyles.json` 的 `iconPath`，例：`/lol-game-data/assets/v1/perk-images/Styles/7200_Domination.png` | `CD/v1/perk-images/styles/7200_domination.png` |
| **分路** | Akari 自己画的 SVG | `assignedPosition`（选人）、`selectedPosition`（gameflow） | — | **建议自己画**（视觉原创，也省请求） |

#### 路径怎么换成 CommunityDragon

2026-11-25 用上表十几个样本验证过。

- `/lol-game-data/assets/<后面>` → `https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/<后面全部小写>`
- `/fe/lol-static-assets/<后面>` → `https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-static-assets/global/default/<后面>`
- `/fe/lol-shared-components/<后面>` → `https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-shared-components/global/default/<后面>`

说明：

- `latest` 跟最新版本，2026-11-25 是 16.23（`content-metadata.json`）；也能写死版本号（`/16.23/` 实测可用）。
- 中文的 JSON 在 `global/zh_cn/v1/…`；图片都在 `global/default/…`。

### 2. 国服客户端有没有差别

对照的是 2026-11-23 录的国服客户端（16.23，`target/lol-probe/gamedata/`）。

- **路径写法一样**：`champion-summary.json` 的 `squarePortraitPath`，`items.json`、`perks.json`、`summoner-spells.json` 的 `iconPath`，和 CommunityDragon 的写法一致。
- **海克斯完全一致**：
  - 国服客户端的 `cherry-augments.json` 600 条，CommunityDragon `latest` 也是 600 条，id 和图标路径一条不差；
  - 腾讯 `kiwi_augments.json` 的 241 条都在里面；
  - 600 条里 `ARAM_` 开头的（海斗）171 条：图在 `UX/Cherry/` 的 121 条，`UX/Kiwi/` 的 48 条，另 2 条在 `Particles/`。
- **名字**：国服客户端给中文；CommunityDragon 的中文在 `zh_cn`。我们已经用腾讯的 `hero_list.js`、`items.js` 等补名字（第 3 步）。
- **没实测的**：
  - 国服客户端的**图片**接口本身。录的时候只存了 JSON，假客户端也只回 JSON（`scripts/manual-e2e/lol_tools_real.py`）。要玩家开客户端时确认 `/lol-game-data/assets/…png` 和 `/fe/lol-static-assets/…svg` 能取到。
  - 国服独有的头像、比全球早上的海克斯，CommunityDragon 可能还没有。这时显示占位；以后用腾讯兜底。
- **速度**（本机 2026-11-25）：
  - 腾讯 CDN 0.07–0.1 秒；
  - CommunityDragon 0.6–1.4 秒；
  - Data Dragon 0.6–1.8 秒。

  有磁盘缓存时只有第一次慢。
- **跨域**：两边都回 `Access-Control-Allow-Origin: *`。CommunityDragon `Cache-Control: max-age=3600`，腾讯 `max-age=600`。

### 3. 对标项目怎么取、怎么缓存

- **LeagueAkari**：
  - 页面里的图都是客户端路径。经它自己注册的 `akari://league-client/…` 协议转给本机客户端，带凭据、流式返回。这条代理**什么方法、什么路径都转**。
  - 客户端没连上时：
    - 大部分图不显示（灰色占位）；
    - 英雄头像改用腾讯 CDN 按英文名取；
    - 海克斯名字和图标改用腾讯 `kiwi_augments.json`。
  - 腾讯的英雄列表、海克斯表每 3 小时刷新一次，OP.GG 的大乱斗平衡数据每 30 分钟一次，都放在内存里。
  - 没找到专门的图片磁盘缓存，靠浏览器自己的缓存（客户端回的缓存头未核实）。
  - 段位图打包在安装包里。
- **AramHex**：
  - 图标全从腾讯 CDN 取，按需下载到数据目录，每个图一个文件；
  - 14 天后有网时重新取；失败的 10 分钟内不重试；
  - 界面先显示文字，图到了再刷新；
  - 安装包里带了一份装备、技能小图（base64），第一次打开没网也有图。

### 4. 建议的做法

这一小节回答 ADR 039 “细节由实现时定”里的三件事：页面怎么拿到客户端的图、磁盘缓存多大、CSP 声明哪些域名。

1. **数据里只放 id 和客户端资源路径**（例如 `/lol-game-data/assets/v1/champion-icons/103.png`），不放完整网址，也不放图片本身。页面数据小，也不用关心在哪个宿主里显示。
2. **页面里一个取图函数**：
   - 宿主给了 `assetBase`（Kotone 自己显示时），就用 `assetBase + 路径`；
   - 没给（Claude、Codex、basic-host），就按上面的规则换成 CommunityDragon 地址；
   - 图出不来时显示占位块（名字的第一个字）；名字一直在 `title` 里。图挂了，页面照样能用。
3. **Kotone 里的取图**：
   - 在本机回环代理上开一条只读路由（MCP 方案第 3 期本来就有这个代理），由英雄联盟插件提供取图函数，宿主只转发，本体里不写英雄联盟的东西。
   - 顺序：磁盘缓存 → 客户端（开着时，带本机凭据）→ CommunityDragon → 占位。
   - 只放行三个前缀：`/lol-game-data/assets/`、`/fe/lol-static-assets/`、`/fe/lol-shared-components/`；只 GET；只回图片类型（png / jpg / svg / webp）。
   - **不能像 Akari 那样把整个客户端接口转给页面**：页面在沙盒里，转了等于让页面能操作客户端（接受对局、选英雄）。
4. **磁盘缓存**：
   - 放在 `KOTONE_HOME/cache/lol-assets/`，按路径存文件；
   - 上限 64 MB，超了先删最久没用的；
   - 14 天后有网时重新取，失败的 10 分钟内不重试（照 AramHex）。
   - 估算：全部英雄头像约 4 MB，装备约 1 MB，海克斯小图约 0.6 MB，符文 1–2 MB；平时用不到 20 MB。
5. **CSP**：
   - App 的 `_meta.ui.csp.resourceDomains` 写 `https://raw.communitydragon.org`；
   - Kotone 自己显示时，宿主再加回环代理的源；
   - 腾讯 CDN 先不加。等 ADR 039 D6 写的“改主意”的情况出现（国服连 CommunityDragon 太慢、连不上），再加 `https://game.gtimg.cn`。它快，但名字规则不一样，要另写一套换算。
6. **测试**：假客户端回几张程序生成的小 PNG（纯色块）。仓库里不放拳头的图，也不在测试里请求 CommunityDragon。
7. **声明和风险**：
   - 照 Riot 的 “Legal Jibber Jabber” 政策，用拳头的游戏素材要在显眼处写明：用了拳头的素材，拳头不认可、不赞助本项目。
   - 同一份政策写着：在应用里用拳头的 IP 要有书面许可或有效的 API Key。Kotone 两样都没有（Akari、AramHex 也一样）。这条写进英雄联盟 App 的风险提示，只提示不拦着（记忆 `warn-dont-gate-risky-features`）。
   - 不用拳头的商标和 logo（ADR 039 D5、D8）。

---

## 四、差距表

标记：**今晚**、**以后**、**不做**。“大小”只给今晚的：小 ≈ 半小时内，中 ≈ 半小时到一个半小时，大 ≈ 更久（都按一个子代理算）。今晚做的按顺序单独列在本节末尾。

| # | 项 | 对标（谁有） | Kotone 现在 | 标 | 大小 |
| --- | --- | --- | --- | --- | --- |
| **素材** | | | | | |
| G0 | 素材层：数据带 id 和路径，页面里取图函数，本机代理加磁盘缓存，CommunityDragon 兜底，CSP 声明（第三节 4） | Akari（客户端代理）、AramHex（CDN + 缓存） | 一张图都没有 | 今晚 | 中 |
| G0.1 | 页面数据补字段：`MatchPlayer` 加 `profileIconId`、`summonerLevel`、`spell1Id/2`、`position`，段位拆成 `{tier, division, lp, wins, losses}`（单双排、灵活）；`PlayerData` 加头像、等级、两个段位；`PlayerGame` / `GamePlayer` 加 `spells`、`perks`（基石、副系）、`items`（id）、海克斯带 `rarity` 和图标路径。都是已经拿到的数据，给琴音的文字不变 | Akari | 只有文字字段 | 今晚 | 小 |
| **选人** | | | | | |
| G1 | 选人时“这局”页：<br>· 双方已选 / 预选的英雄（对面只有英雄）；<br>· 我方分路、召唤师技能、备选席；<br>· 我方 `puuid` 不空、没被隐藏的人，马上查战绩（和加载同一套，进加载后接着用，不重查）；<br>· 排位选人名字被隐藏，只显示英雄，并写明“排位选人看不到名字，进加载再查”；<br>· `lol.scout` 在选人阶段也能答（只有我方） | Akari | 说“进了加载界面才查得到” | 今晚 | 中 |
| G2 | 排位选人时还原被隐藏的名字 | Akari（不开源的组件） | — | **不做**（ADR 035 D1，拳头禁止反匿名） | |
| G3 | 选人时自动把窗口切到“对局”页 | Akari（可选，默认关） | — | 不做。不抢焦点（ADR 032 的精神），用自动卡片和消息提示代替 | |
| G4 | 选人辅助：我的英雄的出装、符文、召唤师技能，一键或自动写进客户端 | Akari 英雄数据窗、U.GG、OP.GG | 英雄页有数据（文字），不能写进客户端 | 以后（写客户端，放在“操作客户端”开关下，默认关） | |
| G5 | 大乱斗备选席：每个英雄的平衡性调整（伤害、承伤…） | Akari 小窗 | 没有 | 以后（数据源要再找：Akari 用的是 OP.GG 网页接口，OP.GG 还没回复 issue #26） | |
| G6 | 重随、交换、秒退、换皮肤 | Akari 小窗 | 没有 | 以后（写客户端，同上） | |
| G7 | 给打过标签的人在选人聊天里发提醒 | Akari | — | 不做（往聊天发字只在玩家说了才发，ADR 035 D7） | |
| G8 | 选英雄覆盖层：<br>· 评级 S–D（照 AramHex，按国服胜率排名：前 10% S、30% A、60% B、85% C、其余 D）；<br>· 备选席里比我高的标绿，最高的描金框 | AramHex（MIT，能借） | 有胜率和名次文字，没评级、没比较 | 今晚（排最后，时间不够挪到以后） | 小 |
| G9 | 海斗开局“选择你的英雄”的 2–3 张卡也贴胜率 | AramHex | 没有 | 以后（客户端接口里对应哪个字段没实测） | |
| **加载 · “这局”页** | | | | | |
| G11 | “这局”每人一行带图：<br>· 英雄头像（裁暗边），角上压等级；<br>· 两个召唤师技能；<br>· 名字#编号；<br>· 段位小徽章 + 短文字；<br>· 近 20 局胜率（≥ 53% 绿、≤ 47% 红）、KDA；<br>· 最近 10 局的英雄小头像，带胜负描边；<br>· 标签；开黑小队用颜色（行左侧色条 + 同色点）；<br>· 突破口整行描边 + 原因 | Akari | 全是文字；有近 20 局胜负色块、开黑色点、突破口 | 今晚 | 中 |
| G12 | 队伍表头：平均胜率、平均 KDA、几黑 | Akari | 没有 | 今晚（并进 G11） | 小 |
| G13 | 更多标签：伤害 / 承伤 / 经济占比、分均补刀、伤转率、视野、单杀、好抓难抓（峡谷）、闪现键位、拿头 / 打工 | Akari | 有连胜连败、第一次玩、隐藏战绩、开黑、突破口 | 以后（多数只要已有的单局详情，小） | |
| G14 | 遇到过、上局队友 / 对手、玩家自己打标签 | Akari | 没有 | 以后（要本地存，结算时记） | |
| G15 | 排序：按分路、胜率、KDA、开黑 | Akari | 没有 | 以后 | |
| G16 | 打野路线和抓人偏好 | Akari | 没有 | 以后（要时间线接口，大） | |
| G17 | 主播模式（隐藏名字） | Akari | 没有 | 以后 | |
| G18 | 房间阶段就查队友 | Akari（默认开） | 没有 | 以后 | |
| G19 | 综合分（Akari Score） | Akari | 有自己的“突破口”打分 | 不做（不抄它的算法；突破口已经覆盖；也不做像“替代官方段位”的排名） | |
| G20 | 直接请求腾讯 / 拳头服务器拿战绩（能按模式筛、一次给全） | Akari | 只用客户端接口 | 不做（ADR 035 D1） | |
| **对局中** | | | | | |
| G21 | 对局中按住快捷键看“这局”浮层 | Akari（默认关） | 问琴音、或切到 Kotone | 以后（要按键钩子，和 ADR 032 的输入规则一起定；看得见的事，要玩家定） | |
| G22 | 技能计时浮层（手点记、能发聊天） | Akari（默认关） | 说一句琴音记下（`lol.spell`），说了才发 | 以后（拳头 25.17 限制计时，照提示） | |
| G23 | 复活倒计时，复活时提示一句 | Akari 侧栏 | 游戏中心有我阵亡 / 复活的事件 | 以后 | |
| G24 | 三选一标签加判断：首选 / 可选 / 可考虑刷新，加依据（稀有度里第几、层级、样本偏少） | AramHex | 有“+分数 · 胜率 · 局数”，最好的描黄边 | 今晚（排最后，和 G8 一起） | 小 |
| G25 | 给装备的海克斯（命运之武器等）显示装备参考 | AramHex | 没有 | 以后 | |
| G26 | 游戏角上按需出现的按钮 → 出装、加点详情 | AramHex | 计划里有，没做 | 以后（覆盖层要能点，要真机测掉帧） | |
| G27 | 野怪 / 水晶复活计时、金币对比、实时表现对比 | OP.GG、Blitz | 没有 | 不做（局内接口给不了这些，只能读内存或识别画面；计时类拳头在收紧） | |
| G28 | 按快捷键往游戏聊天发预设文字（战绩、开黑） | Akari 工具集 | 玩家说了琴音才发 | 以后（国服有禁言风险，ADR 035 D7） | |
| **结算** | | | | | |
| G29 | 结算时“这局结果”：<br>· 进 `PreEndOfGame` / `EndOfGame` 先读 `/lol-end-of-game/v1/eog-stats-block`（国服未实测）；<br>· 读不到就每 5 秒查一次 `/lol-match-history/v1/games/{gameId}`，最多等 60 秒；<br>· “单局详情”页加“刚结束”的头部：胜负、模式、时长，我这一行高亮 | Akari（“对局”页留着、战绩里看）、U.GG 赛后分析 | 没有（复盘叫模型，写进记忆，界面上看不到） | 今晚 | 中 |
| G30 | 结算时的“这局总结”自动卡片（第五节） | 都没有（我们多出来的） | 没有 | 今晚（卡片内容；发卡片的能力是第 17 条） | 小 |
| G31 | 自动点赞、自动回房间、自动再排 | Akari | 没有 | 以后（“操作客户端”开关下，默认关，带 #523 提示） | |
| G32 | 赛后胜点变化、时间线、伤害图、录像 | U.GG、Akari | 没有 | 以后 | |
| **战绩 · 单局详情** | | | | | |
| G33 | 战绩头部：召唤师头像 + 等级、名字#编号、单双排 / 灵活段位徽章、胜负、最高段位 | Akari | 文字段位 | 今晚（并进 G34） | 小 |
| G34 | 战绩每局一行带图：<br>· 英雄头像，胜负描边；<br>· 两个技能、基石和副系；<br>· 6 件装备 + 饰品；<br>· 海克斯，按稀有度描边；<br>· KDA、伤害、模式、时间。<br>单局详情 10 个人同样带图 | Akari | 全是文字 | 今晚 | 小（有 G0 以后） |
| G35 | 单局详情的分页：详尽表格、符文、事件、构建、时间线 | Akari | 一张表 | 以后 | |
| G36 | 高级筛选（英雄、装备、海克斯、队友组合）、连续往前翻 | Akari | 按模式筛 | 以后 | |
| G37 | 英雄熟练度、最近一起玩的人、遇到过的对局 | Akari | 有常用英雄（文字） | 以后 | |
| G38 | 搜人：模糊搜、puuid、最近访问、好友列表；观战 | Akari | 名字#编号 | 以后 | |
| **英雄 · 强度榜** | | | | | |
| G39 | 英雄页、强度榜带图：装备、符文、召唤师技能、技能加点、海克斯（40 px，按稀有度描边） | Akari 英雄数据窗、AramHex | 文字 | 今晚（排在三个阶段之后，时间不够挪到以后） | 小 |
| G40 | 按海克斯查“哪个英雄拿它最好”、英雄组合 | Akari 海斗总览、aramhex.com/data | 有英雄榜、英雄页里最好的海克斯 | 以后 | |
| G41 | 分段数据（娱乐 / 中分 / 前 5%） | AramHex | 全体 | 以后 | |
| G42 | 查自己在区内排前百分之几 | AramHex（发名字给 aramhex.com） | 没有 | 不做（要把玩家名字发给第三方；也像“替代官方段位”的排名） | |
| **操作客户端** | | | | | |
| G43 | 自动接受、自动选 / 禁 | Akari | **已有**（第 3.5 步） | — | |
| G44 | 自动匹配、自动重连、自动处理邀请、预约邀请、自动交换英雄、按英雄自动配符文和技能 | Akari | 没有 | 以后（默认关，带 #523 提示） | |
| **工具集和其他** | | | | | |
| G45 | 领奖励、战利品、好友工具、房间工具、修客户端窗口 | Akari | — | 不做（和游戏陪玩无关） | |
| G46 | 改展示段位、锁离线、改签名 | Akari | — | 不做（给别人看假信息） | |
| G47 | 读游戏内存、注入 | — | — | **不做**（红线） | |
| G48 | 多套主题配色 | Akari | 亮 / 暗 | 设计文档定（`docs/design/lol-app.md`） | |
| G49 | 首次使用的风险声明 | Akari | 开关上有风险提示 | 今晚（第 15 条，加上第三节 4 的拳头素材声明） | — |
| G50 | 对外：MCP 服务器 + App，Claude、Codex 能用 | 两个都没有（Akari 的 `.mcp.json` 只是开发调试用的 Playwright） | 工具能经 `kotone.exe mcp` 调，页面还不行 | 今晚（第 12 条，不在本表算时间） | — |
| G51 | 托盘、系统通知 | Akari、AramHex | — | 今晚（消息提示，第 18 条） | — |

### 今晚做（按顺序）

今晚英雄联盟 App 大约 3–4 个子代理小时，还要做第 12 条（拆成独立 HTML 的 MCP App）。下面 1–6 是必须的，7–9 有时间再做。每条都建议和第 12 条一起，直接做在新的独立 HTML 页面上，不再改旧的 Svelte 页面。

1. **G0 素材层**（中）：取图函数 + 本机代理只读路由 + 磁盘缓存 64 MB + CommunityDragon 兜底 + CSP。
   验收：
   - 假客户端回生成的 PNG，Playwright 里看得到图；
   - 停掉假客户端后，走缓存或 CommunityDragon 还看得到；
   - basic-host 里也看得到；
   - 代理拒绝三个前缀以外的路径和非 GET。
2. **G0.1 页面数据补字段**（小）：给琴音的文字一个字不改，`lol_tools_real.py` 照过。
3. **G11 + G12 加载时的“这局”带图**（中）：最常看的一页，先做。
4. **G1 选人时的“这局”**（中）：玩家第一次在选人阶段就看到队友。
5. **G29 结算的“这局结果”**（中）。
6. **G30 + 开局战绩的两张自动卡片**（小）：卡片是页面的小尺寸版本，内容见第五节；发卡片的宿主能力是第 17 条。
7. **G33 + G34 战绩、单局详情带图**（小）。
8. **G39 英雄页、强度榜带图**（小）。
9. **G8 + G24 覆盖层对齐 AramHex**（小）：评级、比较、首选 / 可选 / 可考虑刷新。照 AramHex（MIT）的规则写，写进 `THIRD_PARTY_NOTICES.md`。

---

## 五、自动卡片：什么时候发、放什么

照 ADR 039 D3：不叫模型；同时往对话末尾追加一句很短的文字给琴音；通话中不插话。

“推卡片”的说法统一叫“自动卡片”（术语表）；右下角那个小窗叫“消息提示”。

### 什么时候发

| 时机 | 发不发 | 理由 |
| --- | --- | --- |
| 选人 | **不发**。只更新“这局”页和选英雄覆盖层 | 这时玩家在客户端里点来点去，覆盖层已经贴在客户端上；排位选人还看不到名字；两分钟内发两张卡太吵。对标里只有 Akari 在选人弹窗口，而且它弹的是出装窗 |
| 加载 | **发一张“开局战绩”** | 这是对标项目公认的“看对面”时机：OP.GG、Blitz、Porofessor 都在这里；加载有 30–90 秒空闲。 |
| 对局中 | 不发 | 打扰；要什么问琴音 |
| 结算 | **发一张“这局总结”** | 对标在这里最弱，这是我们多出来的；复盘照旧，在后台写进记忆，这张卡是给人看的 |

**“开局战绩”什么时候发**：

- 进加载（阶段是 `loading`），而且 10 个人的战绩都查完，就发。按实测每人 1.1–1.4 秒、同时查 2 个，约 7 秒；
- 进加载 20 秒还没查完，也发，写明“还在查 N 人”。
- 一局只发一次：重连（`Reconnect`）、刷新都不再发。
- 训练模式、自定义对局、观战都不发。

**“这局总结”什么时候发**：

- 拿到这局的结果就发（G29）；60 秒还拿不到就不发。
- 重开局（提前投降、有人没进）不发。

插件设置里一个“自动卡片”开关，默认开（ADR 039 D3）。一局最多两张。

### 卡片里放什么

**开局战绩**（聊天里的小卡片，点开是“这局”页的快照）：

- 头一行：“开局战绩 · 海克斯大乱斗”，和“我方 3 黑”这类开黑信息。
- 两块：我方、对面，各 5 行。每行：
  - 英雄头像 24 px；
  - 名字（截断）；
  - 段位小徽章 + 短写（“钻 II”）；
  - 近 20 局“12 胜 8 负”，胜率着色；
  - KDA；
  - 最多 2 个标签（连败 4、第一次玩、隐藏战绩），开黑用行左侧色条。
- 对面最可能的突破口 1–2 行描边，卡片最下面一行写原因：“寒冰：近 20 局 6 胜 14 负，场均死 13 次”。
- 随机英雄的模式（大乱斗、海斗）不写“第一次玩”（ADR 035 D3）。

**这局总结**（点开是“单局详情”，带“刚结束”的头部）：

- 头一行：胜利 / 失败 · 模式 · 时长。
- 我这一行：
  - 英雄头像 32 px；
  - K/D/A；
  - 伤害（全队第几）、承伤、经济、分均补刀；
  - 海克斯（海斗，最多 6 个，按稀有度描边）或基石符文；
  - 6 件装备。
- 下面 10 个人一人一个英雄小头像 + K/D/A，按伤害画一条短条。
- 只写算出来的事实（“伤害全队第一”），不评价人（ADR 035 D3 的口吻）。

### 给琴音的那句

追加在对话末尾，按不可信数据处理（别人起的名字），不超过约 120 字。例：

- 开局战绩：“英雄联盟发来一张卡片：开局战绩（海斗）。对面可能的突破口：寒冰，近 20 局 6 胜 14 负、场均死 13 次。我方有 3 人开黑。”
- 这局总结：“英雄联盟发来一张卡片：这局赢了（海斗，18 分 32 秒）。你用寒冰 12/3/20，伤害全队第一。”

### 消息提示的那句话

琴音的头像 + 一句：

- “对面的战绩到了”
- “这局的总结好了”

点它打开聊天，定位到卡片。游戏独占全屏时不弹（ADR 039 D4）。

---

## 六、早上要玩家试的、还没核实的

1. 国服客户端的图片接口能不能取到：`/lol-game-data/assets/…png`、`/fe/lol-static-assets/images/ranked-mini-crests/*.svg`。
2. 大乱斗、海斗选人时，我方 `puuid` 是不是看得到，`nameVisibilityType` 是什么。2026-11-23 录到的选人接口是 404，当时已经进了对局。
3. `/lol-end-of-game/v1/eog-stats-block` 国服有没有、有哪些字段。单局详情在结算后多久能查到。
4. 海斗开局英雄卡在选人接口里对应哪个字段。
5. CommunityDragon 在国服玩家那里的速度（本机 0.6–1.4 秒）。
6. Blitz、U.GG 官网都回 403，只看了搜索摘要和商店页。

---

## 七、出处（都是 2026-11-25 访问的）

**LeagueAkari**（`dev` 分支）：

- 仓库：https://github.com/LeagueAkari/LeagueAkari 。约 4.8k 星，最新正式版 v1.5.1（2026-07-25）。
  - LICENSE：https://github.com/LeagueAkari/LeagueAkari/blob/dev/LICENSE ，提交历史 `a11ccc1a`、`9d3d52fa`。
- 窗口什么时候弹：`src/main/shards/window-manager/`，其中：
  - `aux-window/window.ts`、`opgg-window/window.ts`、`ongoing-game-window/window.ts`、`cd-timer-window/windows.ts`；
  - 各自的 `state.ts`（默认值）。
- 主窗口自动切页：`src/renderer/src-main-window/shards/main-window-ui/auto-route-watcher.ts`。
- “对局”页的数据和阶段：`src/main/shards/ongoing-game/`，其中 `computed-state.ts`、`champ-select-members.ts`、`side-effects-controller.ts`、`state.ts`。
- “对局”页卡片：`src/renderer-shared/components/ongoing-game-panel/`。
- 战绩卡片：`src/renderer-shared/components/match-card/MatchCardOverview.vue`。
- 图标组件：`src/renderer-shared/components/widgets/`，其中 `ChampionIcon.vue`、`AugmentDisplay.vue`、`ItemDisplay.vue`、`PerkDisplay.vue`、`SummonerSpellDisplay.vue`。
- 取图：
  - `src/renderer-shared/providers/akari-resource/akari.ts`；
  - `src/main/shards/akari-protocol/`；
  - `src/main/shards/league-client/index.ts` 里注册的 `league-client` 代理；
  - `src/renderer-shared/shards/league-client/game-data-assets.ts`；
  - `src/main/shards/extra-assets/`；
  - `src/shared/data-sources/gtimg/index.ts`。
- 中文文案（只用来弄懂功能）：`src/shared/i18n/zh-CN/renderer/*.yaml`。
- README（支持范围、免责）：https://github.com/LeagueAkari/LeagueAkari/blob/dev/README.md 。
- issue #523（美服海斗用了自动匹配、自动接受、自动选英雄后永久封号，2026-10-05，已关闭）：https://github.com/LeagueAkari/LeagueAkari/issues/523 。

**AramHex**：

- 原作：https://github.com/czgreat/aramhex （MIT，74 星，2026-11-20）。读过的文件：
  - `README.md`、`CHANGELOG.md`（1.2.0）、`docs/on-demand-overlay.md`；
  - `lolhex/ui/overlay.py`（三选一标签、选英雄标注、角上按钮）、`lolhex/ui/mainwindow.py`；
  - `lolhex/data/icons.py`（图标缓存）、`lolhex/data/tencent.py`（腾讯地址）；
  - `lolhex/engine.py`（评级换算）、`lolhex/vision/champselect.py`、`lolhex/rank.py`。
- 分叉：https://github.com/saon3r/aramhex （MIT，英文、欧服版；和原作的比较用 GitHub compare 接口看的）。
- 数据：https://github.com/czgreat/aramhex-data （CC BY-NC 4.0）。

**OP.GG、Blitz、U.GG**：

- OP.GG Desktop 英雄联盟页：https://op.gg/desktop/en/games/lol （读过）。
- Blitz 覆盖层页：https://blitz.gg/lol/overlays （403，只看了搜索摘要，**未核实**）。
- U.GG 应用页：https://u.gg/app （403，**未核实**）。U.GG 在 Overwolf 的商店页（只看了搜索摘要）：https://storeclientqa.overwolf.com/app/u.gg-u.gg 。

**素材**（都用 `curl` 请求过）：

- CommunityDragon：https://raw.communitydragon.org/latest/ ，`content-metadata.json` 是 16.23；路径见第三节。
- 腾讯 CDN：
  - https://game.gtimg.cn/images/lol/act/img/js/kiwi/kiwi_augments.json
  - https://game.gtimg.cn/images/lol/act/img/champion/Ahri.png
- Data Dragon 版本表：https://ddragon.leagueoflegends.com/api/versions.json （16.23.1）。
- 客户端段位徽章路径 `/fe/lol-static-assets/images/ranked-mini-crests/{tier}.svg`：取自开源项目 chii-primenumber/sona-plus 的 `src/lib/ranked-assets.ts`（**未在客户端实测**）。
- `/lol-end-of-game/v1/eog-stats-block`：出现在 LeagueAkari 的调试接口列表和 wnzzer/rank-analysis 里（**未在国服实测**）。
- 国服客户端录的数据：`target/lol-probe/gamedata/*.json`、`target/lol-probe/20261123-210745/`（2026-11-23）。

**规则**：

- Riot “Legal Jibber Jabber”：https://www.riotgames.com/en/legal 。

**本仓库**：

- [ADR 035](../adr/035-lol-tools.md)、[ADR 038](../adr/038-kotone-and-apps.md)、[ADR 039](../adr/039-desktop-companion-and-game-apps.md)；
- [插件重构计划](../plugin-refactor.md)第九节第 1、2、3、3.5 步；
- [2026-11-23 对局数据调研](2026-11-23-lol-live-data-akari.md)、[2026-11-23 数据来源调研](2026-11-23-lol-data-sources.md)、[MCP 客户端方案](2026-11-24-mcp-client.md)。
