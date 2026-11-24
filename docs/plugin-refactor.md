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
| `open_chat_key`、`send_key`、`pre_open_delay_ms`、`pre_paste_delay_ms`、`pre_send_delay_ms`、`prefer_clipboard_paste`、`channels`、`console_chat` | 语音输入插件：`plugins.voice-input.games.<游戏>` |
| `allow_automation_input` | 留在游戏配置（本体）：这个游戏容不容忍模拟输入是游戏自己的事（D3 第 9 条），按键程序插件经宿主读（2026-11-24 改，原来要搬进按键程序插件） |
| `computer_use`（让她操作：询问 / 一直允许 / 禁止） | 本体：她的“手”（computer use 留在本体，2026-11-24 改） |
| （设置里的 `lol.enabled`） | 英雄联盟插件：`plugins.lol.sources.client` |

- 内置的 lol、dota2、cs2 是游戏配置，不是插件。英雄联盟插件用 `games = ["lol"]` 挂在 lol 上，以后 CS2、Dota 2 插件也一样。
- 游戏配置页：上面是本体的那几项，下面是各插件提供的小节（语音输入的聊天键和频道、按键程序的允许操作、英雄联盟的数据开关）。照 VS Code 的“按语言的设置”。
- 游戏配置包（导入导出 .zip）要连同插件的这些小节一起带上。
- 迁移：老的游戏配置文件读得进来，插件的字段在第一次保存时搬到插件的设置里（照兼容规则）。哪一步搬哪些字段：语音输入的字段在第 5 步，按键程序的在第 6 步，`lol.enabled` 在第 1 步。

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
| 页面 | 语音输入热键（按住说 / 按一下开关）、悬浮窗、文字处理（翻译、润色、屏蔽词）、每个游戏的聊天键和频道、控制台通道、识别记录 |
| 给她的工具 | `game_chat.send`（她替你发言，走同一套往聊天框打字的办法） |
| 只给页面用的 | 试听、测试打字、文字处理试一下 |
| 用本体的 | “听一句”（录音、语音识别、热词）；游戏配置（哪个游戏） |
| 现在的代码 | `orchestrator.rs` 里语音输入的流程、`inject.rs`、`overlay*.rs`、`runtime.rs` 里语音输入的部分、`crates/kotone-postprocess`、ADR 032 的挡键和控制台通道 |

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
