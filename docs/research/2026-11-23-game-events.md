# 2026-11-23 游戏状态和事件怎么建模：成熟项目的做法（ADR 036 的依据）

起因：玩家问“游戏生命周期的监控怎么做……监控也是工具吗？自动化条件里的触发器能跟游戏生命周期联动吗？”，又说“这个状态机或者说状态订阅可以做的通用一点，未来肯定会有更多的游戏接入，如何定义各种游戏事件也是在一开始就要考虑好的设计”。这里记 2026-11-23 核对过的做法。Valve 的维基不让程序抓取，读的是 archive.org 2026-10-16 的存档。

## 一、Overwolf 游戏事件（GEP）

- **状态和事件分开**（[介绍](https://dev.overwolf.com/ow-native/live-game-data-gep/live-game-data-gep-intro)）：
  - **状态**（Game Info）存着，随时能用 `getInfo()` 读；变了才通过 `onInfoUpdates2` 通知，格式是 `{feature, info: {category: {key: value}}}`；
  - **事件**（`onNewEvents`）是一次性的、不存，要先挂上监听，格式是 `{events: [{name, data}]}`。
- **功能**（feature）把相关的状态和事件分成一组，要先 `setRequiredFeatures()` 声明要哪些，GEP 才开始听；返回里有 `supportedFeatures`。“不在游戏里”“不支持这个游戏”是最终失败，不用重试（[参考](https://dev.overwolf.com/ow-native/reference/games/events)）。
- **订阅时先给一次当前状态**：客户端 0.311.0.6 起，订阅时会先推一次当前状态，所以处理代码要能容忍重复。
- **值没变不通知**：比赛结束时他们把字段设成 `null`，这样下一局的 0 才算变化。
- **游戏启动和退出**（[overwolf.games](https://dev.overwolf.com/ow-native/reference/games/ow-games)）：`onGameLaunched`；`onGameInfoUpdated` 带 `runningChanged`、`focusChanged`、`gameChanged` 这几个标志。
- **每个游戏、每个功能有可用状态**（[验证事件](https://dev.overwolf.com/ow-native/live-game-data-gep/verifying-events-for-your-app)）：0 不支持、1 绿、2 黄（部分事件可能拿不到）、3 红。
- **英雄联盟的功能**（[页面](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/league-of-legends/)）：
  - `matchState`、`match_info`、`death`、`respawn`、`abilities`、`kill`、`assist`、`gold`、`level`、`announcer`、`jungle_camps`、`chat`、`augments`……
  - 例子：`{"name":"kill","data":"{\"label\":\"double_kill\",\"count\":\"2\"}"}`。
  - 选人阶段在单独的“客户端”提供者里（[launcher](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/league-of-legends-launcher/)）：`game_flow` 的 `phase`（Lobby…EndOfGame）、`champ_select`、`end_game`。

## 二、Valve Game State Integration（CS2、Dota 2）

- **配置**（[维基存档](https://web.archive.org/web/20261016142919/https://developer.valvesoftware.com/wiki/Counter-Strike:_Global_Offensive_Game_State_Integration)）：在 cfg 目录放 `gamestate_integration_<名字>.cfg`，游戏启动时读，配置项：
  - `uri`：游戏往这里 POST；
  - `timeout`（1.1 秒）：请求没回来之前不再发；超时以后下一次发整份；
  - `buffer`（0.1 秒）：把这段时间里的变化攒成一次发；
  - `throttle`（1 秒）：两次发送之间至少隔这么久；
  - `heartbeat`：没变化也定期发，接收方据此判断游戏“掉线”；
  - `auth{token}`：每次都原样带回；
  - `data{分节:"1"}`：要哪些分节。
- **推过来的是整份状态**，另带 `previously`（变了的字段的旧值）和 `added`（新出现的字段）。
- **CS2**：`map.phase` = warmup / live / intermission / gameover；`round.phase` = freezetime / live / over；`round.bomb` = planted / exploded / defused。文件放在 `game/csgo/cfg/`（[CounterStrike2GSI](https://github.com/antonpup/CounterStrike2GSI)）。
- **Dota 2**：文件放在 `game/dota/cfg/gamestate_integration/`；`map.game_state` 的取值（都带前缀 `DOTA_GAMERULES_STATE_`，见[枚举](https://github.com/antonpup/Dota2GSI/blob/master/Dota2GSI/Nodes/Map.cs)）是：
  - `INIT`、`WAIT_FOR_PLAYERS_TO_LOAD`、`WAIT_FOR_MAP_TO_LOAD`、`CUSTOM_GAME_SETUP`、
  - `HERO_SELECTION`、`STRATEGY_TIME`、`TEAM_SHOWCASE`、
  - `PRE_GAME`、`GAME_IN_PROGRESS`、`POST_GAME`、`DISCONNECT`。
- antonpup 的库把状态差异翻成有名字的事件（`MapPhaseChanged`、`RoundStarted`、`BombPlanted`、`PlayerDied`……）。Rust 有现成的 `cs2-gsi`、`dota-gsi`。

## 三、Home Assistant 的触发

- **状态触发**（[state](https://www.home-assistant.io/triggers/state/)）：`entity_id`、`from`、`to`、`not_from`、`not_to`、`attribute`、`for`。`for` 期间值又变了就重新计时；重启后计时清零。
- **事件触发**（[event](https://www.home-assistant.io/triggers/event/)）：`event_type` 可以是一个或一组；`event_data` 是**子集匹配**（[源码](https://github.com/home-assistant/core/blob/dev/homeassistant/components/homeassistant/triggers/event.py)：事件里多出来的键不影响）。
- **触发 id**：每个触发可以有 `id`，动作里能读 `trigger.*`。
- **条件**：state、numeric_state、time、template、and / or / not。
- **重复触发怎么办**（[modes](https://www.home-assistant.io/docs/automation/modes/)）：
  - `single`（默认）：正在跑时新的触发忽略，记一条“Already running”；
  - `restart`：停掉正在跑的（包括等待中的）重新开始；
  - `queued`：排队；
  - `parallel`：同时跑；
  - `max` 默认 10。

## 四、AWS EventBridge 的匹配规则（[语法](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-create-pattern.html)）

- 模式和事件同一个形状，没写的字段不管；值写成数组，数组里是“或”；不同字段之间是“且”；字符串精确匹配、区分大小写。
- 运算（[operators](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-create-pattern-operators.html)）：`prefix`、`suffix`、`equals-ignore-case`、`anything-but`、`numeric`（`[">", 0, "<=", 5]`）、`exists`、`wildcard`、`$or`。

## 五、CloudEvents 1.0（[规范](https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md)）

- **必填**：`id`（同一个 `source` 里唯一）、`source`、`specversion`、`type`（建议用反向域名前缀，例如 `com.github.pull_request.opened`）。
- **可选**：`subject`（不解析 `data` 也能按它过滤）、`time`、`datacontenttype`、`dataschema`、`data`。
- **扩展字段**：名字小写字母数字，20 个字符以内。

## 六、Playnite

- **游戏事件**（[events](https://api.playnite.link/docs/tutorials/extensions/events.html)）：`OnGameStarting`（可以取消启动）、`OnGameStarted`、`OnGameStopped`（带 `ElapsedSeconds`）、`OnGameStartupCancelled`。
- **怎么判断游戏在跑**（[game actions](https://api.playnite.link/docs/manual/library/games/gameActions.html)）：
  - 默认：自动选；
  - 进程：启动的进程或它的任何子进程；
  - 只看原始进程；
  - 文件夹：这个目录里的任何进程，有初始等待和轮询间隔；
  - 进程名。

## 七、OpenClaw 和 Hermes：事件怎么交给 Agent

- **OpenClaw**（[webhooks](https://docs.openclaw.ai/automation/cron-jobs/webhooks)，默认关，单独的 Bearer 令牌）：
  - `POST /hooks/wake {text}`：往**主会话**里放一条系统事件。文档说只放你自己控制的短通知。
  - `POST /hooks/agent {message, deliver, …}`：在**独立会话**里跑一轮，完成后默认把结果交回主会话。
  - `hooks.mappings` 把任意事件转换成上面两种之一。
  - “外部事件的内容只当数据。”
- **Hermes**（[webhooks](https://hermes-agent.nousresearch.com/docs/user-guide/messaging/webhooks)）：
  - 每条路由有 `events`（按类型过滤，空 = 全部）、`prompt`（`{点.分隔}` 模板）、`skills`、`deliver`；
  - 每个事件一个新会话；
  - `coalesce{key, window_seconds: 30, max_wait_seconds: 300}` 把一阵突发合成一次，用最新的那份；
  - 每条路由每分钟最多 30 次；同一个投递 id 1 小时内去重。

## 八、其他

- **SteelSeries GameSense**（[发送事件](https://github.com/SteelSeries/gamesense-sdk/blob/master/doc/api/sending-game-events.md)）：
  - 事件格式 `{game, event, data: {value, frame}}`，事件可以预先注册（`register_game_event`）；
  - 15 秒没有事件就当游戏不在了，用 `game_heartbeat` 保活；
  - 值和缓存的一样时不处理。
- **LeagueAkari**：没有通用事件总线，每个模块是 MobX 状态，自动化用 `reaction()` 跟着状态变——“状态优先”。
- Discord 活动、Razer Chroma 是输出接口，没有游戏事件输入模型，不用参考。
