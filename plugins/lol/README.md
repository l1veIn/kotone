# 英雄联盟插件（`lol`）

像 LeagueAkari 那样查这局 10 个人的战绩（隐藏了身份的人、电脑也列出来，只有英雄）、记对面交的召唤师技能；给游戏中心报英雄联盟的对局阶段和局内的事。插件只说“是什么”（数字、出处），谁是突破口这类评价归她（[ADR 043](../../docs/adr/043-plugin-shape-and-pushes.md) D2）。
决定和实测见 [ADR 035](../../docs/adr/035-lol-tools.md)（工具）、[ADR 036](../../docs/adr/036-game-state-and-events.md)（游戏中心）、[ADR 038](../../docs/adr/038-kotone-and-apps.md)（插件），计划见 [插件重构计划](../../docs/plugin-refactor.md)。

## 有什么

| 东西 | 在哪 |
| --- | --- |
| 清单：名字、挂在游戏配置 `lol` 上、`lol` 适配器、数据开关、页面、给她的两段话 | `plugin.toml` |
| 给她和页面的工具 `lol.match`、`lol.spell`、`lol.scout`（都马上返回；`lol.match` 只写英雄、不写玩家名，是可信的，还带一份给自动化程序的结构化数据，见下面“游戏事件和这局的数据”；别的结果按不可信数据给她） | `src/tools.rs` |
| 后台：客户端开着时每 2 秒看一次阶段，进加载界面就查战绩，往游戏中心报阶段和局内的事；工具的实现 | `src/watcher.rs` |
| 客户端和局内接口、游戏资料、战绩、技能计时、写给她看的文字（纯函数 + 只连 127.0.0.1 的 HTTP） | `src/client/` |
| 页面（插件页、游戏配置页里的一节） | `ui/` |
| 界面文字（`plugin_lol_*`，中文要和 `plugin.toml` 一致，有测试） | `messages/` |
| 页面的测试（桌面端的 Playwright 一起跑） | `e2e/` |
| 网上的统计（腾讯 101、aramhex-data、外号和名字表、OP.GG）和它的工具 `lol.champion` / `lol.tier` | `src/stats/`、`src/stats_tools.rs` |
| 推送：选英雄卡片、海斗三选一的卡片 + 语音（照 aramhex 的判断规则，本地文字识别，只看名字那一横条；原来的两个覆盖层，ADR 043 D7）；对局中的提醒（对面到 6 级、关键装备和第一次有重伤（不管你玩什么英雄）、技能转好、资源快刷、对面阵亡（还要 30 秒以上才复活的到 3 个），语音 + 一行；你阵亡时对面装备的卡片）和等级、装备两类事件（B 轮）；开局战绩、这局总结 | `src/pushes.rs`、`src/pushes/`（`alerts.rs` 是对局中的提醒）、`app/src/pages/ChampSelect.svelte`、`AugmentPick.svelte`、`EnemyBuild.svelte`、`AugmentOcr.svelte`；`src/watcher/auto_cards.rs` |
| 页面的数据（这局、战绩、单局详情；图只给 id 和客户端路径） | `src/pages.rs` |
| 素材层：页面要的图（英雄、召唤师头像、段位、海克斯、装备、技能、符文），见下面“素材” | `src/client/assets.rs` |
| 用录下来的真实数据离线跑三个工具的文字；真接口跑一遍统计 | `examples/render_probe.rs`、`examples/stats_probe.rs` |

## 数据从哪来

