# Kotone 插件重构计划（ADR 038）

状态：进行中（2026-11-24 起，分支 `claude/plugins`）。方向见 [ADR 038](adr/038-kotone-and-apps.md)（Accepted）。每一步的实际情况写在第九节“实现记录”。

**到哪了（2026-11-24 第四轮会话结束时）**：第 1–4 步做完（英雄联盟、游戏语音、音乐、同传是插件；浏览器和 computer use 按讨论留在本体；左侧栏不放插件）。第 5 步停在“在本体里按职责拆开”，第 6 步停在两次“只搬不改”：再往下都要玩家先定几件看得见的事（见第九节第 5、6 步的“下一步”）。第 7 步做了插件边界的测试和设置里的“浏览器”一节；英雄联盟的事件目录由插件登记、设置页只剩本体这两件跟着第 5、6 步走。

玩家定了方向，细节由 Claude 按 ADR 038 的原则定。每一步做完向玩家报两件事：本体删了多少行，玩家能看到什么变化。

## 一、要达到什么

### 玩家能看到的

- **设置页只剩本体的几页**：她（角色、声音、怎么回答）、听、看、游戏列表、模型和连接、权限、自动化、插件、关于。每个插件的设置都在它自己的页面里。
- **左侧栏有一个“插件”页**：每个插件一张 App 卡片（照 MCP Apps，卡片上有开关），点卡片进这个插件的 App，整块内容区都是它的、用它自己的界面风格，不叫她也能用；以后第三方的 MCP Apps 也是这里的一张卡片（2026-11-24 玩家定的，见 ADR 038“怎么定的”）。英雄联盟插件像 LeagueAkari 加 OP.GG / aramhex。
- **聊天里出现插件页面的卡片**：例如战绩、英雄克制、正在放的歌。卡片是她那一刻看到的样子，点开在右侧栏显示完整页面，旁边还能接着和她聊。
- **游戏上方的覆盖层**：例如海斗三选一时，评分直接贴在三张海克斯卡片上（照 aramhex）；选英雄时显示胜率。
- **插件能单独开关**：关掉一个，它的页面、她的相关工具、它的游戏事件都一起没了。

### 她能感觉到的

- 工具清单变短，按插件分组。她要做的只是调用工具，或者把你带到对的页面（`ui.open`），不用自己拼界面。
- 能力概览按插件列，关着的插件不列。

### 代码上的（可以量）

基线（2026-11-24，`420d9b20`）：

| 量 | 现在 |
| --- | --- |
| 桌面端后端 | 72 个文件、29,002 行 |
| 设置页 | 53 个 Svelte 文件、11,124 行 |
| 中文文案 | 2,379 条 |
| 她的主会话工具 | 35 个 |
| 工具服务的工具 | 72 个 |
| 改一个英雄联盟工具要动的地方 | 8 处：`crates/kotone-lol`、桌面端 `lol.rs`、`kotone-agent/src/lol.rs`、`catalog.rs`、`tool_config.rs`、游戏页、文案、e2e |

目标：

- **加一个功能只改一个目录**（`plugins/<id>/`），工具只定义一次：她的清单、工具服务、MCP 都从插件清单读。
- **本体变轻**。预计搬出本体的代码：

  | 现在的位置 | 行数 | 搬到 |
  | --- | --- | --- |
  | 桌面端 `lol.rs` + `crates/kotone-lol` | 866 + 2,567 | 英雄联盟 |
  | 桌面端 `music.rs`、`music_cmd.rs` + `crates/kotone-music` | 814 + 3,468 | 音乐 |
  | 桌面端 `voice_chat.rs` | 481 | 游戏语音 |
  | 桌面端 `voice_listen.rs` | 207 | 同传 |
  | 桌面端 `automation.rs` 里程序的部分 + `crates/kotone-script` | 约 500 + 1,819 | 按键程序 |
  | 桌面端 `overlay*.rs`、`runtime.rs` 和 `kotone-core/orchestrator.rs` 里语音输入的流程、`inject.rs`、`crates/kotone-postprocess` | 约 3,000 + 1,657 | 语音输入 |

  桌面端后端大约少 9,000 行（三成），本体 crate 里约 9,500 行搬进插件目录。设置页会搬走的有：游戏页里英雄联盟和控制台通道那段、音乐页、文字处理页、语音输入那一组。

## 二、本体（摘自 ADR 038 D2）

| 本体 | 包括 |
| --- | --- |
| 她 | 角色卡、记忆（含复盘）、对话（主会话、后台）、技能、联网搜索、提醒 |
| 感官和表达 | 耳朵（麦克风、呼叫、唤醒、语音识别、热词）；眼睛（看屏幕、文字识别、读游戏聊天）；游戏列表、游戏配置、游戏中心；嘴（语音、气泡、弹幕）；**手**（computer use、内置浏览器，2026-11-24 改：原来分在“按键程序”和“浏览器”插件，见 ADR 038“怎么定的”） |
| 自动化 | 触发（手动、她调用、热键、定时、游戏事件）和动作（说一句、让她做、调用插件的工具） |
| 宿主 | 插件的清单、开关、权限、她的工具清单、页面显示、对外接口（工具服务、MCP、CLI）、模型连接和用量、诊断、更新 |

**插件用本体的能力，不自己再做一份**：听、看、说、调她都由本体给。语音识别只有一份。

### 游戏配置怎么分（2026-11-24 补）

游戏配置（`GameProfile`）留在本体，变瘦。现在的字段分给这几处：

| 字段 | 归谁 |
| --- | --- |
| `id`、`display_name`、`icon`、`process_names`、`window_title_patterns`、`launch_path` | 本体：游戏列表（游戏中心靠它认出在玩什么） |
| `hotwords`、`removed_builtin_hotwords` | 本体：耳朵（语音识别） |
| `companion_notes` | 本体：她（游戏说明写进提示词） |
| `chat_region` | 本体：眼睛（读聊天的截图区域） |
| `open_chat_key`、`send_key`、`pre_open_delay_ms`、`pre_paste_delay_ms`、`pre_send_delay_ms`、`prefer_clipboard_paste`、`channels`、`console_chat` | 留在游戏配置（本体）：往游戏聊天里打字归本体，琴音的 `game_chat.send` 和语音输入插件都用（2026-11-25 改，[ADR 039](adr/039-desktop-companion-and-game-apps.md) D2；原来要搬进语音输入插件的 `plugins.voice-input.games.<游戏>`，作废） |
| `allow_automation_input` | 留在游戏配置（本体）：这个游戏容不容忍模拟输入是游戏自己的事（D3 第 9 条），按键程序插件经宿主读（2026-11-24 改，原来要搬进按键程序插件） |
| `computer_use`（让她操作：询问 / 一直允许 / 禁止） | 本体：她的“手”（computer use 留在本体，2026-11-24 改） |
| （设置里的 `lol.enabled`） | 英雄联盟插件：`plugins.lol.sources.client` |

- 内置的 lol、dota2、cs2 是游戏配置，不是插件。英雄联盟插件用 `games = ["lol"]` 挂在 lol 上，以后 CS2、Dota 2 插件也一样。
- 游戏配置页：上面是本体的那几项（含聊天键、频道和发送时序），下面是各插件提供的小节（按键程序的允许操作、英雄联盟的数据开关）。照 VS Code 的“按语言的设置”。聊天键和频道原来算语音输入的小节，2026-11-25 改成本体的（ADR 039 D2）。
- 游戏配置包（导入导出 .zip）要连同插件的这些小节一起带上。
- 迁移：老的游戏配置文件读得进来，插件的字段在第一次保存时搬到插件的设置里（照兼容规则）。哪一步搬哪些字段：按键程序的在第 6 步，`lol.enabled` 在第 1 步。语音输入的聊天键和频道不搬了（留在游戏配置，原计划作废，ADR 039 D2）。

## 三、插件清单

每个插件都能**自主运行**：有自己的页面和开关，不叫她也能用；不依赖别的插件，关掉别的照样能用。它们跑在 Kotone 里，用本体的能力（第八节）。

### 1. 英雄联盟（`lol`）

像 LeagueAkari 加 OP.GG / aramhex：查这局 10 个人、看任何一个人的战绩、点进任何一局；英雄强度、克制、出装、海斗海克斯；选英雄和三选一时贴在游戏上的胜率和评分。原来打算写的 ADR 037（数据类工具）并进这里。

**为什么是一个插件**（玩家 2026-11-24 同意合并）：最好用的功能都要同时用到客户端数据和网上的统计。例如选英雄时给已选的英雄标胜率、三选一按这局的英雄打分、阵亡时按对面的装备说出装。分成两个插件的话，这些要么只能经过她（每次一次 AI 调用，不叫她就用不了），要么得打破“插件之间不直接调用”。LeagueAkari 自己就带 OP.GG 和腾讯的数据，aramhex 也是一个软件里既有选英雄胜率又有海克斯评分。数据来源和风险不一样的问题，用插件里的两个开关解决。