| 数据开关 | 读什么 | 默认 |
| --- | --- | --- |
| `client` 读客户端数据 | 英雄联盟客户端的本机接口（LCU，端口和令牌从 `LeagueClientUx.exe` 的命令行读，不用管理员）和局内接口（Live Client Data，`127.0.0.1:2999`）。只读，只连 127.0.0.1 | 开 |
| `stats` 用网上的统计 | 腾讯 101 / 掌盟（`mlol.qt.qq.com`，峡谷强度、克制、出装、符文、加点，海斗英雄榜和海克斯榜）；[czgreat/aramhex-data](https://github.com/czgreat/aramhex-data)（海斗英雄 × 海克斯胜率，**CC BY-NC 4.0**，运行时下载、页面上署名、不能商用）；腾讯 `game.gtimg.cn` 的英雄外号和装备、符文、召唤师技能名字。按需请求，缓存在插件目录 | 开 |
| `opgg` 用 OP.GG 的数据 | OP.GG 官方 MCP（`mcp-api.op.gg/mcp`）：页面上把来源标签换成 OP.GG、她查的时候才请求，最多同时 2 个，存 24 小时，标签上放 OP.GG 的字标和 op.gg 的链接（[issue #26](https://github.com/opgginc/opgg-mcp/issues/26) 里答应的用法；ADR 043 D9 起不再等回复） | 开 |
| `control` 操作客户端 | 照 LeagueAkari 自动接受对局、自动选 / 禁英雄，她替你接受 / 选 / 禁（`lol.client`）。写客户端的本机接口 | **关** |
| `chat` 在客户端里发聊天 | 发到游戏（ADR 041 第 2 期）：客户端在前台时（选英雄、房间里），你的语音输入和她替你发的话经客户端的聊天接口发（`POST /lol-chat/v1/conversations/{id}/messages`），不按键；几行连成一条。只在有人要发时发。要 `client` 也开着。见下面“发到游戏” | 开 |

设置在 `config.json` 的 `plugins.lol`（`enabled`、`sources.client`）。以前的 `lol.enabled` 读进来时搬到 `plugins.lol.sources.client`。

## 发到游戏（ADR 041 第 2 期，`src/chat.rs`）

清单 `[[send_to_game]]` 登记 `LeagueClientUx.exe`：前台（语音输入：开始收音的那个窗口）是英雄联盟客户端时，本体的“发到游戏”入口把整理好的几行交给这个插件，不拿输入锁、不等修饰键、不切窗口。对局里（`League of Legends.exe` 在前台）不归这里，本体照游戏配置按键（英雄联盟的 Enter / Shift+Enter、行间 65 毫秒）。

- **阶段**（发的时候现读 `/lol-gameflow/v1/gameflow-phase`）：`ChampSelect` → 选英雄的聊天；`Lobby` / `Matchmaking` / `ReadyCheck` → 房间的聊天；对局里、结算、客户端首页 → 不发，说清楚为什么（结算的聊天这一期不做）。
- **会话**：`GET /lol-chat/v1/conversations`，选英雄认 `type == "championSelect"` 而且 id 里有 `lol-champ-select` 的，房间认 `type == "customGame"`；id 里有 `/`、`?`、`#` 的不拼进路径。每次发都现查，不缓存。
- **频道**：选英雄、房间只有自己这边看得到（`team`）；自定义对局（房间的 `gameConfig.isCustom`、选英雄的 `isCustomGame`）两边都看得到（`all`）。要的频道和这里一样才发，否则不发（“发给更多人不退”）。
- **多行**：用换行连成一条发，请求体 `{"body": "...", "type": "chat"}`（照 LeagueAkari）。
- **只发一次**：截止时间前一定返回；客户端回错误 = 没发；没在期限内回话 = 可能已经发出去了，照实说、不重发、也不退回按键。日志不记文字。
- **出处**：照 LeagueAkari 的做法（`dev` 分支提交 `4f8981f8` 的 `in-game-send/send-executor.ts`、`league-client/lc-state/index.ts`），只学做法、不抄代码（ADR 039 D5）。
- **还没在真客户端上核对**：客户端窗口的前台进程名、换行在选英雄聊天里怎么显示、10 行 × 200 字收不收、自定义对局和斗魂竞技场的选英雄聊天谁看得到、国服屏蔽词的表现。

## 一键喊话（ADR 041 第 3 期，`src/watcher/shout.rs`）

`lol.shout` 按这局已有的数据现生成几行（不调 AI、不现查），经宿主交给本体唯一的发话入口（`Host::send_to_game`，和 `game_chat.send` 同一个入口）：选英雄、房间里客户端在前台时又交回这个插件的客户端聊天接口发（上一节），对局里是本体照游戏配置按键。插件只生成内容、不按键。

| 预设（`[[commands]]`） | 参数 | 发什么 | 没东西可喊时 |
| --- | --- | --- | --- |
| 喊对面闪现 | `flash` / `enemy` | 每个还没好的一行“亚索 闪 14:32好”（和 `lol.spell` 的 `chat` 同一个写法），再加“对面没带闪：阿狸” | 接口里没有冷却，只有你报过的（说“对面亚索闪了”，她用 `lol.spell` 记下，这局结束清掉）；没记过又都带着闪就不发，说为什么 |
| 喊对面战绩 / 喊我方战绩 | `records` / `enemy`、`ally` | 一人一行：段位、近 20 局胜负、KDA、连胜连败、常玩 / 没玩过这个英雄 | 还有人没查完就不发（进加载界面就在后台查）；选英雄时看不到对面 |
| 喊开黑情况 | `premade` / `all` | 一组一行，对面在前 | 看不出开黑不发 |

- 都发队伍频道，不带默认键：在「设置 → 热键」里“英雄联盟”那几行录一个键就行；按下时照 `lol.shout` 的权限（默认允许，和 `game_chat.send` 一样），成了弹幕一行，没成说原因，“停下”能停掉还没发完的几行。
- 只写英雄，不发玩家名（选英雄时还没选的用分路、几楼）。
- 她也能用：你说“把对面闪现发出去”时一次调用；`preview: true` 只看会发什么。
- 照 LeagueAkari 的“游戏内发送”预设学做法（`dev` 分支 `src/main/shards/in-game-send/` 的 `preset-controller.ts`、`send-executor.ts`），不抄代码。没做它的“打野偏好”和“自定义模板”。

## 游戏事件和这局的数据（ADR 036、ADR 046）

插件报事实，判断在自动化：事件说“刚发生了什么”，这局现在的样子随时用 `lol.match` 查。事件登记在 `plugin.toml` 的 `[[events]]`（字段名 camelCase，`data` 里只放英雄名，不放玩家名），`src/event_catalog_tests.rs` 查报的和登记的对得上。

- **局内接口的事件列表**（`client/events.rs` 的 `new_events`）：一血、龙、先锋、大龙、塔、水晶、团灭、三杀以上。
- **比前后两份局内数据**（每 2 秒一份，和对局中的提醒同一处，`pushes/alerts.rs`）：等级、成装（只报对面和自己），和阵亡：
  - `lol.death`：两边每有一个英雄阵亡报一件，以 `isDead` 由假变真为准（和“你阵亡时”的卡片、“对面阵亡”同一个判断），同一段时间里这个人的 `ChampionKill` 补上击杀者（隐藏了身份的人按 `summonerName` 对）。字段 `team`、`champion`、`me`、`killer`（塔、小兵、野怪杀的或对不上时是空的）、`respawnSeconds`、`alliesAlive`（含自己）、`enemiesAlive`、`meAlive`。同一份数据里死了几个：按击杀时间排，一人一件，人数是“这一个死了之后”的，所以“最后一个队友倒下”只有那一件 `alliesAlive = 1`。佛耶戈附身时阵亡或击杀，英雄还是佛耶戈（按这局第一次看到他时的英雄）。
  - 每局第一份数据只记样子、之前的不补；中间没读到接口那段死了又复活的不补；看不出自己在哪边（观战）时不报；最多晚约 2 秒。
  - `lol.my_death` / `lol.my_respawn` 字段不变，由同一处报：`lol.my_death` 紧跟在自己那件 `lol.death`（`me = true`）后面。
  - 还没在真机上核对：守护天使、基兰大招、赛恩被动、艾尼维亚的蛋、扎克被动时 `isDead` 会不会闪一下（录的那把海斗是每 20 秒一份）。
- **这局结算了**（`watcher/auto_cards.rs`，ADR 050 第 6 条）：`lol.game_end` 和“这局总结”同一时刻、同一份数据（单局详情）报，这条推送关着也报；自定义、训练、观战、重开局、60 秒还拿不到结果的不报。字段 `win`、`queue`、`durationSeconds`、`champion`（短名字）、`kills`、`deaths`、`assists`、`damageRank`（1 = 全队第一）、`pentaKill`（详情里有这一项才给）、`todayWinStreak` / `todayLossStreak`（算上这局，今天现在几连胜 / 几连败，另一个是 0；结算时读一次自己的战绩，读不到两个都不给）。内置自动化“英雄联盟这局打得特别好或特别差时她说几句”（`builtin-lol-game-end-cheer`，默认开）按它触发：赢了而且伤害全队第一、五杀、今天三连败以上才 `ask()` 交给她。
- **`lol.match` 的结构化数据**（`client/text.rs` 的 `match_data`，程序里 `tool("lol.match")` 拿到的就是它；字段名是约定，写在工具说明里）：`status`、`reason`（不是 `ok` 时：`loading` / `champ_select` / `not_in_game` / `client_closed`，或读不到的原因）、`mode`、`gameTime`（整数秒）、`score {ally, enemy}`、`alive {ally, enemy}`、`players[]`（我方在前）：`champion`、`alias`、`team`、`me`、`bot`、`level`、`kills`、`deaths`、`assists`、`cs`、`dead`、`respawnSeconds`、`position`、`spells[{key, name, readyAt?}]`、`items[{id, name}]`。接口给不出的不放：别人的金币、技能冷却、海克斯。没有页面，宿主不把它放进她的上下文、也不出卡片。

## 素材（图片）

安装包里不带图（ADR 039 D6，[对标调研](../../docs/research/2026-11-25-lol-app-parity.md)第三节）。页面数据里只有 id 和客户端的资源路径（例如 `/lol-game-data/assets/v1/champion-icons/103.png`），页面自己决定从哪取：

- Kotone 里：经回环代理交给 `src/client/assets.rs`，顺序是磁盘缓存（插件目录下的 `asset-cache/`，上限 64 MB，超了删最久没用的，14 天重新取）→ 客户端（开着、而且“读客户端数据”开着时，带本机令牌，只读）→ CommunityDragon → 页面显示占位。
- 只放行 `/lol-game-data/assets/`、`/fe/lol-static-assets/`、`/fe/lol-shared-components/` 三个前缀下的图片（png / jpg / svg / webp，按内容认），路径里有 `..`、`%`、查询串的一律拒绝。不把客户端的别的接口转给页面：页面在沙盒里，转了等于让页面能操作客户端。
- 别的宿主（Claude、Codex）里：页面直接换成 CommunityDragon 的地址。
- 测试：`KOTONE_LOL_CDRAGON=<本机地址>` 把 CommunityDragon 换成本机的假服务器（只认本机，写错了就不连网）。

**CommunityDragon**（https://raw.communitydragon.org ，社区维护的客户端资源镜像）：客户端路径换成它的地址的规则照它的目录结构（`/lol-game-data/assets/…` → `plugins/rcp-be-lol-game-data/global/default/…`，后面全部小写）。只请求公开的图片路径，不带玩家的任何数据。

**拳头的声明**（Riot Games “Legal Jibber Jabber”，https://www.riotgames.com/en/legal ）：Kotone isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all associated properties are trademarks or registered trademarks of Riot Games, Inc.

英雄、召唤师头像、段位、海克斯、装备、技能、符文的图属于拳头；Kotone 用了这些素材，拳头不认可、也不赞助 Kotone。同一份政策写着：在应用里用拳头的 IP 要有书面许可或有效的 API Key，Kotone 两样都没有（LeagueAkari、AramHex 也没有）。这一条写进英雄联盟 App 的风险提示，只提示，不拦着。不用拳头的商标和 logo（ADR 039 D5、D8）。

## 风险

国服用这类功能可能违反腾讯的用户协议，LeagueAkari 等工具有封号、禁言的报告；Riot 25.17 起限制第三方的技能冷却计时。默认开，界面上写着风险，用不用由玩家决定。

“在客户端里发聊天”（`chat`，默认开）：经接口发的话照样过腾讯的屏蔽词和禁言规则（LeagueAkari 用户报告过模板里的词被屏蔽，[#480](https://github.com/LeagueAkari/LeagueAkari/issues/480)；WeGame 去掉了能改内容的一键喊话，[#524](https://github.com/LeagueAkari/LeagueAkari/issues/524)）；Riot 说客户端接口不正式支持第三方使用（[developer.riotgames.com/docs/lol](https://developer.riotgames.com/docs/lol)）。没查到因为经接口发聊天被封号或禁言的报告。默认开的理由：只在有人要发时发，和玩家自己打字是一类；关着时选英雄里她不替你发，你的语音输入按键打进客户端窗口，效果不可预料。

**不做**：读游戏内存、注入。只用客户端和游戏自己提供的本机接口。

## 在 Claude Desktop、Codex 里用

英雄联盟插件本身就是一个独立的 MCP 服务器 + MCP App（ADR 039 D5）：`kotone.exe mcp` 是标准的 stdio MCP 服务器，Claude Desktop、Codex 这类宿主直接连它就能调英雄联盟的工具；会显示 MCP App 的宿主（Claude、ChatGPT、VS Code、Goose 等）还会把结果画成英雄联盟 App 的页面，和 Kotone 里显示的是同一份。

**前提**：Kotone 在跑，而且开着工具服务（默认关：让外部 Agent 先调 `kotone_launch`，它会打开工具服务、在后台启动 Kotone；或者自己退出 Kotone → `kotone.exe config set toolService.enabled true` → 重新启动，见 [工具服务手册](../../docs/tool-service.md)第二节）。`kotone.exe mcp` 自己不带业务逻辑，每次调用都转给运行中的 Kotone：Kotone 里关掉英雄联盟插件或者某个数据开关，对应的工具和页面也就没了。

Claude Desktop：设置 → 开发者 → 编辑配置，打开 `%APPDATA%\Claude\claude_desktop_config.json`，加上：

```json
{
  "mcpServers": {
    "kotone": {
      "command": "C:\\Users\\<你的用户名>\\AppData\\Local\\Kotone\\kotone.exe",
      "args": ["mcp"]
    }
  }
}
```

Codex：`%USERPROFILE%\.codex\config.toml` 里加上：

```toml
[mcp_servers.kotone]
command = 'C:\Users\<你的用户名>\AppData\Local\Kotone\kotone.exe'
args = ["mcp"]
```

`command` 写 `kotone.exe` 的完整路径（安装包默认装在 `%LOCALAPPDATA%\Kotone\`；JSON 里的反斜杠要写两个，TOML 用单引号就不用）。改完重启 Claude Desktop / Codex。

**能用的工具**（对外的名字是 Kotone 工具名的点换成下划线、前面加 `kotone_`；只给页面按钮用的标 `visibility: ["app"]`，宿主不交给模型）：

| 工具 | 做什么 | 页面 | 要开的数据开关 |
| --- | --- | --- | --- |
| `kotone_lol_scout` | 这局 10 个人的战绩、开黑（只给事实） | `ui://kotone/lol/match` 这局（选人、加载、对局中、结算） | 读客户端数据 |
| `kotone_lol_tier` | 强度榜（峡谷按位置、海斗） | `ui://kotone/lol/tier` 强度榜 | 用网上的统计 |
| `kotone_lol_match` | 这局的样子、谁的召唤师技能没好（不带玩家名；`structuredContent` 是给程序的结构化数据） | — | 读客户端数据 |
| `kotone_lol_spell` | 记对面交的召唤师技能 | — | 读客户端数据 |
| `kotone_lol_player` | 任何人的战绩 | `ui://kotone/lol/player` | 读客户端数据 |
| `kotone_lol_champion` | 英雄的强度、克制、出装、海斗海克斯 | `ui://kotone/lol/champion` | 用网上的统计 |
| `kotone_lol_client` | 操作客户端：接受对局、选 / 禁英雄 | — | 操作客户端（**默认关**） |
| `kotone_lol_game`、`kotone_lol_champions`、`kotone_lol_source` | 只给页面用：单局详情、英雄列表、来源标签换来源（记在 `plugins.lol.settings.sourceFor`） | — | 同上 |
| `kotone_lol_history` | 只给页面用：主页、战绩页翻页（客户端按人只给一段战绩，所有地方都要同一段最近 50 局、插件自己切页，翻到头写清客户端给了几局） | — | 读客户端数据 |

页面一页页从 Kotone 里的 Svelte 换成 App，以 `src/app.rs` 的 `APP_PAGES` 为准：是 App 的，工具带 `_meta.ui.resourceUri`、`resources/list` 里有它；还不是的，外部宿主里这个工具只给文字。宿主不支持 MCP Apps 的（例如终端里的 Codex CLI）照样能用，只拿给模型的那几行字。图片在别的宿主里直接从 CommunityDragon 取（域名写在资源的 CSP 声明里），不经 Kotone。

**风险提示**（跟着服务器说明发：`initialize` 的 `instructions` 里有同样的话，宿主会交给模型）：读本机客户端和对局的接口可能违反腾讯（国服）和拳头（Riot）的用户协议，LeagueAkari 等工具有封号、禁言的报告；外服有人用了自动接受、自动选英雄后被永久封号（[LeagueAkari #523](https://github.com/LeagueAkari/LeagueAkari/issues/523)）。用不用由玩家决定，只提示、不拦着。操作客户端默认关，要玩家自己在 Kotone 的英雄联盟插件设置里打开。图片属于拳头，Kotone 没有拳头的许可（见上面“素材”）。

**怎么验的**：`scripts/manual-e2e/mcp_app_external_real.py` 用 MCP Apps 官方的参考宿主（`modelcontextprotocol/ext-apps` 的 `examples/basic-host`）经一个 stdio ↔ Streamable HTTP 的桥连 `kotone.exe mcp`，在里面调 `kotone_lol_tier` 和 `kotone_lol_scout`（假客户端），看 App 在它的沙盒里画出来、图从 CommunityDragon 来。Claude Desktop 里的实际效果还没试。

## 照着谁做

[LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)（MIT 的 `dev` 分支，不拷 v1.3.x–v1.4.3 的 GPL 版本）：读命令行、接口路径、模式急速、开黑分组、腾讯 101 的解析。突破口的算法是自己写的。[aramhex](https://github.com/czgreat/aramhex)（代码 MIT）：选英雄的评级、三选一的判断规则（`src/pushes/rules.rs`），截图识别怎么不拖慢游戏（只在需要时看、识别线程低优先级、截图时不认自己画的）。

## 测试

```powershell
cargo test -p kotone-plugin-lol
# 录下来的一局 → 给她的字，加上三个页面的数据样例（去掉玩家名和 puuid）
cargo run -p kotone-plugin-lol --example render_probe -- target/lol-probe/20261123-210745 target/lol-probe/gamedata target/plugin-shots/round7
python -X utf8 -I scripts/manual-e2e/lol_tools_real.py --while-playing   # 假客户端 + 真 DeepSeek，不开游戏
python -X utf8 -I scripts/manual-e2e/game_events_real.py                 # 假客户端按时间线放一局
```