| 项 | 内容 |
| --- | --- |
| 页面（标签页） | 这局（10 个人、段位、近况、标签、隐藏战绩、开黑、突破口、召唤师技能计时）、战绩（任何玩家的最近战绩、段位、常玩英雄、按模式筛，点进单局详情）、英雄（强度榜：峡谷各路、大乱斗、海斗；英雄详情：克制、出装、符文、加点、召唤师技能、搭配、对线技巧）、海克斯（海斗英雄 × 海克斯胜率）、设置 |
| 覆盖层页面 | 海斗三选一：评分贴在三张海克斯卡片上；选英雄：自己和队友的英雄胜率、评级；对局中：角上的按钮，鼠标移过去才出现，点开看出装和加点。都照 [aramhex](https://github.com/czgreat/aramhex)（MIT） |
| 两个数据开关（各带风险提示） | **读客户端数据**（默认开）：本机的客户端接口（LCU）和局内接口（2999），只读；风险见 ADR 035 D5（国服可能违反腾讯协议，Riot 限制冷却计时）。**用网上的统计**（默认开）：腾讯 101 / 掌盟（国服的峡谷、大乱斗、海斗，首选）、czgreat/aramhex-data（海斗英雄 × 海克斯，CC BY-NC，页面上署名）、腾讯 `hero_list.js` 的外号；接口没有文档、随时可能改，照 LeagueAkari 容错解析。OP.GG 官方 MCP（全球数据和对线指南）：玩家 2026-11-24 同意替他去 opgginc/opgg-mcp 提 issue 问（照 Glyph 的 #17：英文、说明是不收费的开源项目、怎么用、会显示 OP.GG 的标志和出处；不写玩家的个人信息）。OP.GG 回复同意前，接入先做好但默认关 |
| 操作客户端（第三个开关，默认关，带封号风险提示） | 照 LeagueAkari（MIT 的 `dev` 分支）：自动接受对局、自动选 / 禁英雄（按玩家设的顺序）、其他 LeagueAkari 有的客户端操作按需加。玩家 2026-11-24：“操作英雄联盟客户端好像 LeagueAkari 也干了……读内存确实是不应该。”风险提示写上 LeagueAkari #523（外服有人用了自动接受、自动选英雄后被永久封号）。读和写都只走客户端的本机接口（LCU），不读游戏内存、不注入。给她一个工具（例如 `lol.client`：接受对局、选 / 禁某个英雄），玩家说了才用 |
| 只关一个开关时 | 只关“读客户端数据”：战绩和这局用不了，统计照样能用；选英雄、三选一的覆盖层退回只靠截图识别（和 aramhex 一样）。只关“用网上的统计”：战绩照样能用，英雄和海克斯页、评分用不了 |
| 怎么知道该显示覆盖层 | 选英雄阶段：客户端接口给己方英雄（关了就截图识别）。选海克斯的时刻：接口里没有，靠本体的截图和文字识别；插件说认哪块区域、认什么，本体去截、去识别。只在需要的阶段、插件开着时才看 |
| 给她的工具 | `lol.match`、`lol.spell`、`lol.scout`；新加 `lol.player`（某个人的战绩）、`lol.champion`（一个英雄的强度、克制、出装或海克斯）、`lol.tier`（强度榜）。都带页面 |
| 只给页面用的 | 翻战绩、单局详情、按名字搜玩家、刷新、切模式 / 分段 / 版本、搜英雄、外号对英雄 |
| 游戏中心适配器 | `lol`（阶段、局内事件、我阵亡 / 复活，ADR 036） |
| 用本体的 | 游戏中心、截图和文字识别、覆盖层、她（突破口由工具算，她来说） |
| 插件里面的模块 | `client`（客户端和局内接口、战绩、突破口、技能计时）和 `stats`（网上的统计、缓存、署名），各自测试；页面和覆盖层把两边合起来 |
| 现在的代码 | `crates/kotone-lol`、桌面端 `lol.rs`、`kotone-agent/src/lol.rs`、游戏页里的开关；统计部分是新的（[调研](research/2026-11-23-lol-data-sources.md)） |

### 2. 语音输入（`voice-input`）

| 项 | 内容 |
| --- | --- |
| 页面 | 语音输入热键（按住说 / 按一下开关）、悬浮窗、文字处理（翻译、润色、屏蔽词）、识别记录。每个游戏的聊天键、频道和控制台通道留在游戏配置（2026-11-25 改，ADR 039 D2） |
| 给她的工具 | 没有。`game_chat.send`（她替你发言）是本体的工具，名字不变，关掉语音输入也照样能用（2026-11-25 改，ADR 039 D2；原来由这个插件登记） |
| 只给页面用的 | 试听、测试打字、文字处理试一下 |
| 用本体的 | “听一句”（录音、语音识别、热词）；往游戏聊天里打字（注入器、ADR 032 的挡键和控制台通道）；游戏配置（哪个游戏、聊天键） |
| 现在的代码 | `orchestrator.rs` 里语音输入的流程、`overlay*.rs`、`runtime.rs` 里语音输入的部分、`crates/kotone-postprocess`。`inject.rs`、ADR 032 的挡键和控制台通道属于本体（ADR 039 D2） |

### 3. 游戏语音（`game-voice`）

| 项 | 内容 |
| --- | --- |
| 页面 | 给队友的虚拟麦克风（VB-CABLE）、路由、测试 |
| 给她的工具 | `voice_chat.say` |
| 只给页面用的 | 安装、测试、设成默认麦克风、状态 |
| 用本体的 | 嘴（语音合成） |
| 现在的代码 | 桌面端 `voice_chat.rs`、`voice_chat.*` 工具（ADR 020） |

### 4. 同传（`interpreter`）

| 项 | 内容 |
| --- | --- |
| 页面 | 听哪个程序、译成什么、最近译了什么 |
| 给她的工具 | 开始 / 停下 |
| 用本体的 | 耳朵（只听某个程序 + 语音识别）、嘴（弹幕） |
| 现在的代码 | 桌面端 `voice_listen.rs`、`voice.listen.*`（ADR 021） |

### 5. 音乐（`music`）

| 项 | 内容 |
| --- | --- |
| 页面 | 正在放、每日推荐、歌单、搜索、登录 |
| 给她的工具 | `music` |
| 只给页面用的 | 扫码登录、翻歌单、播放队列、音量 |
| 数据来源 | 网易云（非官方接口，ADR 034） |
| 现在的代码 | `crates/kotone-music`、桌面端 `music.rs`、`music_cmd.rs`、音乐页 |

### 6. 按键程序（`key-programs`）

| 项 | 内容 |
| --- | --- |
| 页面 | 程序编辑、内置的宏（连发、连点、按住、连招、定时喊话、看到文字 / 颜色就按）、每个游戏允不允许程序按键 |
| 给她的工具 | 运行程序 |
| 只给页面用的 | 编辑器里的校验、存模板图片 |
| 用本体的 | 眼睛（截图、文字识别）；自动化（程序是自动化的一种动作） |
| 现在的代码 | `crates/kotone-script`、桌面端 `automation.rs` 里程序的部分（ADR 013）。`computer_use.rs`（ADR 019）留在本体 |

### ~~7. 浏览器~~（2026-11-24 改：留在本体）

内置浏览器（右侧栏的网页标签、给后台的 `browser.*`，ADR 031）和 computer use 一样是她的“手”，留在本体。它也是玩家自己会用的完整浏览器，留在本体的理由是：它和覆盖层一样是本体给的显示面，代码是主窗口里的网页视图、离不开 Tauri，搬成插件一行也隔离不了。设置里给“浏览器”一节（开关默认开、扩展、清除登录、同意过的网站）。讨论经过和玩家的话见 ADR 038“怎么定的”。

### 以后

- CS2、Dota 2：GSI，默认写配置文件（ADR 036 D9），各自带游戏中心适配器。
- 规则：**一个游戏一个插件**；同一个游戏里不同的数据来源，用插件里的开关和风险提示分开。别的游戏照英雄联盟的样子加。

## 四、契约

### 清单（`plugins/<id>/plugin.toml`）

```toml
id = "lol"
name = "英雄联盟"
description = "这局和任何玩家的战绩、英雄强度和克制、海斗海克斯，选英雄和三选一时贴在游戏上的胜率"
games = ["lol"]                 # 挂在哪些游戏配置上；不写 = 所有游戏
adapter = "lol"                 # 游戏中心适配器
enabled_by_default = true

[[sources]]                     # 插件里的数据开关，各带风险提示
id = "client"
name = "读客户端数据"
enabled_by_default = true
risk = "国服用这类功能可能违反腾讯的用户协议……"

[[sources]]
id = "stats"
name = "用网上的统计"
enabled_by_default = true
risk = "数据来自腾讯和 aramhex 的非官方接口，可能随时变；aramhex 的数据要署名、不能商用……"

[[pages]]
uri = "ui://kotone/lol/match"
title = "这局"
needs = ["client"]

[[pages]]
uri = "ui://kotone/lol/champion"
title = "英雄"
needs = ["stats"]

[[overlays]]
uri = "ui://kotone/lol/augment-scores"
title = "海克斯评分"
needs = ["stats"]

[[tools]]
name = "lol.scout"
visibility = ["model", "app"]   # 照 MCP Apps：model = 给她，app = 给页面
page = "ui://kotone/lol/match"
needs = ["client"]

[[events]]                      # 游戏中心的事（ADR 036 D4；2026-11-25 第 7 步加的，事件只在这里登记）
type = "lol.dragon_kill"        # <适配器>.<名字>
label = "拿了小龙"
needs = ["client"]
data.team = "哪一边：ally（我方）/ enemy（对面）"
```

工具的说明和参数（JSON Schema）写在插件的 Rust 代码里，和现在 `kotone-agent/src/lol.rs` 一样，清单只引用名字。是不是全放进清单，第一步实测时再定。`needs` 写这个页面 / 工具要哪个数据开关；开关关了，它就不出现（她的工具清单照样只在开关变时变）。

### `kotone-plugin-api`（新 crate，宿主和插件之间唯一的缝）

- **插件实现**：
  - `manifest()`；
  - `tools()`：名字、说明、参数、`visibility`、副作用、结果是不是不可信数据；
  - `call(host, tool, args) -> ToolResult`；
  - `start(host)` / `stop()`：后台循环，例如英雄联盟插件每 2 秒看客户端。
- **`ToolResult`**（照 MCP 工具结果）：
  - `content`：给她的文字；
  - `structured_content`：给页面的数据；
  - `ui`：页面地址和参数。
  - 只有 `content` 进她的上下文；`structured_content` 存进对话记录，聊天卡片用它。
- **宿主给插件（`Host`）**：
  - 开口说、弹幕；
  - 自己那份设置（`plugins.<id>`）和存储目录；
  - 凭据库（自己的命名空间）；
  - 往游戏中心报阶段和事件；
  - “听一句”（录音 + 语音识别）；
  - 截图和文字识别；
  - 给界面发事件。
- **不依赖 Tauri**：插件 crate 只依赖 `kotone-plugin-api`、`kotone-core` 的纯数据类型和自己的第三方库。

### 页面

- 插件的 `ui/` 下是 Svelte 组件，按 `ui://` 地址登记。
- **三个显示位置**，都由本体提供：
  - 聊天卡片：小、只读快照；
  - 右侧栏（或从侧栏打开的完整页面）；
  - **游戏上方的覆盖层**：盖在游戏窗口上，贴在某个角或者某块区域上（例如识别出的三张海克斯卡片）。MCP Apps / OpenAI 有“嵌在对话里、全屏、画中画”几种显示方式，覆盖层是 Kotone 多加的一种。
- **覆盖层只有一个，在本体里**：Kotone 本来就有气泡、弹幕、语音输入的悬浮窗这些盖在游戏上的窗口。难的部分只写一次：置顶、点击穿透、无边框游戏（独占全屏显示不了，和 aramhex 一样要玩家改无边框）、多屏和缩放、性能（掉帧、黑闪，aramhex 专门写了排查文档）、截图识别时遮掉自己画的东西。插件只说放什么、放在哪、什么时候显示。
- 每个页面拿到两样东西：`structuredContent` 和参数。
- 宿主给页面的桥照 MCP Apps：
  - `callTool(name, args)`：调本插件的工具，只给页面用的也能调；
  - `openPage(uri, params)`：在页面之间跳；
  - 主题和语言。
- 同一个页面两种尺寸：聊天卡片（小、只读快照，点开）和完整页面（右侧栏或从侧栏打开，能操作、会刷新）。

### 宿主

- **插件注册**：一期是内置插件的静态列表，编译在一起。
- **她的工具清单**：开着的插件里标了 `model` 的工具，一段对话里不变（前缀缓存，ADR 024）。
- **工具服务和 MCP**：开着的插件的全部工具，带 `visibility`。
- **新工具 `ui.open {page, params}`**：她和外部 Agent 打开任意页面，本体的设置页也有地址。
- **权限**：照旧按工具名。
- **文案**：插件的 `messages/<语言>.json` 并进 Paraglide。`project.inlang` 能不能列多个目录，第一步实测；不行就构建时拷进来。
- **测试**：
  - 插件 crate 自己的单测；
  - `plugins/<id>/e2e/` 由桌面端的 Playwright 一起跑；
  - 用词测试也查插件的文案。
- **配置迁移**：插件的设置放到 `plugins.<id>`，例如 `lol.enabled` → `plugins.lol.sources.client`。照兼容规则，旧配置读得进来，第一次保存时搬过去。

## 五、目录

```
crates/          本体：kotone-core、kotone-agent / -ai、kotone-stt / -tts / -ocr / -platform-windows、
                 kotone-plugin-api（新）、kotone-tools / -cli
apps/desktop/    宿主：窗口、聊天、侧栏、本体设置、插件注册
plugins/<id>/    plugin.toml、Cargo.toml + src/、ui/、messages/、skills/、e2e/、README.md
```

## 六、步骤和验收

在新分支 `claude/plugins` 上做，从 `claude/adr-035-lol-tools`（或合并后的 main）分出。每一步：

- 能单独编译、单独提交；
- 相关的测试、clippy、Playwright 全过；
- `lol_tools_real.py`、`game_events_real.py` 照样过；
- 报“本体删了多少行、你能看到什么变化”，附截图。

| 步 | 做什么 | 玩家能看到 | 验收 |
| --- | --- | --- | --- |
| 1 | `kotone-plugin-api`；插件注册；英雄联盟现有的三个工具搬进 `plugins/lol/`，当第一个用户 | 设置里多一页“插件”（列表、开关），侧栏多一个“插件”分组；其余不变 | 三个工具行为不变（两个真机脚本过）；工具只在一个地方定义 |
| 2 | 页面和卡片：`ui.open`、工具结果的 `structured_content` / `ui`、聊天卡片、右侧栏打开插件页面；英雄联盟的这局、战绩（含单局详情）两个页面 | 问“查一下这把的战绩”，聊天里出现一张战绩卡片，点开在右侧栏，能点进玩家和单局 | Playwright 覆盖卡片和跳转；真机看一局 |
| 3 | 英雄联盟插件的统计部分（新功能）：腾讯 101、aramhex、外号，英雄和海克斯页；覆盖层（本体）和三个覆盖层页面；去 opgginc/opgg-mcp 提 issue，OP.GG 的接入做好但默认关 | “亚索克制谁”出卡片；侧栏能直接查英雄；海斗三选一时评分贴在卡片上 | 接口容错的单测；页面上有署名；覆盖层掉帧测试（照 aramhex） |
| 3.5 | 英雄联盟插件的“操作客户端”（新功能，照 LeagueAkari）：自动接受对局、自动选 / 禁英雄、`lol.client` 工具；默认关，带风险提示 | 英雄联盟插件的设置里多一个“操作客户端”开关和几项偏好 | 用假客户端（`lol_tools_real.py` 那个 HTTPS 小服务加上写接口）测：发了对的请求、开关关着时什么都不发 |
| 4 | 音乐、游戏语音、同传搬进插件（浏览器 2026-11-24 改成留在本体） | 音乐页搬到“插件”分组；设置里少了游戏语音 | 原有测试照过 |
| 5 | 语音输入搬进插件（最大的一块） | 语音输入那一组设置搬进插件页面 | 语音输入、她替你发言、控制台通道的真机测试照过 |
| 6 | 按键程序搬进插件（computer use 2026-11-24 改成留在本体） | 自动化里写程序的编辑器由插件提供 | 自动化的真机测试照过 |
| 7 | 收尾：设置页只剩本体；配置迁移收口；英雄联盟的事件目录由插件登记；设置里的“浏览器”一节（开关、扩展、清除登录、同意过的网站，浏览器留在本体）；插件依赖的边界测试（不碰 Tauri、桌面端、她）；文档 | 设置页变成第一节说的样子 | 旧配置升级测试 |
| 以后 | `kotone.exe mcp` 提供 `ui://` 资源；显示别人的 MCP Apps（沙盒 iframe） | Claude Desktop 里能看到 Kotone 的战绩页 | |

## 七、风险和要留意的

- **语音输入最难搬**：它和 `orchestrator.rs`（3,573 行）缠在一起。搬的时候行为一点都不能变，放在后面做，先靠前面几步把契约磨好。
- **第 5 步按职责拆 `orchestrator.rs`，不是整个搬进插件**（2026-11-24 玩家问它为什么 3,000 多行）。它大，是因为本体的“耳朵”和语音输入的流程长在一个文件里：

  | 现在的部分 | 大约行数 | 拆到 |
  | --- | --- | --- |
  | 会话状态机：热键（按一下 / 按住）、开始、结束、取消、代际和票据 | 900 | 本体“听一句” |
  | 录音和识别：`begin_locked`（一个函数 550 行：麦克风、静音检测、识别会话、降低扬声器音量） | 550 | 本体“听一句”，函数拆成几段 |
  | 她听你说话：呼叫、唤醒、聊天页的语音按钮 | 150 | 本体 |
  | 文字处理 | 120 | 语音输入插件 |
  | 打进聊天框：占住发送、等修饰键松开、执行、确认、重发、控制台通道 | 450 | 语音输入插件 |
  | 识别记录、保存录音 | 150 | 语音输入插件 |
  | 给界面发状态 | 100 | 两边各自 |
  | 单元测试 | 340 | 跟着各自的代码走 |

  目标：`orchestrator.rs` 拆没；本体的“听一句”大约 1,200 行；插件那边按发送、文字处理、记录、热键分成几个文件。拆之前先把 `crates/kotone-core/tests/orchestrator.rs` 的测试当成行为基准，拆完一个不改照样过。不按行数拆（上次拆 `agent.rs`、`lib.rs` 是纯搬家；这个文件状态缠在一起，只有按职责拆开才有意义）。
- **页面放在 `apps/desktop` 外面**：Vite 要能从 `plugins/` 引 Svelte 组件，依赖也要解析得到（别名或 pnpm workspace），第一步实测。
- **前缀缓存**：她的工具清单只在开关变的时候变，不跟着现在在玩的游戏变。
- **配置兼容**：老配置里的字段（`lol.enabled`、`voiceChat`、文字处理……）都要读得进来。
- **和发版的关系**：这是大改动，在自己的分支上做，不挡 0.2.0 的试玩和 RC。
- **覆盖层影响游戏的流畅度**：要测掉帧和黑闪（照 aramhex 的[排查文档](https://github.com/czgreat/aramhex/blob/main/docs/display-performance.md)），只在需要时显示；截图识别要遮掉自己画的东西。
- **以后的选项：每个插件一个进程**（一个 MCP 服务器）。好处是隔离、第三方插件、崩了不拖垮她；代价是进程间通信和打包。一期都在一个进程里。

## 八、“自主运行”是什么意思

- **能**：
  - 每个插件不叫她也能用（打开页面）；
  - 能单独开关；
  - 不依赖别的插件；
  - 外部 Agent 能单独调用它的工具。
- **不是**：单独的程序。插件跑在 Kotone 里，用本体的能力（听、看、说、游戏中心）。
- **以后**：通过 `kotone.exe mcp`，别的宿主（Claude Desktop 等）也能用某个插件、显示它的页面，前提是 Kotone 开着。

## 九、实现记录

基线：分支起点 `c8c0b8b4`（main，2026-11-24）。桌面端后端 72 个文件 29,002 行；`crates/` 108,336 行；设置页（`routes/settings` 下的 Svelte）12,420 行；中文文案 2,387 条。

### 第 1 步：契约、插件注册、英雄联盟搬进 `plugins/lol/`（2026-11-24）

**玩家能看到的**：

- 左侧栏多了一组“插件”，里面是“英雄联盟”；点开是英雄联盟插件的页面（这一步只有“设置”：插件开关、“读客户端数据”开关和风险提示）。截图：`target/plugin-shots/step1/sidebar-plugins-group.png`、`lol-plugin-page.png`。
- 设置 → 通用里多了“插件”：每个插件一张卡片（名字、说明、开关、数据开关和风险提示、“打开”）。截图：`target/plugin-shots/step1/settings-plugins.png`。
- 游戏列表 → 英雄联盟：原来的“让她读这局的数据”变成英雄联盟插件提供的一节（同一个开关，改的是 `plugins.lol.sources.client`），旁边有“打开英雄联盟插件”。
- 其余不变：她用这三个工具的方式、说的话一样（工具说明、参数、给她的那段“工作方式”逐字搬过去）。

**本体少了多少**：

| 量 | 之前 | 之后 |
| --- | --- | --- |
| 桌面端后端 | 72 个文件 29,002 行 | 73 个文件 28,520 行（删了 `lol.rs` 866 行，加了宿主 `plugins.rs` + `plugins_cmd.rs` 362 行） |
| `crates/`（本体） | 108,336 行 | 106,547 行（`kotone-lol` 2,704 行搬出去；新的 `kotone-plugin-api` 576 行，含测试） |
| `kotone-agent` 里写死的英雄联盟 | `lol.rs` 81 行 + `catalog.rs` / `prompt.rs` 里 3 处名单和一段提示 | 0（工具清单、能力概览、“工作方式”都从开着的插件来） |
| 改一个英雄联盟工具要动的地方 | 8 处 | `plugins/lol/` 一个目录（工具定义在 `src/tools.rs`，实现在 `src/watcher.rs`，给她的话在 `plugin.toml`，界面在 `ui/`，文案在 `messages/`） |

`plugins/lol/`：Rust 3,767 行（其中 `src/client/` 是原来的 `kotone-lol`），界面 100 行，文案 12 条。

**定下来的细节**（ADR 038 D5 说“第一步实测时再定”的）：

- **工具定义在插件的 Rust 代码里，不在 `plugin.toml`**：照 MCP 的 `tools/list`，`visibility`、页面（`resourceUri`）、要的数据开关（`needs`）、“马上返回”（`instant`，主会话也拿）、“结果不可信”和说明、参数写在一起（`ToolSpec`）。`plugin.toml` 只写插件这一级的：名字、说明、挂在哪些游戏上、适配器、数据开关和风险提示、页面、覆盖层、给她的两段话（`[agent] overview / rule`）。清单和代码对不上由 `Manifest::check_tools` 在测试和启动时查。
- **她的工具清单**：宿主按开关算好每个插件给她的东西（`Offer`），`Catalog::build` 拼进来；一段对话开始时定下，之后不变（ADR 024）。插件关了或数据开关关了，下一段对话起工具就没了；一段对话中途关掉，调用时插件自己回答“已关闭”。
- **工具服务、MCP**：开着的插件的工具列在 `system.capabilities` 里，带 `plugin`、`untrustedResult`、`_meta.ui`；调用经 `AppHost::invoke` 交给插件，结果照 MCP（`content` / `structuredContent` / `_meta.ui`）。`kotone.exe mcp` 不用改就有了。外部 Agent 的说明在 `docs/tool-service.md`“插件”一节。
- **宿主给插件的**（`Host`）：插件开关、数据开关、自己的设置和目录、后台任务、按名字找进程和读命令行（英雄联盟的端口和令牌在这里）、往游戏中心报阶段和事件、给自己的页面发事件。插件不碰 Tauri，也不再直接依赖 `kotone-platform-windows`。（2026-11-24 改：插件可以用系统适配层里没有状态的系统调用，例如游戏语音的音频设备列表和重采样、语音输入的打字注入；全进程只有一份的钩子、停下她、免问只经宿主。见 ADR 038“怎么定的”。）
- **配置迁移**：`lol.enabled` 读进来时搬到 `plugins.lol.sources.client`（已有新字段的不覆盖），之后不再写 `lol`。装回旧版时旧版读不到 `lol`，按默认“开”。
- **文案**：插件的 `messages/<语言>.json` 加进 `project.inlang` 的 `pathPattern`（可以列别的目录，实测可行）；键名以 `plugin_<id>_` 开头；中文和 `plugin.toml` 的一致（测试查）。界面上的名字、开关、风险提示读文案，没有时用清单里的。
- **页面放在 `apps/desktop` 外面**：Vite 别名 `$plugins` → `plugins/`，`resolve.dedupe` 让 `plugins/*/ui` 里的裸 import（svelte、@lucide/svelte……）解析到桌面端的 `node_modules`；`tsconfig` 把 `plugins/*/ui` 一起交给 svelte-check；Playwright 也跑 `plugins/*/e2e/`；用词测试也查插件的文案。
- **没搬的**：英雄联盟的事件目录（`lol.dragon_kill` 等，自动化保存时校验用）还在 `kotone-core::games`；插件关着时这些事不再发生，但自动化编辑里还能选到。第 7 步收尾时改成由插件登记。

**检查**（都在 `claude/plugins` 上，提交前跑的）：

- Rust：`kotone-plugin-api` 5、`kotone-plugin-lol` 33、`kotone-agent` 77、`kotone-tools` 111、`kotone-core` 306 + 集成测试、桌面端 108，全过；全工作区 `cargo clippy --all-targets -D warnings`、`cargo fmt --check` 干净。
- 界面：svelte-check 0 错误 0 警告（也查 `plugins/*/ui`）；Playwright 148 个全过（两遍；插件的 3 个在 `plugins/lol/e2e/`）。有一遍 `copilot-flow.spec.ts` 第 8 个（设置说明里的每个位置都能打开）超时一次，单独跑 3 次、之后两遍全量都过，记成可能不稳。
- 真机（不开游戏，假客户端 + 真 DeepSeek，不发任何系统按键）：`lol_tools_real.py --while-playing` 17/17，`game_events_real.py` 11/11。她照旧用 `lol.scout` / `lol.spell` / `lol.match`，游戏中心的阶段、按事件运行的自动化、复盘都照旧。

**还没测**：真客户端、真对局（同 ADR 035、036 的“还没测”）；装回旧版后 `lol.enabled` 的表现。

### 第 2 步：页面和卡片（2026-11-24）

**玩家能看到的**：

- 问“查一下这把的战绩”，聊天里她那句话上面多一张“这局”卡片：对面 5 个人的近况、谁是突破口、我方一行。这是她查的那一刻的样子。截图：`target/plugin-shots/step2/chat-card.png`。
- 点卡片，右侧栏多一个标签（标题就是页面名），显示完整的“这局”：两边 10 个人的段位、近 20 局胜负、KDA、场均死、同模式分均伤害、标签、隐藏战绩、开黑、突破口和原因；“刷新”重新查。截图：`sidebar-match-page.png`。
- 点一个人进他的“战绩”：段位、常玩英雄、最近每一局；点一局进“单局详情”：两边 10 个人的 KDA、伤害、经济、补刀、装备、海克斯；左上角可以一层层返回。截图：`sidebar-player-page.png`、`sidebar-game-page.png`。
- 左侧栏的英雄联盟插件页面多了“这局”“战绩”两个标签（战绩里可以按“名字#编号”或这局里的名字搜），原来的“设置”还在。截图：`plugin-page-player-tab.png`。
- 说“我最近怎么样”“对面亚索最近战绩怎么样”，她用新工具 `lol.player`，聊天里出现那个人的战绩卡片；说“打开我的战绩”“打开插件设置”，她用 `ui.open` 在 Kotone 窗口里打开（不会把窗口拉到游戏前面）。

**本体变了多少**：这一步主要是加东西，本体没删代码。

| 量 | 第 1 步后 | 第 2 步后 |
| --- | --- | --- |
| 桌面端后端 | 28,520 行 | 28,639 行（`ui.open` 和三种调用者） |
| `kotone-ai` | — | 工具结果多一个 `display`（给界面的，不发给模型），约 20 行 |
| 宿主界面（`apps/desktop/src`） | — | 卡片、右侧栏的页面标签、页面栈、`ui.open`，约 600 行（新文件 5 个） |
| `plugins/lol` | Rust 3,767 行，界面 100 行 | Rust 4,324 行（页面数据 `pages.rs`、两个新工具），界面 654 行 |

**定下来的细节**：

- **工具结果怎么进对话记录**：`ToolOutcome` / `Message::Tool` 多一个 `display`（`structuredContent` 和 `ui`），存进对话、发给界面，**不发给模型**（有测试：DeepSeek 的请求体里没有页面数据）。她看到的还是原来那几行字。旧的对话记录没有这个字段，照样读得进来。
- **谁能调哪种工具**（`plugins::Caller`）：她只能调给她的（`model`）；插件自己的页面只能调自己插件给页面的（`app`）；外部 Agent 经工具服务 / MCP 两种都能调（清单里都列出来、带 `visibility`，照 MCP Apps 给宿主的那一份）。
- **`ui.open`** 是工具服务的工具（她和外部 Agent 共用），主会话也有。页面地址：插件的页面 `ui://kotone/<插件>/<页面>`（插件关着、或页面要的数据开关关着时报错），本体的设置页 `ui://kotone/settings/<分页>`。界面收到 `kotone://ui-open`：插件页面切到聊天页、在右侧栏打开；设置页跳到那一页。
- **页面的两种尺寸**（照 MCP Apps）：插件界面登记 `views`：每个 `ui://` 地址一个完整页面和一个可选的卡片。页面拿到 `{uri, params, data, bridge}`；`bridge.call(tool, args)` 调本插件的工具（`plugin_call`），`bridge.openPage(uri, params, data)` 在页面之间跳（右侧栏里压栈，可以返回）。`data` 为空时页面自己去查。
- **卡片是快照**：从卡片点开的页面先显示“这是她查的时候的样子”，点“刷新”才重新查。
- **英雄联盟的页面数据**（`plugins/lol/src/pages.rs`）和她拿到的文字用同一份统计（`scout::summarize`）。单局详情用客户端的 `/lol-match-history/v1/games/{id}`；按“名字#编号”找人用 `/lol-summoner/v1/alias/lookup`（**这个接口没在真客户端上试过**，假客户端里有）。

**检查**：

- Rust：`kotone-ai`（加了“页面数据不发给模型”）、`kotone-agent` 77、`kotone-plugin-api` 6、`kotone-plugin-lol` 37、`kotone-tools` 111、桌面端 109，全过；clippy `-D warnings`、fmt 干净。
- 界面：svelte-check 0 / 0；Playwright 151 个全过（新加 3 个：卡片 → 右侧栏 → 战绩 → 单局 → 返回；`ui.open` 打开插件页面和设置页、拒绝不存在的地址；插件页面的三个标签和搜索）。
- 真机（假客户端 + 真 DeepSeek，不按键）：`lol_tools_real.py --while-playing` 26/26（新加：`lol.scout` 的结果带这局的页面数据、页面数据没进她的上下文；她问一个人的战绩时用 `lol.player`；经工具服务调 `lol.player`、`lol.game`、`ui.open`），`game_events_real.py` 11/11。

**还没测**：真客户端里按“名字#编号”找人；真窗口里 `ui.open` 打开右侧栏（Playwright 用的是浏览器里的同一个处理函数）。右侧栏里页面的“关闭”和侧栏自己的“关闭”挨在一起、长得一样，以后改。

### 第 3 步：英雄联盟插件的统计、覆盖层，OP.GG 的 issue（2026-11-24）

**玩家能看到的**：

- 英雄联盟插件多了两个数据开关：**用网上的统计**（默认开）和 **用 OP.GG 的数据**（默认关），各带风险提示；OP.GG 的提示里有我们替你提的 issue 链接。截图：`target/plugin-shots/step3/settings-three-sources.png`。
- 插件页面多了“英雄”和“强度榜”两个标签（现在是 这局 / 战绩 / 英雄 / 强度榜 / 设置）：
  - 英雄：搜名字、称号或外号（“刀妹”“亚索”）；峡谷看这一路的强度、胜率、选用率、禁用率、难打 / 好打的英雄、出门装、鞋、核心三件、四到六件、符文、加点、召唤师技能；海斗看英雄榜名次、最搭的队友、最好的海克斯（比这个英雄平均胜率高几个百分点）。截图：`champion-rift-page.png`、`champion-mayhem-page.png`。
  - 强度榜：峡谷五路、海斗英雄榜，点英雄进英雄页。截图：`tier-page.png`。
  - 每一页都写着数据从哪来：腾讯 101（掌盟）和日期；海斗的海克斯数据写着 **github.com/czgreat/aramhex-data（CC BY-NC 4.0）**，能点开数据仓库。
- 问她“亚索克制谁”“刀妹出什么”“海斗里这个英雄拿什么海克斯”“现在上单什么强”，她用 `lol.champion` / `lol.tier`，聊天里出现英雄或强度榜的卡片，她只说一两句并带出处（真模型实测：“刀妹（艾瑞莉娅）现在上路 T3，胜率 50.3%（16.23 版本，掌盟数据）”）。
- **游戏上方的覆盖层**（第一次有）：
  - 选英雄时，客户端窗口右上角一个小面板：我方 5 个英雄的胜率（海斗是英雄榜名次和胜率，峡谷是那一路的强度和胜率），大乱斗 / 海斗还有备选席。截图：`overlay-champ-select.png`。
  - 海斗三选一时，每张海克斯卡片上面贴一个分：“+7.9 · 57.8% · 1.1万局”，三张里最好的那张描黄边，样本少的变淡。截图：`overlay-augment-scores.png`。
- 我们替你在 OP.GG 的仓库提了 issue：https://github.com/opgginc/opgg-mcp/issues/26 （英文，照 Glyph 的 #17：不收费的开源项目、按需请求、缓存 24 小时、`client_app` 写 kotone、显示 OP.GG 的标志和出处；没写你的任何个人信息）。OP.GG 回复同意前“用 OP.GG 的数据”默认关。

**本体变了多少**：这一步主要是插件里的新功能。本体加了覆盖层（`overlay_layer.rs` 247 行，含测试）、插件能用的“截一块识别文字”（`computer_use.rs` 里 40 行）和三项宿主能力；桌面端后端 28,639 → 29,024 行。插件：Rust 4,324 → 7,381 行（统计 `stats/`、工具 `stats_tools.rs`、覆盖层 `overlays.rs`），界面 654 → 1,427 行，文案 118 条。

**定下来的细节**：

- **数据来源**（`plugins/lol/src/stats/`）：
  - 腾讯 101（`mlol.qt.qq.com`）：解析照 LeagueAkari `dev` 分支（MIT）的 `qq101-protocol.ts` 逐字段移植，读不懂的跳过。实测和 Akari 不一样的两处：分路要写 `MIDDLE` / `BOTTOM`（`MID` / `ADC` 返回空）；海斗海克斯榜在字段 `R15381`。海斗英雄榜按天出，当天的常常是空的，往前找最近有数据的一天。
  - czgreat/aramhex-data：照它的 AGENTS.md，先读 `latest.json`，文件按 SHA-256 校验后存在插件目录下（按哈希命名，对不上的不用）；打分用 `(wins + 英雄胜率 × 300) / (games + 300)` 平滑，和这个英雄的平均胜率比。
  - 外号：腾讯 `hero_list.js` 的 `keywords`，补了 4 个它没有的（刀妹、德玛、EZ、VN）。装备、符文、召唤师技能的名字：腾讯 `items.js` / `rune_list.js` / `summonerskill_list.js`（客户端没开时也有名字）。
  - 缓存：内存 + 插件目录下的磁盘文件；101 存 6 小时，名字表 1 天；网络连不上时用磁盘上旧的，并标出来（`stale`）。
  - OP.GG 官方 MCP：Streamable HTTP（`initialize` → `notifications/initialized` → `tools/call`），`client_app` 写 `kotone/<版本>`，结果存 24 小时，只在打开英雄页 / 她查一个英雄、而且开关开着时才请求。**这一步没有真的调用 OP.GG**（提 issue 以后就没再请求，单测用的是录好的回复格式）。
- **覆盖层只有一个，在本体里**（`overlay_layer.rs`）：插件只给条目（`OverlayItem`：覆盖层页面、盖在哪个进程的窗口上、位置（一块区域或贴一个角）、数据）。本体：
  - 只在目标窗口在前台时显示，切走、最小化就藏起来，每秒跟一次位置；
  - **窗口只盖住条目的并集**，不盖满游戏：照 aramhex 的显示性能排查，盖在无边框游戏上的透明置顶窗口可能让游戏退出 Independent Flip、帧率被压到刷新率；窗口越小、显示的时间越短越好；
  - 不进截图（`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`），截图识别不会把自己画的认进去；
  - 第一次要显示时才建窗口（没人用覆盖层时不多一个网页进程）；点击穿透、不抢焦点。
- **截一块识别文字**（`Host::screen_text`）：只截前台是指定进程的窗口（前台是别的程序就不截），只用本地文字识别（在线识别要十几秒、要花钱，就不识别）；识别线程用“低于正常”的优先级（照 aramhex：不用最低，低配机器上会饿死）。
- **海克斯评分**：海斗对局里每 2 秒截一次画面中间那块，认出两个以上 aramhex 的海克斯名才算三选一（名字整行对上，或一行里包含 3 个字以上的名字，长名字优先）；卡片没了 4 秒收起。
- **选英雄面板**：数据来自客户端的选英雄接口（我方英雄、备选席）加网上的统计；位置先放在客户端窗口右上角（没在真客户端上看过会不会挡东西）。

**检查**：

- Rust：`kotone-plugin-lol` 51（新加：101 的解析、aramhex 的打分和校验、外号、名字表、OP.GG 的协议、认卡片和打分、覆盖层条目）、`kotone-plugin-api` 7、`kotone-agent` 77、`kotone-tools` 111、桌面端 110（新加：覆盖层窗口只盖并集）；clippy `-D warnings`、fmt 干净。
- 真接口（`cargo run -p kotone-plugin-lol --example stats_probe`）：腾讯 101、aramhex-data、外号、名字表都解析对了；第一次请求 101 约 0.15 秒、aramhex 三个文件约 3 秒（500 万局），之后从缓存来不到 1 毫秒。
- 界面：svelte-check 0 / 0；Playwright 156 个全过（插件 11 个，新加 5 个：英雄页峡谷 / 海斗和署名、强度榜点进英雄、数据开关和 OP.GG 默认关、覆盖层按位置画三张卡片的评分和选英雄面板）。
- 真机（假客户端 + 真 DeepSeek + 真网络，不按键）：`lol_tools_real.py --while-playing` 32/32（新加：她问“刀妹出什么装，克制谁”用 `lol.champion`、结果带英雄页和出处；经工具服务查强度榜、海斗英雄 × 海克斯带 CC BY-NC 署名、OP.GG 默认不请求），`game_events_real.py` 12/12（新加：选英雄阶段插件要了选英雄面板；录到的选英雄接口是 404，测试里用这局的人合成了一份）。

**还没测（要你开局）**：

- 覆盖层真的盖在客户端 / 游戏上：位置、大小、多屏和缩放、切走时藏起来；
- **掉帧**（照 aramhex 的测法）：用 PresentMon 看 Present Mode 和帧率，分别记“Kotone 没开”“覆盖层显示时”“覆盖层藏起来时”，看会不会从 `Hardware: Independent Flip` 变成 `Composed: Flip`、帧率被压到刷新率；有没有黑闪；
- 海斗三选一时真画面里认不认得出三张卡片（要装本地文字识别模型），识别的 CPU 占用；
- 选英雄面板的位置挡不挡东西；峡谷里的分路（`assignedPosition`）。

### 第 3.5 步：英雄联盟插件的“操作客户端”（2026-11-24）

**玩家能看到的**：

- 英雄联盟插件的“设置”里多了第四个数据开关 **操作客户端**（默认关），风险提示写着 LeagueAkari #523（外服有人用了自动接受、自动选英雄后被永久封号），能点开那个 issue。截图：`target/plugin-shots/step3.5/lol-control-settings.png`。
- 打开以后多一块偏好：**自动接受对局**（可以设等几秒，0–8 秒）、**自动选英雄**（按你排的顺序，挑第一个能选的；“选了就锁定”或只亮出来；轮到你以后等几秒）、**自动禁英雄**（按顺序，不禁队友想玩的）。
- 你说“帮我接受对局”“帮我选亚索”“禁掉劫”“先亮一下亚索”，她用 `lol.client` 去做；**默认要先问你一次**（这个工具的副作用是“会操作英雄联盟客户端”，权限默认询问）。开关关着时这个工具不出现。

**本体变了多少**：本体只加了一个权限分类（“会操作英雄联盟客户端”默认询问）和“打开页面”默认允许的修正（`f0227384`：第 2 步漏了，`ui.open` 按“未归类的副作用”默认要问；加了测试，工具服务里每个副作用都要归类）。插件：`control.rs`（规则和每一轮，约 470 行含测试）、`lol.client` 和只给页面用的 `lol.champions`、客户端接口的 POST / PATCH。

**定下来的细节**（照 LeagueAkari MIT `dev` 分支的 `auto-gameflow`、`auto-select`）：

- 接受对局：阶段是 `ReadyCheck`、自己还没回应（`playerResponse` = `None`）时，等你设的秒数，`POST /lol-matchmaking/v1/ready-check/accept`；你自己点了就不管。
- 选 / 禁：当前这一轮（第一组还有没做完的动作）里轮到自己、正在进行的那个动作；按顺序挑第一个能用的：客户端说能选 / 能禁（`pickable-champion-ids` / `bannable-champion-ids`）、没被禁、没被别人选；禁的时候不禁队友想玩的（`championPickIntent`）。`PATCH /lol-champ-select/v1/session/actions/{id}` `{championId, completed}`；禁英雄总是直接禁，选英雄看“选了就锁定”。每个动作只试一次（失败了不反复发）。
- 和 Akari 不同的地方：Akari 收客户端的事件（WebSocket）马上反应，我们每 2 秒看一次（和读阶段同一轮），所以“等几秒”实际会多 0–2 秒。确认对局一共约 10 秒，所以等待最多 8 秒。
- 只走客户端自己的本机接口（读和写），要“读客户端数据”也开着；不读游戏内存、不注入。偏好存在 `plugins.lol.settings`（`autoAccept`、`acceptDelaySeconds`、`pick`、`ban`）。

**检查**：

- Rust：`kotone-plugin-lol` 55（新加：轮到谁、挑哪个英雄、跳过被禁 / 被选 / 队友想玩的、锁不锁、准备确认、偏好的范围）、`kotone-agent` 78（新加：工具服务里每个副作用都归了类）；clippy、fmt 干净。
- 真机（假客户端，不开游戏、不按键、不用模型）：新脚本 `scripts/manual-e2e/lol_client_real.py` 13/13：
  - 开关关着（默认）、偏好全开：准备确认和两次轮到自己都过去了，**一个写请求都没有**，`lol.client` 不出现；
  - 开关开着：恰好一次接受、一次禁（157，锁定）、一次选（跳过客户端说不能选的，选 39 并锁定），没有别的写请求，日志写明做了什么；
  - 开关开着、自动的偏好关着：自己不动；经工具服务 `lol.client` 接受、亮出亚索（不锁定）、禁劫；准备确认过了再说接受会说“现在没有要确认的对局”，不写。
- 界面：svelte-check 0 / 0；Playwright 158 个全过（新加 2 个：操作客户端默认关、LeagueAkari #523 的链接看得到、打开后才有偏好；自动接受的等待最多 8 秒、加两个英雄、调顺序、锁定、禁英雄列表都写进 `plugins.lol.settings`，别的偏好不丢）。
- 原有的两个真机脚本：`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12。

**还没测（要你开局）**：真客户端里的接受、选、禁（接口路径和请求体照 Akari，假客户端按 Akari 的写法回答）；被封号的风险只有你能决定。

### 第 4 步（一）：游戏语音搬进插件（2026-11-24）

**玩家能看到的**：

- “设置 → 她怎么回答”里没有“游戏语音”了；它成了左侧栏“插件”组里的 **游戏语音**（设置 → 插件 里也有它的卡片和开关）。截图：`target/plugin-shots/step4/game-voice-page.png`、`target/plugin-shots/step4/settings-plugins.png`。
- 插件开关就是原来的“开启游戏语音”（默认关）。关着时页面只有开关和一句“开着会怎样”；开着时和原来一样：VB-CABLE 的状态和安装、混入我的麦克风、我的麦克风和音量、Windows 默认麦克风（改 / 改回）、测试音、说一句。
- 原来设过的都还在：读旧配置时 `voiceChat.enabled` 搬成插件开关，其余（混入麦克风、麦克风、音量、记下的原来的默认麦克风）搬进 `plugins.game-voice.settings`。
- 她的工具名没变（`voice_chat.status / install / default_mic / say / test`），玩家设过的权限照旧；主会话照旧有 `say` 和 `status`。插件关着时这五个工具不出现（以前一直在，关着时她会先查状态）。

**本体变了多少**：Rust 净少 1,223 行（+290 −1,513），界面净少 301 行（+137 −438）。搬走的：桌面端 `voice_chat.rs`（481）、系统适配层的 `voice_chat.rs`（321）和 `voice_router.rs`（485）、kotone-core 的 `voice_chat.rs`（60）、工具服务里的五个工具定义、提示词里游戏语音那一段、每轮附注里的“游戏语音：开着”、设置页的游戏语音一节（158）。插件 `plugins/game-voice` 共 2,094 行（含页面、文案、测试）。

**宿主为它加的能力**（`kotone-plugin-api`）：

- `Host`：`companion_enabled`（她开着没有）、`update_settings`（改插件自己的设置，和界面上保存走同一条路）、`can_synthesize` / `synthesize` / `speak`（她的嘴：合成 PCM、在玩家的扬声器上说）、`download`（和模型下载共用“同一时间只下一个”）。
- `Plugin`：`stop`（玩家让她停下时，还没对队友说完的话也停）、`settings_changed`（每次保存设置，转发器跟着开关、麦克风、音量重开）。
- 清单 `tool_prefix`：搬进插件以前就有的工具保留原来的名字（游戏语音是 `voice_chat`）。

**检查**：

- Rust：`kotone-plugin-game-voice` 8（新：清单和工具一致、文案和清单一致、设置读不懂时按默认）、`kotone-core` 新加“旧的 voiceChat 搬进游戏语音插件”、`kotone-agent` 的游戏语音测试改成用插件真实的工具（主会话有哪几个、默认问不问和以前一样）、桌面端新加“游戏语音是插件、工具名不变”；clippy、fmt 干净。`tests_http::shutdown_lets_http_long_polls_deliver_stopping` 第一次没过（已知不稳定），重跑通过。
- 界面：svelte-check 0 / 0；Playwright 159 个全过（游戏语音的测试搬进 `plugins/game-voice/e2e/`：关着只有开关 → 打开 → 没装 → 安装 → 运行中 → 改 / 改回默认麦克风 → 测试音“通了”；旧位置和 `ui.open` 都打开插件页）。

（2026-11-24 后来：左侧栏不放插件入口，游戏语音、英雄联盟都从“设置 → 插件”点开；见 ADR 038“怎么定的”。）

### 第 4 步（二）：音乐搬进插件（2026-11-24）

**玩家能看到的**：

- 音乐从左侧栏“游戏陪玩”组挪进了“插件”组（`plugin:music`；之后左侧栏不放插件入口，见下一节）。音乐页、左侧栏的小播放条、聊天页的“登录网易云音乐”卡片都和原来一样。截图：`target/plugin-shots/step4/music-page.png`、`sidebar-plugins.png`、`music-logged-out.png`。
- 设置 → 插件 里多了“音乐”（默认开，放歌以前一直都在）。关掉以后：正在放的歌停下，小播放条和聊天里的登录卡片不出现，她没有 `music` 工具；关着时 Kotone 启动不连网易云。
- 原来设过的音质、音量自动搬进 `plugins.music.settings`；左侧栏顺序和隐藏里的 `music` 换成 `plugin:music`；网易云的登录还在（凭据库里的名字没变，`music:netease`）。

**本体变了多少**：`crates/kotone-music` 整个搬成 `plugins/music`（3,468 行，包名 `kotone-plugin-music`）；桌面端 `music.rs`（729）、`music_cmd.rs`（85）、kotone-agent 的 `music.rs`（33）、她的提示词里放歌那一段、设置里的 `MusicConfig`、界面的音乐页和 `music.json` 文案（各 76 条）搬进插件。本体为它新加的：宿主的凭据（`secret` / `set_secret` / `delete_secret`，名字按插件分开）、网页登录小窗口（`open_web_window` / `web_window_open` / `web_window_cookies` / `close_web_window`，无痕、每个插件一个）、她在不在出声（`she_is_audible`，音乐压低用）；界面加了两个插件位置：左侧栏底部（小播放条）和聊天页上方（登录卡片）。

**定下来的细节**：

- 音乐页原来的 11 个 `music_*` 命令成了插件只给页面用的工具 `music.state`、`music.search` ……，参数和返回值不变；状态变化从 `kotone://music` 事件改成插件事件 `state`。
- 播放器在插件启动时建（宿主的 tokio 运行时里），插件关着也建（不出声、不联网），打开就能用；登录过、插件也开着才在启动时查账号。
- 顺手修了第 2 步留下的问题：Tailwind 的 `@source` 写成了 `plugins/*/ui`（匹配不到文件），插件页面里才用到的样式类从来没生成过（英雄联盟页面的一些颜色），改成 `plugins/*/ui/**/*.{svelte,ts}`。英雄联盟的截图没有重拍。

**检查**：

- Rust：`kotone-plugin-music` 33（新：清单和工具一致、文案和清单一致、设置读不懂按默认、页面的放歌请求）、`kotone-core` 新加“旧的 music 和左侧栏搬进放歌插件”、桌面端插件表测试；clippy、fmt 干净。
- 界面：svelte-check 0 / 0；Playwright 161 个全过（音乐的测试搬进 `plugins/music/e2e/`，新加：音乐在插件组、关掉插件就停、小播放条和登录卡片不出现、旧的左侧栏顺序照样认）。
- 真机：`music_real.py`（不登录的部分）14/14：旧配置照样搬过去、搜歌 0.5 秒、音乐页的登录卡片、官网登录窗口（宿主开的）开和关、没登录时页面上点放歌会打开登录窗口、打字“我要听孤勇者”她直接用 `music`、不弹窗口、聊天页出登录按钮；`lol_tools_real.py --while-playing` 32/32；`game_events_real.py` 12/12。

**还没测（要你扫码）**：`music_real.py --login` 那一半（扫码登录、每日推荐、暂停继续、切歌、她说出歌名）。

**提交的事故**：搬文件用的 `git mv` 已经进了暂存区，之后两个只改文档的提交（`54cb20e4`、`76be9574`）把这些搬动一起带了进去，这两个提交单独编译不过；这一步的提交补齐（没有强推改历史）。

### 第 4 步（三）：左侧栏不放插件入口（2026-11-24，`255f137e`）

玩家看了游戏语音的截图后定的：“插件组没必要吧，进入插件页面再点击插件详情好一点，没必要侧边栏放一堆插件入口”。

- 左侧栏没有“插件”组了，“全部页面”里也不列插件；旧的左侧栏顺序 / 隐藏里的 `plugin:*`、`music` 直接跳过。
- “设置 → 插件”：每个插件一张卡片，点名字或“打开”进插件页面，开关留在卡片上；插件页面左上角“返回插件”。在插件页面时左侧栏亮“设置”。音乐的小播放条照旧在左侧栏底部（在放的时候），点它打开音乐页。
- 截图：`target/plugin-shots/step4/sidebar.png`、`settings-plugins.png`、`lol-page.png`（第 4 步（二）的 Tailwind 修好以后，英雄联盟页面的颜色都在了：“突破口”卡片高亮、胜负格子有颜色）。

### 第 4 步（四）：同传插件（2026-11-24）

**玩家能看到的**：“设置 → 插件”里多了“同传”（默认开）。页面上设好听哪个程序（空 = 现在的游戏，可以填 Discord.exe、KOOK.exe……）、译成什么（中文 / 英语 / 日语 / 韩语）、用弹幕还是气泡，点“开始”就等于你在聊天里对她说“帮我盯着队友说的话，当同传：听 …，每句外语翻成 …，用 … 显示。”（这句话出现在聊天里），她照 ADR 021 的做法交给后台去听、去翻；点“停止”等于说“停止同传”。页面上显示她的耳朵现在在听哪些程序。截图：`target/plugin-shots/step4/interpreter-page.png`。

**为什么这样做**（四个问题，见 [软件哲学](philosophy.md) 原则 12）：同传本来就是她的本事（ADR 021 定了翻译不做成工具、不在工具里藏另一个模型，由后台会话做）；耳朵（`voice.listen`）和后台是本体的，有好几个用户。插件只加玩家不叫她也能用的那部分：页面和偏好。所以宿主只加了两样：`player_says`（玩家在页面上点了按钮 = 玩家对她说了这句话，算玩家亲口说的，能交给后台）和 `listening`（耳朵在听哪些程序）。工具都只给页面用；她的做法写在清单的 `[agent] rule` 里。

**检查**：Rust：`kotone-plugin-interpreter` 3（清单和工具一致、开始那句话照设置说、文案和清单一致）、桌面端插件表；clippy、fmt 干净。界面：svelte-check 0 / 0；Playwright 164 个全过（新加 2 个：设好程序、语言、显示 → 开始 → 在听 → 停止 → 不听；关掉只剩开关；空程序按“现在的游戏”）。真机：`interpreter_e2e.py` 16/16（原来的 9 项照过；新加 7 项：页面上的“开始”替玩家说了那句话、出现在聊天里、她交给后台、页面显示在听 kotone-speaker.exe、“停止”停了后台和监听）；`music_real.py` 14/14；`lol_tools_real.py --while-playing` 32/32；`game_events_real.py` 12/12。

**第 4 步合起来**：游戏语音、音乐、同传三个插件搬完（浏览器按讨论留在本体）；设置里少了游戏语音，音乐页从左侧栏挪进“设置 → 插件”。本体 Rust 净少约 1,200 行（游戏语音）+ 3,468 行的放歌 crate 整个搬出 + 约 850 行桌面端放歌代码；宿主为插件新加的能力：她开着没有、改自己的设置、她的嘴（合成、在扬声器上说）、下载、凭据、网页登录小窗口、她在不在出声、替玩家对她说一句、耳朵在听什么；插件加了 `stop`、`settings_changed`；清单加了 `tool_prefix`。

**还没测（要你来）**：游戏语音在真游戏里队友能不能听到；音乐扫码登录以后的那一半（`music_real.py --login`）；同传在真游戏 / Discord 里的识别和翻译质量。

### 第 5 步：语音输入——停在“按职责拆开”（2026-11-24，`553107a8`）

**做了的**：3,573 行的 `crates/kotone-core/src/orchestrator.rs` 拆成 `orchestrator/` 目录：`mod.rs`（状态、组装、胶水，490 行）、`capture.rs`（开始一次会话、录音，800）、`hotkeys.rs`（语音输入热键、频道、文字处理热键，369）、`assistant_input.rs`（她的耳朵，248）、`finalize.rs`（结束一句，209）、`lifecycle.rs`（取消、中止、确认，198）、`voice_input/`（“听到以后”：识别出的文字、文字处理、发送、记录，共 995）、`tests.rs`。只搬不改：逐行核对，新加的只有 mod / use、可见性、impl 包装、rustfmt 折行（145 行加、73 行删）；测试名和数量前后一样（单元 307、集成 85），`tests/orchestrator.rs` 一个字没改；clippy 干净；命令行 wav 直灌和虚拟声卡两条路都走到预览（隔离的 KOTONE_HOME、真的流式模型）。玩家看不到任何变化。

**为什么停在这里**（实施前请规划代理把 3,573 行和 4,216 行的测试过了一遍）：

- **“测试一个字不改”和“搬进插件”冲突**：`tests/orchestrator.rs` 直接用 `Orchestrator::new(设置, 引擎, 录音, 注入器, 焦点, 事件)` 六个参数构造，然后要求文字处理、打字发送、记录、重发都能用；本体不能反过来依赖插件。所以“听到以后”的代码这一轮只能在本体里按职责分开，不能出 `kotone-core`。
- **设置也被测试钉住**：测试读写 `postProcessing`、`history`、`autoSend`、`interactionMode`；搬进 `plugins.voice-input` 是配置格式的单向门，要单独做、单独测降级。
- **没有真机基线**：“按语音输入热键 → 悬浮窗 → 打进测试窗口”没有现成的真机脚本，规划时定的规矩是没有基线就只做纯搬。

**下一步**（按顺序，每步能单独编译）：

1. 写 `scripts/manual-e2e/voice_input_real.py`（隔离实例、VB-CABLE 当麦克风、测试窗口、exe 拷到仓库外），先在现在的版本上跑出基线；
2. 拆 `capture.rs` 里 540 行的 `begin_locked`（只提函数，顺序不变）；
3. 录音的旁听者（评测录档和识别记录都要 PCM、第一个字的时间、VAD 段落：两个用户，可以抽）；
4. “听一句”和“听到以后”之间的缝收成约 6 个方法（听到了一句、确认、会话结束、发送在不在跑、目标窗口、重发），本体的文件不再直接碰发送和记录的类型；
5. 之后要玩家先定的：测试里加一行把“听到以后”接上（或者把这部分测试搬进插件）；设置什么时候搬、怎么搬；关掉“语音输入”插件会怎样（没有语音输入热键、没有悬浮窗、她没有 `game_chat.send`？）。然后才是搬进 `plugins/voice-input`、文字处理（`kotone-postprocess`，要先断开它对 `kotone-ai` 的依赖）、`game_chat.send` 用 `tool_prefix = "game_chat"` 保留原名、界面（文字处理页、识别记录页、悬浮窗）。
6. 宿主要加的：注册“听到以后”的接口和会话句柄、替插件登记几个热键（频道、重发、文字处理开关）、共用的注入器、给插件调模型（翻译、润色，Key 留在宿主）、`Plugin::call` 带上是谁在调。打字注入可以直接用系统适配层（没有状态的系统调用）。

**顺带修的**：`scripts/e2e-virtual-audio.sh` 引擎名过时、改写玩家的 `~/.kotone/config.json`；跑它时路径 2 用了真实注入器，把一句识别结果打进了前台窗口（`4324441a` 修好：`listen --no-hotkey` 缺省只到预览，要打字加 `--send`；脚本用隔离的 KOTONE_HOME）。

### 第 6 步：按键程序——停在两次“只搬不改”（2026-11-24，`b5105334`、`cd2b8bf0`）

**做了的**（玩家看不到任何变化）：

- `b5105334`：识别图片里的文字（`ocr_image` / `OcrLine`）从桌面端的 `automation.rs`（按键程序那边）挪进 `ocr_engines.rs`。computer use 的 `game.look` 和给插件的 `screen_text` 都用它：它是本体的“眼睛”。
- `cd2b8bf0`：桌面端 `automation.rs` 拆成 `automation/mod.rs`（自动化：库、运行、事件触发、定时、停下、界面命令、启动器、模板图片）和 `automation/program.rs`（只有按键程序用的 `DesktopHost`，Luau 程序的 `ScriptHost` 实现，约 250 行）。
- 两次都只有 use、可见性、调用路径这几类改动（逐行核对）；桌面端测试 111 个，名字和数量一样（`mean_luma` 的测试跟着函数挪进 `program.rs`，模块路径变长）；clippy、rustfmt 干净。

**为什么停在这里**（实施前请规划代理把程序、自动化、computer use 和测试过了一遍）：

- 再往下是把 Luau 运行时（`crates/kotone-script`，1,835 行）和 `program.rs` 搬进 `plugins/key-programs`，宿主要给插件一个“这一次运行能动的游戏窗口”的句柄（绑窗口、截图、按键 / 松开 / 打字 / 鼠标、取消、识别文字、说一句、模板图片）。这是新的大权力，以后“每个插件一个进程”时收不回来（单向门，原则 11），要玩家先看过。
- 真按键的真机测试要机器空着（测试窗口要在前台、会抢焦点），这轮玩家一直在用电脑，没跑。目前程序的真机检查只有手跑的步骤（`docs/research/2026-11-16-companion-scripts.md`）。
- 被测试钉住的：`AutomationScheduler::open`、`ProgramLauncher`（`FakeLauncher`）、内置宏 `BUILTIN_PROGRAM_IDS` 和 `ensure_builtin`、`automation.*` 工具名。

**下一步**：

1. 写 `scripts/manual-e2e/key_programs_real.py`（真按键只发给测试窗口；exe 拷到仓库外；隔离的 KOTONE_HOME；有游戏在跑或前台不是终端 / 测试窗口就退出），在现在的版本上跑出基线：按节拍按键、每个内置宏、停下时松开按住的键、窗口关了算失败、`allowAutomationInput=false` 拒绝、第 4 个同时运行的程序 BUSY、程序在跑时 `game.act` 被拒、退出时松键、窗口失焦就暂停不补、保存时语法错误被拒、`find_image` / `find_text` / `say` / `toast`；
2. 两把输入锁（`computer_use::busy` 和 `automation::program_running` 互相查）合成本体的一把锁（顺带堵一个小竞态）；
3. 一次提交搬过去（不能搬一半）：`kotone-script` → `plugins/key-programs/src/script/`（测试不改）、`program.rs` 改成用宿主给的句柄；插件清单不写 `[agent]`（她的提示词不变）；两个内部工具 `key_programs.run`、`key_programs.check`；
4. 要玩家定的（看得见的）：关掉“按键程序”会怎样（程序类的自动化、内置宏、`saved.*`、快捷指令是拒绝并说明、还是消失、还是根本没有开关）；内置宏挪到插件页面，还是两边都有；程序编辑器留在自动化的编辑器里吗；每个游戏的“允许程序按键”留在游戏配置里（推荐：它也挡她的 `game.act`）。

### 第 7 步（部分）：插件边界的测试、设置里的“浏览器”一节（2026-11-24）

- `95e526d4`：**插件的边界写成测试**（`crates/kotone-plugin-api/tests/boundaries.rs`）：读 `plugins/*/Cargo.toml`，本体里只许依赖 `kotone-plugin-api`、`kotone-core`、`kotone-platform-windows`；不许 Tauri、别的 `kotone-*`、别的插件；插件源码里不出现 `hotkey_ll`、`input_gate`、`SetWindowsHookEx`。另有一个测试确认规则真能抓到违反。
- **设置里的“浏览器”一节**（玩家选的：开关放设置，不做成插件卡片）：“设置 → 浏览器”在“插件”前面：使用内置浏览器（`browser.enabled`，新字段，默认开；旧配置读成开）、清除浏览器登录、扩展（从“权限”页挪过来）。关掉以后：右侧栏没有“网页”、开着的网页关掉、不再建新网页；她从下一段对话起没有 `browser.*`（`Catalog::without_browser()`，在桌面端按设置过滤；旧对话里再调会说“内置浏览器关着（设置 → 浏览器）”）；登录过的账号和扩展都留着，再打开就回来。界面上的“浏览器插件”都改叫“扩展”（术语表加了“内置浏览器 / 网页”“扩展”，`浏览器插件` 列为禁用说法）。她也能用 `settings.update` 开关它（和别的设置一样先问玩家）；扩展只有玩家自己能加、删。截图：`target/plugin-shots/step7/browser-settings.png`。
  - 检查：`kotone-core` 309、`kotone-agent` 79、桌面端 111 + 编排 84、`kotone-tools` 111；clippy 干净；svelte-check 0 / 0；Playwright 165；真机：`browser_probe_real.py --minimized` 13/13、`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12。
- **还没做的**：英雄联盟的事件目录由插件登记（现在是 `kotone-core/src/games.rs` 里的常量，本体的校验和测试都在用，要先有“插件登记事件”的机制）；设置页只剩本体（语音输入、按键程序的设置跟着第 5、6 步走）；同意过的网站还没有界面。

### 2026-11-24 第五轮：方向的两处更正

- **插件页**：玩家改成“插件是一个单独的左侧栏页面，每个插件是一张卡片，对应 MCP App 的概念，点卡片进 App；不同插件可以有完全不同的界面设计语言；以后会从别的仓库拉第三方 MCP Apps”。左侧栏一个“插件”入口 → App 卡片墙 → 点开整块内容区都是这个 App；“设置 → 插件”去掉。ADR 038“怎么定的”和“不做”已改。
- **第 5 步的“测试当行为基准”**：玩家的任务原文是“拆之前把 crates/kotone-core/tests/orchestrator.rs 当行为基准，拆完一个不改照样过”（照抄自本计划第 5 步那段）。意思是里面的检查一个不改、照样全过，用来证明行为没变；上一轮把它读成了“测试文件一个字都不能动”，比本意严，结果把“听到以后”卡在了本体里。按本意：测试可以加一行把语音输入插件接上（或连同语音输入搬进插件），检查一条不改、照样全过。玩家确认语音输入要搬进插件。
- **关掉语音输入插件**：照 ADR 038“怎么定的”（“往聊天框打字的一套都在语音输入插件里，她替你发言也走它”）：语音输入热键、悬浮窗、文字处理、识别记录、她替你在游戏聊天框打字（`game_chat.send`）一起停；她的耳朵（呼叫键、唤醒词、和她说话）在本体，不受影响。语音识别只有一份，在本体；语音输入插件调用本体的“听一句”（没有变）。

### 插件页：左侧栏一个“插件”、App 卡片墙（2026-11-24 第五轮）

- 左侧栏一项“插件”（页面 id `plugins`，在“自动化”下面，能像别的页面一样排序、隐藏）；旧的 `plugin:*`、`music` 跳过。
- “插件”页：每个插件一张卡片（图标、名字、一句话说明、开关；关着的变淡、标“已关闭”）；点卡片进 App，点开关只切换。
- App：占满内容区，宿主只留“← 插件 · 名字”；打开时左侧栏亮“插件”。下面是宿主不管样式的通用容器，以后第三方 MCP App（沙盒 iframe）放同一个位置。`fullBleed` 选项删掉了，插件一律占满。
- “设置 → 插件”去掉；旧的 `ui.open ui://kotone/settings/plugins` 和“设置 → 插件 → X”“插件 → X”“音乐”这些位置都落到插件页或对应的 App。
- 截图：`target/plugin-shots/apps/plugins-page.png`、`lol-app.png`、`music-app.png`。
- 检查：svelte-check 0 / 0；Playwright 165（新加：左侧栏只有一项“插件”、4 张卡片、卡片开关只切换不进 App、点卡片进 App、“← 插件”回来、设置里没有“插件”、旧地址落在插件页）；真机：`music_real.py` 14/14（脚本改成左侧栏“插件” → 音乐卡片）、`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12。

### 第 5 步（续）：收窄“听一句 / 听到以后”的缝，跑出语音输入的真机基线（2026-11-24 第五轮）

- `12d157bc`：540 行的 `begin_locked` 按原顺序拆成几步（开识别会话、开麦、记录、VAD、录音泵 `CapturePump`、发布）。
- `fda2f8bd`：录音的“旁听者”（`CaptureObserver`）：评测录档（本体）和识别记录（“听到以后”）走同一个接口，顺序不变。
- `81c6aef8`：本体的文件只经 `voice_input/mod.rs` 的 9 个方法碰“听到以后”（`send_is_running`、`stop_send`、`start_record`、`on_capture_started`、`on_transcribed`、`on_heard`、`heard_audio_ms` / `discard_record`、`confirm_heard`、`on_session_ended`），其余只在 `voice_input/` 里可见，越过这道缝编译就不过。
- 三次都是行为不变的重构：`kotone-core` 单元 309、集成 85（检查一条没改），集成每次跑两遍；测试名单逐字节一样；`scripts/e2e-virtual-audio.sh` 两条路都过。
- **真机基线** `scripts/manual-e2e/voice_input_real.py`（新）：隔离实例、VB-CABLE 当麦克风、测试窗口在前台时才发键；隔离配置用 `hotkeyBackend: "register"`（低级钩子不认模拟按键），按 F22 走真的语音输入热键那条路。在 `81c6aef8` 上 **36/36 过、2 个跳过、50 秒**：按住说自动发送（测试窗口正好收到识别的字、只按了档案的两次回车）、录音笔模式预览不打字确认才发、屏蔽词把“打野”换成“JG”、识别记录、悬浮窗状态顺序、她的 `game_chat.send`（脚本里的模拟模型）和 `companion.type`、控制台通道没游戏时退回键盘、`~/.kotone` 没被动过。跳过：外部 Agent 调不了 `game_chat.send`；真的控制台通道要 Steam 装的 Dota 2 / CS2 在前台。跑之前玩家要离开电脑约 3 分钟（前台是 GameViewer 时脚本按规矩没跑）。
- **下一步**：`voice_input/` 整块搬进 `plugins/voice-input`（测试加一行接上或连同搬过去，检查一条不改）、设置读旧写新、界面（文字处理、识别记录、悬浮窗）进插件 App、`game_chat.send` 用 `tool_prefix` 保留原名；每一步前后都跑 `voice_input_real.py`、85 个集成检查和英雄联盟的两个脚本。

### 第 5 步（续二）：“听到以后”搬进 `plugins/voice-input`（2026-11-24 第六轮）

先重读了 ADR 038“怎么定的”和本节（第五轮的约定），没靠摘要。玩家这一轮的话：“`tests/orchestrator.rs` 可以加一行接上插件或者连同搬过去，但里面的检查一条不改、照样全过；设置读旧写新；文字处理、识别记录、悬浮窗进插件 App；`game_chat.send` 保留原名。”

**（一）Rust 搬家**（玩家看不到变化）：

- `crates/kotone-core/src/orchestrator/voice_input/`（6 个文件 1,398 行）没了。发送、文字处理的运行、识别记录、频道和文字处理热键搬进新 crate `kotone-plugin-voice-input`（`plugins/voice-input/src/`：`send.rs`、`postprocessing.rs`、`records.rs`、`hotkeys.rs`，`git mv` 保留历史），改动只是 `self`（编排器）换成 `orch` 参数、插件自己的状态换成插件自己的锁；原来 `orchestrator/tests.rs` 里 9 个语音输入的单元测试跟着搬进 `src/tests.rs`（名字、内容不变）。
- 本体留下“听一句”和一道缝 `orchestrator/after_hearing.rs`（448 行）：
  - `AfterHearing`（插件实现）：开始收音时建识别记录草稿（`RecordDraft`，同时是录音旁听者 `CaptureObserver`）、发布会话时记下注入目标窗口、识别完成、听到了一句、确认、会话结束；
  - 插件改会话状态只经 `lock_op()`（begin / end / cancel / confirm 共用的临界区）和 `session()`（状态锁的句柄 `SessionGuard`：代际、状态、用途、待处置的文字、Processing 的取消令牌、发送权）；锁的顺序：op → 状态锁 → 插件自己的锁；
  - 留在本体的数据：待处置的文字（`PendingText`：识别原文 / 可交付文字 / 她的请求，状态机和热键都要看）、发送权（`SendOperation`：发送没结束时不能开始新会话、不能改热键）；
  - 没接语音输入时听写听到的字没有去处：以 Error 结束并说明（“语音输入插件关着……”）。
- 接法：`VoiceInputExt::with_processors(...)`（桌面端、命令行原来就这样写，加一行 `use`）；重发、频道、文字处理开关这几个热键也在这个 trait 上。命令行（`kotone-cli`）依赖了这个插件，和它依赖放歌插件一样（它是不带窗口的宿主）。
- **`tests/orchestrator.rs`**：留在原处，`kotone-core` 多一个开发依赖（和 `kotone-stt`、`kotone-postprocess` 一样的依赖环）。改动只有接线：1 行 `use kotone_plugin_voice_input::VoiceInputExt;`，10 处直接 `Orchestrator::new(...)` 的地方各接一行 `.with_voice_input()`（原来这些地方用的是默认的空处理器表，接上后一样）；`assert` 和测试的操作一行没动。识别记录的存储（`kotone_core::history`）和文字处理的配置类型留在本体：测试的检查直接读 `kotone_core::history::list_in` 和 `settings.post_processing`。
- 检查：`kotone-core` 单元 300 + 插件 9 = 309（和原来一样）、集成 84 过 + 1 个手动跑的跳过（= 85，检查一条没改）；桌面端 111；`kotone-cli` 34；插件边界测试 3；clippy（core、插件、命令行、桌面端，`-D warnings`）干净。真机：`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12（前后都跑）。`voice_input_real.py` 要玩家离开电脑，还没跑。

**（二）语音输入成了插件清单里的一项；`game_chat.send` 由它登记（名字不变）**：

- `plugins/voice-input/plugin.toml`：`voice-input`“语音输入”，默认开；`tool_prefix = "game_chat"`（搬进插件以前就叫 `game_chat.send`，玩家设过的权限和她的习惯都不变）；给她的一句能力概览。桌面端插件表加上它（`VoiceInputPlugin`）。
- `game_chat.send` 不再是她的内置工具（`kotone-agent` 的 `MAIN_TOOLS`、内置工具表、`BuiltinHost::send_to_game_chat` 都删了），改成语音输入插件的工具：说明、参数、副作用（`input`、`records-run`）照搬，标了马上返回，所以主会话照样有；默认权限、只读模式下拒绝、玩家设的规则都和以前一样（`kotone-agent` 的测试改成从插件拿这个工具，检查不变）。要打的字在插件里检查（空、超过 200 字、换行），打字本身经宿主：新加 `Host::type_in_game_chat`（共用的注入器和 ADR 032 的挡键全进程只有一份，在宿主；前台不是这个游戏时拒绝；记进「自动化 → 记录」）。原来桌面端 `agent/host.rs` 里那段挪到 `companion::send_to_game_chat`，测试 `game_chat_is_only_typed_into_the_game` 跟着走。
- 插件工具现在知道是谁调的：`Caller::Her` 带上这段对话（`agent:<对话 id>`，执行记录的发起方和以前一样）；外部 Agent 经工具服务调时记成 `tool-service`。**变化**：插件的工具都给工具服务，外部 Agent 现在也能调 `game_chat.send`（以前只有 `companion.type`）；`voice_input_real.py` 第 6 项原来按“调不了就跳过”写的，这一项以后会真的打字（测试窗口在前台时）。
- **关掉语音输入插件**（ADR 038“怎么定的”）：语音输入热键不登记（低层钩子用已有的“占位主键 + 停用”跑着，呼叫键、快捷消息这些具名热键照常；RegisterHotKey 后端不注册主键），频道 / 重发 / 文字处理开关热键也不登记，没准备好时的语音输入提示键也不盯；悬浮窗不显示（和“不显示”一样，开关一变就收起 / 重建）；她的工具清单里没有 `game_chat.send`。呼叫她、唤醒词、她的耳朵不受影响。为此本体的按键匹配改了一处：主键停用时，会话中的 Esc 照样取消（以前主键停用就全部放行，语音输入关着时她的会话就没法 Esc 取消了），加了单元测试。
- 界面还没动（下一步）：插件页里还看不到语音输入的卡片。
- 检查：`kotone-core` 单元 301（多 1 个 Esc 的）、集成 84 + 1 跳过；她 79；插件 12；桌面端 112（多 1 个“语音输入是插件、开关带走热键”）；系统适配层 90；clippy 干净。真机：`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12。

**（三）界面：语音输入的整块搬进插件的 App**（玩家看得到）：

- 左侧栏没有单独的“语音输入”了（左侧栏只有一个“插件”入口，ADR 038“怎么定的”）；“插件”页多了一张“语音输入”卡片（排第一，默认开，卡片上有开关），点开就是原来的语音输入：输入 / 文字处理 / 历史三页，悬浮窗、语音识别、语音输入热键、其他热键都在“输入”页，和原来一样。开关在 App 最上面；关着时只有开关和一句“开着会怎样”。
- 文件从 `apps/desktop/src/routes/settings/pages/`（`GeneralPage`、`TextProcessingPage`、`HistoryPage`、`voice/*`）`git mv` 到 `plugins/voice-input/ui/`，只改了 import 的写法；新加 `VoiceInputApp.svelte`（开关 + 三个分页）和 `index.ts`（图标、App）。插件的文案 `plugins/voice-input/messages/` 并进 Paraglide。插件页面里用到的 `@tauri-apps/api/*`、`@tauri-apps/plugin-dialog` 在 `tsconfig.json` / `vite.config.ts` 里指到桌面端的那一份（和 `svelte`、图标库一样）。
- 插件的 App 能打开在某一处：`openTarget({ page: "plugin", id, params })`，`PluginPageProps` 多了 `open`（照 MCP Apps 打开页面时带的参数）。原来的位置都还认：“语音输入 → 悬浮窗 / 交互模式 / 其他热键”“文字处理”“历史记录”“设置 → 其他热键 → …”（她在聊天里给的“去看看”）落到 App 里对应的分页和分组，“插件 → 语音输入 → 文字处理”也认。“打开软件时显示：语音输入”、0.1.x 升级来只用语音输入的玩家、引导里选“只用语音输入”、标题栏的“启动本地识别”都落到这个 App。“让 Kotone 帮我配置”照旧（她连好了就请她帮忙，没连就先填 Key：新加的位置 `setup-key`）。
- 左侧栏原来的“语音输入”一组只剩 0.1.x 升级用户的“提醒助手”；旧配置里左侧栏顺序 / 隐藏里的 `general` 认不出来就跳过（和 `music` 一样）。术语表“语音输入”一行改了。

**（四）设置读旧写新**：

- 语音输入插件的设置——文字处理（`postProcessing`）、识别记录（`history`）、悬浮窗（`overlay`）、频道切换 / 重发 / 文字处理开关和切换流程四个热键——在配置文件里写进 `plugins.voice-input.settings`；读的时候新位置为准，没有就用旧的顶层字段（`settings.rs` 的 `lift_voice_input_json` / `stash_voice_input_json`，加了两个测试）。
- 程序里它们仍是 `Settings` 上的字段：本体的“听一句”、宿主、界面都照旧读写，`tests/orchestrator.rs` 的检查（例如“文字处理开关热键落盘”读的是 `settings.post_processing`）一条不改照样过。所以这一步只动配置文件的样子，改动集中在读写两处。
- 代价（单向门，和 `lol.enabled`、`voiceChat`、`music` 搬家一样）：装回旧版本时，旧版本在顶层找不到这几项，会用它的默认值（自定义的屏蔽词、润色流程、识别记录设置看不到；旧版本一保存就丢了）。
- 语音输入热键本身（`hotkey`）、交互模式、静音判停这些“听一句”的设置留在本体（她的耳朵也用）；每个游戏的聊天键和频道还在游戏配置里（计划里写的 `plugins.voice-input.games.<游戏>` 没做，游戏配置是另一份文件，单独做）。

**（三）（四）的检查**：`kotone-core` 单元 303（多 2 个读旧写新）、集成 84 + 1 跳过（检查一条没改）；她 79；工具服务 111；svelte-check 0 / 0；Playwright 165 全过（跟着新结构改的：左侧栏顺序里没有 `general`、语音输入经“插件 → 语音输入”进；`speech-settings` 的手写桩补上插件清单；删了没人用的 `shell_sidebar_voice_input`；放歌的 e2e 多查一条“旧配置左侧栏里的 general 跳过、不算隐藏”）。真机：`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12。没有测试覆盖“打开 App 时滚到下面的分组”（例如标题栏的“启动本地识别”），代码里做了。`appearance.spec` 有一次整套跑时开页超时、单跑和重跑都过，像是开发服务器第一次优化依赖时卡了一下，留意。
**真机 `voice_input_real.py`**（玩家离开电脑时跑，`50348033` 上）：**37/37 过、1 个跳过、53 秒**（搬家前的基线 36/36、2 个跳过）。多过的一项是 6a：外部 Agent 经工具服务调 `game_chat.send` 现在能调，正好打进测试窗口。按住说自动发送、录音笔预览确认、屏蔽词“打野 → JG”、识别记录、悬浮窗状态顺序、她的 `game_chat.send`（执行记录有 runId）、`companion.type`、控制台通道没游戏时退回键盘、`~/.kotone` 没被动过，都和以前一样。跳过的还是 7b（真的控制台通道要 Steam 装的 Dota 2 / CS2）。

### `game_chat.send` 回到本体（2026-11-25）

玩家原话（[交接第十一节](research/2026-11-23-session-handoff.md)）：“ASR、游戏场景的输入（回车——unicode 输入——回车）是属于本体的。语音输入法接用了这两个本体的能力。”决定写在 [ADR 039](adr/039-desktop-companion-and-game-apps.md) D2。

**玩家看到的**：关掉“插件 → 语音输入”以后，琴音照样能替你在游戏聊天里发言；外部 Agent（经工具服务 / MCP）调 `game_chat.send` 也照样能用。语音输入卡片的说明去掉了“她替你在聊天框发言也走这里”，关着时那句改成“关着时呼叫她、唤醒词照常能用，她也照样能替你在聊天框发言”。名字、说明、参数、默认权限、玩家设过的规则、只读模式拒绝、前台不是这个游戏时拒绝、记进「自动化 → 记录」都和以前一样；每个游戏的聊天键、频道和发送时序还在游戏配置里。

**改了什么**：

- 本体的工具开始用统一的 `ToolSpec` 登记（ADR 038“怎么定的”，原则 15）：新文件 `crates/kotone-agent/src/body.rs`，`game_chat.send` 的说明、参数、副作用（`input`、`records-run`）、马上返回照搬，`chat_text` 检查要打的字。她的工具清单里是新来源 `Source::Body { instant }`，主会话拿不拿看 `instant`（`main_offers`）：**没加进 `MAIN_TOOLS`，也没另写名单**。别的本体工具还是原来的写法，以后照它挪（MCP 方案 1.5 期简化 `MAIN_TOOLS` 时一起）。
- 执行：琴音经 `BuiltinHost::body_tool`（记在 `agent:<对话 id>` 下），外部 Agent 经工具服务 → `tool_config::invoke`（记成 `tool-service`），两条路都到桌面端的 `companion::body_tool` → `send_to_game_chat`（本体的注入器、ADR 032 的挡键、前台检查、执行记录，没改）。工具服务里列的是 `ToolSpec::body_service_spec()`（和插件的工具一样，只是没有 `plugin`），由 `tool_service::hosted_tool_specs` 放在插件的工具前面；她的清单里不重复（`service_entries` 跳过本体的）。
- 语音输入插件：去掉 `tool_prefix = "game_chat"`、这个工具和 `[agent]` 概览，不再给她工具。概览里讲 `game_chat.send` 的话不用挪：本体的提示词里一直有（能力概览“替玩家在聊天框打字（game_chat.send）”、开头“替玩家在游戏聊天里发言”），就是搬进插件前的写法。
- 删了 `Host::type_in_game_chat`（没有插件再用，少开给插件一样权力，ADR 038“不做”），连同桌面端 `DesktopHost` 只为它记的调用方（`Caller::Her` 不再带对话 id）。
- 界面桩 `pluginsMock.ts` 里语音输入不再列这个工具；术语表“语音输入”一行、本计划第二节“游戏配置怎么分”和第三节“语音输入”表改了（聊天键留在游戏配置，原来搬进插件的计划作废）。

**行为改了、所以改了的测试**（不是把检查改弱）：

- 桌面端 `voice_input_is_a_plugin_and_its_switch_takes_the_hotkeys_with_it`：原来查“`game_chat.send` 归语音输入插件、标了马上返回”，现在查“语音输入插件没有工具、`game_chat.send` 不归任何插件”；热键那一半没动。新加 `game_chat_send_stays_when_voice_input_is_off`：关掉语音输入，工具服务里还有它（只有一份、不带 `plugin`），她的清单里也有、主会话也拿。
- `kotone-agent`：测试夹具原来要接上语音输入插件才有 `game_chat.send`，现在一个插件都不接也有；`builtins_validate_then_reach_the_host` 交给桌面的那一路从 `plugin` 变成 `body_tool`；主会话清单多查一项 `game_chat.send`；新加 `game_chat_send_is_the_bodys_and_stays_without_any_plugin`（插件都关着她也有、交给 `body_tool`；外部 Agent 经工具服务能调、交给宿主；清单里不重复）。`chat_text` 的测试从插件搬到 `body.rs`。
- 语音输入插件的 `manifest_and_tools_agree` 改成 `manifest_gives_her_no_tools`。
- `voice_input_real.py`：6a 原来“外部调不了就跳过”，现在调不了算失败；新加第 8 项（关掉语音输入：工具服务里还有它、不带 `plugin`；外部调用和琴音〔新对话、脚本里的假模型〕都照样打进测试窗口；执行记录的发起方分别是 `tool-service` 和 `agent:<对话 id>`；最后再打开插件）。

**检查**：`kotone-core` 单元 303、集成 84 过 + 1 个手动跑的跳过（`tests/orchestrator.rs` 一行没动）；她 82（多 3：`body.rs` 2 个、新的 1 个）；语音输入插件 11（`chat_text` 的测试搬走了）；插件契约 13；工具服务 113；桌面端 118 + 1 忽略（含同时在做的别的改动；这件事多 1 个）；命令行 33 + 3 + 2 + 2（1 忽略）；clippy `-D warnings`（她、语音输入、插件契约；桌面端用 `--no-deps`，因为同时在改的英雄联盟插件有别的 clippy 报错）干净；svelte-check 0 / 0；相关的 Playwright（导航、外壳、语音设置、术语、右侧栏、聊天、语音输入插件）49 个里 48 过，没过的 `speech-settings` “leaving the page discards old model…” 在撤掉这件事的界面改动后照样不过，是同时在做的设置重排（选模型挪出“她怎么回答”）带来的，全量没跑。真机：`lol_tools_real.py --while-playing` 32/32、`game_events_real.py` 12/12、`voice_input_real.py` **42/42 过、1 个跳过（7b）、58 秒**（之前 37/37）。

### MCP 第 1 期：`kotone-mcp`、OP.GG 迁过去（2026-11-25）

照 [MCP 方案](research/2026-11-24-mcp-client.md) 4.2、4.7 和第六节第 1 期。

**玩家看到的**：看不到变化。“用 OP.GG 的数据”还是默认关（opgginc/opgg-mcp#26 到 2026-11-25 还没人回）；打开以后，英雄页和琴音查英雄时的 OP.GG 那块和以前一样，只是第一次以后快了：同一个会话里一次调用一个请求（以前每次调用三个请求，每次都重新握手）。

**定了什么**：

- **新 crate `crates/kotone-mcp`**：在官方 `rmcp` 上包一层（`=3.11.0`，Apache-2.0，最低 Rust 1.88，工具链是 stable 1.98）。锁到补丁版本：rmcp 一个月出好几个小版本，小版本之间也改过接口；升级跟着 Goose，一次只改这一个 crate。对外的类型是协议原样的字段（`Tool`、`ToolCallResult`、`Resource`、`ResourceContents`，子对象留成 JSON），不露 rmcp 的类型（原则 13：藏住会变的决定）。
- **接口**：`McpClient::connect(配置)` → `list_tools` / `call_tool` / `list_resources` / `read_resource`（第 3 期显示 `ui://` 页面用）→ `close`；超时照 Codex（启动 10 秒、每次请求 60 秒，可配；超时时 rmcp 给服务器发 `notifications/cancelled`）；`tools_changed()` 是一个计数，服务器每发一次 `tools/list_changed` 加一（第 2 期用）。只做子进程（stdio）和 Streamable HTTP；不做旧的 HTTP+SSE、OAuth、`.mcpb`、目录。
- **握手用 `initialize`（2025-11-25）**，没用 Goose 的 `Auto`（先发 2026-07-28 的 `server/discover`，不认再退回）：现在的服务器几乎都是旧版，不认 `server/discover` 又不回话的 stdio 服务器要等 10 秒才退回，正好撞上启动超时。双向门，以后改一行。
- **子进程**放进 Job Object（`KILL_ON_JOB_CLOSE`）：`close` 时先关它的标准输入、等它 2 秒，然后结束整个 Job（连同它启动的进程）；Kotone 退出或崩溃时系统关掉句柄，一样全结束。Job Object 的代码从 `kotone-exec` 抽到系统适配层 `kotone_platform_windows::job`（第二个用户出现了，原则 14）：它是没有状态的 Win32 调用，`kotone-exec` 和 `kotone-mcp` 都在它上面（原则 13）。没让 `kotone-mcp` 依赖 `kotone-exec`：那是琴音子代理的命令工具（ADR 030），插件经 `kotone-mcp` 会间接依赖它。
- **环境变量**：先清空，只给安全基线（Codex 的 `WINDOWS_CORE_ENV_VARS`，加 Hermes `_build_safe_env` 多出来的几个：PATH、PATHEXT、SYSTEMROOT、TEMP、APPDATA、LOCALAPPDATA……都没有密钥）和配置里写的。**程序路径**：`npx` 这类名字按子进程的 PATH + PATHEXT 解析成 `npx.cmd` 的完整路径（照 Codex）；`.cmd` / `.bat` 由标准库经 `cmd.exe` 跑，参数转义、遇到转义不了的参数直接拒绝，都是 Rust 1.77.2 起标准库做的（CVE-2024-24576），不自己拼 `cmd /c`。不弹控制台窗口；stderr 留最后 4 KB，启动失败时说“程序退出了（退出码 N）。它最后说：……”。
- **HTTP**：reqwest 0.13，加密用 ring（和 kotone-ai 一样，不引 aws-lc）；会话过期（404）时 rmcp 自己重新握手一次。
- **插件能不能依赖它**：能。`kotone-mcp` 只是一个协议客户端库，连接归调用方自己，不碰本体的状态（窗口、全进程只有一份的钩子、停下琴音、免问），放在哪个进程都一样能用。边界测试的允许名单、ADR 038 D6、哲学原则 13 都加上了。没改成经宿主 `Host` 给：英雄联盟插件连 OP.GG 是插件自己的数据来源；第 2 期“第三方服务器 = 远程插件”时连接归宿主，那是另一件事。
- **OP.GG**（`plugins/lol/src/stats/`）：手写的连接（`opgg_post` / `opgg_call`，手拼三个 JSON-RPC 请求、自己解析 SSE）删了，换成 `kotone-mcp`。连接放在 `Stats` 里：第一次调用时连上，之后复用；连不上、会话没了就丢掉，下次再连。`client_app = kotone/<版本>`、`clientInfo.name = kotone`、24 小时磁盘缓存、只在开关开着时请求、15 秒超时、出错时的说法（“连不上 OP.GG：……”“OP.GG 返回 502 Bad Gateway”“OP.GG：……”“OP.GG 返回的是空的”）都照旧。
- 英雄联盟插件的 `rust-version` 留在 1.77.2（工作区的写法本来就是下限）：改成 1.88 的话 clippy 会按新的最低版本要求改 6 处和这件事无关的 `map_or`。

**检查**：

- `kotone-mcp` 20 个：单元 5；HTTP 5（一个会话调多次只握手一次、5 个 POST 里握手以后的 4 个都带会话、`clientInfo` 和请求头、JSON-RPC 错误和工具自己报错分开、请求超时、读资源、没人在听时说连不上）；stdio 10（列 / 调 / 读资源、环境变量只有基线和配置的、请求超时后连接还能用、启动超时、启动就崩时说退出码和它的 stderr、找不到程序时说名字、`close` 后服务器和它启动的进程都没了、不 `close` 直接丢掉也都没了、`tools/list_changed`、经 PATH 找到的 `.cmd` 包装收到的参数原样带空格和 `&`、没被 cmd 当成第二条命令）。测试服务器都是 rmcp 自己的服务端：HTTP 的在进程内（`kotone_mcp::testing`，特性 `test-support`，正式构建不带），stdio 的是 `crates/kotone-mcp/examples/stdio_server.rs`。
- `kotone-plugin-lol` 56 个过 + 1 个手动跑的（新加：经本地测试服务器的 OP.GG——四次请求做两次调用、缓存命中不再请求、出错的说法、服务器没了说连不上并丢掉连接）；`kotone-exec` 36（Job 的超时 / 取消结束子进程那几个照过）、`kotone-platform-windows` 90 + 集成 2、插件边界 3、`kotone-plugin-api` 13、桌面端 113；clippy `-D warnings` 干净；`cargo audit` 和改之前一样（没有新的警告）。
- **OP.GG 真调了一次**（`cargo test -p kotone-plugin-lol opgg_real -- --ignored --nocapture`，一个工具 `lol_get_champion_analysis` 查刀妹，上路、中路各一次）：服务器自称 “OP.GG MCP Server 1.0.0”，谈好的协议 2025-06-18；第一次（握手 + 调用）1,986 ms、5,575 字，第二次（复用会话）586 ms、5,520 字，都是以前那种类声明文字（`class LolGetChampionAnalysis: champion,position,data`……）；两次是同一个会话。
- 真机 `lol_tools_real.py --while-playing`（假客户端 + 真 DeepSeek，不按键）：**32/32**，和以前一样；最后一项“OP.GG 默认不请求”照过。

### 英雄联盟 App：素材层和页面数据（2026-11-25）

照 [对标调研](research/2026-11-25-lol-app-parity.md) 差距表 G0、G0.1 和第三节，[ADR 039](adr/039-desktop-companion-and-game-apps.md) D6。玩家原话：“素材不要内置到 kotone 吧？太大了，应该可以直接从客户端拿在线的。”

**玩家看到的**：现在还看不到。旧的 Svelte 页面不画图；新的 App 页面（第 12 条）接上素材层以后才有英雄头像、召唤师头像、段位、海克斯、装备、技能、符文的图。琴音说的话一个字没变。

**素材层**（`plugins/lol/src/client/assets.rs`，入口 `LolPlugin::fetch_asset`）：

- 安装包里不带图。顺序：磁盘缓存 → 客户端（插件和“读客户端数据”开着、客户端开着时，带本机令牌）→ CommunityDragon（`https://raw.communitydragon.org/latest/…`，前缀换成它的目录、后面全部小写）→ 找不到（页面显示占位）。重新取失败时用缓存里过期的那份；CommunityDragon 失败的 10 分钟内不再问（照 AramHex）。
- **只放行三个前缀下的图片**：`/lol-game-data/assets/`、`/fe/lol-static-assets/`、`/fe/lol-shared-components/`；扩展名 png / jpg / svg / webp；内容也要真是图片（按文件头认，客户端回一段 JSON 不算），SVG 里有脚本、事件属性、`javascript:` 的不要。路径每段只许字母、数字和 `._-`，所以 `..`、`%` 编码、查询串、反斜杠、空白、非 ASCII 都拒绝。不把客户端的别的接口转给页面（LeagueAkari 那样什么都转，等于让沙盒里的页面能接受对局、选英雄）。
- **磁盘缓存**：`KOTONE_HOME/plugins/lol/asset-cache/`（插件目录，和统计的缓存放一起；调研写的是 `KOTONE_HOME/cache/lol-assets/`，都在 `KOTONE_HOME` 下，选插件目录是为了删插件数据时一起删）。文件名是路径（小写）的 SHA-256 前 16 字节 + 扩展名；上限 64 MB，超了先删最久没用的（最后访问时间也写回文件，重启以后还知道谁最近用过）；14 天重新取。先写临时文件再改名，不会写坏；同一个路径同时来几次只取一次。
- 页面数据里的路径由 `champion_icon`、`profile_icon`、`ranked_crest` 和客户端资料里的 `iconPath` 给，都过得了上面的检查（有测试）。

**页面数据补的字段**（`pages.rs`，camelCase，只加不改，旧页面照常）：图都给客户端路径，拿不到时是空字符串。

- 共用：`champion` 多了 `icon`；召唤师技能 `{id, name, icon}`；装备 `{id, name, icon, slot}`（`slot` 0–6，6 是饰品，空格不列）；海克斯多了 `rarity`、`icon`；符文 `perks: {keystone, primaryStyle, subStyle}`（各是 `{id, name, icon}` 或 null，海斗、斗魂竞技场整个 null）；段位 `ranked: {solo, flex}`，各是 `{queue, tier, division, lp, wins, losses, highestTier, label, crest}` 或 null（没定级时 `tier` 空、`label` 是“未定级”）。
- 这局的每个人：`profileIconId`、`profileIcon`、`summonerLevel`、`spells`、`position`（`TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`，大乱斗是空）、`ranked`；`recent` 每局多了 `championIcon`。
- 战绩页：`profileIconId`、`profileIcon`、`summonerLevel`、`ranked`；每局多了 `level`、`spells`、`items`、`perks`。
- 单局详情的每个人：`profileIconId`、`profileIcon`、`spells`、`perks`；`items`、`augments` 多了上面的字段。
- 数据从哪来：都是已经在读的接口（召唤师、段位、战绩、gameflow session）；客户端资料多读两个文件 `perks.json`、`perkstyles.json`（读不到就没有符文名字和图标）。

**定了什么**：

- 素材层放在 `client/` 下面（`client/mod.rs` 里登记），没动 `lib.rs`（同时在做的第 3 期在改它）。
- 第 3 期的接口提交（`8e4edbaf`）以后接上了：`lib.rs` 的 `Plugin::asset` 调 `fetch_asset`，回环代理的 `/assets/lol/<路径>` 就到素材层。测试用的 `KOTONE_LOL_CDRAGON=<本机地址>` 把 CommunityDragon 换成本机的假服务器（只认本机，写错了就不连网），和 `KOTONE_LOL_LCU`、`KOTONE_LOL_LIVE` 一样。
- 回环代理回素材时已经带 `X-Content-Type-Options: nosniff`；建议再加 `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`（SVG 直接打开时也跑不了东西）。素材层自己也挡了带脚本的 SVG。
- 声明：CommunityDragon 的出处和拳头 “Legal Jibber Jabber” 的声明写在 `plugins/lol/README.md` 的“素材”一节。

**检查**：

- `kotone-plugin-lol` 75 个过 + 1 个手动跑的（其中同时在做的第 3 期 2 个；这件事新加 17 个：素材层 12 个——生成的 PNG、三个前缀和写法、CommunityDragon 的地址、按内容认类型和挡 SVG、页面路径都过检查、客户端优先然后缓存（重启以后也在）、客户端没开 / 没这张图时问 CommunityDragon（失败的 10 分钟不再问、不放行的路径谁都不问）、同一个路径同时三次只取一次、上限满了删最久没用的、14 天重新取和没网时用旧的、坏文件和残留临时文件、本机假 CommunityDragon 走一遍真的 HTTP；页面字段 1 个；段位和战绩字段 2 个；资料里的图标和符文 1 个；**“给琴音的文字不变”** 1 个：同一份带新字段的输入，`render_scout` 和 `player_text` 的输出和改之前录的一字不差）。测试里的 PNG 是程序生成的纯色块，仓库里没有拳头的图，测试不连 CommunityDragon。clippy `-D warnings` 干净，svelte-check 0 / 0。
- 录下来的一局（`target/lol-probe/20261123-210745`）：`render_probe` 给琴音的字改前改后 67 行一字不差；三个页面的数据样例存在 `target/plugin-shots/round7/`（`match-sample.json`、`player-sample.json`、`game-sample.json`，玩家名和 puuid 换成了“玩家 N”“puuid-N”，查过没有漏）。样例里 555 个图片路径（229 个不同的）全过素材层的检查；抽 10 个按规则换成 CommunityDragon 地址，都是 200。录的时候没录段位，所以样例里 `ranked` 都是 null。
- 桌面端：新加 `plugin_apps::tests::the_proxy_takes_lol_assets_from_the_plugin`——真的回环代理 + 真的英雄联盟插件 + 本机假 CommunityDragon：`/assets/lol/lol-game-data/assets/v1/champion-icons/103.png` 回 200、`image/png`；第二次走磁盘缓存、假服务器没再被问；代理放行但插件不放行的（`lol-champ-select/v1/session.png`、`champion-summary.json`）回 404，假服务器没被问；哪儿都没有的回 404。`kotone-tauri` 119 过 + 1 忽略，clippy `--no-deps -D warnings` 干净。
- 真机 `lol_tools_real.py --while-playing`：**34/34**（以前 32 项照过，多 2 项：这局页面每个人都有英雄头像、召唤师头像的路径和两个召唤师技能，给她的字里没有路径；战绩页和单局详情有头像、等级、技能、装备图标、带稀有度的海克斯）。

**没验的（要玩家开客户端时看）**：国服客户端的图片接口本身（`/lol-game-data/assets/…png`、`/fe/lol-static-assets/images/ranked-mini-crests/*.svg`）能不能取到；取不到时会自动走 CommunityDragon。

### 英雄联盟 App 的页面（2026-11-25）

照 [设计文档](design/lol-app.md)（风格“记分纸”）、[对标调研](research/2026-11-25-lol-app-parity.md)“今晚做”第 3–8 条、[ADR 039](adr/039-desktop-companion-and-game-apps.md) D5、D6。玩家原话：“需要把这个插件当作一个独立的 MCP apps 来做……这些项目里怎么展示英雄头像、玩家头像，怎么展示海克斯 logo 都需要复刻。”“风格不需要我来拍板，你替我决定就好。”两个提交：第一段（`7dae6aa4`，骨架、“这局”、强度榜），第二段（本节其余）。

**玩家看到的**：

- 左侧栏「插件 → 英雄联盟」整页就是 App（fullscreen），导航在 App 里：**这局 · 战绩 · 英雄 · 强度榜**；宿主只留顶上一条“← 插件 / 英雄联盟”，右边一个“设置”按钮（数据开关、风险提示、操作客户端的偏好，还是宿主的 Svelte 页面）。插件关着、两个数据开关都关着时整页就是设置。
- 这局：选人 / 加载 / 对局中 / 结算四种状态（结算顶上“刚结束 · 胜利 / 失败”，点“看这局详情”进单局详情、带“刚结束”小牌）；每人一块：英雄头像压召唤师等级、两个召唤师技能、段位徽章、近 20 局胜负和胜率着色、KDA、最近 10 局小头像（赢亮输淡）、标签、开黑字母、突破口墨线圈出来；全屏时每 5 秒（不在对局里 10 秒）自己更新。
- 战绩：搜人；召唤师头像 + 等级、单双排 / 灵活的段位、近 20 局、常玩英雄；每局一行（结果格、英雄和等级、技能、符文、KDA、伤害、6 + 1 格装备、带稀有度边框的海克斯）；点一局看单局详情，“←”回来。
- 单局详情：两队（知道我是谁时是“我方 / 对面”，我那一行高亮；不知道时是“胜方 / 负方”）、伤害条、承伤、经济、补刀、装备、海克斯；点名字看那个人的战绩。
- 英雄：峡谷（分路、强度、难打 / 好打点进去、出装、符文、加点、召唤师技能、OP.GG 折叠）和海斗（名次、搭档、最好的海克斯）；强度榜点一行进英雄页（同一种模式、同一路）。
- 聊天里这几个工具的卡片都是同一个 App 的 inline：开局战绩、战绩、这局总结、英雄、强度榜；右侧栏打开是 fullscreen。
- 每页页脚：数据出处、图片出处（客户端 / CommunityDragon）、拳头 “Legal Jibber Jabber” 的声明、可以展开的“风险”（就是 `plugin.toml` 里 client / control 两个数据开关的风险原文和 LeagueAkari #523 的链接；在 Claude 这类宿主里也看得到；只提示，不拦着）。
- 没数据时的话按工具结果的 `status`（和以后的 `reason`）选翻译好的文案（“英雄联盟客户端没开……”“现在不在对局里……”“要先在设置里打开……”给“去设置”），不再直接显示给琴音的原话（原话放在 title 里方便排查）。

**结构**（`plugins/lol/app/`，Vite + Svelte，单文件打包编进插件）：

- `tokens.css`（设计文档原样）、`src/app.css`（共用组件样式）、`src/components/`（图：`Pic`、`ChampIcon`、`Augment`、`Items`、`Spells`、`Perks`、`Recent`；`RankBadge`、`Tags`、`LaneIcon`、`TopBar`、`Footer`、`StateBlock`）、`src/pages/`（`Match` + `match/`、`Player`、`Game`、`Champion`、`Tier`）、`src/status.ts`（状态码 → 文案）、`src/host.ts`（ext-apps 的 `App`：主题、宿主的颜色变量和字体、语言、显示方式写到 `<html>`）。
- 文案在 `plugins/lol/messages/`，写法和宿主一样是 `m.<键>()`（`i18n.svelte.ts` 的 Proxy）；`apps/desktop/e2e/terminology.spec.ts` 也扫 `plugins/<id>/app/src/`。
- 图：页面数据只给客户端路径，`assets.ts` 决定从哪取（Kotone 里先走回环代理的素材层，再退到 CommunityDragon；别的宿主直接 CommunityDragon）。英雄页的出装、符文、召唤师技能、海斗海克斯的图标（`stats_tools.rs`）用读过的客户端资料（`loaded_data`），没读过是空字符串（页面显示名字的第一个字）。
- 宿主：`PluginUi.settings`（通用：登记了，插件页顶栏就有“设置”按钮；App 里的“去设置”是 `ui://kotone/<插件>/settings` 链接，宿主在插件页打开它）；`McpAppFrame` 带参数、没带数据打开时（她 `ui.open`、别处跳过来）宿主先替 App 调这一页的工具再推结果；英雄联盟的五个页面在 `ui/index.ts` 都标成 App；`LolPage` 只是 App 的容器（“读客户端数据”开着从“这局”开始，只开了统计从“强度榜”开始）。旧的 Svelte 页面删了（这局、战绩、单局详情、英雄和它们的卡片、`stats.ts`），覆盖层、设置、游戏配置页的小节留着。
- 页面数据（`pages.rs`）：`match_page_in` + `MatchStage`（`phase`、`phaseSince`、`namesHidden`、`bench`、`result`、`resultPending`、每人的 `pick`）；标签带种类 `{kind, text}`（`scout::tagged`，字和给琴音的同一份）；`game_page_for(…, me)` 给我那一行 `me`、我那一队 `ally`。原来的 `match_page`、`game_page` 签名不变（自动卡片在用）。

**定了什么**：

- App 自己带导航，在 Claude 这类宿主里也能用；Kotone 插件页不再有宿主那一排分页（主会话定的）。App 里没有“设置”分页。
- 我自己的标记用“我”（新键 `plugin_lol_me_tag`）；选英雄覆盖层还是“你”（`plugin_lol_me`，它在跟玩家说话）。
- 卡片上的标签用短说法（“3 连胜”“第一次玩”“常玩 8 局”），完整的话在 title。
- 开黑按“哪一队 + teamParticipantId”分组（这个编号只在一队里有意义），整局按出现顺序给 A、B、C、D。
- 页面不设 `min-height: 100vh`：宿主按 App 报的高度定 iframe 的高，100vh 会让页面只长不缩。
- 截图脚本 `plugins/lol/app/shots.mjs`：打好的 App + 最小的假宿主（postMessage JSON-RPC），用 `target/plugin-shots/round7/*-sample.json` 改出各种状态，截到 `target/plugin-shots/round7/lol-app-*.png`。

**后台接线**（`watcher.rs`、`watcher/champ_select.rs`，自动卡片提交以后改的）：

1. 原因码：`not_in_game` 的结果多一个 `reason`：客户端没开 `client_closed`、选人 `champ_select`、加载 `loading`、别的 `not_in_game`；“这局刚结束”是 `ended`。给琴音的话一个字没变。
2. 阶段：每一轮按客户端阶段、局内接口有没有游戏时间、这局进没进过加载算出页面的阶段（`stage_of`：ChampSelect → 选人，GameStart / InProgress / Reconnect 没有游戏时间 → 加载、有 → 对局中，WaitingForStats / PreEndOfGame / EndOfGame 和打完回到大厅 → 结算；选人被秒退的不算这局），变了就记时间。`lol.scout` 和“开局战绩”自动卡片都改调 `match_page_in`。
3. 选人（G1）：ChampSelect 时读 `/lol-champ-select/v1/session`：双方的英雄（锁了 / 预选 / 还没选）、我方的分路和召唤师技能、备选席、名字是否被隐藏；我方看得到名字的人马上查战绩（同一个对局 id，进了加载接着用，不重查）；对面和看不到名字的队友没有 `puuid`，不查，也不算“还在查”。`lol.scout` 在选人时也答，只说我方。
4. 结算（G29）：“这局总结”自动卡片读到这局的详情时，顺手把“我那一队赢没赢、时长”写进这局（`MatchStage.result`）；结算后 60 秒内没拿到是 `resultPending`，过了页面说“这局的结果没拿到，去战绩里看”。
5. `lol.game` 和“这局总结”卡片都改调 `game_page_for(…, 我的 puuid)`：我那一行、我方 / 对面。

**检查**：

- svelte-check：桌面端 0 / 0，App 0 / 0；`pnpm -C apps/desktop build:apps` 过。
- `cargo test -p kotone-plugin-lol` 88 个过 + 1 个手动跑的（这件事新加：标签带种类、阶段字段、`game_page_for`、没有 puuid 的人不算“还在查”、选人 session 的两种（排位隐藏名字、海斗备选席）、页面阶段跟着客户端阶段；“给琴音的文字不变”照过）；`cargo clippy -p kotone-plugin-lol --no-deps --all-targets -- -D warnings` 干净（不加 `--no-deps` 会撞上别人正在改的 `kotone-core`）。
- Playwright：`plugins/lol/e2e/` 19 个全过（插件页整页是 App、四个分页、这局加载 / 结算 / 不在对局里、战绩搜人、单局详情和返回、英雄峡谷 / 海斗、强度榜点一行进英雄页、设置按钮、App 里的“去设置”、聊天卡片和右侧栏、`ui.open` 带参数）；全量 181 过、1 个没过（`terminology` 说 `plugin_lol_event_*` 十个键没用到：别的子代理正在加的游戏事件文案，不是这件事的）。
- 截图（自己看过，和样张对过）：`target/plugin-shots/round7/lol-app-*.png`（App 单独打开：这局四种状态、亮暗、窄屏、英文、卡片 420 / 360、战绩、单局详情、这局总结卡片、英雄峡谷 / 海斗、强度榜）和 `kotone-*.png`（Kotone 里：插件页、设置、聊天卡片、右侧栏；预览里的图是假代理给的色块）。
- 没跑：真机 `lol_tools_real.py`、`game_events_real.py`（要重编整个桌面端，`real.lock` 一直有人在用）。要主会话提交以后跑一遍：给琴音的字有单元测试管着，选人阶段的“这局”只能在真客户端里看。

**审查发现、已修（2026-11-25 夜）**：前端（`ui/index.ts`）和浏览器预览的模拟（`pluginsMock.ts`）把五页都标成了 App，Rust 的 `app.rs` `APP_PAGES` 却只有强度榜和这局，旧的 Svelte 页面又删了，于是真 Kotone 里单独打开战绩、单局详情、英雄（聊天卡片、结算后的“这局总结”自动卡片、`ui.open`、外部宿主）都是“页面没有”；预览和 Playwright 照过，真机只开过强度榜和这局，`mcp_app_resources_real.py` 还把“这三页不是 App”写成了断言。修法：`APP_PAGES` 补齐五页（数据开关照清单的 `needs`，CSP 和另外两页一样只有 CommunityDragon），`kotone.exe mcp` 里 `kotone_lol_player` / `game` / `champion` 也连上了自己的页面。加了边界测试（原则 16）：插件里“清单除设置以外每页都是 App、都写了 `needs`”；桌面端 `plugin_apps::tests::app_pages_agree_with_the_frontend` 读 `plugins/<id>/ui/index.ts` 和 `pluginsMock.ts` 的文本，和每个内置插件的 `resource` 比集合、和清单的 `needs` 比开关，三处对不上就报错（把 `APP_PAGES` 改回两页试过，会报）。真机：`mcp_app_resources_real.py` 改回事实 22/22；`mcp_app_real.py` 接上假客户端，插件页从“这局”开始，再用 `ui.open` 在右侧栏单独打开战绩、单局详情、英雄、强度榜，29/29，截图 `target/plugin-shots/round7/real-*.png`。

### 设置重排（2026-11-25）

交接第十一节第 8 条。玩家原话：“陪玩相关的设置项我觉得都应该在游戏陪玩-角色页面配置吧，现在这个页面有角色、记忆、技能，再加上唤醒、交互、权限、用量我觉得很合理啊。然后模型选择这块，我觉得还是放在 chat 聊天吧。”

**玩家看到的：**

- 左侧栏「游戏陪玩」里琴音那一项（显示角色的名字）点开是角色页，分页一排七个：**角色 · 记忆 · 技能 · 唤醒 · 交互 · 权限 · 用量**。后四个是原来「设置」的陪玩组整排搬过来，内容不变，只改两个叫法：“怎么叫她” → **唤醒**，“她怎么回答” → **交互**。顶栏原来那句“{name}的设置在「设置」里 →”去掉了。窗口窄时分页横着滚，不换行。
- 「设置」只剩一排：模型 · API 连接 · 声音 · 调参 · 外观 · 系统 · 浏览器。只有一组，就不写组名；说明下面一行“唤醒、交互、权限和用量在{name}自己的页面里。去「{name}」→”，找老位置的人一眼看得到。0.1.x 升级来、还没启用琴音的玩家不显示这一行，角色页也没有这四个分页（和原来设置里不显示陪玩组一样；跳过去会落到聊天页的填 Key 卡片）。
- **她用的模型**回到聊天页：输入框底栏右边、权限盾牌左边，一个小按钮写着当前模型（`test ▾`），点开按 API 连接分组列模型（服务商的模型列表，读不到用预置的），选别的连接里的模型会同时换琴音用的连接；最下面“管理 API 连接 →”。改的是和原来「设置 → 她怎么回答 → 她用的模型」同一组字段（`assistant.connectionId`，升级用户是 `companionBindings.connectionId`；连接的 `model`），换模型不自检。没有填了 Key 的连接时按钮写“选模型”（黄色），菜单里写“还没有填了 Key 的 API 连接：在上面填 DeepSeek Key，或者到「设置 → API 连接」里加一条”；空白对话没 Key 时照旧只有填 Key 的卡片。聊天页上面“{name}还没准备好”那一条的按钮从“前往设置”改成“换个模型 →”，直接打开这个菜单。
- 「交互」里没有选模型了，最上面留一行“她用的模型在聊天页输入框旁边选。去选 →”。以后的右下角弹窗开关照样加一个 `SettingsSection` 进「交互」（`AssistantPage.svelte` 里留了注释）。

**旧位置怎么落：**

- `ui.open ui://kotone/settings/call | reply | permissions | usage`：地址不变（桌面端 `SETTINGS_SECTIONS` 照旧认），界面收到后打开角色页的唤醒 / 交互 / 权限 / 用量。做法是 `navigation.ts` 的 `normalizeTarget`：`openTarget`、`openLocation` 和设置窗口收跳转请求时都经过它，一处落地。`ui.open` 的工具说明（`catalog_host.rs`）和 `docs/tool-service.md` 跟着改了。
- 设置说明（`settings_guide.rs`）全部改成新写法：“琴音 → 唤醒 → 呼叫键”“琴音 → 交互 → 气泡”“琴音 → 权限”“琴音 → 用量”“琴音 → 角色”“聊天页 → 她用的模型”；测试把“怎么叫她 / 她怎么回答 / 她是谁 / 设置 → 权限 / 设置 → 用量”列为不许再出现。旧对话里的写法都还认：“设置 → 怎么叫她 → …”“设置 → 她怎么回答 → …”“设置 → 权限 / 用量”“琴音 → 设置 → …”“琴音 → 她是谁”落到同一处；“她用的模型”“AI 连接”的旧位置落到聊天页并打开模型菜单。
- 系统提示（`prompt.rs`：权限只能玩家自己改、没设呼叫键时建议去哪设）、没选连接时的提示（`companion.rs`）、`docs/assistant.md`、游戏配置页和游戏语音插件里的提示都换成新位置。
- 左侧栏顺序 / 隐藏（`ui.sidebar`）和 `ui.startPage`：存的是左侧栏的页面 id（`assistant`、`advanced`……）和 `chat` / `voiceInput`，设置里的分页从来不进这两处，没有要搬的旧值。
- 术语表“界面用语 v2”加了“角色页”“唤醒”“交互”“她用的模型”“用量”，旧说法写进去，`设置 → 怎么叫她`、`设置 → 她怎么回答`、`设置 → 权限`、`设置 → 用量` 列为禁用说法（`terminology.spec` 查）。

**截图**：`target/plugin-shots/round7/`：`role-identity.png`、`role-memory.png`、`role-skills.png`、`role-call.png`（唤醒）、`role-reply.png`（交互）、`role-permissions.png`、`role-usage.png`、`settings.png`、`chat-composer.png`、`chat-model-menu.png`。

**检查**：svelte-check 0 / 0；Playwright 全量 169 个：最后一次整套 167 过，2 个没过（`danmaku.spec` 的弹幕样式、`music.spec` 的关掉插件），原因是跑的时候别的子代理在改文件，Vite 把页面整页重载了（调试时看到测试中途页面又导航了一次、角色页回到聊天页），这两个文件单跑 15/15 过；改动相关的几个 spec 重跑 40/40、8/8。`kotone-core` 303 + 集成 84（1 跳过）、`kotone-agent` 82、`kotone-tools` 111、桌面端 113（1 跳过）；clippy：core、agent、tools 干净；桌面端 `-D warnings` 卡在不是这次改的两处（`plugins/lol` 的 6 个 `unnecessary_map_or`、`plugins.rs` 测试里的 `field_reassign_with_default`，HEAD 上在这个工具链 1.98 下就有），放过这两条后干净。没跑真机脚本（只动了界面和文字）。

### 自动卡片和消息提示（2026-11-25）

交接第十一节第 17、18 条，[ADR 039](adr/039-desktop-companion-and-game-apps.md) D3、D4（怎么做的写在 ADR 039 的实现记录里）。玩家原话：“检测到对局可以自动推送一些双方战绩之类的卡片进上下文，可以不触发 LLM……”“自动推卡片的同时右下角可以弹个消息，这样用户知道在哪里看新内容，更有那种桌面聊天陪伴的感觉。”

**玩家看到的**：英雄联盟进加载、战绩查完时聊天里多一张“英雄联盟 发来 · 开局战绩”（“这局”的卡片），结算后一张“这局总结”（单局详情）；同时右下角弹消息提示（琴音的头像、“琴音 · 英雄联盟”、“对面的战绩到了”），6 秒后消失，点它打开聊天、滚到卡片。开关在“游戏陪玩 → 角色 → 交互 → 自动卡片”。不叫模型、不花钱。

**改了哪里**（按层）：

- 契约：`kotone-plugin-api` 的 `AutoCard` 和 `Host::auto_card`（默认拒绝，只有桌面端给内置插件实现）。
- 纯逻辑：`kotone-agent/src/auto_card.rs`（两条消息怎么拼、去重、“压着”的卡片）；`kotone-ai` 的 `Message::Notice` 多一个 `card` 字段（只给界面），对话标题不拿来源 `auto-card` 的消息。
- 设置：`kotone-core` 的 `agent.autoCards {enabled, popup}`（默认都开）和设置说明两条（“琴音 → 交互 → 自动卡片”）。
- 桌面端：`agent/auto_card.rs`（发到哪段对话、琴音空着才记进对话、`agent_held_cards`、`auto_card_open`）；`relay.rs` 的 `main_is_quiet` 改成 `pub(super)`（同一个空闲判断）；`message_popup.rs` 和窗口 `message-popup`（`tauri.conf.json`、`capabilities/message-popup.json`）；`overlay_layer::exclude_from_capture` 改成 `pub(crate)`；系统适配层 `fullscreen::popup_blocked` / `notification_state`。
- 界面：聊天里的 `AutoCard.svelte`（`timeline.ts` 让卡片自成一轮、给模型的那条不显示；`chatController` 处理“压着”的卡片和定位）；消息提示页面 `routes/messagePopup`；“交互”里的 `AutoCardSettings.svelte`；文案在 `messages/*/chat.json`、`companion.json`。
- 英雄联盟：`plugins/lol/src/watcher/auto_cards.rs`（什么时候发、那两句话怎么写），`watcher.rs` 只加了模块、`State.cards` 和 `tick` 里一行。

**检查**：

- Rust 单元测试（新加 14 个）：`kotone-agent` 的 `auto_card` 6 个——发卡片时假模型 0 次调用、下一轮模型看到那句话但看不到页面数据（玩家名不在请求里）；DeepSeek 请求体前面每一条逐字节不变、那句话在玩家下一句前面；她在忙时压着、这一轮里一条都没插进去、结束后接在她的回答后面；同一个插件同一个键只发一次；关着不发、只关消息提示时照发不弹、旧配置按开；消息形状和截字。`kotone-core` 1 个（旧配置读进来两个开关都开）；`kotone-plugin-api` 1 个（默认实现拒绝、页面要是清单里的）；`kotone-platform-windows` 1 个（哪些通知状态不弹）；`kotone-plugin-lol` 5 个（开局战绩等查完或 20 秒、一局一次、开打后才看到的不发；自定义 / 训练 / 观战不发；开局那句的突破口、开黑、还在查；这局总结那句；重开局和详情还没出来）。
- `cargo test`：`kotone-agent` 102、`kotone-ai` 92、`kotone-core` 304、`kotone-plugin-api` 14、`kotone-platform-windows` 91（9 忽略）、`kotone-plugin-lol` 81（1 忽略）、`kotone-tauri` 118（1 忽略），全过；clippy `-D warnings`（这七个 crate、`--all-targets`）干净；新文件 rustfmt 过。
- svelte-check 0 / 0（英雄联盟 App 也 0 / 0）。Playwright：新的 `e2e/auto-card.spec.ts` 5/5（聊天里“英雄联盟 发来”的卡片、点开到右侧栏、给她的那句不显示；她在回答时来的卡片先显示在最后、这一轮的字留在她那一轮、记进对话后不重复；点消息提示从别的页面回到聊天、卡片滚进来闪一下；“交互”里两个开关默认开、改了存 `agent.autoCards`、关掉自动卡片时消息提示的开关变灰；消息提示页面 320 × 72、“琴音 · 英雄联盟”、点 × 只关、点它是打开）；`terminology.spec` 4/4；全量 178 个 177 过，没过的 1 个是 `sidebar-edit.spec` 新加的“每个插件卡片有自己的 logo”（同时在做的插件 logo，`fetch` 开发服务器上 `plugins/*/logo.svg` 的地址失败，单跑也一样，和这件事无关）。
- 截图：`target/plugin-shots/round7/chat-auto-card.png`、`settings-auto-cards.png`、`message-popup-preview.png`（Playwright），`message-popup-real.png`（真机，见下）。
- 真机 `auto_card_real.py`（新脚本，假客户端的一局 + 真 DeepSeek）：**13/13**。选人时没有卡片；进加载 2 秒后“开局战绩”接在“你好”那一轮后面（前面两条逐字不变），给琴音的那句是“开局战绩（海克斯大乱斗）。对面可能的突破口：艾希，最近 20 局只赢了 6 局、场均死 13.2 次。我方有 3 人开黑。”；消息提示弹出来了（`WS_EX_TOPMOST | WS_EX_NOACTIVATE`，前台窗口前后是同一个），截图 `target/plugin-shots/round7/message-popup-real.png`；结算 2 秒后“这局总结”（“这局赢了（海克斯大乱斗，16 分 54 秒）。你用韦鲁斯 8/9/43，伤害全队第三。”）；一局正好两张；从第一张卡片到下一次打字，`usage.jsonl` 一条没多；再打字问“刚才英雄联盟发来的卡片说了什么”，她答“开局说对面艾希是突破口……这局赢了，你韦鲁斯 8/9/43”。第一次跑时消息提示没弹：`--background` 启动后藏着的主窗口“Kotone 设置”占着前台（看不见），“Kotone 的窗口在前台”的判断把它算进去了；改成只算看得见、没最小化的窗口。

### 第 7 步：英雄联盟的事件目录由插件登记（2026-11-25）

交接第十一节第 20 条。第 1 步“没搬的”和第 7 步（部分）记着：英雄联盟的事件目录（`lol.dragon_kill` 等）还是 `kotone-core::games` 里的常量，插件关着时自动化编辑里还能选到；要先有“插件登记事件”的机制。

**怎么做的**（ADR 036 D4 的目录形状，ADR 038 D3 第 10 条，原则 15、16、19、21）：

- **登记方式只有一种**：插件在 `plugin.toml` 里用 `[[events]]` 登记（`type = "<适配器>.<名字>"`、`label` 中文说明、`needs` 要的数据开关、`data.<字段> = "说明"`）。`Manifest::parse` 查：要写 `adapter`（不能叫 `game` / `generic`），类型以 `<适配器>.` 开头、名字小写加下划线，有说明，字段名 camelCase，`needs` 是自己的开关，不重名。英雄联盟的十种从 `kotone-core` 搬进 `plugins/lol/plugin.toml`（都要“读客户端数据”）；本体（`kotone_core::games::generic_events`）只留通用的 `game.*`。“团灭”“三杀以上”原来在本体目录里的补充说明挪进了 `data.team` / `data.count` 的说明，`label` 和界面上的名字一样（插件文案 `plugin_lol_event_<名字>`，测试查一致）。
- **合并**：`kotone_plugin_api::event_catalog(清单, 设置)` → `kotone_core::games::EventCatalog`（本体的 + 每个插件登记的，关着的也在，标着 `off`：“英雄联盟插件关着”“英雄联盟插件的“读客户端数据”关着”）。桌面端 `games::event_catalog` 用内置插件表算；自动化保存时的校验（`AutomationLibrary::save` 多一个目录参数）、`game.state` 的 `events`（`EventCatalog::summary`）都用它。界面照同样的规则从插件清单（`manifest.events`）现算（`gameEvents.ts` 的 `eventCatalog`），自动化编辑、自动化列表、游戏配置页的“游戏事件”都用它；前端写死的英雄联盟事件表和本体文案里的 `game_event_lol_*` 删了。
- **插件关着时**：它的事不出现在选择里；已经保存的不删、不报废：列表里多一行“英雄联盟插件关着，这个触发不会发生；到「插件」里打开以后照常触发。”，编辑时当前选的那项照样显示（“拿了小龙（英雄联盟插件关着）”）、下面同一句提示，照样能保存；`automation.save` 照常成功，结果多一项 `triggerNotes`（同一句）；`game.state` 里英雄联盟那一组不列事件，只有一句 `off`。运行时照旧只是收不到。没装这个插件（目录里没有）的名字照旧报“没有这种游戏事件”。
- **插件只报得出登记过的**：`DesktopHost::game_event` 挡掉清单里没有的（记日志）。`plugins::enabled` / `source_enabled` 改成调契约里的 `Manifest::enabled_in` / `source_enabled_in`（事件目录也用它，规则只有一份）。
- **旧配置**：自动化文件格式没变，旧版存的 `lol.*` 触发照样读进来、照样触发（测试用 0.2.0-rc.7 的样子）；装回旧版时旧版用自己的常量，也认得这些名字。

**边界写成测试**：

- `crates/kotone-plugin-api/tests/event_boundaries.rs`：插件登记的事件名在本体代码里一个都不出现（`crates/*/src`、`crates/*/tests`、桌面端后端和界面、本体文案；跳过生成的 `paraglide` 和预览里假装插件后端的 `*Mock.ts`）；两个插件不登记同一种事；扫描真能抓到（插件自己的源码里有）。本体的测试改用一个假的“示例”插件（`demo.*`），`kotone-tools` 的工具说明里不再举 `lol.dragon_kill` 的例子。
- `plugins/lol/src/event_catalog_tests.rs`：局内接口的每种事都换得出来、都在清单里、`data` 字段都登记了；源码里 `game_event` 报的字面量都登记了、不是字面量的只能是 `new_events` 的 `kind`；登记的每种源码里都报得出（没有空登记）；文案和清单一致；宿主的目录跟着插件开关；旧的自动化读得进来、照样触发，插件关着时改名保存照常、提醒说清楚，没有这个插件时报错。

**检查**：`kotone-core` 307 + 编排 84（1 跳过，检查没改）、`kotone-plugin-api` 17 + 边界 3 + 事件边界 3、`kotone-plugin-lol` 85（1 跳过）、`kotone-tools` 113、桌面端 123（1 跳过），全过；clippy `-D warnings`（这五个 crate、`--all-targets`）干净；svelte-check 0 / 0。Playwright：新的 `e2e/plugin-events.spec.ts`（插件开着能选“拿了小龙”、带“哪一边”；关掉英雄联盟插件后保存过的还在、列表和编辑里说清楚、照样能保存、新建时选不到；打开回来又能选）和相关的 `automation`、`game-profile`、`terminology`、`plugins/lol/e2e/lol-plugin` 16/16（`terminology.spec` 的“文案都用到了”认插件文案 `plugin_<id>_event_<名字>` 这种按约定取的键）；全量 182 个 174 过，没过的 8 个（`post-processing`、`right-sidebar`、`sidebar-edit`、`update-dialog`、游戏语音、同传、`lol-plugin` 的“设置位置打开插件页”）单独重跑 23/23 过，是跑的时候别的子代理在改文件、开发服务器重载页面。真机（不开游戏、不发系统按键、隔离的 `KOTONE_HOME`）：`game_events_real.py` **12/12**（真 DeepSeek；进加载时她用 `lol.scout` 说“对面艾希最近二十局只赢六场，场均死十三次，先盯她。”，对面拿龙时播报“对面拿了火龙”，`game.state` 的事件目录里英雄联盟那一组来自插件）；另跑了一个一次性的检查（不叫模型）6/6：英雄联盟插件关着时 `automation.save` 存 `lol.dragon_kill` 照常成功、带 `triggerNotes`，`game.state` 里英雄联盟那一组没有事件、只有 `off`，`lol.pentakill` 被拒；打开以后没有提醒、列出 10 种。

### `--background` 启动不抢焦点（2026-11-25）

做自动卡片时看到的老问题：`kotone.exe --background` 启动后，藏着的“Kotone 设置”窗口成了前台窗口（`GetForegroundWindow` 是它），终端的焦点被抢走。外部 Agent 用 `kotone_launch` / `kotone-cli app launch` 后台启动 Kotone 时，玩家要是在打游戏，游戏就丢了焦点。这和 `--background` 的说法（只驻留托盘、不打开窗口、不抢焦点）、[ADR 032](adr/032-game-chat-send-reliability.md) 的精神不合。

**原因**：设置窗口建的时候默认要焦点（`focus: true`），WebView2 把键盘焦点移进还藏着的窗口（wry 的 `MoveFocus`），把它激活成了前台。正常启动时代码接着就显示窗口、`set_focus`，所以一直没看出来。

**怎么改的**：

- `tauri.conf.json` 里设置窗口加 `"focus": false`（气泡、覆盖层、提醒、消息提示这些窗口本来就是）。
- 只改这一处的话，正常启动时窗口到了前面，页面却收不到键（要先点一下）：桌面端开了 Tauri 的 `unstable`（多 WebView），页面是窗口里的子 WebView，激活窗口不会自己把键盘焦点交给它。所以 `window.rs` 加了 `show_and_focus`（显示、取消最小化、拿到前面、再把焦点交给页面），玩家自己打开窗口的四个地方都用它：正常启动、托盘、第二个实例、点消息提示（`auto_card_open`）。
- `--background` 的判断没变（`lib.rs` 里照旧不显示窗口）。开机自启不带 `--background`（照旧打开窗口），没动。

**玩家看到的**：后台启动 Kotone 时，游戏或终端的焦点不再被抢；双击打开、托盘打开、点消息提示，窗口照旧到前面，能直接打字。

**检查**：

- 真机 `scripts/manual-e2e/background_focus_real.py`（新脚本；隔离的 `KOTONE_HOME`，不发按键、不点鼠标，拿 `real.lock`，前台是游戏或已经是 Kotone 就不跑，`--normal` / `--open` 只在前台是终端时跑）：
  - 改前（`--background`）**2/5**：0.85 秒时前台从终端（warp.exe）变成 kotone.exe 的“Kotone 设置”（看不见），它还是自己线程的活动窗口；
  - 改后 `--background` **5/5**：前台一直是终端，设置窗口藏着、没被激活；
  - `--normal`（不带 `--background`）**4/4**：窗口显示、成为前台、键盘焦点在页面（WebView）里。只加 `focus: false` 时这里 3/4（焦点停在窗口本身），所以加了 `show_and_focus`；改前也是 4/4；
  - `--open`（后台启动，再经调试端口调 `auto_card_open`，和托盘是同一个函数）**10/10**。
- `cargo test -p kotone-tauri` 123 过（1 忽略）；clippy `-D warnings`（`-p kotone-tauri --all-targets`，没加 `--no-deps`）干净；改过的文件 rustfmt 过。
- `message_popup.rs` 的 `kotone_in_front` 只算看得见的窗口（自动卡片那次为这个问题加的），留着：判断本来就该这样。

### 覆盖层对齐 AramHex（2026-11-25）

交接第十一节第 14 条（[对标调研](research/2026-11-25-lol-app-parity.md)“今晚做”第 9 条，差距表 G8、G24）。玩家原话：“完成度对标应该是 leagueAkari 和 aramHex……这些项目里怎么展示英雄头像、玩家头像，怎么展示海克斯 logo 都需要复刻。”覆盖层本身的做法（本体的覆盖层、只盖条目的并集、不进截图、不抢焦点）没动，只改了插件给的数据和两个覆盖层页面。

**玩家看到的**（截图 `target/plugin-shots/round7/overlay-augment-scores.png`、`overlay-champ-select.png`）：

- **海斗三选一**：每张卡片上面的标签从一行变成三行，照 AramHex 的三选一标签：
  - 第一行：评级小牌（S–D）+ 判断：**首选**（胜色）/ **可选**（正文色）/ **可考虑刷新**（负色）。首选的不到 1000 局时写“相对较好 · 仅供参考”；样本太少的写“样本太少 · 建议刷新”；谁都没拿过的新海克斯写“无数据”。
  - 第二行：原来的“+7.9 · 57.8% · 1.1万局”；这个英雄一局都没拿过的写“这个英雄几乎不拿”。
  - 第三行：依据（“棱彩第 2/38 · 样本少”）和出处 aramhex-data。
  - 首选那张从金色描边改成 2 px 白色墨线（设计文档第四节 4，不用拳头的金色）；不到 300 局的照旧变淡。标签高了（游戏画面高度的 5.5% → 8.5%），底边还是在卡片名字上面一点，盖住海克斯图标的下半截（AramHex 的默认位置）。
- **选英雄面板（海斗）**：每个英雄前面一个评级小牌，后面是胜率和“第 31/172”；“你”那一行淡蓝底。备选席两个一行、按胜率从高到低，胜率比你现在的英雄高的标 ↑ 和胜色，最高的那个用白色墨线圈出来（值得换），小标题写“备选 · ↑ 胜率比你现在的英雄高”。面板高度跟着行数走：以前固定是客户端高度的 32%，现在 5 个人约 22%，带 4 个备选约 31%，10 个备选约 39%。
- 重随、和备选席换英雄以后 2 秒内跟着变（照旧每一轮重读选人接口，内容一样不重发），和 AramHex 一直重认画面的效果一样。
- 一项数据都没有时不显示面板（腾讯的海斗榜拿不到；普通大乱斗没有分路，腾讯 101 也没有大乱斗的榜），不拿一块“—”挡着客户端。峡谷照旧是分路强度（腾讯的 T0–T5）和胜率，不加 S–D。

**规则**（`plugins/lol/src/overlays/rules.rs`，照 AramHex 原仓库 czgreat/aramhex `main` 分支 `17ef65cd` 的源码用 Rust 重写；MIT，出处写在 `THIRD_PARTY_NOTICES.md`，许可证全文 `licenses/aramhex-MIT.txt`）：

- **评级**（`engine.py` 的 `GRADES`、`rank_grade`、`hero_brief`）：只按胜率重排名次（腾讯榜自带的名次掺了选用率：真数据里腾讯排第 1 的塔姆，按胜率是第 8），前 10% S、30% A、60% B、85% C、其余 D。
- **备选比较**（`overlay.py` 的 `ChampSelectLayer`）：胜率比我现在的英雄高的标出来，其中最高的圈出来；我的英雄没有胜率时不比。
- **三选一**（`recommend.py` 的 `evaluate`）：分数是平滑后的胜率比英雄平均高几个点（k = 300，和原来一样）；本英雄不到 1000 局算“样本偏少”；同屏最多那张够 1000 局时，不到 1000 局的不当首选，不到它十分之一的“样本太少、建议刷新”；首选在样本够的里面挑分最高的；其余的比平均低 0.5 个点以上、或比首选低 2 个点以上的“可考虑刷新”，别的“可选”；这个英雄没拿过、别的英雄拿过的按 −3 分算。几张一起判断：换了一张，另外几张也跟着重算。

**替玩家定的**：

- **海克斯也给 S–D**：AramHex 标签里的“S 级”是腾讯给每个英雄的官方层级，我们没有这份数据。改成同稀有度里的名次、用同一张表换算；名次按**平滑后**的胜率排（AramHex 的“金色胜率第 3/40”按原始胜率排，几十局全胜的会排第一；平滑以后和显示的分数一致）。
- 没做 AramHex 的“没有胜率时按官方层级估计”“按分段综合”“测试服数据”“没认出英雄时按全英雄的数据”：数据只有 aramhex-data 的 英雄 × 海克斯；没认出英雄时照旧不贴。
- 备选比我高的除了胜色还加 ↑：设计文档写着颜色从不单独表达意思。
- **换卡的那一下**：AramHex 先藏起正在换的那张、另外两张留着原来的结果。我们每 2 秒整组重认，正在翻的那张认不出时只剩两张、按两张判断，下一轮三张都在再重算，首选最多可能闪 2 秒。要照做得多存一份上一轮的判断，这次没做。
- 覆盖层的颜色和评级小牌放在 `plugins/lol/ui/overlay.css`（值和 App 暗色的 `--lol-*` 一样）：覆盖层是本体窗口里的 Svelte，用不到 App 的 `tokens.css`。
- 选人接口没有 `localPlayerCellId` 时，以前每个人都被当成“我”（`None == None`），顺手改了；备选席里 id 是 0 的不算。

**检查**：

- `kotone-plugin-lol` 104 过 + 2 个手动跑的（这件事新加 8 个：评级表的边界、只按胜率排名、首选 / 可选 / 可考虑刷新、样本偏少和样本太少、几乎不拿和无数据和名次并列；选英雄的评级和备选比较、重随以后跟着变、没数据不显示和高度跟着行数；改了 1 个：三张卡片的判断、评级、依据进覆盖层数据）；clippy `-D warnings`（`--all-targets`）干净；改过的文件 rustfmt 过。
- 真数据（新加的手动测试 `real_data_overlay`：腾讯 101 + aramhex-data）：海斗榜 173 个英雄，S 17、A 34、B 52、C 44、D 26；艾瑞莉娅面对终极九头蛇 / 夺金 / 登峰造极境：首选、棱彩第 1/47 是 S（+7.9，10,891 局）/ 样本太少 · 建议刷新（511 局，不到 10,891 的十分之一）/ 可考虑刷新、47/47 是 D（−9.3）。
- svelte-check 0 / 0；Playwright：覆盖层两条（三选一：判断、评级、依据、颜色、首选是白色墨线不是金色、变淡、位置、点击穿透、字放得下；选英雄：评级和颜色、备选的 ↑ 和圈、按行数给的高度放得下）2/2；全量 191 个 172 过；没过的 19 个（`copilot-flow`、`navigation`、`onboarding`、`sidebar-edit`、`update-dialog`、游戏语音、同传、`lol-plugin` 的“设置位置打开插件页”，都不碰覆盖层）连同 `lol-stats` 所在的 9 个文件单独重跑 44/44 过：跑的时候别的子代理在改设置和插件页的文件，开发服务器在重载页面。

**还没测（要玩家开局）**：三行的标签盖在真的三选一卡片上挡不挡东西；面板变高以后挡不挡客户端右上角的东西；海斗选人时备选席满 10 个的样子。

### 英雄联盟 App：英雄页的技能图标（2026-11-25）

交接第十一节“素材对等”的“技能”：做 App 的子代理留下“英雄技能图标（QWER）还是字母格子：英雄技能资料要另读”。照 [对标调研](research/2026-11-25-lol-app-parity.md)第三节 1“英雄技能”、[设计文档](design/lol-app.md)第一节 7。

**玩家看到的**：英雄页“加点”那块，主升那一排是被动 + Q W E R 的技能图标、前几级的加点顺序每一级一个图标，图上右下角压键位字母（深底白字小牌），鼠标停上去是“Q · 斩钢闪”“被动 · 浪客之道”。取不到资料或图时还是原来的字母格子（R 墨底），没有被动那一格。给琴音的字一个没变。

**怎么做的**：

- 资料：`/lol-game-data/assets/v1/champions/{id}.json` 的 `passive`、`spells[]`（`gamedata::abilities` 解析：按 P Q W E R 排，`spellKey` 认大小写、没写按位置，图标路径过素材层的检查，不放行的写成空的）。取法在 `plugins/lol/src/client/abilities.rs`：内存 → 客户端（连着、“读客户端数据”开着时）→ 磁盘缓存（插件目录下的 `abilities/`，7 天）→ CommunityDragon 上同一个文件（`global/zh_cn/v1/champions/{id}.json`，中文）→ 过期的缓存 → 空的。
- 页面数据：`lol.champion` 峡谷的 `structuredContent` 加 `abilities: [{key, name, icon}]`（设计文档原来写的是对象，改成列表，文档同时改了）。
- 页面：新组件 `AbilityIcon.svelte`（复用 `Pic`，`Pic` 多一个 `badge`：有图时压在右下角，没图时字母已经在格子中间，不重复）。

**定了什么**：

- **素材层不加前缀**：技能图标在 `/lol-game-data/assets/ASSETS/Characters/…` 下，已经在放行的 `/lol-game-data/assets/` 里（测试查过）。英雄资料的 JSON 不经素材层（素材层只给图片），由插件自己在后台读。
- **客户端没开时用 CommunityDragon 的同一个文件**，不用腾讯的 `js/hero/{id}.js`：腾讯给的图标是它 CDN 的地址，素材层和 App 的 CSP 都不认（调研第三节 4 第 5 条）。CommunityDragon 的路径和客户端一样，图照素材层的规则取。
- **不拖慢琴音**：和腾讯 101 的请求一起等，客户端、CommunityDragon 各最多 2 秒；CommunityDragon 慢了这次先不带（字母格子），后台取完存进缓存，下次就有；同一个英雄同时只取一份，失败的 10 分钟内不再试（照素材层）。今晚从这台机器 `curl` 实测 CommunityDragon 一次 0.5 秒、一次 17 秒，所以要这个上限。
- 磁盘上只存解析好的五条（一个英雄几百字节），不存整个文件（60–90 KB，大半是皮肤）；读回来的图标路径再过一遍素材层的检查。

**检查**：

- `cargo test -p kotone-plugin-lol` 104 过（2 个手动跑的忽略）；这件事新加 7 个：解析（按键位排、图标路径都过素材层的检查，不用加前缀）、怪文件（没写键、乱序、重复、不认识的键、不放行的路径）、两个地址、客户端优先并记住（重启以后磁盘上只有五条）、客户端没开问本机假 CommunityDragon（没有的 404、10 分钟内不再问）、CommunityDragon 慢时这次空着、后台取完下次就有且只取一份、磁盘文件被改过或过期。测试不连真的 CommunityDragon。clippy `--all-targets -D warnings` 干净，改过的文件 rustfmt 过。
- `pnpm -C apps/desktop check`：桌面端 0 / 0，App 0 / 0。
- Playwright：`plugins/lol/e2e/lol-pages.spec.ts` 新加“英雄页有技能图标”（艾瑞莉娅：被动 + Q W E R 五个、图走 Kotone 的素材层、加载出来、键位小牌、鼠标停留提示；贾克斯没有资料：四个字母格子、没有图）；`lol-pages` + `lol-stats` 12/12。全量 190 个 181 过、9 个没过（设置位置、`ui.open` 旧地址、侧栏、更新对话框，跑的时候别的子代理在改文件、开发服务器重载），单独重跑 12/12 过。
- 截图（自己看过）：`target/plugin-shots/round7/lol-app-champion-rift-{light,dark}.png`（亚索的技能图标，图从 CommunityDragon 来）、`lol-app-champion-rift-letters-light.png`（没资料时的字母格子）、`kotone-champion-skills.png`（Kotone 里，图是预览的假代理给的色块）。
- 没跑：真机 `lol_tools_real.py`（要重编桌面端；给琴音的字没改，有单元测试管着）。要玩家开客户端时看：国服客户端的 `champions/{id}.json` 能不能取到（取不到时自动走 CommunityDragon）。
