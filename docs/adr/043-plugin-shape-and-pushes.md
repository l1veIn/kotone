# ADR 043：插件形态与推送——插件是 App、事件和内置推送、工具；推送有卡片、一行、语音三种样子；覆盖层改成悬浮窗

状态：Accepted（2026-11-26，玩家定的方向；细节由主会话照讨论记录定）。D3 的“推送只给内置插件”由 [ADR 045](045-automation-is-programs.md) 修订（2026-11-27：自动化的程序也能 `push()`）。部分修订 [ADR 039](039-desktop-companion-and-game-apps.md)（D3 的去重和“前面一个字不改”的说法、D4 消息提示、D5 覆盖层那一条）和 [ADR 038](038-kotone-and-apps.md) D5（覆盖层是插件页面的第三个显示位置）。

依据只有一份：[2026-11-26 rc.9 试玩讨论记录](../research/2026-11-26-rc9-playtest-discussion.md)（下面写“讨论记录第几节”）。玩家原话逐字引自那份记录。讨论记录里标“我的建议”的，玩家没有反对；玩家 2026-11-26 交代这一轮时说“ADR 写完不用等我，按讨论记录里已经定的内容直接做”，所以这里把它们写成决定。

叫法照 ADR 039：**本体**（Kotone 自己）、**插件**、**琴音**（角色）。界面上的字照[术语表](../glossary.md)“界面用语 v2”。

## 背景

玩家装了本地打包的 rc.9，打了两把海克斯大乱斗。插件往聊天里发卡片，玩家说“非常牛，是一种全新的体验”，玩了一把就养成了看聊天的习惯：“主动探测游戏状态、根据局势推送有价值信息……这些功能我都想要，包括一键符文出装。”（讨论记录第二节）

同时暴露了几件事：

- 已经发出去的卡片不能更新：同一个插件、同一个 `key` 只发一次，再推一次被丢掉（第五节）。
- 英雄联盟插件的两个覆盖层（选英雄、海克斯三选一评分）和聊天里的卡片是两套显示；玩家定了“覆盖层出现改为事件推送”（第四节）。
- 插件推测“突破口”越过了插件的边界（第七节）。
- 三选一评分第一把没出来，原因只写在日志里（第八节）；开局战绩里我方少了两个隐藏了身份的人（第八节）。
- 风险提示散在卡片下面和设置页里，玩家嫌乱（第八节）。

### 核对过的现状（2026-11-26，`1246c799`）

- **自动卡片**：`kotone-plugin-api` 的 `AutoCard`（`key`、`title`、`uri`、`params`、`data`、`note`、`popup`）和 `Host::auto_card`（默认实现拒绝，“自动卡片只给内置插件”）；桌面端 `DesktopHost::auto_card`（`apps/desktop/src-tauri/src/plugins.rs`）→ `agent::post_auto_card`（`agent/auto_card.rs`）。去重：`kotone_agent::auto_card::Desk` 记最近 256 个 `插件:key`，重启后看对话里有没有同一张（`cards::posted_in`）。一张卡片是两条消息：`Message::Notice` 带 `card`（不发给模型）+ 来源 `auto-card` 的隐藏用户消息（给琴音的那句）。她正忙时压在 `agent/auto-cards-held.json`，空了再记（`when_free.rs`）。
- **没有任何更新已发卡片的路**：后端把重复的 `key` 当成重复丢掉；界面 `chatController.svelte.ts` 的 `#showHeld` 也跳过已有的卡片；对话存储只有整段 `save` / `load`。
- **消息提示**（`message_popup.rs`，窗口 `message-popup`）：不可聚焦、`SW_SHOWNA`、不进截图、点得到（真机验证过点击；滚轮没验证），独占全屏、演示模式时不弹（`attention::popup_blocked`）。**说话气泡**（`bubble.rs`，窗口 `companion`）：整窗鼠标穿透，只查独占全屏。两个都经 `screen_corner` 排在前台窗口所在屏幕的角上。开关：`agent.autoCards.enabled` / `popup`、`assistant.bubble.*`，在“角色 → 交互”。
- **覆盖层**（`overlay_layer.rs`，窗口 `plugin-overlay`）：透明、置顶、穿透、不可聚焦、不进截图、跟着目标窗口。用它的只有 `Host::overlay`，调 `Host::overlay` 的只有英雄联盟（`plugins/lol/src/overlays.rs`）。
- **插件清单**（`Manifest`，`deny_unknown_fields`）：有 `sources`、`pages`、`overlays`、`events`、`commands`；没有“内置推送”。插件设置在 `plugins.<id>`（`PluginConfig{enabled, sources, settings}`）。
- **固定的话**：`Host::speak` → `assistant::speak_in_turn`（排在她说话后面、她的声音、跟着她停）；不显示气泡。
- **术语**：术语表“自动卡片”那一行的“不说”里有 `推送卡片`，`terminology.spec` 会拦。“悬浮窗”现在专指语音输入那个小条（术语表“气泡 / 悬浮窗 / 弹幕”那一行）。

## 参考了谁

| 照什么 | 来自 |
| --- | --- |
| 同一个 id 再发一次就是更新，已显示的那条原地重画；更新时可以选不再响一次 | Android 的 `NotificationManager.notify(id, …)` 和 `setOnlyAlertOnce`；Telegram 的 `editMessageText`、Discord 的编辑消息（机器人改自己发过的消息） |
| App 在清单里声明会发哪几类通知，系统给每类画一个开关、由系统执行开关，App 改不了 | Android 的通知渠道（`NotificationChannel`，Android 8 起） |
| 插件的能力写在清单里，宿主按清单画界面 | VS Code 扩展的 `contributes` |
| 悬浮窗：一个小球 / 一行字和展开的小面板两种状态，能拖，记住位置 | [mini-yifan/dsh-orb-cordis](https://github.com/mini-yifan/dsh-orb-cordis)（MIT，本地 `target/research/orb-cordis`）；它的“可聚焦、悬停展开”不照搬（[2026-11-22 调研](../research/2026-11-22-call-mode-and-orb.md)第四节否掉过，游戏里会抢焦点） |
| 不抢焦点的小窗 | Kotone 自己的消息提示（ADR 039 D4 实施记录：`focusable: false` 也点得到） |
| 开局战绩的排布 | [LeagueAkari](https://github.com/LeagueAkari/LeagueAkari) 的对局页（只学排布，视觉原创，ADR 039 D5） |
| 海克斯三选一的判断规则 | [AramHex](https://github.com/czgreat/aramhex)（MIT，`plugins/lol/src/overlays/rules.rs` 已经照它） |
| 许可证要求的署名做成来源标签 | aramhex-data 的 CC BY-NC 4.0；OP.GG 在 [opgg-mcp#26](https://github.com/opgginc/opgg-mcp/issues/26) 里我们答应过的标志和链接 |

## 决定

### D1 插件是什么

玩家原话：“独立的 App + 事件推送器 + 提供可调用的工具。”（讨论记录第七节）

一个插件由这几样组成（主会话补的四点见讨论记录第七节，这里写成决定）：

1. **App**：自己的页面、设置（ADR 038、039 D5）。
2. **事件和内置推送**，两类东西：
   - **事件**：游戏里出了什么事（一血、到了几级……），给自动化和琴音用（ADR 036）。
   - **内置推送**：“时机 → 推送”，面向玩家；在插件清单里声明，插件页上每条一个开关，内容不能改（D4）。想改内容，玩家自己建自动化，自动化能订阅同样的事件。
3. **工具**：调用方不只是琴音，还有别的 Agent（经 MCP）、插件自己的 App 和卡片上的按钮、热键（ADR 040 的插件命令）、自动化。
4. **游戏适配器**（游戏插件才有）：告诉本体这个游戏现在在哪个阶段（房间 / 选人 / 加载 / 对局中 / 结算），本体靠它切游戏配置、做复盘、按局去重（ADR 036）。

**对内的边界**：插件要用本体的东西（截图识别、往游戏里打字、悬浮窗和聊天、语音、存储、凭据），只能经过宿主（`Host`），并在清单里写明（`needs`、`pushes`……）。麦克风、识别、键盘钩子、输入锁这些共用资源归本体（ADR 042，还是 Proposed）。“独立”指 App 和工具在别的 MCP 宿主里也能用；推送要靠 Kotone。

- **有多深远**：说法和清单字段。清单字段只给内置插件用（第三方插件是 MCP 服务器，没有 `plugin.toml`），双向门。
- **什么事实会让我们改主意**：某个插件的东西放不进这四样。

### D2 插件说“是什么”，琴音说“意味着什么”

玩家原话：“突破口跟插件没关系，这个插件不应该推测谁是突破口，推测突破口是完完全全的模型行为，只存在于自动化。插件的边界要清晰，只做游戏插件分内的事。”（讨论记录第七节）

照讨论记录第七节的清单：

- **插件分内的事**：读游戏（客户端接口、局内接口、截图识别）；查数据并原样给出、写明出处；发事件；按固定规则推送（同样的输入永远得到同样的输出：统计排序、照 AramHex 规则的三选一判断、等级 / 装备 / 刷新提醒都算）；玩家点了按钮或开了开关以后替玩家操作客户端；自己的页面、设置、推送开关。
- **归琴音的事**：评价人（突破口、谁会坑）；看局势说该怎么打；措辞和语气；把几个插件、几个数据源放在一起判断；记住玩家。
- **判断方法**：说的是游戏里的东西（数字、装备、时间、数据源给的结论）是什么，还是对一个人、对局势的看法？前者归插件，后者归琴音。

要改的：英雄联盟插件去掉 `scout::weak_spots` 这一套（`lol.scout` 的文字、页面数据的 `weak`、开局战绩给琴音的那句、卡片上的“突破口”行、工具说明和清单里的说法）。开局战绩卡片、给琴音的说明、`lol.scout` 只给事实：最近的胜负、连胜连败、这个英雄打过几局、谁在开黑。自动化模板“进加载播报突破口”（`builtin-lol-scout-on-loading`）留着，那里是琴音判断的地方。

- **有多深远**：双向门（插件里的文字和页面数据）。`lol.scout` 的工具名不变。
- **什么事实会让我们改主意**：玩家觉得只给事实的卡片没用，而琴音一句话的判断又总是来得太晚。

### D3 推送：卡片、一行、语音三种样子

叫法：界面、文档、代码统一叫“**推送**”（push）。“自动卡片”不再用，卡片是推送的一种样子。

三种样子（讨论记录第二节）：

- **卡片**：发进聊天；悬浮窗的小窗里也弹出来（D6）。
- **一行**：悬浮窗气泡里显示一行字；可以分成几格（三选一是一行三列，每格一个判断）。
- **语音**：琴音说一句固定的短话，几个说法轮着用，**不调模型**。

规矩：

- 要紧的用语音，其余显示一行；**全都记进聊天**（一行和语音在聊天里是一条小字记录，卡片照旧是卡片）。
- 同一时刻只说最要紧的一句；说过一句以后 20 秒内，同级和更低级的不再说（只显示一行）；更高级的可以说，但不打断正在说的那句。她自己正在说话时，推送的语音排在后面，排了 5 秒还没轮到就不说了（过时了）。（2026-11-26 实现时补：同一条推送——同一个 key——内容变了的更新不受这 20 秒限制，照样不打断、照样 5 秒过时：三选一刷新后最好的那张变了要再说，讨论记录第四节；冷却只管不同的推送别连着说。）
- 每条推送单独一个开关（D4）。
- 插件的推送不调模型、不花对话的钱。（2026-11-27 实现时补：语音推送用她的声音合成，在线语音每句约 0.002 元，本地语音不花钱；原来写的“不花钱”不准。）给琴音的那句话（`note`）照 ADR 039 D3：作为一条隐藏的说明追加在对话末尾。说明要少给，只在关键时刻带（例如选人锁定、选人结束），中间的变化只更新卡片。

**接口**（`kotone-plugin-api`，取代 `AutoCard` / `Host::auto_card`）：

```rust
/// 插件发的一条推送。
pub struct Push {
    pub trigger: String,          // 清单里 [[pushes]] 的 id；关着的宿主直接丢掉
    pub key: String,              // 同一个 key 再推一次就是更新（D5）
    pub card: Option<PushCard>,   // { title, uri, params, data }：uri 是清单里的页面
    pub line: Option<PushLine>,   // { cells: [{ text, tone }] }，tone = good / ok / bad / 空
    pub voice: Vec<String>,       // 同一句话的几个说法，宿主轮着用；空 = 不说
    pub note: String,             // 给琴音的一句，空 = 不给
    pub alert: bool,              // 更新时让悬浮窗把卡片再弹一次（D5）
}

trait Host {
    /// 默认实现拒绝：推送只给内置插件（ADR 039 D3 的单向门不变）。
    fn push(&self, push: Push) -> Result<PushOutcome, String>;  // Posted / Updated / Unchanged / Off
    /// 这条推送结束了（例如三选一的卡片从屏幕上消失）：悬浮窗收起它的那一行；聊天里的记录留着。
    fn push_done(&self, trigger: &str, key: &str);
}
```

- 宿主按清单核对：`trigger` 要在清单里、开着、`needs` 的数据源开着；`card` / `line` / `voice` 只放清单里这条声明过的样子，没声明的丢掉。
- 文字长度由宿主截：一行最多 3 格，每格最多 12 个字（只有一格时最多 40 个字：同传的一句话，2026-11-27 实现时补）；语音每个说法最多 20 个字；`note` 照旧 160 字。
- 聊天里的记录：`Message::Notice` 带推送的数据（插件、插件名、`trigger`、`key`，以及卡片 / 一行 / 说了哪句）。旧对话里带 `card` 的 Notice 照旧显示（读旧写新）。
- 界面事件：新的一条推送、一条推送更新了、一条推送结束了，各一个（具体名字实现时定，写进实施记录）。悬浮窗和聊天页都听它。

- **有多深远**：给内置插件的接口是双向门（一起编译）；给第三方仍然是单向门，这次照旧不给（ADR 039 D3）。Notice 里的数据格式要能读旧的（原则 19）。
- **什么事实会让我们改主意**：玩家常关掉语音推送（嫌吵）；20 秒的间隔让要紧的提醒说不出来。

> 2026-11-27 被 [ADR 045](045-automation-is-programs.md) 修订：推送不再只属于插件。自动化的程序里也能 `push(text)`：本体自己合成一条“一行”的推送（来源记成 `automation`，界面写“自动化”，`trigger` 是自动化的 id，临时程序是 `adhoc`；每条一个新 `key`，不原地更新；≤40 字；两次至少隔 1 秒），走同一条 `post_push` 的路——记进聊天、悬浮窗显示一行、不调模型、不出声，附一条给她的说明。它不经过插件清单的 `[[pushes]]` 核对，也没有插件页的开关：要不要推由玩家写的程序决定。给第三方插件的单向门不变。

### D4 推送在插件清单里声明，插件页列出来，开关由宿主执行

玩家决定（讨论记录第八节）：插件页里显示它会推送的每一条，写明时机和做什么，每条一个开关，用户不能改。

清单（`plugin.toml`）加 `[[pushes]]`：

```toml
[[pushes]]
id = "augment-pick"
title = "海克斯三选一"
when = "三选一出现、每次刷新"
what = "三张各一个判断：首选 / 可选 / 可刷新；她说一句“中间那个”"
forms = ["line", "voice"]      # card / line / voice
priority = "high"              # high / normal / low，缺省 normal
enabled_by_default = true      # 缺省 true
needs = ["stats"]              # 要哪些数据源开着
```

- 插件页（宿主画的，插件自己的 App 不用画）的“设置”里有一节“推送”：每条一行，名字、时机、做什么、样子（卡片 / 一行 / 语音），一个开关。
- 开关存在 `plugins.<id>.pushes.<推送 id>`（新字段，原则 19 只加不改）。**关掉的推送宿主直接丢掉**，开关一定管用，不靠插件自己记得查。
- 旧的总开关：“角色 → 交互”里的“自动卡片”总开关（`agent.autoCards.enabled`）去掉。读旧写新：旧配置里它是关的，就把内置插件里只有卡片的推送（rc.9 的自动卡片就是这样）当成关着，直到玩家在插件页上动了开关；带一行或语音的（三选一）不受影响（2026-11-26 实现时定：三选一为了问要不要下载模型也声明了卡片，原来的说法会把它一起关掉）。“消息提示”开关（`agent.autoCards.popup`）改成悬浮窗的“推送时弹出小窗”（D6）。
- 自动接受这类“替玩家操作客户端”的内置行为（讨论记录第七节第 1 点也算内置触发）这一轮不搬：它们照旧在英雄联盟插件自己的设置里（“操作客户端”数据源开关 + 各自的开关）。搬进同一张表是双向门，以后再说。
- 和自动化的区别（写进插件页那一节的说明）：推送是插件自带的、只能开关；自动化是玩家自己建的、能改，也能订阅同样的事件。

- **有多深远**：清单字段只给内置插件，双向门；配置字段是单向门，只加新字段。
- **什么事实会让我们改主意**：一个插件的推送多到插件页列不下，玩家找不到开关。

### D5 同一个 key 再推一次就是更新

讨论记录第五节（玩家：“可以”）：

- 新的 `key`：追加一条新推送。
- 同一个 `key`、内容变了：改那条 Notice 里的推送数据，聊天和悬浮窗里的那张原地重画，位置不动。
- 同一个 `key`、内容一样：什么都不做（`Unchanged`）。
- 这次推送带了给琴音的一句话：追加一条新的隐藏说明；不带就不追加。
- 推送还在排队（她正忙）时：直接换掉排队里的那条。
- 带 `alert` 的更新：悬浮窗把更新后的卡片再弹一次（这张卡片上面已经有别的消息时，插件用它提醒玩家）。
- 宿主记住每个 `插件:key` 最后一次的内容和它在哪段对话里；重启以后照旧去对话里找。

**ADR 039 D3 那条改准确**：原来写“作为一条新消息追加在对话末尾，前面的东西一个字不改”。改成：**发给模型的部分只追加；通知里的卡片数据可以改。** Notice 本来就不发给模型（`kotone-ai` 的 `agent/mod.rs`），改它不动发给模型的前缀，前缀缓存照常命中（ADR 024）。测试照 ADR 039 的做法：更新卡片前后，DeepSeek 请求体前面的每一条逐字节不变。

- **有多深远**：双向门。
- **什么事实会让我们改主意**：前缀缓存命中数掉了（`usage.jsonl`）。

### D6 悬浮窗：气泡和小窗两种状态

> **2026-11-26 晚修订**（[rc.10 试玩反馈](../research/2026-11-26-rc10-playtest-feedback.md)）：玩家：“右下角的悬浮窗，现在这个好像是重新写了一个？完全不需要呀，直接给一个实时chat内容就行啊……所谓的‘收到了推送的事件，右下角展示小窗，本质就是一个小的chat界面呀’……右下角肯定有角色头像，但是有的时候是聊天气泡，因为是一条简单消息，有的时候是chat小窗，因为是复杂的消息……可以在右下角头像上放一圈进度条，走完了就自动消失，鼠标移动上去就保持”。改成：
> - 悬浮窗出现时右下角一定是**她的头像**；简单的一句话（她在听 / 在想 / 说了一句短话、推送的一行）头像旁边是**气泡**；复杂的（卡片、工具调用、长回答、推送的卡片）头像旁边是**聊天小窗**。
> - 聊天小窗的内容就是聊天页那段对话**实时的最近几条**，用聊天页的同一个时间线和消息组件，不另写一套（A 轮写的只放推送的“推送流”去掉）。
> - 头像外面一圈**进度条**：走完了整个收起；鼠标停在上面就一直显示，移开再接着走。
> - 下面其余各条（不抢焦点、能拖、记住位置、跟着游戏所在屏幕、独占全屏不显示、不进截图）不变。

玩家给的规格（讨论记录第三节）：“两种状态：气泡（现在这样，一行字）和小窗”；小窗“本质上就是聊天界面，用来显示插件推送的卡片”；“显示着的时候可以关、可以拖，拖到哪里，下次就从哪里出现”；参考 dsh-orb-cordis，“只显示关键信息”。

叫法：“**悬浮窗**”从这一轮起指这个窗口（玩家的叫法）。语音输入原来那个也叫“悬浮窗”的小条改叫“**输入条**”，一个词只指一样东西（原则 17）。术语表、`terminology.spec` 跟着改。

决定：

- **两种状态**：
  - **气泡**：一行字。她在听、在想、在说（原来说话气泡显示的），或者一条推送的一行。
  - **小窗**：聊天页的组件（不另写一套），只放推送：最近的卡片和一行记录，新的在下面，滚轮往回翻。有新卡片时弹出，高度跟着卡片走，最多是屏幕高度的 40%。
- **并进来**：右下角的消息提示（ADR 039 D4）和她说话时的气泡（ADR 018）都并进这一个窗口，`message-popup` 窗口去掉。
- **能点、能关、能拖**：卡片上的按钮点得到；× 关掉（下一条推送来了再出来）；拖到哪里，下次就从哪里出现（记住的是相对屏幕工作区的位置，换一块屏幕照样用）。鼠标停在上面时不消失，移开后再开始倒计时。点“打开 Kotone”（或卡片上的打开）才把主窗口拿到前面，那是玩家自己的动作。
- **硬规矩：在游戏里绝不抢焦点**。不可聚焦（`focusable: false`）、`SW_SHOWNA` 显示；点、滚、拖都不让游戏失去前台（拖动照 ADR 039“游戏中途不被抢焦点”那次的做法：Kotone 自己跟着光标挪）。**独占全屏时不显示**（推送照样进聊天）。不进截图（`WDA_EXCLUDEFROMCAPTURE`，截图识别看不到它）。
- **哪块屏幕**：游戏在哪块屏幕上，悬浮窗就显示在哪块屏幕上（前台窗口所在的那块）。
- **用覆盖层的底子**：覆盖层去掉以后（D7），它的“置顶、不抢焦点、不进截图、跟着游戏所在屏幕”这几样拿来做悬浮窗；`overlay_layer.rs`、`plugin-overlay` 窗口、`Host::overlay`、清单的 `overlays` 删掉。
- **设置**（“角色 → 交互”）：显示悬浮窗（读旧的 `assistant.bubble.enabled`）、推送时弹出小窗（读旧的 `agent.autoCards.popup`）、位置还原。弹幕照旧。
- 滚轮在不可聚焦的窗口里还没在真机上验证（讨论记录第十一节），实现时先验证；收不到滚轮就在小窗里放上 / 下翻的按钮，写进实施记录。

- **有多深远**：双向门（一个窗口和几个设置；设置只加新字段，读旧的）。
- **什么事实会让我们改主意**：真机里它抢了游戏的焦点或让游戏掉帧；玩家常关掉它。

### D7 英雄联盟的覆盖层全部改成推送

> **2026-11-26 晚修订**（[rc.10 试玩反馈](../research/2026-11-26-rc10-playtest-feedback.md)）：玩家：“现在的ocr范围是不是全屏？队友有时候聊天框发送了海克斯也会触发海克斯推荐，然后就是海克斯弹窗的问题，不需要单独给海克斯做一个UI，就直接推卡片就行啊，就像战绩一样推送，卡片上写清楚三个海克斯推荐程度就行。”改成：
> - 三选一推一张**卡片**（和开局战绩一样进聊天、悬浮窗里是聊天小窗），写清三个海克斯各自的推荐程度（首选 / 可选 / 可刷新 / 没数据，照 AramHex 的规则）；刷新时同一个 key 原地更新；“中间那个”这句语音照旧。下面写的“一行三列”去掉。
> - 识别范围收窄到三张卡片名字所在的那一横条，并且要三个名字左右排开才算出现了三选一；聊天框里的字不算。

玩家原话：“同意，记成决定吧，覆盖层出现改为事件推送。”（讨论记录第四节）

- **选英雄覆盖层 → 选人卡**：`ui://kotone/lol/champ-select` 从覆盖层改成插件页面，作为一张卡片推送，选人期间同一个 `key` 原地更新。这一轮先放原来覆盖层的内容（海斗 / 大乱斗：我方 5 人的评级和胜率、备选席里比你现在高的；峡谷：每个人这个位置的强度）。完整内容和一键导入是 B 轮。给琴音的说明只在选人结束时带一句。
- **海克斯评分覆盖层 → 悬浮窗里一行三列 + 语音**：截图识别三张卡片照旧；一行三列，左 / 中 / 右各一格：首选 / 可选 / 可刷新（照 AramHex 的规则，`rules.rs`）。卡片在屏幕上时每秒看一次（本地识别一次 0.1–0.3 秒），刷新了哪张就原地换哪一格；她在卡片刚出现时说一次（例如“中间那个”），刷新后最好的那张变了才再说，结论没变不重复；卡片消失就收起（`push_done`）。聊天里记一条，跟着原地更新。
- **三选一固定用本地识别**，和全局的识别设置无关：有装好的本地模型就用（全局选的是本地的就用它，否则用装好的第一个本地模型）；一个都没有时，第一次用推送一张卡片问要不要下载（下载走本体的模型下载，下完**不改**全局的文字识别选择）；用不了时用一行说清楚原因（每局最多一次），不再只写日志。
- 剩几次刷新还不知道（第八节、第十一节），这一轮不做。
- 拳头的规定：“Products cannot display win rates for Augments or Arena Mode items. This applies to all websites, applications and overlays.”（[原文](https://developer.riotgames.com/docs/lol)）换成聊天和悬浮窗显示，风险一样。这次改动是为了简单，不是为了合规（第四节）。照“提醒风险、不拦”，写进英雄联盟插件的风险弹窗（D8）。

- **有多深远**：页面地址 `ui://kotone/lol/champ-select` 沿用（单向门，外部宿主可能已经认它）；`ui://kotone/lol/augment-scores` 不再是页面，删掉（它只是覆盖层，从没对外当页面提供过）。
- **什么事实会让我们改主意**：玩家在三选一时来不及看悬浮窗、更想要画在卡片上面。

### D8 风险说明：每个插件一个弹窗，来源是一个小标签

玩家原话（2026-11-26，记在记忆 `warn-dont-gate-risky-features`）：“卡片下面的风险提示，去掉，这种提示太啰嗦太干扰用户了 … 配置页面有大量这样的信息，把界面搞的很乱 … 把这些信息去掉或者收敛到一个弹窗里”。讨论记录第八节。

- 每个插件的风险和许可说明收进**一个弹窗**：第一次打开有风险的开关时弹一次；默认就开着的有风险的开关，第一次打开这个插件的页面时弹一次；以后从插件设置里的一个小链接（“风险和许可”）打开。只提示，不拦着（原则 3）。
- 卡片下面的风险提示、设置页里的风险框（`PluginSwitches.svelte` 的琥珀色框、英雄联盟游戏页上的客户端风险、“操作客户端”设置里的“有封号风险”）去掉。
- 许可证要求的署名留成一个**来源标签**（aramhex-data 是 CC BY-NC；OP.GG 是标志 + 链接）。这个标签同时就是数据源的切换：同一块内容一次只用一个数据源，不把不同来源的数字混在一张表里；点标签可以换来源，选了会记住。默认：国服的峡谷和大乱斗用腾讯 101；海斗的海克斯用 aramhex-data；101 没有的（全球数据、对线指南）用 OP.GG。琴音被问到时可以用任何一个来源，说清出处。
- 别的说明（数据只在本机、会上传截图这类）不是风险说明，这一轮不动。整个设置页照这个思路清理是以后的事。

- **有多深远**：双向门（界面）。
- **什么事实会让我们改主意**：玩家开了有风险的开关以后说“没人告诉过我”。

### D9 英雄联盟的几处数据

- **隐藏了身份的人照样显示**：局内接口里名字为空、客户端不给 puuid 的人，照样显示英雄，标“隐藏了身份”，不当成电脑漏掉；不去反查（第八节）。电脑看局内接口的 `isBot`。日志“选人时开始查 N 个队友”把自己也算进去了，措辞改准。
- **开局战绩卡片照 LeagueAkari 的排布**：大家习惯了。适度简化，要在小窗里显示：窄的时候两队上下排，每人一行，点开看详情。只学排布，不抄代码，视觉原创（ADR 039 D5）。
- **OP.GG 默认打开**（玩家决定，讨论记录第八节）：不再等 opgg-mcp#26 的回复。issue 里答应过的用法照旧：按需请求、最多 2 个并发（现在代码里只有注释，要真的做）、缓存 24 小时、`client_app` 写 `kotone`、放标志和链接、琴音说的时候带上出处。

### D10 语音输入照新形态

讨论记录第九节“语音输入按新形态”（屏蔽词那部分暂缓，这一轮不动）：

- **App**：现有的语音输入页。
- **推送**：
  - 发出去一句：推一张小卡片“你向「队伍」发了：……”，显示处理后的话，有原话时也显示原话。连着发的几句（同一个游戏、同一个频道，上一句发出后 2 分钟内）并成同一张卡片（同一个 `key` 更新）。给琴音的说明附上发出去的那句话，她就知道你刚才在游戏里说了什么（下次叫她时，这句话会随对话一起发给模型服务商；关掉这条推送就不给）。
  - 没发出去：推一张卡片，写明原因，带一个“重发”按钮。重发是玩家点的，照常经本体“发到游戏”的入口发到前台的游戏；不是替玩家补按键（ADR 032）。
- **事件**：`voice-input.sent`（频道、游戏、文字）、`voice-input.failed`（原因），给自动化和琴音用。（2026-11-26 实现时定：琴音在 `game.state` 里看到的最近几件事不带那句话——事件的字段可以标成只给自动化（`private`）；那句话只经“发出去的话”这条推送给她，关掉就不给。）
- **历史记录加上频道和游戏**（`HistoryRecord` 加字段，旧记录没有就不显示）。
- 没有游戏适配器。

### D11 别的插件（B 轮）

照讨论记录第九节：同传（翻好一句 → 悬浮窗一行，同时并进“这局的同传”一张一直更新的卡片；事件 `interpreter.line`）、游戏语音（“她对队友说了：……”）、网易云音乐（登录过期卡片）。第三方 MCP 插件：App 加工具，推送照旧不开放。

## 不做

- 不读游戏内存、不注入、不抓包（红线不变）。局内接口没有冷却、没有海克斯（讨论记录第六节，一整局验证过），不靠猜。
- 不替玩家补按键（ADR 032）。
- 推送不给第三方插件。
- 屏蔽词这一轮不动（讨论记录第九节：“有点纠结，屏蔽词这块先别动了”）。
- 不识别 Tab 计分板上对面的海克斯（第六节）。
- 三选一的剩余刷新次数这一轮不识别。

## 分期

照讨论记录第十节。A 轮全部验收过了再开始 B 轮。

- **A 轮**：默认管理员（[ADR 044](044-run-as-admin-by-default.md)）；推送的接口、卡片更新、清单里的推送和插件页开关、“推送”的叫法（D3–D5）；测试卫生（`capture_lifecycle_orchestrator` 关掉历史，测试写不了真实的 `~/.kotone`）；风险弹窗和来源标签（D8）；悬浮窗（D6）；英雄联盟搬家（D2、D7、D9）；语音输入（D10）。
- **B 轮**：选人卡的完整内容和一键导入（符文 / 出装 / 召唤师技能，操作客户端，默认关）；局内提醒，新增“等级”“装备”两类事件；结算卡片加对面的海克斯、“今天的状态”卡片、风险弹窗写进拳头那条原文；同传、游戏语音、网易云音乐的推送。
- 不在这一轮：ADR 040 第 2 期第 4 步（等真人按键）、ADR 042（等批准）。

## 实施记录

### 测试卫生：测试永远不写玩家真实的 ~/.kotone

**玩家看到的**：语音输入的历史里不会再冒出没说过的“提醒”。以后哪个 crate 的测试忘了关历史、日志、诊断记录、用量账本、配置，写的也只是 `target/tmp/kotone-test-home/<进程号>`，碰不到玩家的 `~/.kotone`，也碰不到玩家存在 Windows 凭据库里的 Key。`pnpm dev`、真机脚本跑的 `target/debug/kotone.exe` 和安装版照旧用 `~/.kotone`。

**怎么做的**：
- 夹具：`crates/kotone-core/tests/orchestrator.rs` 的 `capture_lifecycle_orchestrator` 像 `make_orchestrator_tuned` 一样把 `history.mode` 设成 `Off`。用它的两个测试都不看历史，所以不用给临时目录。原来的断言一个没改。
- 防线（只改一处）：`crates/kotone-core/src/settings.rs` 的 `kotone_dir()` 先看新加的 `isolated_home()`：设了 `KOTONE_HOME` 就用它；没设、但当前进程是 cargo 的测试程序，就用这个进程自己的 `<target>/tmp/kotone-test-home/<进程号>`（doctest 用系统临时目录）；两样都不是，才用 `~/.kotone`。第一次用到时在 stderr 说一句。进程号会被复用，所以第一次用到时先把这个目录清空，每次都从空目录开始（和 CI 上一样）。
- 怎么认出测试程序：看 exe 放在哪、叫什么。cargo 的测试程序名字总带 16 位哈希。旧布局在 `<target>/<profile>/deps/<名字>-<哈希>.exe`；Cargo 1.100 起（build-dir 新布局，cargo#17354）在 `<target>/<profile>/build/<包>/<哈希>/out/<名字>-<哈希>.exe`，本机的 cargo 1.100.0 就是这样。nextest 跑的是同一批程序。doctest 放在 `rustdoctest*` 临时目录里。别的办法都不行：`cfg!(test)` 只在被测的 crate 里成立，管不到集成测试和别的 crate（这次出事的正是集成测试）；加 cargo feature 会被合进 `cargo test` 顺手编出来的 `target/debug/kotone.exe`；`.cargo/config.toml` 的 `[env]` 连 `cargo run`（`pnpm dev`）也会改到。
- 凭据库：`crates/kotone-ai/src/secrets.rs` 的服务名改成按 `isolated_home()` 区分。测试程序没设 `KOTONE_HOME` 时，服务名也是 `kotone@<临时目录>`，碰不到玩家那个叫 `kotone` 的 Key。
- 手动跑的真模型测试 `crates/kotone-stt/tests/eval_e2e.rs`（`#[ignore]`）以前直接用 `settings::load()` 读玩家真实的配置（配置坏了还会被改名成 `.corrupt`）。现在照 `interpreter_sensevoice` 的做法用临时的 `KOTONE_HOME`，模型目录从 `KOTONE_MODELS_DIR` 读（只读）。
- 查过整个仓库：Kotone 的路径都经过 `kotone_dir()`。直接调 `dirs::home_dir()` 的只剩诊断包脱敏（`apps/desktop/src-tauri/src/diagnostics.rs`），它只是把路径里的用户目录换成 `%USERPROFILE%`，用的不是 Kotone 的目录。
- 顺带发现的第二处泄漏：玩家的 `~/.kotone/diagnostics/events.jsonl` 里有 375 条 `inject_wait_modifiers`，`context` 都是空的，其中 200 条和测试写出来的一模一样（`waitedMs=20` / `timeoutMs=1000`，2026-09-07 起就有）。这是 orchestrator 测试经 `send_to_game` 写的。现在这些都写到测试自己的目录里了（这次跑完的临时目录里正好有这两条）。玩家文件里已有的这些没动，要不要清掉得问玩家。

**测试**：
- 新增 3 个测试：`settings::tests::test_binary_never_resolves_the_real_kotone_home`（在测试程序里，`kotone_dir()` / `config_path()` 是这个进程自己的临时目录，不是 `dirs::home_dir()/.kotone`）；`settings::tests::test_binary_tmp_only_matches_cargo_test_binaries`（新旧两种布局、带 target 三元组、doctest 都认得出；`target/debug/kotone.exe`、`kotone-cli.exe`、release 版、example、`deps` 里没哈希的、哈希对不上的、装好的 Kotone 都不算）；`capture_lifecycle_orchestrator_records_no_history`（这个夹具不记历史，而且这个集成测试程序的 `kotone_dir()` 和 `history_dir()` 都不在真实的 `~/.kotone` 下）。
- `cargo test -p kotone-core --locked`：413 + 87 通过（1 个本来就忽略）；`cargo test -p kotone-stt -p kotone-ai --locked`：93 + 99 通过（忽略的照旧）。`cargo clippy -p kotone-core -p kotone-ai -p kotone-stt --all-targets --locked -- -D warnings` 没有警告；改过的文件 `rustfmt --check` 通过。
- 真机核对（只读）：跑测试前后各记一次 `C:\Users\A\.kotone`。`history.jsonl` 前后都是 105 行，修改时间都是 2026-11-26 15:45:54.969；整个目录里（不算 `browser/`、`logs/`）最新的文件前后都是 `kotone.log` 16:31:32（玩家的 Kotone 那时退出了，测试时没开着）。也就是说一个字节都没变。测试写的东西都在 `target/tmp/kotone-test-home/<进程号>/` 下：kotone-core 的集成测试写了 `config.json` 和 `diagnostics/events.jsonl`，kotone-stt 的单元测试写了 `config.json`。以前这些都会落到玩家的目录里。

**还没测 / 没管的**：
- 测试里起的子进程（`kotone-cli.exe`、`kotone.exe`）本身不算测试程序，没设 `KOTONE_HOME` 的话照旧用真实目录。现有的都设了（`agent_cli.rs` 给了 `KOTONE_HOME`；`tools_cli*.rs` 只给了 `KOTONE_TOOL_SERVICE_DIR`，跑的 `tools` 子命令也只用那个目录）。
- `kotone-stt` 的 `sherpa_runtime::tests::local_model_accepts_lol_hotwords`（`#[ignore]`，手动跑的真模型冒烟）现在找不到玩家的模型，会打印“X-ASR 模型未下载，跳过”。要跑得给一个配置里 `models.dir` 指向模型目录的 `KOTONE_HOME`。
- `eval_e2e` 改完只编译过，没有真跑（要 `engine-sherpa` 和真模型）。
- 别的 crate（桌面端、插件）的测试没有单独跑，它们经过的是同一个 `kotone_dir()`。
- 如果测试里存了 Key，会在 Windows 凭据管理器里留一条 `kotone@…\kotone-test-home\<进程号>`。现在没有测试这么做。
- `target/tmp/kotone-test-home/` 下按进程号留着小目录，不自动删，`cargo clean` 会一起清掉。

### 实施记录（推送的接口、原地更新、清单里的推送和插件页开关，2026-11-26）

ADR 043 D3、D4、D5（A 轮第 2 条）。讨论记录第二节、第五节、第八节最后一条。

**玩家看到的**：

- 聊天里插件发来的东西都叫“推送”：卡片上面那一行是“英雄联盟 推送 · 开局战绩 · 时间”（原来是“英雄联盟 发来”）。旧对话里 rc.9 发的自动卡片照样是卡片。
- 一行和语音在聊天里是一条小字记录：一行是几格带颜色的字（好 = 绿、可以 = 蓝、不好 = 琥珀），语音是喇叭图标 + 她说的那句。现在还没有插件推这两种（英雄联盟的三选一是 B2 的事）。
- 同一张卡片再推一次时在原地重画：位置不动、不多一张；卡片里的英雄联盟 App 收到新的数据就重画，不重新加载（App 里点到哪还在哪）。
- 插件页「设置」最下面多了一节“推送”（宿主画的）：每条一行，名字、样子（卡片 / 一行 / 语音）、“时机：……”“做什么：……”、一个开关；说明里写明和自动化的区别（推送是插件自带的、只能开关，想改就自己建自动化）。英雄联盟有两条：开局战绩、这局总结，默认都开。插件关着时开关灰着；要的数据开关关着时那一行写“要先打开上面的‘读客户端数据’”。
- 「角色 → 交互」里原来的“自动卡片”两个开关只剩一个：“推送时弹出消息提示”（旧的 `agent.autoCards.popup`）。总开关去掉了；旧配置里总开关是关的，带卡片的推送算关，直到玩家在插件页上动了那一条。
- 消息提示上那句现在是卡片的名字（“开局战绩”“这局总结”），不是原来插件给的“对面的战绩到了”（新接口里没有这一项；悬浮窗做完以后消息提示整个换掉）。

**怎么做的**：

- **接口**（`crates/kotone-plugin-api/src/push.rs`，取代 `AutoCard` / `Host::auto_card`，旧的直接删了，调用方只有英雄联盟）：名字和 ADR D3 的草图一样。
  - `Push { trigger, key, card: Option<PushCard{title, uri, params, data}>, line: Option<PushLine{cells: [PushCell{text, tone: Option<PushTone>}]}>, voice: Vec<String>, note, alert }`，`PushTone = good | ok | bad`（没有 = 普通）。
  - `Host::push(Push) -> Result<PushOutcome, String>`，`PushOutcome = posted | updated | unchanged | off`；默认实现拒绝（“推送只给内置插件”）。`Host::push_done(trigger, key)` 默认什么都不做。
  - 清单 `[[pushes]]`：`PushSpec { id, title, when, what, forms: [card|line|voice], priority: high|normal|low（缺省 normal）, enabled_by_default（缺省 true）, needs }`。`Manifest::parse` 查：id 写法、不重名；title / when / what 不空；forms 至少一种、不重复；needs 是自己的数据开关；不认得的字段不收。
  - 开关规则只写一处：`Manifest::push_switch_in(settings, id)`（开关的位置：玩家动过按 `plugins.<id>.pushes.<推送>`；没动过按清单默认，但 `agent.autoCards.enabled == false` 时带卡片的算关）和 `Manifest::push_off_in(settings, id)`（插件关着、needs 关着、开关关着、清单里没有 → 原因）。界面 `lib/plugins.ts` 的 `pushSwitch` / `pushEnabled` 照同一条写。
  - `Push::shaped(spec)`：只留声明过的样子；一行最多 3 格、每格最多 12 字（空格去掉）；语音每个说法最多 20 字（空的去掉，不加“…”）；`note` 160 字。`Push::check(manifest)`：有 key、截完还有样子、卡片的页面是自己清单 `pages` 里的、有名字、params 是对象。
- **宿主**（`apps/desktop/src-tauri/src/plugins.rs` 的 `DesktopHost::push`）：只给静态表里的内置插件；清单里没有这条 → `Off`（记日志）；`push_off_in` 有原因 → `Off`；`shaped` + `check`；交给 `agent::post_push`。`push_done` 只认清单里有的 trigger。
- **新的、更新、一样**（纯逻辑 `crates/kotone-agent/src/push.rs`，原来的 `auto_card.rs` 改名重写；桌面端 `apps/desktop/src-tauri/src/agent/push.rs`，原来的 `agent/auto_card.rs`）：
  - 内容的指纹 `content_hash`（FNV-1a，算 trigger、卡片、一行、语音的说法；不算插件名、这次说的哪句、更新时间）。宿主记住每个 `插件:key` 的 `Record { conversation, at_ms, hash, voice, said }`（最近 256 个，只在内存）；内存里没有（重启过）就去当前对话里找（`find` + `Record::of`，新的 `push` 和旧的 `card` 都认得；旧的自动卡片当作发过、不改它）。`decide`：没有 → 新的；指纹一样 → `Unchanged`（什么都不做，带了 note 也不追加）；不一样 → 更新。一次只处理一条推送（`serial` 锁，在“桌子”和 `Inner` 外面）。
  - 新的：照旧发到当前主会话（`target_conversation`），推送那条 + 有 note 才有的给她的那句，经 `when_free::post`（她空着马上记，不空排着）。
  - 更新：新的 Notice 用第一次推的 `at_ms`（聊天里的位置和标识跟着它），`push` 里多一个 `updatedAtMs`。先看排着的里面有没有（`Desk::update_held`：还没记进对话的、排着的更新，直接换掉，最新的为准）；没有就 `when_free::replace` → `replace_queued_locked`：那段对话不在跑、前面也没有排着的，马上 `replace_locked`（`turn.rs`，和 `append_locked` 同一把锁，在跑时不写）换掉那一条（找不到就追加）；在跑就排一个“原地更新”（`Held.replace = true`，同一条只留一个），这一轮结束和别的排着的一起照先后记。上一次那段对话被删了：当成新的发到当前对话。带了 note：再追加一条“插件更新了推送「…」”给她（只追加）。
  - **发给模型的部分只追加**：推送是 `Notice`（不发给模型），改的是它的 `push`；测试照 ADR 039 的做法比 DeepSeek 请求体（见“测试”）。
  - 排着的文件照旧是 `agent/auto-cards-held.json`；`Held` 多一个 `replace`（只在 true 时写，旧版本读到会当成追加：多一张，不丢）。启动时读回来的推送也记进 `Record`。
- **对话里存成什么**（`kotone-ai` 的 `Message::Notice` 新字段 `push`；`card` 留着只读旧的，新的不写它）：

  ```text
  Notice {
    role: "notice", text: "英雄联盟 推送：开局战绩", atMs,          // text 给旧版本看
    push: {
      plugin: "lol", pluginName: "英雄联盟",
      trigger: "opening", title: "开局战绩",                       // 清单 [[pushes]] 的 id 和名字
      key: "opening:<对局 id>",
      card?: { title, ui: { uri, params? }, structuredContent },     // 和工具结果的 display 同一个形状
      line?: { cells: [{ text, tone? }] },                           // tone = good / ok / bad
      voice?: ["说法1", "说法2"], said?: "这次说的那句",
      updatedAtMs?                                                   // 更新过才有
    }
  }
  ```

  给她的那句：来源 `auto-card` 的用户消息（存盘的值不改），“（系统消息，不是玩家说的话：英雄联盟插件发来一条推送「开局战绩」……）”，更新时是“更新了推送「…」”，`note` 按数据附上。
- **界面事件** `kotone://push`（`agent/push.rs` 的 `emit_push` / `push_done`；聊天页和悬浮窗都听）：

  ```text
  { kind: "posted" | "updated", conversationId, plugin, trigger, key,
    held,      // 还排着没记进对话（她在忙）；界面照样马上显示
    alert,     // 带 alert 的更新：悬浮窗把卡片再弹一次（posted 总是 false）
    message }  // 整条 Notice（上面的形状）
  { kind: "done", plugin, trigger, key }               // Host::push_done
  ```

  `posted` 在记进对话时还会照常来一条 `kotone://agent-event` 的 `appended`（同一条），界面按标识 `plugin:key` 去重；`updated` 是原地换掉同一个标识的那条。原来的 agent 事件 `autoCard` 去掉了。命令 `agent_held_cards`（排着的推送，含排着的更新）、`auto_card_open`、焦点事件 `kotone://auto-card-focus` 名字不变。
- **聊天页**（`chatController.svelte.ts`、`timeline.ts`、`PushItem.svelte`，原来的 `AutoCard.svelte`）：`pushOf` 把新的 `push` 和旧的 `card` 都变成同一个样子；推送自成一轮。`posted` / `updated` / 读对话时的排着的推送 / `appended` 都按标识放：对话里已经有同一条就原地换掉，没有就放在最后面（排着的，这一轮新来的消息插在它前面）。每一轮的标识是第一条消息的位置和时间，原地换掉时组件不重建。
- **卡片里的 MCP App 收到更新**（`McpAppFrame.svelte`）：握手以后 `data` 变了就再推一次 `tool-result`（MCP Apps 的结果通知可以来多次；英雄联盟 App 的页面本来就 `session.onResult` 重画），iframe 不重新加载；握手期间变了的，握手完推最新的。
- **语音**（纯逻辑 `crates/kotone-agent/src/push_voice.rs`；桌面端 `agent/push.rs` 的 `speak_in_order` + `assistant::speak_push`）：说法按 `插件:trigger` 轮着用；更新时说法没变就不再说（结论没变不重复，三选一不用自己记）。排队：300 毫秒内来的算同时、只留级别最高的（同级留先来的）；开口以后 20 秒内同级和更低级的不说；更高级的可以说，但等正在说的那句说完；从推来时算 5 秒还没开口（排在她的话、另一条推送、麦克风后面）就不说，没说出来的不占冷却。说用的是插件的话那条路（`take_turn_and_speak` 多了一个期限参数：到期还没拿到播报锁、麦克风还开着就不放），说不出来（不是过时、不是被停下）照旧改用弹幕。
- **插件页**（`PluginPage.svelte` + 新的 `lib/components/plugins/PluginPushes.svelte`）：清单里有推送的插件，栏上就有“设置”按钮；设置里插件自己的设置下面是宿主画的“推送”一节。开关写 `plugins.<id>.pushes.<推送>`（`PluginConfig.pushes`，新字段，没动过的不写），走普通的设置保存（`update_settings`），没加新命令。名字 / 时机 / 做什么用插件的文案 `plugin_<id>_push_<推送>_title|when|what`（英雄联盟的测试查和清单一致）。
- **英雄联盟**：`plugin.toml` 加 `[[pushes]]` `opening`（开局战绩）、`summary`（这局总结），都是卡片、默认开、要 `client`；`watcher/auto_cards.rs` 改调 `Host::push`，卡片、给她的那句、key 都和原来一样。
- **设置说明**（`settings_guide.rs`）：`agent.autoCards.popup` 改叫“推送时弹出消息提示”（琴音 → 交互 → 推送）；`agent.autoCards.enabled` 挪进 `INTERNAL`（只读旧值，她不改它）；`plugins.*` 加上 `pushes.<推送 id>` 和英雄联盟的两条。
- **叫法**：术语表“自动卡片”那一行换成“推送”（不说：`自动卡片`、`事件卡片`），“交互”那一行改了，架构那一节加了“推送（架构）”；“消息提示”那一行只把“发自动卡片时”改成“插件推送卡片时”（整行留给悬浮窗那一轮）。`terminology.spec` 多查 `crates/kotone-agent/src/push.rs`（记进聊天的那一行、给她的那句），插件文案的约定键加上 `push_<推送>_title|when|what`。真机脚本 `auto_card_real.py`、`message_popup_real.py` 跟着改了读法（Notice 的 `push.card`、`push-item`、日志 `push:`），没跑。

**测试**：

- Rust（2026-11-26，League 开着，低于正常优先级 `-j 2`）：`kotone-plugin-api` 26 个（新的：清单的推送和检查、宿主的开关规则含读旧、截字和只留声明过的样子、序列化、默认实现拒绝）；`kotone-agent` 111 个（`push::tests` 9 个：推送不调模型、下一轮看得到那句、追加时前缀逐字节不变、**原地更新以后 DeepSeek 请求体前面的每一条逐字节不变**（没带 note 的更新一个字节都不多，带 note 的只追加）、她在忙时排着、同一个 key 的新 / 更新 / 一样、重启后从对话里找（含 rc.9 的 `card`）、排着的更新换掉排着的那条、存盘读回；`push_voice::tests` 4 个：同时来的只说最要紧的、冷却、更高级的不打断、过时不说和没说出来不占冷却）；`kotone-tauri` 174 个（新的：`turn::tests::a_push_is_replaced_in_place_only_while_its_conversation_is_idle`、`when_free::tests::push_updates_wait_for_a_running_turn_and_then_land_in_place`（在跑时排着、只留最新、这一轮存盘后改在原位、没被盖掉；排着的直接换）、`assistant::tests::a_push_voice_that_waits_past_its_deadline_is_dropped`）；`kotone-plugin-lol` 123 个（两条推送在清单里、文案和清单一致）；`kotone-ai` 93 个、`kotone-core` 414 个、`kotone-remote-plugin` 32 个。clippy（这几个 crate 加 `kotone-core`、`kotone-remote-plugin`，`--all-targets -D warnings`）干净。
- Playwright `e2e/push.spec.ts` 9 个（原来的 `auto-card.spec.ts`）：卡片“英雄联盟 推送”、rc.9 的自动卡片照样显示、打开右侧栏、给她的那句不显示；**同一个 key 再推一次原地更新：位置不动、只有一张，卡片里的英雄联盟 App 显示新的名字、App 里设的标记还在（没重新加载）**，之后的 `appended` 也不多一张；读对话时排着的更新原地换掉；她在回答时来的推送在最后、她说完后还在；一行三格（颜色）和语音的小字记录、原地换一格；点消息提示定位；交互里没有总开关了、“推送时弹出消息提示”写 `agent.autoCards.popup`；**插件页「设置 → 推送」列出两条、旧总开关关着时显示关、开关写 `plugins.lol.pushes.opening`、插件关着时灰掉**；消息提示窗口。把 `McpAppFrame` 里再推结果那一行注释掉，App 那一条测试就挂（试过）。`terminology.spec` 4 个通过。Playwright 全量（本体 + 插件）220 个通过（一个 worker；第一遍跑到一半 vite 开发服务器没了，后 84 个连不上，单独重跑全过）。

**还没测**：真机（玩家在打游戏）：`auto_card_real.py`、`message_popup_real.py` 改了读法没跑；真的英雄联盟里一局的两张卡片；语音推送真的出声（现在还没有插件推语音，排队的规矩只有单元测试）；悬浮窗听 `kotone://push`（悬浮窗那一轮做）。

### 实施记录（悬浮窗：气泡和小窗，2026-11-26）

ADR 043 D6（A 轮第 6 条）。讨论记录第三节。

**玩家看到的**：

- 游戏所在屏幕的角上（默认右下角）只有一个悬浮窗。平时是一行**气泡**：她在听、在想、在说（和原来的说话气泡一样，头像小了一点：48），或者插件推送的一行（三选一是三格带颜色的字：首选 = 绿、可选 = 蓝、可刷新 = 琥珀）。插件推送卡片时弹出**小窗**：上面一行“推送”+ 往前 / 往后翻 + 打开 Kotone + ×，下面是这段对话里的推送（和聊天页同一种卡片和一行记录，新的在下面）。小窗的高度跟着刚推来的那张卡片走，最高是游戏所在屏幕工作区高度的 40%；它在列表最上面，往回翻（滚轮或按钮）看更早的。同一张卡片再推一次在原地重画，位置不动。右下角的消息提示没有了。
- 鼠标停在上面不收；移开后至少再留 3 秒。一句提醒的一行显示 6 秒，小窗 8 秒；清单里写了 `until_done` 的一行（三选一）一直显示到插件说结束（`push_done`），插件一直不说就在最后一次更新后 90 秒收。× 收起现在显示的，下一条推送来了再出来；她那一行收到她换了状态（比如说完回到“通话中”）。
- 按住气泡或小窗上面那一行就能拖（只是点一下不动它），拖到哪里下次就从哪里出现：记的是相对屏幕工作区的位置，换一块屏、换分辨率照样用；拖过的不再贴角排队。点一行推送、卡片上面那一行或“打开 Kotone”，主窗口到前面、定位到那一条（玩家自己的动作），小窗收起。
- 不抢焦点：点 ×、点小窗里的按钮、拖动都不让游戏失去前台；不进截图；游戏独占全屏、演示模式时不显示；Kotone 窗口在前台、别的程序全屏时推送不弹（照样记进聊天）。她的气泡在 Kotone 窗口在前台时照旧出来（原来就是这样）。
- 「琴音 → 交互」里“气泡”和“推送”两节并成一节“悬浮窗”：显示悬浮窗、推送时弹出小窗（关着显示悬浮窗时灰着）、放在哪个角 + 试一下、离边缘的距离、位置还原（拖过才能点）、回答同时用弹幕。
- 叫法：语音输入原来那个也叫悬浮窗的小条改叫**输入条**（语音输入页那一节、提示、设置说明里都改了）；“消息提示”“对话气泡”“右下角弹窗”“通知弹窗”进了术语表的“不说”。

**怎么做的**（照 ADR 043 D6；ADR 留给实现的照原则 2、7、15、19 定）：

- **一个窗口**：`tauri.conf.json` 的 `floating`（去掉 `companion` 和 `message-popup`）：透明、无边框、置顶、不进任务栏、`focus: false`、`focusable: false`、`create: false`，启动时和原来的气泡窗口一样建好（`create_startup_windows`）；权限 `capabilities/floating.json` 只有 `core:event:default`（删了 `companion.json`、`message-popup.json`）。页面 `routes/floating/`（`Floating.svelte`、她那一行 `HerLine.svelte`、小窗的推送 `pushStream.svelte.ts`），删了 `routes/companion`、`routes/messagePopup`、`lib/messagePopup.ts`、`message_popup.rs`。
- **显示什么、什么时候收**：纯状态 `floating/model.rs`（时间用毫秒传进来）：她那一行跟 `bubble.rs`（它每次改状态都调 `floating::her`，她那一行的内容和时长照旧是 `bubble.rs` 的状态机，`kotone://bubble` 照发）；推送由 `agent/push.rs` 调 `floating::pushed`（原来调消息提示的地方）/ `floating::done`（`push_done`）；`auto_card_open` 调 `floating::opened`。界面事件 `kotone://floating`（`{revision, mode: hidden|bubble|mini, her, lines, mini: {conversationId, focus, key}, corner, maxHeight}`）；命令 `floating_current / _layout / _hover / _close / _drag / _reposition`，打开 Kotone 用原来的 `auto_card_open`。小窗的内容由页面自己读（`agent_conversation` + `agent_held_cards`，再跟着 `kotone://push` 原地换、追加，和聊天页同一个规矩）。
- **一行显示多久**：清单 `[[pushes]]` 加了 `until_done`（`PushSpec.until_done`，缺省 false；写了要有 `line` 这种样子，`Manifest::parse` 查）。`Push` 上没加字段（插件的推送是结构体字面量，加字段会让正在改英雄联盟插件的那边编不过）。**英雄联盟的 `augment-pick` 要写 `until_done = true`**（B2 那一条现在没写：不写就每次更新显示 6 秒）。
- **窗口跟着内容走**：页面量好内容（`getBoundingClientRect`，窗口藏着时强制排版也量得出来；内容变了 `ResizeObserver` 再量）报给后端，后端按游戏所在那块屏的缩放摆；窗口只有内容那么大，透明的地方不挡游戏。从收着变成显示，等页面报了这一版的大小才显示，免得用旧大小闪一下（1.5 秒还没报就照旧显示）。
- **在哪**（`floating/place.rs`，纯计算）：游戏（前台窗口）所在那块屏（`screen_corner::foreground_monitor`：`MonitorFromWindow` + `GetDpiForMonitor`）。没拖过：贴着设置里的角，让开输入条（`screen_corner::stack`，`ORDER` 改成 输入条 → 悬浮窗）；拖过：`assistant.bubble.position = {corner, x, y}`（新字段，缺省 null；窗口的哪个角贴在工作区里的哪一点，单位是工作区宽 / 高的万分之一；拖完时窗口中心在工作区哪一块就是哪个角），从这个角往屏幕里长，出了工作区整个拉回来；拖过的 `screen_corner::set_free`，别的窗口出来时不挪它。照 dsh-orb-cordis（MIT，`packages/helper/src/geometry.ts` 的 `FloatingPlacement`：贴着的角不动、往有空的那边长、松手拉回工作区）。
- **拖动**（`floating_drag`）：照语音输入的 `drag_overlay`：按着左键时每 8 毫秒看光标，`SetWindowPos` 不激活地挪；照 dsh-orb-cordis 的拖动：挪过 4 个逻辑像素才算拖（只是点一下不动），跟着走的是按下那一刻光标到窗口的距离。松开记下位置（写设置、发 `kotone://settings-changed`），拉回工作区。拖着时不收。窗口不可聚焦，拖完不用还前台。页面看屏幕坐标分清“点”和“拖完松手”（窗口跟着光标走，页面里的坐标不变）。
- **能不能出来**（`attention.rs`）：新加 `floating_hidden`（独占全屏、演示模式：整个不显示）；推送还看原来的 `popup_blocked`（Kotone 在前台、别的程序全屏时不弹）。`exclude_from_capture` 从覆盖层挪到 `window.rs`（覆盖层删了，B2）；真机脚本设 `KOTONE_FLOATING_CAPTURABLE=1` 才截得到（原来的 `KOTONE_POPUP_CAPTURABLE`）。
- **小窗用聊天页的组件**：`PushItem` / `PluginCard` 多了一个 `onOpen`：给了就交给它（打开 Kotone、定位到那一条），没给照旧在右侧栏打开；卡片里的 MCP App 照旧收得到更新、不重新加载。小窗跟着定位的那一条（它在最上面），玩家自己往回翻了就不跟、翻回最下面又跟上（照 dsh-orb-cordis 的 `followBottom`）。
- **滚轮**：真机还没验证（玩家在打游戏）；先放了往前 / 往后翻的按钮（列表能滚时才出来），真机验证滚轮收得到以后可以去掉。
- **设置**（只加字段）：`assistant.bubble.position`（`FloatingPosition`，`validate` 查 0–10000）；`agent.autoCards.popup` 改叫“推送时弹出小窗”。`settings_guide.rs`：`assistant.bubble.*` 的说法改成悬浮窗、加了 `position`；`overlay.*` 改成输入条（位置“插件 → 语音输入 → 输入条”）。`navigation.ts`：“琴音 → 交互 → 悬浮窗”，旧的“气泡 / 推送 / 消息提示 / 自动卡片”都落到这一节；“语音输入 → 输入条”，旧的“悬浮窗”也认。
- **叫法**：术语表“气泡 / 悬浮窗 / 弹幕”一行重写成“悬浮窗 / 气泡 / 小窗 / 输入条 / 弹幕”（不说：`对话气泡`、`消息提示`、`右下角弹窗`、`通知弹窗`），“消息提示”一行删了，“交互”“呼叫”“语音输入”和架构那一节的“气泡 / 悬浮窗 / 屏幕角落分配”跟着改；`docs/assistant.md` 那一段改了。语音输入的文案（`voice.json` 13 条、`shell.json`、`system.json` 3 条、en 跟着）、`overlay.rs` / `overlay_window.rs` 的三句报错里“悬浮窗”改成“输入条”（代码名不改）。

**测试**（2026-11-26，League 开着：低于正常优先级 `-j 2`，Playwright 一个 worker）：

- Rust：`kotone-tauri` 185 个过（新的：`floating::model::tests` 5 个——一行 → 卡片弹小窗 → 鼠标停住不收 → 移开再留 3 秒 → 收；三选一一直显示、原地换、`push_done` 收、90 秒兜底；带 alert 的再弹、× 等下一条推送、打开 Kotone 收起；她那一行跟状态、× 收到她换状态；拖着不收、最多三行。`floating::place::tests` 4 个——拖到哪下次就在哪（四个方向、差不到 1 像素）、变大时贴着的角不动、换一块大小和位置都不一样的屏落在同样比例的地方、出了工作区拉回来。`screen_corner::tests` 5 个改成输入条 + 悬浮窗，加了拖过的不排；`attention::tests` 改成查 `floating` 不可聚焦、`companion` / `message-popup` 没了）；`kotone-core` 417 + 87 个、`kotone-plugin-api` 28 个（清单的 `until_done`）、`kotone-agent` 111 个。clippy（这四个 crate，`--all-targets -D warnings`）干净。
- Playwright `e2e/floating.spec.ts` 4 个（取代 `bubble.spec.ts` 和 `push.spec.ts` 里消息提示那一条，连跑两遍都过）：她那一行 在听 → 在想 → 在说 → 追问 → 出错、旧序号不覆盖、报上去的大小就是那一块、头像贴着右下；一行三格带颜色、点它打开 Kotone 定位、按下交给后端拖、× 交给后端、鼠标进出告诉后端；小窗：这段对话里的三条推送、定位的那张在列表最上面、高度不超过上限、滚轮和按钮往回翻、同一个 key 原地换（App 里显示新名字、不多一张、位置不变）、新的放最后、卡片上面那一行打开 Kotone 而不是右侧栏；设置：悬浮窗一节、推送时弹出小窗写 `agent.autoCards.popup`、关掉悬浮窗时它灰着、后端写了拖到的位置以后位置还原能点、点了清掉。`push.spec.ts` 交互那一条改查悬浮窗一节；`terminology.spec` 4 个过；svelte-check 0 错 0 警告。截图 `target/plugin-shots/round9/floating-bubble.png`、`floating-mini.png`、`floating-settings.png`。
- 共用的开发服务器开了很久、热更新过：页面里的模块带 `?t=…`，测试里 `import("/src/lib/…")` 拿到的是另一份模块，`push.spec` 插件页那一条、`navigation.spec` 旧地址那一条、`copilot-flow` 几条因此失败（核对过：浏览器里加载的是 `browserMock.ts?t=…`、`navigation.ts?t=…`）；要在新起的开发服务器上重跑。

**还没测**：

- 真机（玩家在打游戏）：`scripts/manual-e2e/floating_real.py`（新脚本，取代 `message_popup_real.py`）写好了、没跑：推来卡片弹小窗时前台不动、置顶不激活、不进截图、和输入条不叠、点小窗里的按钮 / 滚轮 / 拖动 / × 时测试窗口一直在前台没收到键、拖完记下位置、下一次在拖到的地方出现。**滚轮在不可聚焦的窗口里收不收得到还不知道**（F6）。`auto_card_real.py` 跟着改了窗口标题和环境变量，没跑。
- 真游戏里：盖在英雄联盟上点、拖、滚；多显示器（游戏在副屏、缩放不同的屏之间拖）；独占全屏时不出来。
- 英雄联盟的三选一一行要 B2 那条推送写上 `until_done = true` 才会一直显示。

### 实施记录（英雄联盟的覆盖层改成推送、三选一固定用本地识别、本体的覆盖层去掉，2026-11-26）

ADR 043 D7（A 轮“英雄联盟搬家”的一部分）。讨论记录第四节、第六节、第八节第一条。

**玩家看到的**：

- **选英雄**：选英雄一开始，聊天里来一张“英雄联盟 推送 · 选英雄”的卡片（插件的 App，和开局战绩一样的“记分纸”样子）：海斗是我方 5 个人的评级小牌（S–D）、胜率、第几 / 共几，备选席按胜率排，比你现在高的标 ↑，最高的那个用墨线圈出来（值得换）；峡谷是每个人这一路的强度和胜率。重随、和备选席换英雄时同一张卡片原地变，不多一张。选英雄结束时卡片标上“选英雄结束了”，进了游戏才给她留一句很短的说明（你和队友用什么英雄、胜率或强度，只有事实）。窄到约 360 px（悬浮窗的小窗）也放得下：备选席变成一个一行。客户端右上角那块面板没有了。
- **海克斯三选一**：海斗对局里三张卡片出来后，一行三格（左 / 中 / 右：首选 绿、可选 蓝、可刷新 琥珀、没数据；认不出的那张写“没认出”），她说一句“右边那个”（三个说法轮着用：“右边那个”“选右边那张”“拿右边的”）；三张都不好时说“都一般，可以刷新，非要选就中间”。刷新了哪张就原地换哪一格；首选的位置没变就不再说，变了再说。选完、卡片没了 4 秒那一行收起（聊天里的记录留着）。下一次三选一是新的一条。原来贴在卡片上面的标签没有了。
- **本地识别**：三选一只用本地文字识别，和「设置 → 模型 → 文字识别」选的是什么无关（选的是在线识别也照样用装好的本地模型）。一个本地模型都没装时，海斗选英雄时聊天里来一张“文字识别模型”的卡片：“海克斯评分要用本地的文字识别模型（约 21 MB），要下载吗？”，下载 / 不用。点下载在后台下（和设置里的下载同一套，设置页开着时那里也看得到进度），卡片变“正在下载……”，下完变“下好了：下一次三选一就能看到评分”；下完**不改**全局的文字识别选择。点不用：以后不再问，卡片写“以后想用，在「设置 → 模型 → 文字识别」里下载一个本地模型就行”。
- **用不了时说清楚**：每局最多一行“海克斯评分用不了 · 没有本地文字识别模型 / 识别模型没下成 / 游戏要改成无边框 / 文字识别出错”，不再只写日志。游戏切到后台、最小化不算用不了（不截图，回来接着看）。
- 插件页「设置 → 推送」多了两条：选英雄（卡片，要“读客户端数据”和“用网上的统计”）、海克斯三选一（一行 / 语音 / 卡片，要“用网上的统计”），默认都开。

**怎么做的**：

- **选英雄卡片**（`plugins/lol/src/pushes/champ_select.rs`，原来的 `overlays.rs` 拆开）：清单 `[[pushes]] id = "champ-select"`（卡片，`needs = ["client", "stats"]`），页面 `ui://kotone/lol/champ-select`（地址沿用，从覆盖层改成清单 `pages` 里的一页、App 的一页 `app/src/pages/ChampSelect.svelte`，`needs = ["stats"]`）。后台每一轮（2 秒）在选英雄时重读选人接口和对局流程，照原来覆盖层的算法出数据（海斗：`mayhem_panel`，7 天的 101 海斗榜，名次只按胜率、AramHex 的 S–D；峡谷：每个人这一路的 101 强度榜那一行）；英雄改成 `{id, short, icon}`（页面自己取图）。一次选英雄一个 key：选人接口或对局流程里的对局 id，都没有时 `t<进选英雄的毫秒>`。和上次推出去的一样就不推（宿主也会回 `Unchanged`）；一项数据都没有时不推。离开选英雄时把最后一份标 `ended: true` 再推一次（内容变了，宿主原地更新、能带 `note`）；进了游戏（`GameStart` / `InProgress` / `Reconnect`）才带 `note`（`ended_note`：“选英雄结束了（海斗）：你用X（海斗胜率 52.1%，评级 A），队友 …。”），被秒退的只标结束。
- **三选一**（`plugins/lol/src/pushes/augment_pick.rs`）：清单 `[[pushes]] id = "augment-pick"`（`forms = ["line", "voice", "card"]`，`priority = "high"`，`needs = ["stats"]`）。自己一个循环（`watch` 里和主循环一起开）：主循环每一轮告诉它现在是不是海斗对局（局内接口有游戏时间、队列 2400）和自己的英雄；是的话、这条推送开着（新加的 `Host::push_enabled`，关着就不截图）、本地识别能用，就 `screen_text(.., local_only = true)` 截中间那块（`CARD_REGION` 不变）、认卡片名（`match_augments` 不变）、照 `rules::evaluate`（AramHex，MIT，没改规则）判断。纯逻辑 `Tracker::frame`：同样的画面连着看到两次才推（刷新动画里只认出一部分时不推，免得说错位置）；新的一次 key 是 `augment:<对局 id>:<毫秒>`，之后变了同一个 key 再推（宿主原地更新）；没卡片的画面到 4 秒（`CARD_GONE`）`push_done`；游戏不在前台的那一帧什么都不改。卡片在屏幕上时每秒看一次，不在时每 2 秒（原来一律 2 秒）。格子按名字那一行的横向中点分左 / 中 / 右（认出两张时位置也对）。语音的几个说法只跟结论走（首选在哪一格、是不是三张都不好：`rules::all_weak`，首选也比英雄平均低 0.5 个点以上）：结论没变说法就一样，宿主“说法没变不再说”，插件不用自己记。
- **本地识别固定**（宿主 `apps/desktop/src-tauri/src/ocr_engines.rs`、`computer_use.rs`、`plugins.rs`）：`pick_local`：全局选的是本地包而且装好了就用它，否则按清单顺序（推荐的在前）用装好的第一个本地包；`screen_text(.., local_only = true)` 用它（`ocr_image_local`），一个都没有时报“没有装好的本地文字识别模型”、不截图；`local_only = false` 照旧用全局选的。新的宿主调用（`kotone-plugin-api`）：`Host::local_ocr()` → `LocalOcr::Ready{model, name}` / `Missing{model, name, bytes}`（推荐的包，PP-OCRv5 mobile 中英日约 21 MB）/ `Failed{reason}`；`Host::download_local_ocr()` 下 `Missing` 里的那个，走 `model_catalog::download`（同一把下载锁、逐文件校验、照常发 `kotone://download`），不写 `ocr.activeModelId`；默认实现都拒绝（第三方插件没有）。问的那张卡片：页面 `ui://kotone/lol/augment-ocr`（`app/src/pages/AugmentOcr.svelte`），只给页面的工具 `lol.augment_ocr`（`download` 在后台下、马上返回，下完插件原地更新卡片；`decline` 写插件设置 `plugins.lol.settings.augmentOcrDeclined = true`；`status`）。key 固定是 `augment-ocr`，这次启动问过就不再问；海斗选英雄时就问（进了游戏点不了），进了海斗对局还没问过也问。用不了那一行：key `augment-off:<对局 id>`，每局最多一次；独占全屏（截到纯色画面）连着 3 次才算。
- **本体的覆盖层去掉了**：`overlay_layer.rs`、窗口 `plugin-overlay`（`tauri.conf.json`）和它的权限文件、`Host::overlay`、`OverlayItem` / `OverlayTarget` / `Anchor`、清单的 `overlays`（写了读不进来，有测试）、界面的 `#/plugin-overlay` 路由、`pluginUi.ts` 的 `overlays` / `overlayOf`、英雄联盟的 `ChampSelectOverlay.svelte` / `AugmentOverlay.svelte` / `overlay.css` 和 `plugin_lol_overlay_*` 文案都删了。`exclude_from_capture` 用悬浮窗那一轮挪进 `window.rs` 的那份（消息提示改用它）。`ui://kotone/lol/augment-scores` 不再有（它从来不是页面）。
- 设置说明（`settings_guide.rs` 的 `plugins.*`）加上 `pushes.champ-select`、`pushes.augment-pick`。真机脚本 `game_events_real.py` G6 改成看聊天里的选英雄卡片（见“测试”）。`THIRD_PARTY_NOTICES.md` 的 aramhex 一条改成现在的路径和说法（规则没变）。

**测试**：

- Rust（2026-11-26，League 客户端开着，低于正常优先级 `-j 2`）：`kotone-plugin-lol` 137 个（新的：`pushes::augment_pick` 7 个——**一次三选一一帧一帧**：出现（两帧一样才推）→ 一行 + 语音、刷新了中间那张首选没变 → 同一个 key 只换那一格、说法一样（宿主不说）、刷新动画里只认出两张的那帧不推、首选换到左边 → 说法变了、游戏切走那帧不改、没卡片不到 4 秒不收到了收起、之后再出来是新的 key、这局结束收起；三张都不好说“都一般”；认不出的那格“没认出”位置不乱；截图识别的错分类；**没有本地模型**：第一次问一张卡片（约 21 MB）、问过不再问、点“不用”写进插件设置、以后每局一行原因（每局一次）；点“下载”卡片 问 → 正在下载 → 下好了、之后能看画面；`pushes::champ_select` 4 个——海斗卡片的评级和备选比较（照 `lol_tools_real.py` 的合成 session）、一次选英雄一个 key、重随以后再推、一样的不推、结束标 ended + 给她的那句、峡谷的分路强度、没数据不推；`pushes::rules` 加了“都不好”；`app` 七页都是 App，`augment-scores` 不是；工具 `lol.augment_ocr` 只给页面）；`kotone-plugin-api` 28 个（`[[overlays]]` 读不进来、`LocalOcr` 的序列化）；`kotone-tauri` 184 个（`ocr_engines::tests::local_only_picks_a_local_package_whatever_the_global_choice`：全局选在线 + 装了本地 → 用本地；全局选的本地装好了 → 用它；没装好 → 装好的第一个；一个都没有 → 推荐 mobile、约 21 MB；`plugin_apps` 七页跟着开关）；`kotone-core` 417 + 87 个、`kotone-remote-plugin` 32 个。clippy（这几个 crate，`--all-targets -D warnings`）干净。
- Playwright `plugins/lol/e2e/lol-pushes.spec.ts` 4 个：聊天里的选英雄卡片是 App（我方 5 人评级 A S A C B、第 31/172、备选 ↑、最高的圈出来、腾讯 101）；**同一个 key 再推一次（重随）原地重画，App 没重新加载**，结束时标“选英雄结束了”，一直只有一张；窄到 360 px 不横着溢出、备选一个一行；没装本地模型的卡片（约 21 MB、不改全局选择），点下载经宿主调到英雄联盟插件的 `lol.augment_ocr`，卡片跟着工具的结果和后来推来的更新变。原来的两个覆盖层测试（`lol-stats.spec.ts`）删了。`terminology.spec`：我加的文案都用上了、英文齐、没有禁用词（另外几条没用上的文案、`push.spec.ts:303` 是别的分支正在改的，见报告）。
- 截图：`target/plugin-shots/round8/chat-champ-select-card.png`、`chat-champ-select-narrow.png`、`chat-augment-ocr-ask.png`。
- 真机（隔离实例、假客户端、`--background`、不发任何系统按键，玩家开着英雄联盟客户端，2026-11-26）：`game_events_real.py` 14/14 通过。新的 G6：假客户端报海斗队列，选英雄一开始聊天里就有选英雄卡片（腾讯 101 的真数据，备选席里最高的圈出来）；12 秒合成的重随以后同一个 key 原地更新（还是一张、有 `updatedAtMs`、我那一行换成了新英雄）；进了游戏卡片标上结束了，给她的那句是“选英雄结束了（海斗）：你用厄运小姐（海斗胜率 49.9%，评级 B），队友 亚索、卡尔萨斯、布兰德、萨科。”。隔离目录里没有本地识别模型，选英雄时推了“文字识别模型”那张卡片。脚本里关了悬浮窗和推送的小窗（玩家在用这台电脑）。

**还没测**：

- 真游戏里的三选一（要玩家打一把海斗）：本地识别每秒一次在真机上的耗时、卡片刷新动画里两帧一样才推是否够、三格的位置在不同分辨率 / 窗口比例下对不对、她说“右边那个”的时机。（冷却和“首选变了再说”的冲突已经定了，见下面“补”。）
- 真的英雄联盟选英雄（海斗、峡谷排位）里卡片的数据和更新；悬浮窗里的样子（悬浮窗那一轮）。
- 真机下载模型那张卡片（真网络、真模型目录）。

**补：同一条推送的更新不受冷却限制；旧总开关只管只有卡片的推送**（主会话 2026-11-26 定的）：

- 语音排队（`crates/kotone-agent/src/push_voice.rs`）多了 `VoiceArbiter::offer_update`：同一条推送的更新（同一个插件、同一个 key，内容和说法变了；桌面端 `agent/push.rs` 按这次是 `Updated` 判断）冷却里也可以说——三选一刷新以后首选换了位置，是玩家自己点的刷新，正等着新的判断（讨论记录第四节“刷新后最好的那张变了才再说”）。别的规矩不变：不打断正在说的那句、排了 5 秒还没轮到就不说、同一时刻只说最要紧的一句；说出口以后照样重新开始冷却，别的推送照样要等 20 秒。冷却只管不同的推送一句接一句地说。测试 `push_voice::tests::an_update_of_the_same_push_speaks_inside_the_cooldown`。
- 读旧的总开关（`agent.autoCards.enabled == false`）只关**只有卡片**的推送（rc.9 的自动卡片就是这样的：开局战绩、这局总结、选英雄）；还有一行或语音的（海斗三选一，它的卡片只是问要不要下载模型的那张）不受它管。`Manifest::push_switch_in`、界面 `lib/plugins.ts` 的 `pushSwitch` 和设置说明里 `agent.autoCards.enabled` 那一句一起改了；测试 `push::tests::the_host_decides_whether_a_push_is_on` 加了“卡片 + 一行 + 语音的照默认”。
- 三选一在清单里加了 `until_done = true`（悬浮窗那一轮加的字段）：那一行在悬浮窗里一直留到三张卡片没了（`push_done`），不是 6 秒就收起；`pushes::augment_pick::tests::the_manifest_declares_both_pushes` 查着。

### 实施记录（英雄联盟插件只说事实、隐藏了身份的人、开局战绩照 LeagueAkari 排、OP.GG 默认开、来源标签，2026-11-26）

ADR 043 D2、D8（英雄联盟 App 那一部分）、D9（A 轮第 B3 条）。讨论记录第七节、第八节。

**玩家看到的**：

- **没有“突破口”了**：开局战绩卡片、“这局”页面、`lol.scout` 给她的字里都不再写谁是突破口，只写事实：段位、最近的胜负、连胜连败、KDA、这个英雄打过几局、谁和谁开黑。开局战绩给她的那句也只有事实（“对面：寒冰 5 连败、亚索 4 连胜。开黑：我方 伊泽瑞尔 + 锤石 + 璐璐。隐藏了身份：我方 2 人。还在查 1 人。”）。玩家问“对面谁是突破口”时由她按这些数据说；自动化模板“英雄联盟进加载界面播报突破口”照旧在（那里是她判断的地方）。
- **隐藏了身份的人照样在**：那一把海斗我方的卡牌大师、无极剑圣不会再被当成电脑漏掉。卡片和页面里这两行只有英雄和一块淡黄的“隐藏了身份”小牌（鼠标停上去：“这个人隐藏了身份：客户端不给名字和战绩，不查。”），点不开；队伍表头写“2 人隐藏了身份”。电脑只认接口自己的标记，写“电脑”。
- **开局战绩照 LeagueAkari 排**（卡片按自己的宽选样子）：
  - 窄（小于 640 px：悬浮窗的小窗约 360、Kotone 聊天里最宽 544）：两队上下排、对面在上，每人一行（英雄、名字、近 20 局胜负和胜率、KDA），点一行在下面展开详情（段位、等级、胜负、KDA、场均死、这个模式、最近 10 局、全部标签），一次展开一个；
  - 宽（640 以上，例如 Claude 的聊天卡片）：LeagueAkari 用户看惯的样子，每队一行、每人一列（头像、名字、段位、胜率大字、KDA、标签、最近 5 局一局一行）；
  - “这局”全屏（插件页、右侧栏）宽的时候也是这样，每列最近 10 局；窄的时候照旧一人一块。
- **来源标签**：每块数据一个小标签写它从哪来——“客户端 本机，只读”“腾讯 101 国服 · 日期”“aramhex-data” + “CC BY-NC 4.0” 链接、OP.GG 的蓝底字标 + “全球” + “op.gg” 链接。能换来源的两块（英雄页峡谷的英雄数据：腾讯 101 / OP.GG；海斗的海克斯：aramhex-data / OP.GG），标签是个按钮，点开选另一个来源，马上换、记住（下次打开、她查的时候照这个）；OP.GG 关着时那一项灰着。一块只用一个来源：换成 OP.GG 时那块是 OP.GG 原样的结果，没有腾讯 101 的数字。
- **卡片和页面上没有风险提示了**：原来卡片底下的“数据：客户端 · 素材归 Riot Games · 风险”和全屏页脚的“风险”折叠块去掉；卡片底下只有来源标签（和“还在查 1 人”）；全屏页脚是来源标签、图片出处、拳头 Legal Jibber Jabber 的那一句（只在全屏页上一行）。风险和许可在插件的“风险和许可”弹窗（B4）。
- **OP.GG 默认开**；不再等 issue 的回复。英雄页默认还是腾讯 101（国服），OP.GG 只在玩家把标签换成它、她查的时候才请求。
- 日志的两句改准了：“lol: 这局开始查 8 人的战绩（含你自己）；隐藏了身份的 2 人、电脑 0 个不查”、“lol: 选人时开始查我方 3 人的战绩（含你自己）”（原来写“N 个队友”，把自己也算进去了）。

**怎么做的**：

- **去掉突破口**：`client/scout.rs` 的 `WeakSpot` / `weak_spots` / `MIN_GAMES` 删掉，只留统计和标签；`text::render_scout` 去掉最后那行，多一行“隐藏了身份（客户端不给名字和战绩，不查）：我方 N 人”；页面数据去掉每个人的 `weak`；`watcher/auto_cards.rs` 的 `opening_note` 改成只写事实：两边谁连胜连败 / 这个英雄最近没玩过 / 这个英雄几局（每边最多 3 人）、开黑（`text::parties`）、隐藏了身份几个、还在查几个，160 字放不下时先省我方的、再省开黑，句子不截断。工具说明（`tools.rs`）、清单的说明和她的规矩（`plugin.toml`：“它只给事实……谁是突破口由你按这些数据判断”）、文案、README、`Cargo.toml`、设置说明（`settings_guide.rs`）、工具服务手册跟着改。`relay.rs` 里的“进加载播报突破口”是自动化的名字，没动；`games.json` 自动化编辑器的示例（“用 lol.scout 看看对面谁是突破口”）也是自动化里的话，没动。
- **隐藏了身份**：`text::Identity { Known, Hidden, NameHidden, Bot }`，`ScoutPlayer.identity` 和 `pending()`（只有 Known 且没查完才算“还在查”）。`watcher.rs`：`members()` 不再跳过没有 puuid 的人——客户端标了电脑（`botDifficulty` 不是空 / NONE、`isBot`、`playerType = BOT`，`flagged_bot`）才是电脑，没有 puuid（空的、全 0 的 `EMPTY_PUUID`，`real_puuid`）的真人是 Hidden；`on_session` 的合并抽成 `merge_session`（顺序跟 gameflow、只查 Known、之前有这一轮没出现的查得到的人留着、局内接口补上的电脑留着）；局内接口的 `isBot`（`mark_bots`，按队伍 ORDER = teamOne 和英雄的英文 id 对上）每轮补一次（`tick` 里多一行 `mark_live_bots`）；`match_view` 带上 identity，`me` 比较时 puuid 空的不算（以前空 puuid 和还不知道的 me 会对上）。选人（`champ_select.rs`）：看不到名字的队友、对面是 NameHidden，标了电脑的是 Bot。开局战绩的“查完了”只等 Known 的人。一键喊话的战绩行：隐藏了身份 / 电脑写一行“隐藏了身份”“电脑”，不算没查完。局内接口里的人多了 `hidden_identity()`（不是电脑、名字空），`lol.match` 给她的那行写“隐藏了身份”。页面数据每个人多一个 `identity`（`known` / `hidden` / `name_hidden` / `bot`），`pending`、`waiting` 只数查得到的。
- **开局战绩的排布**（`plugins/lol/app`）：看了 LeagueAkari 的 `src/renderer-shared/components/ongoing-game-panel/`（`OngoingGamePanel.vue`、`OngoingGameTeam.vue`、`player-info-card/*`，GitHub 上读的，只学排布）：两队上下、每队一个表头（队伍色点、名字、开黑等标签），下面一排固定 240 px 宽的卡片（放不下换行），卡片里从上到下是头像 + 等级、名字 + 编号、单双排 / 灵活段位、胜率和 KDA、标签、常用英雄、最近战绩一局一行。我们的：`match/TeamGrid.svelte`（宽，5 列平分、列顶 3 px 队伍色条、开黑是字母小牌、胜负配字、最近几局淡绿 / 淡红底）、`match/CardRow.svelte`（窄的一行 + 展开）、`MatchCard.svelte` 按 `bind:clientWidth` 在 640 处切换；`MatchFull.svelte` 加载 / 对局中 / 结算时宽于 640 用 TeamGrid，窄的照旧 PlayerBlock，选人照旧。testid 沿用：卡片每人 `lol-card-row`、全屏每人 `lol-match-player`（真机脚本 `mcp_app_external_real.py` 认这两个）。
- **OP.GG**：`plugin.toml` 的 `opgg` `enabled_by_default = true`；“最多 2 个并发”原来只是注释（会话的锁只管握手，调用是并发的），现在真的限了：`Stats.opgg_slots`（`tokio::sync::Semaphore`，`OPGG_CONCURRENCY = 2`，握手和调用都在里面）。缓存 24 小时、`client_app = kotone/<版本>`、出处照旧。原来峡谷英雄页“开了 OP.GG 就多一块折叠的全球数据”去掉了：OP.GG 变成那块的另一个来源，只在选了它时请求（`rift_champion_opgg` 调 `lol_get_champion_analysis`；海斗的海克斯换成 OP.GG 时调 `lol_list_aram_augments`，带 OP.GG 要求的 `desired_output_fields`）。
- **来源标签**（ADR 043 D8）：后端 `stats_tools.rs`：`KIND_RIFT` / `KIND_MAYHEM_AUGMENTS` 两块能换，来源 `qq101` / `opgg` / `aramhex`；`chosen_source`（插件设置 `plugins.lol.settings.sourceFor.<块>`，没写或不认得 → 默认）、`source_for`（她在参数里写了 `source` 就用她的、不改玩家选的；OP.GG 关着退回默认）、`source_tag`（页面数据的 `source: {kind, id, options: [{id, available}]}`）；新工具 `lol.source`（只给页面，`kind` + `source`，整份 `sourceFor` 一起写进插件设置，经 `Host::update_settings`；别的 MCP 宿主改不了设置时回 `not_saved`，页面写“这次换了，但没记住”）；`lol.champion` 多一个参数 `source`。前端：`components/SourceTag.svelte`（标签 + 菜单，OP.GG 的字标是蓝底白字的“OP.GG”，不是它的图片）、`Footer.svelte`（去掉风险折叠块，`sources` 列出这一页的来源；卡片上只有标签）、英雄页每块标题旁一个标签、强度榜页脚一个。
- **文案**：`plugins/lol/messages/{zh-CN,en}.json` 加了 `identity_*`、`source_tag_*`、`source_note_*`、`source_menu_*` 等，去掉没人用的 `weak`、`card_weak_why`、`card_foot`、`risk_title`、`foot_client`、`sources`、`source_with`、`opgg_title`、`col_rank`、`list_sep`；`state_disabled` 去掉“设置里写着风险”。
- 设计文档 `docs/design/lol-app.md` 跟着改：开头一段改了什么，第一节 1 / 3 / 6 / 9 / 11（来源标签整节重写）、第二节 1（`identity`、去掉 `weak`）、第三节 1（宽排布的图、开局战绩卡片整段重写）、第三节 4（换来源）、第四节给她那句的例子、第六节出处。`THIRD_PARTY_NOTICES.md`：LeagueAkari 那条加了“对局页的排布，只学排布”，OP.GG 那条改成默认开和答应的用法。

**测试**：

- Rust（2026-11-26，League 客户端开着，低于正常优先级 `-j 2`）：`kotone-plugin-lol` 137 个通过（新的：`watcher::identity_tests` 3 个——照 `20261126-mayhem-midgame` 那一把拼的 gameflow（名字换成编的）ORDER 5 行、两个 Hidden、只查 8 个、第二轮不重复查、局内接口 isBot 补电脑且下一轮不变回、`botDifficulty` 标的电脑不查、页面我方 5 行两行 `identity = hidden` 不算还在查、给她的字写“隐藏了身份”不写突破口；`champ_select::bots_are_only_what_the_client_flags`；`live::hidden_identity_is_a_human_without_a_name`；`text::scout_text_states_facts_hidden_players_and_parties_only`；`auto_cards` 的那句只有事实、挤的时候先省不要紧的；`stats::opgg_runs_at_most_two_requests_at_once`（5 个请求，服务器那边同时最多 2 个）；`stats_tools::source_tags_switch_and_remember`、`opgg_is_on_by_default`；`pages::text_for_her_is_unchanged` 只少了突破口那一行）；`kotone-core` 418 + 87 个通过。clippy（`kotone-plugin-lol`、`kotone-core`，`--all-targets -D warnings`）干净；rustfmt 只跑了改过的文件。
- 前端：`pnpm -C apps/desktop check`（含 LoL App 的 svelte-check）0 错误 0 警告。Playwright（一个 worker）：`plugins/lol/e2e/lol-pages.spec.ts` 7 个、`lol-stats.spec.ts` 5 个全过，其中新的：开局战绩卡片在 360 宽是“上下两队、每人一行”（行不溢出），点一行展开详情（最近 10 局、标签）、换一行、收起，隐藏了身份的两行点不开；720 宽变成每人一列（每列最近 5 局）；聊天里的卡片没有突破口、没有“风险”、底下是“客户端”标签；插件页宽的“这局”是 LeagueAkari 排布、两列“隐藏了身份”、页脚有拳头的那一句、没有风险折叠块；来源标签：英雄页默认腾讯 101，点开菜单换成 OP.GG（那块变成 OP.GG 的结果、没有 101 的数字、`plugins.lol.settings.sourceFor.rift = "opgg"`），换分路还是 OP.GG，换页再查还是 OP.GG（记住了），海斗的海克斯 aramhex-data ↔ OP.GG 各记各的，强度榜的标签不能换，点别处菜单关上。`e2e/terminology.spec.ts` 4 个通过。截图在 `target/plugin-shots/feed-and-orb/`：`opening-card-mini-360.png`、`opening-card-mini-360-expanded.png`、`opening-card-wide-720.png`、`plugin-page-match-grid.png`、`source-tag-menu.png`、`source-tag-opgg.png`。

- 真机（2026-11-26，League 客户端开着，`lol_tools_real.py --while-playing`：隔离的 KOTONE_HOME、`--background`、假客户端放 `20261123-210745` 那一把、玩家的 DeepSeek Key 只在内存里、发话 / 切窗口的工具都禁了、不发任何系统按键）：39 过 2 不过。T1“查一下这把的战绩，对面谁是突破口”：`lol.scout` 的结果里没有“突破口”，页面数据 5 + 5 人、每人带 `identity`；她自己按数据说（“艾希最近 20 局 6 胜 14 负、场均死 13.2 次，是死得最多的那个”，再说谁和谁开黑），读起来自然。没过的两条是 T4“发出去”：她这次用了 `lol.shout`（和 `game_chat.send` 一样被禁）而不是 `game_chat.send`，和这次的改动无关。

**还没测**：开局战绩作为推送真的到聊天里（`auto_card_real.py` 会弹悬浮窗、`game_events_real.py` 是 B2 那边在跑的，玩家在用客户端，这两个没跑；卡片用的是同一个 App，Playwright 截图见上）；真的英雄联盟里一把有人隐藏身份的对局（只用录的局内数据拼了 gameflow；真的 gameflow 里隐藏身份的人 `championId` 有没有、电脑的 `botDifficulty` 长什么样没实测，代码按 LeagueAkari / hh-lol-prophet 的类型写的）；OP.GG 两个工具的真实回复（`lol_get_champion_analysis` 2026-11-24 真调过；`lol_list_aram_augments` 和它要的 `desired_output_fields` 没真调过）；OP.GG 的标志用的是字标（蓝底白字的“OP.GG”），不是它的图片——要正式的图片等它回复或给素材包。

### 实施记录（风险和许可的弹窗，2026-11-26）

ADR 043 D8 的宿主部分（A 轮）。讨论记录第八节“风险提示”。来源标签（数据源切换）和英雄联盟 App 页脚里的风险是 B3 的事，不在这里。

**玩家看到的**：

- 插件「设置」里、开关下面的琥珀色风险框没了；游戏配置页里英雄联盟那一节的客户端风险没了；“操作客户端”关着时那句改成“先打开上面的“操作客户端”，这里的偏好才会生效。”（去掉了“有封号风险，看清楚提示”）。
- 每个插件一个“风险和许可”弹窗：标题“英雄联盟：风险和许可”，下面一句“只是告诉你一声：开不开由你决定，关掉这个窗口不会改任何开关。”；先是“风险”（每个有风险的数据开关：名字、现在“开着 / 关着”、风险提示，里面的 github.com/… 和 developer.riotgames.com/… 能点，用系统浏览器打开），再是“许可和出处”（每条一个标题、一段话、链接）；最下面“以后在插件的「设置」里点“风险和许可”还能再看。”和“知道了”。
- 什么时候自己弹：插件页开着时，第一次看到这个插件有风险的开关开着——英雄联盟的“读客户端数据”默认开着，所以第一次打开英雄联盟就弹；插件原来关着、在设置里打开插件（默认开着的有风险的开关跟着开了）或者打开一个有风险的开关时也弹；游戏配置页里第一次打开“读客户端数据”时也弹。每个插件只弹一次，以后从插件「设置」栏上右边的“风险和许可”（盾牌图标，在“设置”按钮左边）打开。弹窗从不拦开关：开关先照玩家点的存下，弹窗只是告诉一声。
- 英雄联盟的内容：风险五条（读客户端数据、用网上的统计、用 OP.GG 的数据、操作客户端、在客户端里发聊天）；“用网上的统计”的风险里照原文引了 Riot 的规定“Products cannot display win rates for Augments or Arena Mode items. This applies to all websites, applications and overlays.”，写明海斗三选一的评分和海克斯胜率在这条里面、换到聊天和悬浮窗里风险一样；“用 OP.GG 的数据”的风险改成 OP.GG 默认开以后的说法（还没回复同意、网站协议不许程序自动访问、不想用就关掉）。许可和出处四条：aramhex-data（CC BY-NC 4.0：署名、不商用）、OP.GG（数据来源和 opgg-mcp#26 里答应的用法：按需、最多同时 2 个、存 24 小时、写明是 Kotone、标志和链接、她说时带出处）、图片（先从客户端取、没开时从 CommunityDragon 取，安装包不带，归 Riot Games）、Riot Games 的声明（Legal Jibber Jabber）。

**怎么做的**：

- **清单**（`kotone-plugin-api`）：`Manifest.notices: Vec<NoticeSpec{id, title, body, links}>`（`[[notices]]`，serde default）。`Manifest::parse` 查：id 写法、不重名，title / body 不空，链接都是 `https://` 开头、不带空白；不认得的字段不收。数据开关的 `risk` 不变（文字里带的地址界面上做成链接），没给开关加新字段。
- **记没记过**：`PluginConfig.riskSeen: u32`（`plugins.<id>.riskSeen`，新字段，0 = 没看过、不写出去）。照 ADR 044 `adminPromptRevision` 的做法记版本：界面 `lib/plugins.ts` 的 `RISK_REVISION = 1`，比它小就算这一版没看过；以后风险变大了（多了有风险的开关、说法有实质变化）加一，玩家会再看到一次。弹窗弹出来就记（不等点“知道了”）；没存上只是下次再弹。
- **界面**：`lib/components/plugins/PluginRiskDialog.svelte`（shadcn-svelte 的 Dialog，照 `AddMcpServerDialog` 的样子）；`lib/plugins.ts` 加 `hasRiskNotes`、`riskUnseen`、`riskyOn`（有风险的开关里有开着的，插件也开着）、`markRiskSeen`、`noticeTitle` / `noticeBody`（插件文案 `plugin_<id>_notice_<条目>_title|body`，没有时用清单的中文）。
  - 插件页（`PluginPage.svelte`）只有一条规则：这页开着、这一版没看过、`riskyOn` 为真 → 弹一次（每次打开插件页最多一次）。“第一次打开插件页时默认开着的”和“在这页上第一次打开有风险的开关”都是这一条，不在每个开关上各写一份。栏上的链接只在设置里显示（插件没有设置时一直显示）。
  - 游戏配置页（`plugins/lol/ui/GameSection.svelte`）不在插件页里：在“读客户端数据”从关到开、存好以后看一下，没看过就弹同一个弹窗。只是看这一页不弹（那不是插件的页面）。
  - 去掉了 `PluginSwitches.svelte` 的风险框和它自己的链接识别、`GameSection.svelte` 的客户端风险。别的说明（数据只在本机这类）没动。
- **浏览器预览**（`ipc/browserMock.ts`）：默认当作英雄联盟的已经看过（`plugins.lol.riskSeen = 1`），别的测试打开英雄联盟、打开有风险的开关不被弹窗挡住；`?mockRisk=unseen` 从没看过开始。`pluginsMock.ts` 的英雄联盟清单加了 `notices`。
- **文案**：宿主 `messages/{zh-CN,en}/system.json` 的 `system_plugins_risk_*`、`system_plugins_notice_section`；英雄联盟 `messages/{zh-CN,en}.json` 的 `plugin_lol_notice_*`、改了 `source_stats_risk`、`source_opgg_risk`、`control_off`（`plugin.toml` 同步，`tests::messages_match_the_manifest` 查许可和出处也一致）。`terminology.spec` 的插件约定键加上 `notice_<条目>_title|body`。

**测试**（2026-11-26，League 开着，低于正常优先级 `-j 2`，Playwright 1 个 worker）：

- Rust：`kotone-plugin-api` 28 + 4 + 3 个全过（新的 `notices_are_parsed_and_checked`：读进来、camelCase、没写链接是空的，写错的 8 种读不进来），clippy 干净；`kotone-core` 416 个过（新的 `plugin_risk_seen_is_a_new_field`：旧配置读进来是 0、不写出去，记过的写回去读回来，别的字段照旧；挂的 1 个是同时在改的悬浮窗设置说明），`--lib` clippy 干净；`kotone-plugin-lol` 137 个过（新的 `risks_and_notices_cover_what_we_promised`：Riot 规定原文、#523、#26、四条许可；`messages_match_the_manifest` 加了许可和出处）。
- Playwright `e2e/plugin-risk.spec.ts` 5 个：第一次打开英雄联盟弹一次（五条风险、开着 / 关着、Riot 规定原文、链接用 openExternal 打开、四条许可），弹了就记 `riskSeen = 1`、开关一个没动，第二次打开不弹；插件关着时打开插件页不弹，在设置里打开插件就弹，按 Esc 关掉开关还是开的，之后打开“操作客户端”不再弹；游戏配置页关掉不弹、再打开弹一次、第二次不弹；看过以后“风险和许可”在设置栏上、点了打开、叉关掉设置一样，设置里没有风险框、没有“封号”“用户协议”字样；网易云音乐没有链接也不弹。改了的旧测试：`game-profile.spec`（不再有风险字、只比 `sources`）、英雄联盟的 `lol-control`、`lol-pages`、`lol-plugin`、`lol-stats`（风险框都换成“没有了”）。`terminology.spec` 4 个通过。

**还没测**：真机（玩家在打游戏）：真的 Kotone 窗口里第一次打开英雄联盟弹不弹、`config.json` 里写没写 `plugins.lol.riskSeen`；英文界面只看了文案没截图。

### 实施记录（语音输入照新形态：两条推送、重发、两件事、识别记录加频道和游戏，2026-11-26）

ADR 043 D10（A 轮第 8 条）。讨论记录第九节“语音输入按新形态”；屏蔽词那部分暂缓，没动。

**玩家看到的**：

- 语音输入往游戏里发出去一句，聊天里来一张卡片“语音输入 推送 · 发出去的话”：上面一行“你向「队伍」发了：”，右边是游戏名；下面一句一行、带时间；文字处理改过的（润色、翻译、屏蔽词），下面一行小字“原话：……”。同一个游戏、同一个频道、上一句发出后 2 分钟内的几句并在同一张卡片里（原地变长，不多一张）；换了频道、隔了 2 分钟以上、一张满了 20 句，另起一张。
- 没发出去，聊天里来一张“没发出去”：那句话、为什么（琥珀色的一句，例如“Ctrl、Alt、Shift 或 Win 一直按着（等了 1 秒），这次没发：松开再发”），一个“重发”按钮。点了：回到当时那个游戏窗口、用当时的频道再发一次；发出去了卡片原地变成“✓ 已重发到「队伍」”，这句同时进“发出去的话”；又没发出去，卡片原地换成新的原因、写“没发出去 2 次”，还能再点。找不到游戏窗口（游戏关了、Kotone 重启过而你在 Kotone 里点的）就不发，写“没找到要发去的游戏窗口……回到游戏里再点重发”——绝不打进 Kotone 自己的窗口。正在说话或者正在发上一句时点，写“等它结束再点重发”。输入条上停着的那句没发出去的，点卡片上的“重发”和点输入条上的“重试发送”是一回事。
- 插件页「设置 → 推送」（语音输入页右上角多了“设置”）列出这两条：发出去的话、没发出去，都是卡片、默认开。关掉“发出去的话”，她就不知道你在游戏里说了什么（那张卡片给她留的一句说明里是你发出去的话，下次叫她时随对话发给模型服务商）。
- 自动化和她：多了两件事“语音输入发出去一句”（文字、频道）、“语音输入有一句没发出去”（原因、代号）。不是游戏插件，所以选任何游戏、不选游戏都能选到。自动化拿到发出去的那句话；她看“现在在玩什么”（`game.state`）时只看到在哪个频道发了一句，看不到那句话——那句话只经“发出去的话”推送给她，关掉那条就不给。
- 识别记录（语音输入 → 历史）：每条下面“语音时长 2.5 秒 · 英雄联盟 · 队伍”，详情里多一行“发到”。旧记录、没发出去的没有这一截。

**怎么做的**：

- **推送台**（新的 `plugins/voice-input/src/pushes.rs`）：每发一次（听写、输入条上的重试、重发最近一条热键、卡片上的重发）都把结果（入口回的 `ChatOutcome`，或者没发出去的原因和代号）交给它；它算好推送和事件，交给后台线程（`spawn_blocking`，一次一条、按先后），不在发送的路上等。合并规则是一个纯函数（`plan`）：同一个游戏 id、同一个频道 id、上一句发出后 `MERGE_WINDOW_MS`（2 分钟）内、不满 `MAX_LINES`（20 句）→ 同一个 key（`sent:<第一句的 id>:<时间>`）再推一次，否则另起。“没发出去”的 key 是 `failed:<这一句的 id>`，同一句再失败（输入条重试、卡片重发）更新同一张、次数加一；之后发出去了，那张标成已重发、原地更新。多行发了一部分：发出去的几行进“发出去的话”，没发的几行进“没发出去”（重发只发没发的那几行）。内存里记最近 32 句没发出去的（窗口、频道、文字），给重发用。
  - 卡片的数据：`sent = {game:{id,name}, channel:{id,name}, lines:[{text, source?, atMs}]}`；`failed = {id, text, source?, reason, code, atMs, attempts, channel?:{game,id}, resent?:{game, channel, atMs}}`。页面 `ui://kotone/voice-input/sent`、`…/failed`（清单 `[[pages]]`），组件 `ui/SentCard.svelte`、`ui/FailedCard.svelte`，聊天里的卡片和右侧栏里的完整页面是同一个组件。
  - 给她的那句（只有“发出去的话”带）：“玩家刚用语音输入在英雄联盟的「队伍」频道发了：……”，每发一句追加一条（并进同一张卡片时宿主照 D5 追加“更新了推送”那一条），所以只在这条推送开着时她才知道。
  - 宿主：插件启动时（`Plugin::start`）把 `DesktopHost` 存进推送台；命令行、本体的测试没有宿主就不推。开关由宿主执行（`push_off_in`），这里不看。
- **重发**（`send.rs` 的 `resend_card`；页面工具 `voice-input.resend {id, channel?}`，只给页面、`side_effects = ["input"]`）：照旧占住发送权（`claim_send`，和重发最近一条同一条路，输入条照常显示发送中 / 已发送 / 没发出去）再交给本体唯一的入口（ADR 041）。窗口和频道用这一句当时的（内存里有：开始收音时的前台窗口、当时选的频道）；Kotone 重启过、内存里没有：从识别记录里找回那句话（同一个 sessionId 最新的一条，是 error 才发；已经是 sent 的算重发过），频道用卡片上记的，不知道窗口就不切。输入条上停着的正是这一句（Error、process case id 一样）时，走输入条“重试发送”那条路（同一份草稿）。识别记录：重发的不是现在这一句时，照原来那条记录再记一条（sent / error，“失败 → 重发”两条，和输入条重试一样）。
  - **入口加了一个开关** `ChatRequest::strict_target`（`crates/kotone-core/src/send_to_game.rs`，默认 false，只加不改）：开着时拿到锁、切回 `restore` 以后再查一遍——切不回去（窗口关了、`SetForegroundWindow` 不让）或者前台不是要发去的那个窗口 → `ChatFailure::WindowGone`；没给 `restore` 时前台是 Kotone 自己 / 桌面 / 没有前台 → `OwnWindow` / `Desktop` / `NoForeground`。一个键都不按。原因：玩家在 Kotone 主窗口里点重发时前台是 Kotone，原来的规矩“窗口没了就发到现在的前台”会打进 Kotone 自己的输入框。语音输入平时的发送不带它，照旧。
  - 卡片上的重发要在“听一句”以外的时候用到本体：`AfterHearing` 加了 `attached(&Arc<Orchestrator>)`（默认什么都不做，`Orchestrator::into_arc` 调一次），语音输入在这里记下本体的弱引用；插件的工具从这里拿到编排器和 `VoiceInput`。
  - 没发出去的原因：`SendFailure` 的话照旧（入口的 `ChatFailure` 的话、超时、发送线程异常）；重发切不回窗口的几种（`WindowGone` / `OwnWindow` / `Desktop` / `NoForeground`）统一成“没找到要发去的游戏窗口（关了，或者前台是 Kotone 自己）：回到游戏里再点重发”，代号 `TARGET_WINDOW_GONE`（过程日志的错误码多了这一个）。
- **事件**：清单 `[[events]]` 登记 `voice-input.sent`（data：text、channel、channelName；事件的游戏 = 入口挑中的游戏配置）和 `voice-input.failed`（data：reason、code；游戏 = 当前选中的游戏配置）。原来只有带游戏中心适配器的插件能登记事件：`kotone-plugin-api` 的 `Manifest::event_scope`——带适配器的以适配器 id 开头，没有适配器的以插件 id 开头；事件目录、`game.state` 的分组按它；界面 `gameEvents.ts` 同一条规则，没有适配器的插件的事选任何游戏、不选游戏都在选择里。报事件照旧经游戏中心（`Host::game_event` → `games::push`）：**游戏中心里有这个游戏（在跑）时才报得出去**，发到“通用（任意前台窗口）”时没有事件。
- **识别记录**：`HistoryRecord` 加 `game`、`channel`（`{id, name}`，`HistoryPlace`；没有时不写这两个键，旧记录照样读，旧版本读新记录不管多的键）。取自入口的 `ChatOutcome`（发出去了、或者多行时发了一部分）；没发出去的没有。`profileId` 照旧是开始收音时选中的游戏配置。界面 `contracts.ts` 的 `HistoryRecord` 跟着加。
- **她看不到你在游戏里说的那句话（除非“发出去的话”开着）**（主会话定的，照 D10“关掉这条推送就不给”）：事件的清单可以写 `private = ["<字段>"]`（`EventSpec::private`，只加不改；要是 `data` 里登记过的字段，清单检查查）。`game.state` 的最近几件事经 `EventCatalog::data_for_her` 去掉这些字段（桌面端 `games.rs` 的 `recent_for_her`），事件目录里这个字段的说明后面写“只交给自动化，这里的最近几件事里不显示”；自动化拿到的还是整份事件（那是玩家自己的自动化）。语音输入的 `voice-input.sent` 标了 `private = ["text"]`：她在 `game.state` 里只看到“在「队伍」发了一句”，那句话只经推送给她的说明交给她。外部 Agent 能调 `voice-input.resend` 照旧（ADR 014）。
- 设置说明（`settings_guide.rs` 的 `plugins.*`）加了语音输入的两条推送 `pushes.sent` / `pushes.failed` 和位置“插件 → 语音输入 → 设置 → 推送”，她能帮玩家开关；插件的测试查设置说明和清单一致。
- 叫法：推送、频道、重发、原话照术语表；文案在 `plugins/voice-input/messages/*.json`（推送的名字 / 时机 / 做什么、事件的名字用约定的键 `plugin_voice_input_push_<推送>_title|when|what`、`plugin_voice_input_event_<名字>`，插件的测试查和清单一致）。

**测试**：

- Rust（2026-11-26，英雄联盟客户端开着，低于正常优先级 `-j 2`）：`kotone-plugin-voice-input` 20 个（新的 11 个：合并规则——同一个频道 2 分钟内同一个 key、刚好 2 分钟还算、换频道 / 超过 2 分钟 / 满 20 句另起；事件名都在清单里、不知道游戏不报；失败的卡片、同一句再失败更新、之后发出去标成已重发、多行发了一部分拆成两张；接上编排器、换掉入口和宿主的：发出去两句并成一张 + 两件 sent + 识别记录有游戏和频道 + 入口拿到的是 Player、回到窗口 42、不 strict；**八种没发出去**（修饰键、键盘被占、客户端没发、被拦截、要管理员、注入失败、超时、发送线程异常）各一张“没发出去”、原因和代号对、报 failed、记录是 error 没去处；**卡片重发经入口、回到窗口 42、频道“所有人”、strict**，卡片原地标已重发、识别记录多一条 sent，再点不发；重发切不回窗口时卡片换原因、次数 2、还能再点；**重启以后从识别记录找回**（不切窗口、strict、用卡片上的频道），早就发出去的、找不到的不发；重发最近一条热键是自己的卡片；工具缺 id 直接说）。`kotone-core`：`history` 新测试（有游戏和频道的写、读回来一样，没有的不写键，旧版本的一行照样读）、`send_to_game` 新测试 `a_strict_resend_never_types_into_kotone_or_another_window`（切回来了照常发；切不过去 → WindowGone 一个键没按；没窗口时前台是 Kotone / 桌面 / 没有 → 不发；不带 strict 的照旧发到前台）；`tests/orchestrator.rs` 87 个照常通过（没改）。`kotone-plugin-api` 28 个 + 边界测试（新的：没有适配器的插件以插件 id 登记事件、进目录、关着时标原因）。clippy（这三个 crate，`--all-targets -D warnings`）干净。
- Playwright（一个 worker）：`plugins/voice-input/e2e/pushes.spec.ts` 3 个：聊天里“发出去的话”两句一张、原话一行、游戏名；“没发出去”的原因和“重发”，点重发调 `voice-input.resend`（id + 当时的频道）、不打开右侧栏、显示“已重发到「所有人」”，推送原地更新以后还是一张；插件页「设置 → 推送」两条、默认开；识别记录列表和详情写“英雄联盟 · 队伍”，旧记录不写。`plugin-events.spec`、`push.spec` 其余、`terminology.spec`（除了别的任务正在改的英雄联盟文案）通过。截图在 `target/plugin-shots/round9/`。

**还没测**：真机（玩家在打游戏）：真的在游戏里发出去 / 没发出去时卡片来没来、在悬浮窗里点重发（游戏在前台，不切窗口）、在 Kotone 主窗口里点重发（切回游戏窗口再打字）、Kotone 重启以后点旧卡片的重发。`scripts/manual-e2e/voice_input_real.py` 没跑（有游戏进程时它自己拒绝），它也还不查推送和重发。事件在真的游戏中心里报出去、自动化收到。

### 实施记录（整合：五个部分合在一起，2026-11-26）

A 轮五个部分（悬浮窗、英雄联盟的覆盖层改推送、英雄联盟的事实和来源标签、风险和许可的弹窗、语音输入的推送）并行做完后，在同一份工作区上跑全量检查、修各部分之间的接缝。

**修了什么**：

- **选英雄卡片的来源**（ADR 043 D8）：B2 的选英雄卡片底下原来是一行字“数据：腾讯 101（掌盟）”，改成和别的页面一样的来源标签（B3 的 `SourceTag`，“腾讯 101 国服”）。插件给页面的 `source` 从中文名改成来源 id `qq101`（`plugins/lol/src/pushes.rs`），页面认不得的旧值按腾讯 101 显示；文案 `plugin_lol_champ_select_source` 删了（中英）。
- **英雄联盟的两份文案文件各有一处多出来的回车**（`\r\r\n`，某个脚本写进去的）：git 因此把整个文件当成非文本，提交时不转换换行、差异整份显示。去掉了，现在和别的文件一样。
- **术语表**加了“风险和许可”一行（每个插件一个弹窗：先风险、再许可和出处，只告诉一声、不拦开关）。
- **设置说明**：`plugins.<id>.riskSeen` 进了不给她改的那一栏（界面提示状态，弹窗弹出来时界面自己记）。
- **开发文档**加了一句：没设 `KOTONE_HOME` 的 cargo 测试程序用 `target/tmp/kotone-test-home/<进程号>`，碰不到 `~/.kotone`。
- 删了没人读的 `AUTO_ADMIN_PROMPT_FLAG`（ADR 044 以后用不到）。
- 收尾单上的另外几条核对过已经是对的：`plugin_lol_state_disabled` 不再提风险（B3 改了）、`THIRD_PARTY_NOTICES` 的 OP.GG 一条写的是默认开（B3 改了）、`plugin_lol_risk_title` / `chat_popup_*` 这些没人用的文案已经删了、术语表 / 助手手册 / 架构文档里没有“插件覆盖层”“消息提示窗口”的旧说法了（只剩“原来的……并进了它”这种历史说明）。

**检查**（2026-11-26，英雄联盟客户端开着、后来进了对局：低于正常优先级、`-j 2`、Playwright 一个 worker）：

- `cargo fmt --all --check` 干净；`cargo clippy --workspace --all-targets --locked -- -D warnings` 干净；`cargo test --workspace --locked --no-fail-fast` 1732 过、0 挂、34 个忽略（54 个测试程序）。
- `pnpm -C apps/desktop check`：本体 0 错误 0 警告，英雄联盟 App 0 错误 0 警告。
- Playwright 全量（desktop + plugins 两个项目，一个 worker）：232 个全过（本体 189、插件 43），没有时过时不过的。
- 跑之前要重启共用的开发服务器：改过的模块在页面里带 `?t=…`，测试里 `import("/src/lib/…")` 拿到另一份，第一次全量里 `agent-chat` 两条就是这样挂的（改了 `stores/ui.ts` 以后没重启），重启后过了。英雄联盟 App 是打包好的 `plugins/lol/app/dist`，改了 App 的源码要先 `pnpm build:apps`。

**还没测**：真机（玩家在打游戏）。`game_events_real.py` 没跑：它关了悬浮窗和小窗，但“对面拿龙就说”的自动化用弹幕、关了悬浮窗时她的回答也走弹幕，会在玩家的屏幕上出字。

### 实施记录（悬浮窗：她的头像 + 气泡 / 聊天小窗 + 进度圈，2026-11-26 晚）

ADR 043 D6 的晚修订（[rc.10 试玩反馈](../research/2026-11-26-rc10-playtest-feedback.md)第 4 条）。A 轮那份实施记录里“一个窗口、不抢焦点、拖动、位置、哪块屏、独占全屏、不进截图”的做法都没动，这里只写改了的。

**玩家看到的**：

- 悬浮窗出来时，角上一定是她的头像（和聊天页同一个：上传的头像、角色自带的表情、没头像时名字的第一个字），头像外面一圈进度条。旁边是**气泡**或**聊天小窗**：她在听 / 在想 / 说了一句短话、推送的一行是气泡；推送了卡片、她这一轮用了工具（比如“放歌”）、带卡片的结果、只写不念的细节、长回答（超过 60 个字或分了行）、同时来了不止一条（她说话时来了推送、一下来了两条推送、这一次出现里她答了不止一轮）就是聊天小窗。这一次出现里换成了小窗就一直是小窗，收起以后再从头算。
- 聊天小窗就是聊天页：玩家的话是右边的气泡，她说出口的话是带喇叭的一句，工具是一行“放歌 · 完成”，推送是同样的“英雄联盟 推送”一行 + 卡片；只是没有每一轮上面的小头像和名字、没有页脚（复制、用量），间距紧一点。它是这段对话最近的几轮（这一次来的，加上前面最多 10 轮），新的在下面；高度跟着这一次来的那几轮走，最高是游戏所在屏幕工作区高度的 40%；往回翻用滚轮或上面那一行的 ↑ ↓。上面那一行：她的名字、↑ ↓（能翻时才有）、打开 Kotone、×。
- 进度圈：最后一次变化以后，气泡走 6 秒、聊天小窗走 10 秒，走完整个收起（头像一起）。她在听 / 在想 / 在说 / 等你回答时圈是满的，跟着她的状态呼吸或转（颜色和原来的光环一样），不走；她说完了从满的一圈开始走，她最后那句一直留到圈走完。鼠标停在上面圈就停住，移开接着走（至少还剩 2 秒）；拖着时也停住。推送了一行、弹出卡片、聊天小窗开着时卡片原地更新，都从头走。
- × 和“打开 Kotone”都是整个收起，下一条推送来了再出来；她那一行要等她换了状态才再出来。原来的“推送流”（只放推送的小窗）没有了。
- 设置页“悬浮窗”一节和“推送时弹出小窗”的说明照新的样子改了（“鼠标移开 8 秒后收起”改成“没有新消息 10 秒后收起”）；术语表“悬浮窗 / 气泡 / 小窗”一行：小窗全称聊天小窗。

**怎么做的**：

- **规则只在一处**：`routes/floating/shape.ts` 的 `floatingShape`（纯函数）。输入是后端弹没弹小窗（`view.mini`）、她这一行、气泡里推送的几行、`freshTurns`（这一次出现以后来的那几轮：她正在回答的、后端记的 `since` 以后开始的——推送记进对话比窗口出来早几毫秒，放宽 2 秒——、推送在 `since` 以后更新过的、弹出小窗定位的那条）。`lines + max(她这一行, 她的几轮) > 1` 就算“同时来了几条”；`complexTurn` 看她那几轮：有工具（`speak`、`reply.ask`、`danmaku` 是她怎么说话，不算）、结果带卡片、`speak.written`、长回答、复盘。推送那几轮不单独算（由“弹没弹”和“几行”代表），所以同一个 key 原地更新的卡片不会把气泡变成小窗（同传那种“每句一行 + 一张一直更新的卡片”还是气泡）。
- **聊天小窗用聊天页的东西，不另写**：悬浮窗页面里 `provideAgentChat(new AgentChat())`——和聊天页同一个对话状态类、同一套订阅（`kotone://agent-event`、`kotone://push`、`kotone://agent-ask`……），`buildTurns` 同一个时间线，`ChatTurn` 画每一轮（加了 `compact`、`onOpenPush`、`onOpenCard` 三个可选属性，聊天页不传，样子不变；`ToolCallCard` 加了 `onOpenCard`；`Turn` 加了 `startedAtMs`）。小窗里没有右侧栏：卡片上面那一行、“打开 Kotone”都走 `auto_card_open`（主窗口到前面、定位到那条推送；工具卡片定位到那段对话）。窗口藏着时不算时间线（`buildTurns` 只在显示着时读）。删了 A 轮的 `pushStream.svelte.ts`。名字和头像从 `settingsStore`（悬浮窗页面自己读设置、跟着 `kotone://settings-changed`），头像组件就是 `KotoneAvatar`。
- **后端管显示不显示和进度圈**：`floating/model.rs` 重写成“一扇窗 + 一个钟”：内容是她这一行、推送的几行、弹出的那张、或者界面报了“小窗”；钟在 `Clock { total, left, from }`，停着 / 走着由 `held()` 定（鼠标在上面、拖着、她在忙、写了 `until_done` 还没等满 90 秒的那一行）。界面每次量好大小时报 `floating_layout(revision, width, height, mini)`，`Model::shape` 据此把一圈换成 10 秒、从头走（更早那一版报的不算）。给界面的样子多了 `shown`、`since`、`ring {totalMs, leftMs, running, atMs}`（去掉了 `mode`）；鼠标进出、拖动只改圈，发一次样子、不重摆窗口（`announce`）。`bubble.rs` 只改两处：每次改状态告诉悬浮窗“她在不在忙”（`Model::active`：在听 / 听清了 / 在想 / 等你回答，在说且正在念），念完一句（`spoken`）也告诉它（圈从这时起走）。她那一行被 `bubble.rs` 收起时：她已经说完了的留着最后那句（界面画最后一个不是 hidden 的快照），还在忙时被取消的马上收起；聊天小窗开着时小窗留着、再走一圈。`opened`（打开 Kotone）现在和 × 一样整个收起。
- **页面**：`Floating.svelte`（拿样子、算规则、量大小）、`Avatar.svelte`（头像 + SVG 圈；鼠标一停上去先在页面里停住，不等后端那一版；走着时每 50 毫秒重画——`requestAnimationFrame` 在无头 Chromium 里会停在最后几帧，改成了定时器）、`HerLine.svelte`（只剩气泡；`compact` 是聊天小窗最下面那一行：在听、听清了、等你回答、收尾、出错这些对话里还没有的）、`MiniChat.svelte`（照 A 轮的 followBottom：跟着最新的那一轮，比框高就从它上面开始看；玩家往回翻了不跟，翻回最下面、弹出新的又跟上）。头像在角上，旁边的往屏幕里长（右边的角 `row-reverse`，上边的角顶对齐）。
- 时长：气泡 6 秒、聊天小窗 10 秒（brief 给的例子），移开至少还剩 2 秒（A 轮是“移开再留 3 秒”，现在是“接着走”，2 秒是怕一移开就没了）。

**测试**（2026-11-26 晚—11-27 凌晨；WeGame / tcls 开着：低于正常优先级 `-j 2`，Playwright 一个 worker）：

- Rust：`kotone-tauri` 196 个过。`floating::model::tests` 9 个：一行推送 6 秒走完收起；鼠标停住不走、移开接着走（至少 2 秒）；她在听 / 在想 / 在说时不走、念完从满的一圈走、`bubble.rs` 收起后最后那句留到走完、走完收起的她再开口又出来；在听时被取消马上收、小窗开着时小窗留着再走 10 秒；换成小窗一圈 10 秒、旧版本报的不算、小窗开着时原地更新从头走；`until_done` 的一行停着圈、`done` 后接着走、90 秒兜底；alert 再弹、×、打开 Kotone；× 收起她那一行到她换状态（说完了不算）；拖着停住、最多三行。`bubble::tests` 加了一个（在忙 / 不在忙）。clippy（`kotone-tauri`、`kotone-core`，`--no-deps --all-targets -D warnings`；不加 `--no-deps` 时卡在英雄联盟插件正在改的三处）干净；`kotone-core` 的设置说明测试过。
- Playwright `e2e/floating.spec.ts` 6 个，5 个过：规则（纯函数：一条简单的是气泡；卡片、工具、`speak.written`、长回答 / 分行、两条以上、她答了两轮是小窗；说话 / 问一句 / 弹幕不算工具；`freshTurns` 的放宽、更新、定位、正在回答；进度圈的算法）；气泡（她的头像在角上、在听 → 在想 → 在说 → 等你回答 → 出错、旧序号不覆盖、报上去的大小和“气泡”、她说完后接着画最后那句、收起时什么都不画）；一行推送（圈在走、鼠标停上去停住并告诉后端、点它打开 Kotone、按下交给后端拖、×、移开后接着走到 0、后端收起后什么都不画）；弹出卡片的聊天小窗（这段对话的 4 轮用聊天页的组件画：玩家的话、她的回答、三格一行、开局战绩的 MCP App；没有每轮的小头像和页脚；报“小窗”；高度不超过上限、卡片从上面开始看；滚轮、↑ ↓；同一个 key 原地换、App 里显示新名字、不多一张；新推送放最后；卡片上面那一行和“打开 Kotone”都定位到那张；×；宽 440）；她的一轮（在听、在想时是气泡，`music.play` 那一行一出来换成小窗，工具做完、她说的那句和聊天页一样，结束后还是小窗，收起再出来一句提醒又是气泡）。设置那一个前半段（“悬浮窗”说明里有聊天小窗、推送时弹出小窗、角、开关）过了，后半段“拖过以后位置还原能点”没过：共用的开发服务器热更新久了，页面里 `stores/ui.ts` 加载了三份（`?t=` 不同，查过），后端发的 `settings-changed` 写进的是另一份——和 A 轮记的是同一个毛病，要在新起的开发服务器上重跑。`agent-chat.spec`（3 个）、`push.spec`（2 个）同样是设置 / 导航那几个，同一个原因；`terminology.spec` 4 个里“不说的词”过了，“每条文案都用到了”卡在英雄联盟插件新加的 `plugin_lol_cs_spells_rate`（不是这边的）。svelte-check 0 错 0 警告。截图 `target/plugin-shots/roundB/floating-bubble.png`、`floating-mini-card.png`、`floating-mini-tool.png`。
- 真机 `scripts/manual-e2e/floating_real.py`（玩家睡了、交出了机器；隔离的 `KOTONE_HOME`，假客户端时间线；改了：等开局战绩推来、窗口不再变大再点，查头像和圈、鼠标停上去圈停住、鼠标不在上面时圈走完自己收起）：**29/29 过**。推来卡片弹出聊天小窗时前台一直是测试窗口、没收到键；置顶 + `WS_EX_NOACTIVATE`；不进截图；和输入条不叠；小窗里是文字识别模型、选英雄、开局战绩三张（同一段对话，后来的往下加）；点 ↑ ↓、拖上面那一行（窗口跟着走 -160, -120，记下 `{bottom-right, 9521, 8707}`）、× 时前台都没动；**滚轮收得到**：列表滚了一格（100 像素），页面上却没记到 `wheel` 事件（滚动是 Chromium 自己做的；这台机器开着“滚动非活动窗口”），↑ ↓ 留着；下一次在拖到的地方出现（右下角差 0 像素）；鼠标不在上面时 10 秒左右自己收起。× 以后来了一条“对面到 6 级”的推送一行，悬浮窗照样出来成了气泡（英雄联盟插件 B 轮新加的）。

**还没测**：

- 真游戏里：盖在英雄联盟上看、点、拖、滚；她的一轮语音（在听 → 用工具 → 换成小窗）在真机上的样子（真机脚本只有推送，没有叫她）；多显示器、缩放不同的屏之间拖。
- 透明的地方：聊天小窗旁边头像上面那一条（56 像素宽）是透明的，但还是窗口的一部分，会挡住下面游戏的点击（以前的气泡窗口也有，现在高一些）；要的话以后用 `SetWindowRgn` 只留内容那几块。
- “滚动非活动窗口”关着的电脑上滚轮可能收不到（↑ ↓ 在）。

### 实施记录（B 轮：选人卡片的完整内容和一键导入，2026-11-27）

ADR 043 B 轮第 1 条（讨论记录第二节“选人（排位）”“选人（海斗 / 大乱斗）”，玩家原话“这些功能我都想要，包括一键符文出装”）；D2（只说事实）、D3（给她的那句只在结束时）、D5（同一个 key 原地更新）、D8（一块一个来源）。

**玩家看到的**：

- **峡谷（排位、匹配）的选英雄卡片跟着选人的进度长**（还是那一张，原地更新；右上角一个小牌写现在到哪一步：预选 / 禁用 / 还没轮到你 / 轮到你选 / 你锁定了）：
  - 预选后、禁用结束前：**可以禁的**（腾讯 101）——“上路：艾瑞莉娅最难打的、这个版本最强的”，先列你预选的英雄这一路对它胜率最低的三个（“你对它 44.6%”，红字），再补这一路强度最高的（“T0 · 胜率 52.8%”），一共最多 5 个；队友的英雄、已经禁了的不列。
  - 轮到你选：**你的英雄池**（你自己最近的峡谷对局 + 腾讯 101）——你最近玩过的英雄（这一路榜上有的），每个写“最近 6 局 4 胜”、这一路的强度和胜率，对上对面已经锁了的英雄“好打 / 难打”（101 的数）；没有对位数据的写明“和对面已锁的没有对位数据”。被禁、被别人选了的不列。
  - 锁定以后：**对线**（OP.GG 的对线指南）——“艾瑞莉娅 对 亚托克斯 · 上路”，写明“对手是猜的：对面在这一路选用率最高的”，对线占优 / 单杀占优是谁、建议打法（均势……），OP.GG 给的提示原文（英文，折起来）。
  - 我方 3 个人选了英雄以后：**阵容**（客户端的英雄资料）——“我方 5 人 物理 4 · 魔法 1 · 混合 0　前排 2 · 强控 1 · 远程 2”，照固定规则标出“全是物理伤害 / 全是魔法伤害 / 没有前排 / 没有强控”；对面锁了 3 个以后多一行对面的。下面一行小字写怎么数的。
  - 有英雄（预选也算）就有**我的英雄**（腾讯 101 这一路）：符文（局数最多的那一页，9 个）、出门 / 鞋 / 核心三件 / 后期可选、加点（主升 Q > E > W · 前 6 级）、召唤师技能，和**一键导入**。
- **海斗**：原来的评级、胜率、备选席照旧；多了**我的英雄**（aramhex-data）：最好的 5 个海克斯（比这个英雄平均高几个点）、核心三件（两套，带胜率）、用得最多的召唤师技能、一键导入（这份数据没有符文，写明）。
- **一键导入**：“操作客户端”开着时，一点就把卡片上那一份写进客户端：符文页（上次导入的那一页还在就改它；没有、还能加就新建；满了就换掉当前页，结果里说“符文页满了，换掉了「上单」”）、这个英雄的出装方案（名字“Kotone 亚索 中路”，你自己的方案一套不动）、召唤师技能（闪现还在你原来放的那一边）。按钮下面一行一样写成了什么，没写成的写客户端的原话。“操作客户端”关着（默认）时按钮是虚线的，旁边写“要先在插件设置里打开“操作客户端”（默认关）”，点按钮或“去设置”都只是带你去插件设置（打开开关时有风险和许可的弹窗），不替你打开、不写任何东西。选英雄结束以后按钮换成“选英雄结束了，不能导入了”。
- 窄到约 360 px（悬浮窗的小窗）也看得清：每块一个英雄一行；这一步要紧的那块（禁用 / 英雄池 / 对线）在我方那几行下面，再是我的英雄和一键导入，阵容的数在最后。
- 给她的那句照旧只在选英雄结束、进了游戏时带一句事实（“选英雄结束了（峡谷）：你用亚索（中路，强度 T4），队友 亚托克斯、李青、金克丝、锤石。”）。插件页「设置 → 推送」里“选英雄”那条的说明跟着改了。

**怎么做的**：

- **卡片**（`plugins/lol/src/pushes/champ_select.rs`）：原来的数据（`mode`、`team`、`bench`、`source`、`ended`）不变，多了 `stage`、`bans`、`comp`、`pool`、`matchup`、`mine`（没有的是 null）和 `control`（“操作客户端”开没开，开关一变卡片跟着更新）。峡谷只对峡谷的队列（400 / 420 / 430 / 440 / 480 / 490 / 700）长这几块；大乱斗（450）、斗魂竞技场照旧。每块的数据一次选人里只取一次（`CardState` 的 `memo`，键是块和它的输入：英雄、分路、锁没锁、对手），取不到的 15 秒后再试——不每一轮（2 秒）都去网上要，网断了也不卡住后台那一轮。
- **固定规则**（`pushes/draft.rs`，纯函数）：`stage`（`timer.phase` + 我的禁 / 选动作）、`draft`（我的英雄、分路、我方 / 对面的英雄、已禁的、我的召唤师技能）、`ban_candidates`（101 `_confront` 的“难打”按胜率升序最多 3 个，再按强度 T 越小越强、一样时胜率高的补到 5 个）、`my_pool`（最近 40 局里峡谷队列的英雄，按局数、胜场排，最多 8 个）+ `rank_pool`（对面已锁的英雄在“好打”里 +1，在“难打”或强度榜写的克制里 −1，按这个数、这一路胜率、局数排，列 4 个；知道分路时不在这一路榜上的不列）、`guess_opponent`（对面已锁的英雄里这一路选用率最高的）。阵容（`pushes/comp.rs`）：客户端 `/lol-game-data/assets/v1/champions/{id}.json` 的 `tacticalInfo.damageType`（物理 / 魔法 / 混合）、`attackType`、`roles` 里的 `tank` 或 `playstyleInfo.durability` = 3（前排）、`crowdControl` = 3（强控），至少 3 个人才数。
- **数据**：101 加了只取 `_confront` 的 `Stats::matchups`（和英雄页同一个地址，缓存共用）；`qq101::TierRow` / `Matchup` 能反序列化（记进 memo）。**海斗出装找到了数据源**：aramhex-data 自己就有 `core_builds.json`（每个英雄最多 3 套核心三件，带局数、胜率、选用率）和 `hero_spells.json`（召唤师技能组合的局数、胜率），`Dataset::add_builds` 读（两个都是可选文件，`latest.json` 没列就不取，照旧按 SHA-256 校验），讨论记录第十一节“海斗出装的数据源”可以划掉；所以没有用 OP.GG 的大乱斗出装。对线用 OP.GG 的 `lol_get_lane_matchup_guide`（2026-11-26 真调过一次看格式：回的是一段 JSON，`stats/opgg.rs` 的 `parse_lane_matchup` 只取 `lane_advantage_champion`、`lane_solo_kill_advantage_champion`、`recommended_play_style`、`opponent_champion_tip`），只在锁定以后、OP.GG 开着时请求，一次选人一次，照答应的最多 2 个并发、存 24 小时。
- **我的英雄 / 一键导入是同一份**（`pushes/import.rs`）：`rift_loadout`（101：符文局数最多的那一页——101 的符文不按名次排，原来英雄页取 `first()` 可能不是最常用的那页，这里按局数挑；出门选用率最高的一种、鞋前两种、核心三件前两种、第四到第六件各前两种去掉核心里已有的、召唤师技能选用最多的一组）、`mayhem_loadout`（aramhex-data：核心三件前 3 套、召唤师技能用得最多的一组、没有符文）；卡片上那块（`rift_mine` / `mayhem_mine`）由同一份画。
- **写客户端**（`plugins/lol/src/client/loadout.rs`，新的；`client/http.rs` 加了 `Lcu::put`）：照 LeagueAkari 英雄数据窗的做法（`src/renderer/src-opgg-window/opgg/utils/loadout.ts`，MIT，只学做法不抄代码）——符文 `GET /lol-perks/v1/pages` + `GET /lol-perks/v1/inventory`（`canAddCustomPage`；老客户端没有就数 `ownedPageCount`）→ 名字以 Kotone 开头、能改的那页 `PUT /lol-perks/v1/pages/{id}`，没有且能加就 `POST /lol-perks/v1/pages`（带 `current: true`），满了换当前页（当前页是默认页就换第一页能改的），再 `PUT /lol-perks/v1/currentpage`；不齐 9 个的不写。出装方案只走客户端接口（Akari 现在是写游戏目录里的文件）：`GET /lol-summoner/v1/current-summoner` 的 `summonerId` → `GET /lol-item-sets/v1/item-sets/{id}/sets`、去掉同一个 `uid`（`kotone-<英雄>-rift` / `-mayhem`，同一个英雄再导一次是换掉）、我们的放最前、玩家的不动 → `PUT` 整份；同一组里一样的装备并成一格写数量；地图峡谷 11、海斗 12。召唤师技能 `PATCH /lol-champ-select/v1/session/my-selection`，左右照 Akari 的“auto”：新的和现在的交叉了就对调。
- **工具** `lol.import`（`tools.rs`，只给页面，`needs = ["client", "control"]`，副作用 `controls-game-client`，和 `lol.client` 一样；`watcher.rs` 的分派加了一行）：先查“操作客户端”（关着回 `disabled` / `control_off`，不连客户端），再查读客户端和统计两个开关、客户端开没开、在不在选英雄；返回 `{status: ok / partial / error, text（给她的一句）, runes / items / spells: {result: created / updated / replaced / written / no_data / not_now / no_room / error, name, old, error, spells, blocks}}`，日志一行“lol: 一键导入（英雄 157）：写好了：……”。
- **页面**（`app/src/pages/ChampSelect.svelte` + 新的 `pages/champ-select/{Draft.svelte, Mine.svelte, types.ts}`）：每块右上角一个来源标签（可以禁的、英雄池、我的英雄（峡谷）：腾讯 101；我的英雄（海斗）：aramhex-data；对线：OP.GG；阵容：客户端）；一键导入调 `kotone_lol_import`，关着时 `openPage(ui://kotone/lol/settings)`。文案 `plugin_lol_cs_*`（中英）、`plugin_lol_tool_import`；清单 `champ-select` 推送的 `what`、“操作客户端”的说明（加了“选英雄卡片上点‘一键导入’才写符文页、出装方案和召唤师技能”）和 messages 一起改。`THIRD_PARTY_NOTICES.md`：LeagueAkari（一键导入的做法，只学做法）、aramhex-data（核心三件和召唤师技能）、OP.GG（锁定时一次对线指南）。

**测试**：

- Rust（2026-11-27，wegame 开着，低于正常优先级 `-j 2`）：`kotone-plugin-lol` 176 个通过、2 个照旧忽略。新的：`pushes::draft` 5 个（每一步的 stage、读选人接口、可以禁的先克制再强度且不列队友、英雄池只看峡谷局并按“好打 −难打”/胜率/局数排且被禁的不列、对手猜选用率最高的）；`pushes::comp` 1 个（计数和四条固定规则、不到 3 人不数）；`pushes::import` 2 个（峡谷那一份按局数挑符文页、出装各组、卡片那块的 9 个符文 / 四组装备 / 加点 / 召唤师技能；海斗那一份没有符文、地图 12）；`pushes::champ_select::the_rift_card_grows_with_the_draft`（排位选人五步——预选 / 禁用 / 还没轮到 / 轮到你 / 锁定——各有哪几块、可以禁的 [克制 1、劫，强度 112、7、134]、阵容“全是物理”、英雄池 [辛德拉、亚索、乐芙兰]、对线猜劫；OP.GG 关着没有对线那块；各块的数据先记进 memo，不连网、不连客户端）、`import_needs_the_control_switch`（关着回 control_off、不连客户端）；`client::loadout` 5 个（往哪一页写：新建 / 改我们的 / 满了换当前页 / 当前页是默认页换第一页能改的 / 只有默认页没地方写 / 老客户端数页数；请求体；出装方案合并；闪现位置；**假客户端走真 HTTP**：满了时的请求顺序和请求体、新建时 POST 带 current 再设当前页、客户端拒了说原话、数据不齐不发请求）；`stats::aramhex::core_builds_and_spells_for_mayhem`、`stats::opgg::lane_matchup_facts`；`tools` 清单测试加了 `lol.import`。clippy（`--all-targets -D warnings`）干净；rustfmt 只跑了改过的文件。
- 前端：`pnpm -C apps/desktop check`（含 LoL App）0 错误 0 警告。Playwright（一个 worker）新的 `plugins/lol/e2e/lol-champ-select.spec.ts` 4 个通过：峡谷卡片一步一步长（预选：可以禁的 5 个、阵容“全是物理伤害 / 没有前排”、我的英雄 9 个符文 4 组装备；轮到你：英雄池 3 行、对面那一行阵容；锁定：对线、来源 OP.GG；一直一张卡片、App 没重新加载；结束：按钮换成一句话），360 px 不横着溢出；“操作客户端”关着点按钮不调工具；开着点一下调 `lol.import`、列出三行结果；海斗卡片多了 aramhex-data 的我的英雄、没有符文。`lol-pushes.spec.ts` 4 个照旧通过，`terminology.spec.ts` 4 个通过。截图 `target/plugin-shots/feed-and-orb-b/champ-select-*.png`。
- 真机（2026-11-27，`scripts/manual-e2e/lol_champ_select_real.py`：隔离的 KOTONE_HOME、`--background`、debug 版拷到 `target/lol-champ-select-real/bin`、lol_tools_real.py 的假 HTTPS 客户端加了写接口、不调模型、不发任何系统按键、悬浮窗和推送小窗关着；腾讯 101、aramhex-data、一次 OP.GG 对线指南、CommunityDragon 的英雄资料是真网络）：19/19 通过。排位：预选卡片里可以禁的是 101 的真数据（亚索中路：玛尔扎哈 42.0%、奥瑞利安索尔、弗拉基米尔，再补 菲兹 T0、彗 T1），阵容我方 5 人；“操作客户端”关着时 `lol.import` 回 control_off、假客户端一个写请求都没收到；轮到我选时同一张卡片变成英雄池（阿狸、乐芙兰、崔斯特、辛德拉——101 的对位表每个英雄只有好打 / 难打各 5 个，这次都没碰上对面锁的劫、盖伦、德莱厄斯，所以都写“没有对位数据”），对面 3 人那一行；锁定以后对线是 OP.GG 的真数据（对手猜的是劫：对线占优、单杀占优都是劫，均势）；打开开关以后卡片跟着变，一键导入写了 `PUT /lol-perks/v1/pages/51`（换掉“上单”，9 个符文和卡片上的一样）、`PUT /lol-perks/v1/currentpage`、`PUT /lol-item-sets/v1/item-sets/<id>/sets`（我们的在前、5 组、玩家自己的那套还在）、`PATCH …/my-selection`（点燃 + 闪现，闪现还在 F）；开始游戏卡片标上结束了，给她一句“选英雄结束了（峡谷）：你用亚索（中路，强度 T4），队友 亚托克斯、李青、金克丝、锤石。”。海斗：新的一张卡片，我的英雄是 aramhex-data 的真数据（艾瑞莉娅：弹跳蜜蜂 +8.8 等 5 个海克斯、两套核心三件、召唤师技能），一键导入只写了出装方案（地图 12、`kotone-39-mayhem`）和召唤师技能，没碰符文页。

**还没测**：

- 真的英雄联盟客户端里的一键导入（要玩家点）：`/lol-item-sets` 接口国服现在还认不认（LeagueAkari 改成写游戏目录的文件了，原因没写；我们照 B 轮的要求只走客户端接口，不认时卡片会写客户端回的原话）、`POST /lol-perks/v1/pages` 带全的请求体国服认不认、符文页满了换当前页玩家觉得行不行、海斗里能不能换召唤师技能。
- 真的排位选人里各步的时机（`timer.phase` 的值、预选和禁用的先后、对面锁英雄时能看到的东西）只按 LCU 的类型和 LeagueAkari 的做法写，没在真客户端里录过一次排位选人。
- 101 的对位表只有好打 / 难打各 5 个，英雄池对上对面已锁的常常没有数；要更全的数得换数据源（OP.GG 的 `lol_get_champion_analysis` 也只有部分克制），这一轮没做。
- 卡片的来源标签这一轮只写出处、不能点着换（峡谷的“我的英雄”固定腾讯 101，玩家在英雄页把峡谷换成 OP.GG 的，选人卡片不跟）；OP.GG 的英雄分析回的是文字，要做成能导入的结构还要再解析。
- 悬浮窗小窗里的样子（W 那一轮在改悬浮窗）；小窗里要往下滚才看得到一键导入时，游戏里的滚轮还没真机验证（ADR 043 D6）。
- `docs/design/lol-app.md` 还没写选人卡片的这几块（主会话整合时补，或者下一轮）。

### 实施记录（对局中：三选一推卡片、识别只看名字那一横条；局内提醒；“等级”“装备”两类事件，2026-11-26 晚，B 轮）

ADR 043 D7 的 2026-11-26 晚修订（[rc.10 试玩反馈](../research/2026-11-26-rc10-playtest-feedback.md)第 3 条）和 B 轮第 2 条（讨论记录第二节推送清单“对局中”那几行、第六节的数据事实）。

**玩家看到的**：

- **海克斯三选一是一张卡片**：海斗对局里三选一一出来，聊天里（悬浮窗里就是聊天小窗）来一张“英雄联盟 推送 · 海克斯三选一”的卡片，和开局战绩一样：三张从左到右、和游戏里的位置一样排开，每张是图标、名字、评级小牌（S–D）、判断（首选 绿 / 可选 蓝 / 可刷新 琥珀 / 没数据 灰；认不出的那张写“没认出”），样本少的那张下面写“样本少”，首选那张用墨线圈出来；三张都一般时下面一行“三张都一般，可以刷新”；来源标签是 aramhex-data（带数据的版本）。卡片上不写胜率数字。刷新了哪张，同一张卡片原地换；选完、卡片从屏幕上没了 4 秒，卡片标上“选好了”（聊天里留着）。她照旧说“中间那个”（首选换了位置才再说）。原来悬浮窗里的“一行三格”没有了。
- **队友在聊天框里打海克斯名不再触发**：识别只截三张卡片名字所在的那一横条（游戏窗口高度的 34%–48%，左下角的聊天框在 55% 以下），而且要三个名字在同一行、分别在这一横条的左 / 中 / 右三段里（或者认出两个、剩下那一段同一行有没认出的字）才算出现了三选一。
- **对局中的提醒**（插件页「设置 → 推送」里各一个开关，都默认开）：
  - **对面到 6 级**：对面全图 / 远距离大招的英雄（卡牌、梦魇、潘森、慎、死歌、加里奥、寒冰、金克丝、EZ、德莱文、赛娜、星妈、船长、炸弹人、泽拉斯、烬、女警、岩雀、瑞兹、挖掘机、奎因、塞恩、斯莫德）到 6 级、峡谷里对线对手到 6 级：她说“对面寒冰 6 级了”（对线对手没外号时按分路叫：“对面中单 6 级了”），悬浮窗一行“对面寒冰 | 6 级了，有大招”。同一轮几个人一起到 6 级并成一句。
  - **对面的关键装备**：对面出了复活甲、中娅、水银（饰带、弯刀、密银黎明）、女妖：“对面维克托出了中娅”；你玩回血英雄（索拉卡、薇恩……25 个，一张表）时，对面出了重伤装备：“对面岩雀出了重伤”（每个人只说一次，升级成荆棘之甲不再说）。
  - **技能转好了**：你或她用 `lol.spell` 记下的召唤师技能转好的那一刻：峡谷里“中单闪现好了”，大乱斗、海斗里“亚索闪现好了”。
  - **资源快刷了**（只在峡谷）：小龙、远古龙、虚空巢虫、峡谷先锋、大龙刷新前 30 秒：“30 秒后小龙刷新”。
  - **对面阵亡**：对面同时死到 3 个的那一刻：“对面死了 3 个，最快 18 秒复活”（45 秒内只说一次）。
  - **你阵亡时**：一张“对面的装备”卡片：对面 5 个人的等级、KDA、身上的装备、装备总价（8.2k），按装备总价从高到低排；上次你阵亡以后新出的装备描一圈；下面一行你自己的等级、KDA、装备总价。一局一张，每次阵亡同一张原地更新，第二次起悬浮窗再弹一次。窄到小窗时装备换到名字下面一行。
- **自动化能用的两类新事件**：“升级了”（`lol.level`）、“出了一件成装”（`lol.item`）。

**怎么做的**：

- **三选一**（`plugins/lol/src/pushes/augment_pick.rs`）：`CARD_REGION`（中间一大块）换成 `NAME_STRIP = [0.12, 0.34, 0.76, 0.14]`。位置照 AramHex（MIT，`main` 的 `ffb784a`，`lolhex/vision/detector.py`）：标题框 `DEFAULT_BOXES` 在窗口高度的 38.7%–43.4%（国服 4K 实机校准，游戏界面按高度缩放、水平居中），判定“≥2 个能匹配到海克斯字典的文字，在同一水平线上、左右排开”（`_pick_row`，同一行是纵向差 0.03 个窗口高度以内）。这里没有它的整块扫描 + 本机校准，只截一条固定的横条：上下各留约 4.5 个点；横向 12%–88% 和它的 `BAND` 一样，16:9、4:3、21:9 的三个名字中点都各在一段里（测试里按它的换算算过）。新的 `offer_of`：认出来的名字里挑一行（纵向中点差 0.02 以内，比 AramHex 紧一点：卡片说明的第一行离标题约 0.03），按横向中点分进三段、每段留一个；三段都有，或两段有、剩下那段同一行有至少 2 个字没认出才算。推送从“一行 + 语音”改成“卡片 + 语音”：页面 `ui://kotone/lol/augment-pick`（新的一页，`app/src/pages/AugmentPick.svelte`），数据 `View::card`（三格 `{id, name, rarity, icon, grade, verdict, few}` 或 `{unread: true}`、`best`、`weak`、`source: aramhex`、`patch`、`ended`；不带胜率和分数）。一次三选一一个 key（照旧 `augment:<对局 id>:<毫秒>`），变了同一个 key 再推；结束时（卡片没了 4 秒、这局结束、推送关了）把最后推出去的那份标 `ended: true` 再推一次（说法不变，宿主不会再说），再 `push_done`。清单：`forms = ["card", "voice", "line"]`（一行只剩“海克斯评分用不了 · 原因”那一条，每局最多一次）、去掉 `until_done`（卡片在聊天小窗里，没有要一直挂在气泡里的一行了）；`what` 跟着改。`Frame::Cards` 里的 `View` 装进 `Box`（clippy：变体太大）。
- **局内提醒**（新的 `plugins/lol/src/pushes/alerts.rs`，测试在 `alerts/tests.rs`）：主循环每一轮（2 秒）在对局里（局内接口有游戏时间）调 `Alerts::step(对局 id, 这一份局内数据, 客户端资料, 记下的技能)`，纯函数，和上一份比：
  - 认人用 `LivePlayer::key`（新）：有名字时是 `riotId`，隐藏了身份的用 `队伍/summonerName`（录的样子是英雄称号）。不按英雄认：2026-11-26 那把录的里佛耶戈附身时 `rawChampionName` 换成了被附身的英雄、身上的装备也换成那个人的；也不按顺序认：`allPlayers` 的顺序变过。`LivePlayer` 加了 `position`、`summoner` 两个字段。
  - 数据表（都在 `alerts.rs` 开头）：`LONG_ULTS`（英文 id → 外号）、`KEY_ITEMS`（装备 id 后四位 → 叫法：3026、3157、3140、3139、6035、3102）、`GRIEVOUS`（3123、3033、6609、3916、3165、3011、3076、3075）、`HEALERS`、分路叫法、`RIFT_RULES`。海斗、大乱斗的装备 id 前面多两位（`223157` 是海斗的中娅沙漏），一律取后四位查表（`events::base_item`）。
  - 峡谷资源（`next_spawns`，只在 `gameMode = CLASSIC`）：照局内接口的事件（拿龙、拿大龙；拿龙的人是哪边看 `KillerName`）和 `RIFT_RULES` 算下一次刷新：小龙 5:00、被拿 +5:00；一边到 4 条（龙魂）以后只刷远古龙，龙魂 +6:00、远古龙被拿 +6:00；虚空巢虫 8:00（25.09 起一波）；峡谷先锋 16:00（25.S1.1）；大龙 20:00（26.1：“Baron Nashor: 25:00 ⇒ 20:00”，同时去掉了阿塔坎，“Epic Monster spawn times are unchanged unless noted above”），被拿 +6:00。2026-11-26 查的拳头补丁说明。上一份和这一份之间跨过“刷新前 30 秒”就说一次。
  - 优先级（清单）：对面阵亡 high；到 6 级、技能转好、资源快刷 normal；关键装备 low；阵亡卡片 normal。同一轮几条一起来时宿主照 ADR 043 D3 只说最要紧的一句（`push_voice`），别的只显示一行、记进聊天。插件这边只管同一件事不重复：同一个人到 6 级只说一次、同一个人同一件关键装备 / 重伤只说一次、同一轮的并成一条（最多写三个）、“对面阵亡”要这一刻刚到 3 个而且离上次 45 秒以上。说法里太长（超过 20 个字）的那个说法不用（宿主会截断）；一格超过 12 个字时“对面”两个字不写。
  - 阵亡卡片：页面 `ui://kotone/lol/enemy-build`（`app/src/pages/EnemyBuild.svelte`），key `death:<对局 id>`，第二次起带 `alert`；装备总价用客户端 `items.json` 的 `priceTotal`（`GameData` 加了 `item_prices`），没读到资料时不显示总价、按击杀排；“新出的”是和上次阵亡时对面身上的装备比。不放任何人的名字，不评价谁（ADR 043 D2）。
- **等级、装备事件**（`plugins/lol/src/client/events.rs`：`Roster`、`changes`、`roster_events`）：局内接口的事件列表里没有，比前后两份局内数据得出。量的取舍（写进了清单的注释和模块说明）：**只报对面和自己**；等级——自己每升一级都报，对面只报到 6、11、16 级（大招升级的那几级）；装备——只报成装（`items.json` 里 `to` 是空的、`priceTotal` 2000 以上，`GameData` 加了 `completed_items`；没读到资料时只认关键装备那张表），同一个人同一件这局只报一次（佛耶戈附身完装备变回来不算又出了一件）。录的那把海斗（18 分钟，每 20 秒一份）：自己 16 条等级、对面 15 条、装备 32 条（6 个人，海斗出装快）。数据字段：`lol.level` 是 `team`、`champion`、`level`、`me`；`lol.item` 是 `team`、`champion`、`itemId`、`item`、`me`（不放玩家名）。`[[events]]` 登记了，界面名“升级了”“出了一件成装”。
- 清单 `[[pushes]]` 加了六条（`enemy-ult`、`enemy-item`、`spell-ready`、`objective`、`enemies-down`、`my-death`，都要“读客户端数据”），`[[pages]]` 加了两页；文案在 `messages/{zh-CN,en}.json`（`plugin_lol_push_*`、`plugin_lol_event_level / item`、`plugin_lol_pick_*`、`plugin_lol_death_*`）；`app.rs` 的 `APP_PAGES`、App 的 `App.svelte` / `TopBar.svelte`、宿主的 `plugins/lol/ui/{types,index}.ts`、预览的 `pluginsMock.ts`（页面、推送、事件，优先级类型加了 `low`）跟着加。设置说明（`settings_guide.rs` 的 `plugins.*`）里三选一改成卡片，加上六条提醒。`THIRD_PARTY_NOTICES.md` 的 aramhex 一条加上识别位置的出处。`plugins/lol/README.md` 那一行跟着改。

**测试**：

- Rust（2026-11-27，WeGame 开着，低于正常优先级 `-j 2`）：`kotone-plugin-lol` 176 个全过。新的和改的：
  - `pushes::augment_pick` 10 个：三张从左到右认出、判断；**聊天里的名字不算**（聊天框里一行一个名字都在左边一段、一行里写了三个名字、左中右各一个但不在同一行、只认出两个而第三段没字——都不算；真的三选一加一行聊天照样认得出）；横条和三段的几何（16:9、21:9、4:3 的三个名字中点各在一段）；卡片的数据（三格、`unread`、来源 aramhex、版本，不带胜率）；三张都不好；**一次三选一一帧一帧**（先看两帧聊天里的名字不推 → 出现两帧一样才推卡片 + “右边那个” → 刷新动画那帧中间没字不推 → 刷新了中间同一个 key、不说 → 首选换到左边才说 → 游戏切走那帧不改 → 卡片没了 4 秒收起 → 新的一次新的 key → 这局结束收起时带着最后的样子）；结束时同一个 key、同样说法、`ended: true`、`push_done`；清单（卡片 + 语音 + 一行、没有 `until_done`）。
  - `pushes::alerts` 8 个：**录的一整把海斗**（`testdata/mayhem-20261126.json`：`target/lol-probe/20261126-mayhem-midgame` 的 56 份压成 44 KB，名字换成编的，装备总价从客户端 `items.json` 抄的）——等级 / 装备事件（自己 1→18 每次升级一条（20 秒一份，共 16 条）、对面 5 人 × 3 次、只报成装、同一个人同一件一次、没有队友的、不带名字、字段都登记过；没有装备资料时只认关键装备）；提醒（只有岩雀到 6 级说一次、维克托出中娅说一次、没有峡谷资源、对面阵亡说了好几次且间隔 ≥ 45 秒、第一句“对面死了 3 个，最快 3 秒复活”、阵亡卡片一局一个 key、第一次不带 alert 之后都带、按装备总价排、第二次标出新出的）；我换成索拉卡时的重伤（湮灭宝珠、棘刺背心各说一次，荆棘之甲不再说）；**拼的峡谷**：资源每 2 秒看一份，从 0 到 30 分钟说了 9 次、时刻都对（5:00 小龙、8:00 巢虫、被拿后的小龙、16:00 先锋、20:00 大龙、被拿后的大龙、龙魂后的远古龙）；到 6 级（慎按外号、对线的亚索按分路、卢锡安不说、再升级不再说、大乱斗里没有对线对手）；技能转好只在跨过那一刻说一次、峡谷按分路、大乱斗按英雄；**一波团战同一轮**（对面死 3 个 + 两个人到 6 级 + 三个人出中娅）：每种一条、清单里“对面阵亡”最要紧（宿主只说它）、下一轮不重复、换了一局从头来；清单六条的样子 / 优先级 / 要的开关 / 默认开。
  - `client::events` 加 1 个：按人认（倒过来的顺序、佛耶戈附身换了英雄还是同一个人、隐藏了身份的人按队伍 + 称号）、上一份没有的人不算、海斗装备 id、等级事件只有自己和对面、同一件只报一次。
  - `app` 的两个页面测试跟着加了两页；`kotone-tauri` 的 `plugin_apps` 3 个（三处的 App 页面一致）过。clippy（`kotone-plugin-lol --all-targets -D warnings`）干净，rustfmt 过。
- Playwright（WeGame 开着，1 个 worker）：新的 `plugins/lol/e2e/lol-ingame.spec.ts` 4 个全过：三选一卡片（三列从左到右、判断、评级、样本少、首选墨线圈、来源 aramhex-data、没有百分数）；刷新同一个 key 原地重画、App 没重新加载、一直一张，选完标“选好了”；窄到 360 px 三列还在一排、不溢出；阵亡卡片（5 行、KDA、9.8k、阵亡中、我那一行、第二次阵亡原地更新、新出的三件描一圈、窄的时候装备换到下一行）。英雄联盟别的 e2e 38/39（没过的那个 `lol-plugin.spec` “设置位置打开插件页”和这次改的无关：别的子代理改文件、开发服务器热重载以后测试里 `import('/src/lib/stores/navigation.ts')` 拿到的不是页面用的那一份）。`terminology.spec` 4 个里 3 个过，没过的那条是 `plugin_lol_cs_spells_rate` 没用上（选英雄那一部分的，不是这次的）。`pnpm -C apps/desktop check` 0 / 0。
- 截图：`target/plugin-shots/round-b/chat-augment-pick-card.png`、`chat-augment-pick-narrow.png`、`chat-enemy-build-card.png`、`chat-enemy-build-narrow.png`。
- 真机（2026-11-27 0:28，隔离实例 `target/l2-game-events-real/home`、自己拷的一份 `kotone.exe`、假客户端、`--background`、不发任何系统按键，悬浮窗关着）：`game_events_real.py` 的时间线加了两件事——30 秒时对面寒冰、贾克斯从 5 级到 6 级，34 秒时贾克斯出了守护天使。新的 G7、G8 都过：聊天里有“对面到 6 级”那条（说了“对面寒冰 6 级了”，一行“对面寒冰 | 6 级了，有大招”），有“对面的关键装备”那条（“对面贾克斯出了复活甲”）；`game.state` 最近的事里有两条 `lol.level`（艾希、贾克斯 6 级）和一条 `lol.item`（贾克斯 守护天使 3026），拿龙的 G4 照样看得到。整个脚本 16/18：没过的两条是选英雄卡片重随以后原地更新（G6，选英雄那一部分正在改）和紧接着的“加载”阶段（G2：前一条等了 10 秒，再看时已经是对局中了），和这次改的无关。

**还没测**：

- 真游戏里的三选一（要玩家打一把海斗）：名字那一横条在玩家的分辨率、窗口比例、界面缩放下盖不盖得住三张卡片的名字（AramHex 的位置是 4K 实机校准的，它自己还有整块扫描兜底；我们没有——认不出时会一直没有卡片，日志里看得到），队友在聊天里打海克斯名还会不会触发，卡片在悬浮窗小窗里的样子。
- 真游戏里的提醒：到 6 级、关键装备、对面阵亡、阵亡卡片在一局里的频率是否吵（海斗后期对面几乎一直有 3 个死着，45 秒一次）；峡谷资源的时刻（规则是照补丁说明写的，没有真开一局峡谷对过）；记下的闪现转好的那一刻。
- 关键英雄表、回血英雄表、重伤装备表是照常识列的，玩家觉得缺了谁或多了谁就改表。

### 实施记录（B 轮第 3 条：这局总结加对面的海克斯、房间里的“今天的状态”，2026-11-27）

讨论记录第二节推送清单的“排队 / 房间”（“今天的状态（几连胜 / 连败、哪个英雄一直输）”）和“结算”（“这局总结；再加一行对面每个人的海克斯（结算详情里有）”）两行；第六节（局内接口没有海克斯，结算详情里有）；第七节（只说事实）。

**玩家看到的**：

- **这局总结多了一块“海克斯”**：海斗、斗魂竞技场打完，聊天里（和悬浮窗的小窗里）那张“这局总结”最下面先列出对面每个人的海克斯：一人一行，左边英雄头像，右边按选的顺序一格一个（图标 + 名字，边框是稀有度；名字太长省略，完整的在鼠标提示里）。右上角“对面 / 我方”能换成我方的（我那一行淡蓝底）。小窗（约 360 px）一行两格，聊天里一行四格。峡谷、大乱斗这些没有海克斯的模式没有这一块。点开全屏的“单局详情”，两队下面各有一块同样的“海克斯”（原来装备下面那排只有图标的小格子去掉了，不重复）。
- **进房间或排队时推一张“今天的状态”**：“今天 5 局 · 2 胜 3 负 · 胜率 40% · 现在 2 连败”（连胜绿、连败红；只有 1 局时写“上一局赢了 / 输了”），下面一排今天每一局的英雄小头像（新的在前，和“这局”的近况一样：赢的亮、输的淡），再下面“今天输了 2 局以上”：“亚索 3 局 1 胜 2 负”，打了不止一种模式时再写一行“海克斯大乱斗 4 局 1 胜 · 单双排 1 局 1 胜”。卡片底下是“客户端”来源标签。一天一张：打完一局回到房间，同一张原地更新、悬浮窗再弹一次；今天还没打过不推。插件页「设置 → 推送」多了一条“今天的状态”（默认开）。
- 她不会因为这张卡片多说话：它不给她留话（见下面“定的”）。

**怎么做的**：

- **海克斯**：单局详情（`/lol-match-history/v1/games/<id>`）每个人的 `stats.playerAugment1`–`6` 早就进了页面数据（`pages::game_page_for` 的 `augments`：`{id, name, rarity, icon}`，名字和图标来自客户端的 `cherry-augments.json`，海斗的海克斯也在里面）；这次只改页面：新组件 `app/src/pages/game/AugmentRows.svelte`（一人一行、按容器宽度一行两格或四格，每格最多 12em），`Game.svelte` 的卡片最后加一块（有海克斯的队，对面在前；`.seg` 换队），全屏每队下面一块。后端没改数据格式。
- **今天的状态**：
  - 新模块 `watcher/auto_cards/today.rs`（纯函数）：`today_games`（按开局时间 `gameCreation` 换成本地日期，0 点分界；日期用 kotone-core 已有的 `agent::reminder_date`——`chrono::Local`——取逗号前面，插件不加新依赖、`Cargo.lock` 不动）、`streak`（只数今天的，从最近一局往前数到第一局不一样的；跨过 0 点不接着数）、`lost_often`（今天输了 2 局以上的英雄，`LOST_AT_LEAST = 2`，输得多的在前，一样多按局数、再按最近玩的）、`modes`、`today_page`（页面数据 `{date, games, wins, losses, streak, capped, limit, lostAtLeast, lostOften, modes, recent}`；里面没有每次都变的时间戳，没打新的一局时内容一样，宿主回 `Unchanged`、什么都不做）。算哪些局和战绩页一样（`history::parse_match_list`：只算匹配到的对局，自定义、训练、重开局不算）。
  - 推送（`watcher/auto_cards.rs`）：`auto_cards` 每一轮看客户端阶段，从别的阶段进 `Lobby` / `Matchmaking` / `ReadyCheck`（打完一局回来、Kotone 刚连上客户端时就在房间里）时 `today_card`：推送关着（`Host::push_enabled`）就连战绩都不读；读我自己最近 20 局（`/lol-match-history/v1/products/lol/{我}/matches`）；刚打完一局（`CardState.ended`，这局总结排上时记下，自定义、观战不记）时等它出现在战绩列表里（每 5 秒、最多 60 秒，到了照样用读到的发）；客户端的资料（英雄名）刚连上时还在读，最多等 10 秒。`today_push`：trigger `today`、key `today:<本地日期>`、卡片 `ui://kotone/lol/today`、`alert: true`（更新时悬浮窗再弹一次：它上面已经有这一局的开局战绩和这局总结两张了）、没有 `note`。读到的 20 局全是今天的，卡片底下写“只看了最近 20 局”。
  - 清单：`[[pages]] ui://kotone/lol/today`（要“读客户端数据”）、`[[pushes]] today`（卡片、默认开、要 `client`）；`app.rs` 的 `APP_PAGES`、宿主的 `ui/index.ts`、`ui/types.ts` 的 `PAGE_TODAY`、`pluginsMock.ts`（页面、推送、`APP_PAGES`）跟着加；`summary` 那条的“做什么”加了“海斗、斗魂竞技场还列出对面每个人的海克斯”。App：新页 `pages/Today.svelte`（只在推送里出现，和选英雄卡片一样：没数据时一句“进房间或排队时……”；全屏时多一个“看我的战绩”）、`types.ts` 的 `TodayData`、`App.svelte` 的路由。设置说明（`settings_guide.rs` 的 `plugins.*`）加上 `pushes.today`，`summary` 那句加上海克斯。
- **定的**（ADR 留给实现的）：
  - **不给她留话**。这张卡片一天里一直在变，第一次推时留一句，下一局就过时了，她会拿旧数字说话；她要今天的战绩时自己查（`lol.player`）。
  - 今天还没打过不推（“今天 0 局”没有信息）。
  - “哪个英雄一直输”的固定规则：今天输了 2 局以上（不看胜率、不评价）。
  - 连胜连败只数今天的；一天按开局时间、本地 0 点分界。
  - 更新时 `alert`：回到房间时让悬浮窗再弹一次这张卡片（内容没变宿主不动，不会每进一次房间弹一次）。
  - 卡片先显示对面的海克斯，我方的点一下（讨论记录只说了对面）。

**测试**：

- Rust（2026-11-27，WeGame 开着，低于正常优先级 `-j 2`）：`kotone-plugin-lol` 我的 10 个新测试都过：`today::tests` 6 个（本地 0 点分界——23:50 开的那一局算昨天、读不到开局时间的不算、正式的日期格式；连胜连败只数今天的；输了 2 局以上的英雄和排序；页面数据——同样的战绩一模一样、多一局才变；战绩列表里的局 id 含自定义、重开局；房间和排队的阶段）、`auto_cards::tests::the_today_push_is_one_card_per_day_updated_in_place`（key、卡片、不留话、`alert`、过宿主的检查 `Push::check`、同样的战绩推出来一模一样、多一局同一个 key 内容变、第二天没打不推、20 局全是今天的标出来）、`the_card_pushes_are_declared_in_the_manifest`（三条都在清单里、页面要 `client`）、`pages::tests::mayhem_details_carry_every_players_augments_in_pick_order`（字段和海克斯 id 照 `target/lol-probe/20261123-210745` 录的那一把海斗拼的，名字换成编的：按选的顺序、0 不列、名字 / 稀有度 / 图标、资料里没有的照样列 id、峡谷是空的）、`the_recorded_mayhem_detail_has_four_named_augments_per_player`（本机有录的那一把时直接读它：10 个人每人 4 个、名字都查得到；CI 上没有这份录的数据就跳过）。`tests::messages_match_the_manifest`、`app::tests::every_page_but_settings_is_an_app` 过。`kotone-tauri` 的 `plugin_apps::tests` 3 个过（App 页面和前端、模拟对得上）；`kotone-core` 的 `settings_guide` 11 个过。clippy 我改的文件没有警告；rustfmt 只跑了我改的文件。
- Playwright（一个 worker）：新的 `plugins/lol/e2e/lol-postgame.spec.ts` 5 个全过：这局总结卡片的“海克斯”先是对面 5 行、每行 4 格（名字、稀有度）、换成我方（我那一行淡蓝底）、360 px 不溢出且一行两格；峡谷（没有海克斯）没有这一块；今天的状态（“今天 5 局 · 2 胜 3 负”“胜率 40%”“现在 2 连败”、5 个小头像、亚索“3 局 1 胜 2 负”、模式那一行、“客户端”标签、没有“突破口”“风险”），同一个 key 再推一次（又赢了一局）原地变成“今天 6 局 · 3 胜 3 负”“上一局赢了”、App 没重新加载、还是一张；360 / 300 px 不溢出；全屏单局详情两队下面各一块“海克斯”。`e2e/terminology.spec.ts` 4 个过。截图在 `target/plugin-shots/feed-and-orb-b/`：`summary-card-enemy-augments.png`、`summary-card-augments-360.png`、`today-card.png`、`today-card-narrow.png`、`game-page-augments.png`。
- 真机（2026-11-27 0:15，新脚本 `scripts/manual-e2e/lol_postgame_real.py`：隔离的 KOTONE_HOME、`--background`、假客户端放 `20261123-210745` 那一把并按时间线走 房间 → 选英雄 → 对局 → 结算 → 房间，我的战绩改成今天 5 局（其中 1 局自定义）、两局输的换成亚索；打完的这一局结算后 18 秒才出现在战绩里；没有 Key、不调模型、不发任何系统按键、悬浮窗不弹）：6/6 过。P1 一进房间就有“今天的状态”（`today:2026-11-27`，4 局 2 胜 2 负、2 连胜、亚索输 2 局，自定义那局不算），没有给她的话；P2 结算后“这局总结”里 10 个人每人 4 个有名字的海克斯（对面：巨像的勇气、坦克引擎、扇巴掌、神圣干预 | 秘术冲拳、小蜜蜂！！！、台风、鼓声暴起 低音轰鸣 | ……）；P3 回到房间后等到这一局进了战绩（开局后 54 秒）才原地更新成 5 局、3 连胜，还是一张、同一个 key、带 `updatedAtMs`。日志：`push: lol 推送「今天的状态」` → `推送「这局总结」` → `更新了「今天的状态」`。

**还没测**：真的英雄联盟客户端（战绩列表在结算后多久出现这一局、国服的战绩接口一次最多给几局）；斗魂竞技场（16 人、8 队）的单局详情按 `teamId` 分队时“对面”是谁——这次没录到斗魂竞技场的详情；悬浮窗收到 `alert` 的更新时真的再弹一次（W 在做悬浮窗，这次真机脚本把弹出关了）；跨过 0 点还在房间里时那张卡片（下一次进房间才换成新的一天）。

### 实施记录（B 轮：同传、游戏语音、网易云音乐的推送，2026-11-26）

ADR 043 D11（B 轮第 4 条）。讨论记录第九节“其它插件（我的建议）”。

**玩家看到的**：

- **同传**：每翻好一句，悬浮窗的气泡里显示这一句译文（一行，最多 40 个字）；聊天里一局一张卡片“同传 推送 · 这局的同传”，上面一行“2 句 · 译成中文”，下面一句一行：译文、时间，下面一行小字“原话（TslGame.exe）：Enemy behind the red house”，新的在下面，同一张原地变长。卡片里留最近 30 句（多了写“这里是最近 30 句”，更早的在后台那段对话里）。同传页面“怎么显示”默认变成“气泡”（悬浮窗里一行），选“弹幕”时屏幕上照旧是弹幕、悬浮窗不显示这一行，卡片照样记；选项下面一句说明。插件页「设置 → 推送」一条“这局的同传”（卡片 · 一行，默认开）。自动化多了一件事“同传翻好一句”（译文、原话、在听的程序）。
- **游戏语音**：她用游戏语音替你对队友说了一句，聊天里来一张“游戏语音 推送 · 她对队友说了”：“她在游戏语音里对队友说了：”，下面一句一行带时间；上一句说完后 2 分钟内的并在同一张（满 20 句另起），和语音输入的“你向「队伍」发了”对称。插件页「设置 → 推送」一条（卡片，默认开；插件本身照旧默认关）。自动化多了“她对队友说了一句”。
- **网易云音乐**：以前登录过、网易云说登录过期了（启动时查账号查不到，或者用着用着某个接口回“要登录”），聊天里来一张“网易云音乐 推送 · 登录过期”：“网易云音乐的登录过期了：重新登录以后才能放整首歌、每日推荐和你的歌单。”和一个“登录网易云音乐”按钮（和音乐页、聊天页那个是同一个：打开网易云官方的登录页扫码）；在等扫码时写“在弹出的网易云音乐窗口里扫码登录”、按钮换成“重新打开窗口”。登好了这张卡片原地变成“✓ 已登录网易云音乐：昵称”。同一次过期只有一张（Kotone 重启了也是这一张）。从没登录过的照旧：她要放歌时聊天页输入框上面出“登录网易云音乐”那一条。她要放歌时工具告诉她“登录过期了，请玩家点聊天里‘登录过期’卡片上的按钮”。

**怎么做的**：

- **同传**（新的 `plugins/interpreter/src/pushes.rs`）：翻译是后台的模型做的，插件看不到，所以加了一个给后台的工具 `interpreter.show {text 译文, heard? 原话, app? 在听的程序}`（不马上返回的工具：主会话没有它，只给后台和外部 Agent；副作用 `speaks-or-shows-text`，默认允许）。清单 `[agent]` 的规则改成“每翻好一句用 interpreter.show 显示；玩家要弹幕时同一步里再用 danmaku；interpreter.show 回 shown=false 时改用 danmaku”。工具里：算推送（纯函数 `Desk::plan`）、`Host::push`、有在玩的游戏就报 `interpreter.line`，回 `{shown}`（推送关着 → `shown: false` + 让后台改用弹幕）。
  - 哪些句子算“这一局”（主会话定的）：游戏中心知道现在是哪一局（新的 `Host::playing_game` 给的 `match_id`）时按局，同一局隔多久都是一张，下一局另起；分不出局（只看进程的游戏、在听 Discord 而没在游戏里）时，上一句翻好后 10 分钟内、没有在页面上重新点开始 / 停止的并进同一张。key 是 `lines:<这一张第一句的时间>`（不用局号：Kotone 重启以后游戏中心的局号从头来，会和以前那张撞上）。页面的“开始”“停止”把正在加的那张标成关了（同一局的照旧并在一起）。插件不知道她用 `agent.stop` 停了同传：10 分钟内又开，接着加在那张里。
  - 卡片数据 `{target, startedAtMs, total, lines: [{text, heard?, app?, atMs}]}`，最多留最近 30 句（`MAX_LINES`）。页面 `ui://kotone/interpreter/lines`（`ui/LinesCard.svelte`，聊天里的卡片和右侧栏同一个）。
  - 推送的一行：选了气泡（`display` 没存过或不是 `danmaku`）时才带，一格、就是译文；选了弹幕时只推卡片。**默认改成气泡**（主会话定的：讨论记录说“翻好一句 → 悬浮窗显示一行”，默认弹幕的话大多数人看不到这一行）：`Prefs.bubble = display != "danmaku"`，存过 `danmaku` 的照旧弹幕，存过 `bubble` 的照旧气泡（只改没存过时的缺省，不改存的值）。页面、`interpreterMock`、点开始替玩家说的那句（“用气泡显示”）跟着改。
  - **一行只有一格时最多 40 个字**（`kotone-plugin-api` 的 `PushCell::ONE_CELL_MAX_CHARS`，`Push::shaped`）：原来一格一律 12 个字（三选一的“首选 / 可选 / 可刷新”），一句译文截到 12 个字就看不懂了。分了几格的照旧 12 个字。**这条改了 ADR D3 的“每格最多 12 个字”**，要在 D3 里补一句。悬浮窗那边（W 的 `routes/floating/Floating.svelte`）`.cell` 现在是 `white-space: nowrap`、`.line` 最宽 360px：只有一格的一行要能折行，交给悬浮窗那一轮。
  - **不给她留说明**（brief 让定的）：第一张也不带。同传是她自己交给后台的，要紧的话（队友在叫玩家、问玩家）后台照旧会告诉她；队友说的话是别人的话，不进她的主对话（ADR 021“陌生人的话只进同传这个工具受限的子会话”）。
  - 事件 `interpreter.line`（data：text、heard、app），`private = ["text", "heard"]`：她在 `game.state` 里只看到“听到一句”和程序，原话和译文只给自动化（别人的话不进她的主对话）。报到现在在玩的游戏上；游戏中心里没有在玩的游戏就不报。
- **`Host::playing_game`**（`kotone-plugin-api`，新的默认方法，默认 None；`Playing {game, match_id}`）：桌面端 `DesktopHost` 照游戏中心算：在跑的游戏里窗口在前台的那个；都不在前台时只有一个在跑才算（`plugins.rs` 的 `playing_of`，有测试）。同传和游戏语音报事件、同传按局分卡片用它（这两个插件自己不知道是哪个游戏）。
- **游戏语音**（新的 `plugins/game-voice/src/pushes.rs`）：`voice_chat::say` 在送进虚拟麦克风以后交给推送台（`SaidDesk::report`），推送和事件在后台线程里做（`spawn_blocking`，一次一条，说话那条路不等它）。合并规则是纯函数 `plan`：上一句说完后 2 分钟内（`MERGE_WINDOW_MS`，刚好 2 分钟还算）、不满 20 句（`MAX_LINES`）的同一个 key（`said:<第一句的时间>`），否则另起；卡片里照说的时间排。卡片数据 `{lines: [{text, atMs}]}`，页面 `ui://kotone/game-voice/said`（`ui/SaidCard.svelte`）。不给她留说明（她自己说的，她的对话里本来就有这次 `voice_chat.say`）。事件 `game-voice.said`（data：text，不标 private：她自己说的话）。外部 Agent 调 `voice_chat.say` 也一样留一张。
- **网易云音乐**（新的 `plugins/music/src/login_push.rs`）：
  - 什么算过期（纯函数）：`expired_account(存着登录凭据, 查账号的结果)` = 存着 MUSIC_U、网易云说没有账号；`lost_login(存着登录凭据, 出的错)` = 存着、回了“要登录”（301）。查不到（网络、风控）不算；从没登录过的不算。
  - `Music::refresh_account`（启动时、打开插件时）和所有网易云接口出的错（原来的 `error_text` 换成 `Music::failed`）都过这一道；过期了 `login_expired`：当没登录（账号清空，`NeteaseProvider::expire` 忘掉缓存的账号和结果；凭据先留着，重新登录时换掉），推卡片，发状态事件。
  - 卡片的 key 记在插件目录的 `login-push.json`：同一次过期只有一张，Kotone 重启以后（凭据还是过期的）再推是同一张、内容一样宿主什么都不做；登好了（登录窗口那条路成功）推同一个 key 的 `{stage: "done", nickname}`、删掉记录。启动时还没有她的聊天、推不出去的，她下次要放歌（`need_login`）时补推。
  - 卡片 `ui://kotone/music/login`（`ui/MusicLoginPush.svelte`）：按钮经页面的桥调 `music.login_start`（不打开右侧栏）；看着音乐的状态写“在等扫码”；后端那次“已登录”没推出去时，现在的状态有账号也显示已登录。
  - 给她的工具结果：过期时 `need_login` 的 note 改成“登录过期了，请玩家点聊天里‘登录过期’卡片上的按钮……”；规则 `[agent]` 改成“照 note 请玩家点”。
- 三条推送都在各自的 `plugin.toml` `[[pushes]]`（名字、时机、做什么、样子，默认开、不要数据开关），文案在插件的 `messages/zh-CN.json` / `en.json`（约定的键 `plugin_<id>_push_<推送>_title|when|what`、`plugin_<id>_event_<名字>`，各插件的测试查和清单一致）；`pluginsMock.ts` 的三个清单跟着加了页面、推送、事件和 `interpreter.show`。设置说明（`settings_guide.rs` 的 `plugins.*`）加了三条推送的开关和位置（游戏语音、网易云音乐那两段后面，同传原来没写，补了一段：`settings.app / target / display`、`pushes.lines`），各插件的测试查。术语表：“同传”“登录网易云音乐”两行写上推送。
- 真机脚本：`mock_interpreter.py` 的后台收到一句时，插件给了 `interpreter.show` 就用它（否则照旧 danmaku）；`interpreter_e2e.py` 改查 `interpreter.show` 和主对话里那张“这局的同传”；新的 `interpreter_push_real.py`（真 DeepSeek：页面点开始 → 她交给后台 → 后台用 `interpreter.show` → 一张卡片、译文是中文）；`voice_chat_real.py` 加查“她对队友说了”那张卡片、没有给她的说明。

**测试**（2026-11-27 凌晨，WeGame 开着、玩家在睡觉；构建低于正常优先级 `-j 2`）：

- Rust：`kotone-plugin-interpreter` 9 个（新的 6 个：同一局一张卡片、隔多久都是一张、下一局另起、点开始 / 停止后同一局照旧一张、只留最近 30 句写着一共几句；分不出局时 10 分钟内一张（刚好 10 分钟还算）、超过 / 点了开始另起、进了一局另起、一局结束另起；选了气泡才带一行、一格就是译文、宿主截完一整句还在；有在玩的游戏才报 `interpreter.line`、字段对、名字在清单里；工具：没写程序时用正在听的那一个、同一局两次是同一个 key、推送关着回 `shown: false` 让后台改用弹幕、没有译文报错；清单：卡片 + 一行、默认开、页面在、事件 `private = [text, heard]`、设置说明里有开关）；`kotone-plugin-game-voice` 14 个（新的 4 个：2 分钟内一张（刚好 2 分钟还算）、超过 / 满 20 句另起、不给她说明；有在玩的游戏才报 `game-voice.said`；后台线程按先后推两次同一张、第二次带着两句、照说的时间排；清单和设置说明）；`kotone-plugin-music` 36 个（新的 3 个：什么算过期（存着凭据 + 网易云说没账号 / 回要登录；没登录过、网络、风控不算）；同一次过期一张、重启以后读回同一个 key、登好了原地变成已登录带昵称、之后不再推；清单和设置说明），另有一个真网络的 `tests/live.rs::a_stale_login_reads_as_expired`（`--ignored`）：存一个假的失效 MUSIC_U，网易云真的回“没有账号”（`Ok(None)`，84 毫秒，确实问了 music.163.com），判成过期；`kotone-plugin-api` 28 + 7 个（`pushes_are_shaped_by_the_manifest` 加了：一整句的一行 20 个字照样全、60 个字截到 40 个字带“…”，分格的照旧 12 个字）；`kotone-tauri` 的 `plugins::tests::the_playing_game_is_the_focused_one_or_the_only_one`（没有在跑的、只有一个、前台的那个、两个都不在前台 = 不知道）；`kotone-core` 设置说明 11 个、`kotone-plugin-voice-input` 20 个照常通过。clippy：四个插件 crate `--all-targets -D warnings` 干净；`kotone-tauri`、`kotone-core` 用 `--no-deps` 干净（英雄联盟插件那时还在别的任务手里改，带依赖的 clippy 卡在它那里）。
- Playwright（一个 worker）：新的 `apps/desktop/e2e/other-plugin-pushes.spec.ts` 4 个通过：“这局的同传”两句（译文、“原话（TslGame.exe）：……”）、推送的一行是最新那句，原地更新成三句以后还是一张、一行换成新的那句，点开在右侧栏是同一个组件；“她对队友说了”两句；“登录过期”的按钮调 `music.login_start`、不打开右侧栏，推送原地更新以后写“已登录网易云音乐：小琴”、按钮没了；三个插件的「设置 → 推送」各一条、样子（卡片 · 一行 / 卡片）、默认开。截图在 `target/plugin-shots/round-b-plugins/`。`svelte-check` 0 错 0 警告。`terminology.spec` 只挂了英雄联盟那一条（别的任务正在加的文案还没用上），这三个插件的文案都过。同传、游戏语音、网易云音乐自己的 e2e 有 6 个挂在保存设置 / 提示 / 跳转上：共用的 vite 开发服务器热更新多了，同一页面里加载了 3 份 `stores/ui.ts`、2 份 `navigation.ts`（请求里带不同的 `?t=`），设置存进去了界面却不变——和这次改动无关，开发服务器重启后要重跑；改了的那几步（默认“气泡”、换成“弹幕”存进 `display: danmaku`）在挂掉之前都过了。
- 真机（隔离的 `KOTONE_HOME`，不发任何键盘鼠标输入，出声：SAPI 英语句子、她的声音）：`interpreter_e2e.py` 20/20（假 AI、真 SenseVoice）：后台用 `interpreter.show`，主对话里一张“这局的同传”，两句并进同一张（日志一次“推送”一次“更新了”），卡片里有译文、原话、`kotone-speaker.exe`，推送的一行是译文，没有给她的说明；页面的开始 / 停止照旧。（第一次跑卡在脚本自己经调试端口调 `agent_conversation` 的那一步，重跑就过了。）`interpreter_push_real.py` 9/9（**真 DeepSeek**）：页面点开始 → 她 3.5 秒交给后台（tongchuan / 同传）→ 后台照规则调 `interpreter.show {text: "在红房子后面。", heard: "Behind the Red house.", app: "kotone-speaker.exe"}` → 一张卡片、中文译文、推送的一行就是它、没有说明 → 停止停掉，一共 13 秒。`voice_chat_real.py` 全过（真 DeepSeek + VB-CABLE）：她先 `voice_chat.say "我去拿龙了"` 再 `speak`，队友那边和玩家这边都有声音，聊天里一张“她对队友说了”写着“我去拿龙了”，没有给她的说明。

**还没测**：

- 悬浮窗的气泡里真的显示同传那一行（悬浮窗正在重做；现在的 `.cell` 不折行、`.line` 最宽 360px，40 个字的一整句会超出去，要让只有一格的一行能折行）。
- 真的游戏里：游戏中心里有在玩的游戏时 `interpreter.line`、`game-voice.said` 报出去、自动化收到；英雄联盟里按局分卡片（`match_id`）。
- 网易云真的过期一次：真应用里启动时推“登录过期”、在卡片上点按钮扫码、卡片原地变成已登录（只测了网易云对失效凭据的回答和推送台的逻辑）。启动时推的那一张会让悬浮窗弹一次小窗（新卡片），要看玩家觉不觉得打扰。
- 同传、游戏语音、网易云音乐自己的 Playwright e2e 在干净的开发服务器上重跑。

### 实施记录（整合（B 轮），2026-11-27 凌晨）

B 轮六个部分（呼叫以后的三种用法、悬浮窗 = 头像 + 气泡 / 聊天小窗 + 进度圈、选人卡片和一键导入、三选一卡片和对局中的提醒、总结的海克斯和今天的状态、同传 / 游戏语音 / 网易云音乐的推送）做完以后，在同一份工作区上跑全量检查、跑真机脚本、修各部分之间的接缝。玩家睡了、交出了机器（只有 WeGame / tcls 开着）。

**玩家看到的**：

- 悬浮窗透明的地方不再挡游戏：聊天小窗比头像高时，头像上面那一条透明的地方以前还是窗口的一部分，点下去点不到游戏；现在窗口只留头像和旁边那一块，别的地方点得到下面的游戏。拖动、点按钮、滚轮、不抢焦点、不进截图照旧。
- 悬浮窗头像外面不再有一个白色方框（真机上才有：进度圈的样式名和界面框架的一个工具类重名了）。
- 同传翻好的一整句在气泡里折行（最宽 360），不再一行顶出去；分了几格的一行（“对面寒冰 | 6 级了，有大招”）照旧一行。
- 海斗选英雄卡片的“我的英雄”：窄的时候标签不再压在图标上（“最好的海克斯”改成“海克斯”，两套核心三件的胜率和峡谷一样放在右边那一列）；一键导入写进客户端的核心三件和卡片上列的一样多（两套，原来卡片两套、写进去三套）。
- “你阵亡时”卡片：客户端资料里没有装备价钱时，“你”那一行不再写“装备 0”。

**怎么做的**：

- **同传插件只依赖插件契约**：P 为了记日志、在测试里查设置说明给 `plugins/interpreter` 加了 `kotone-core`。去掉：推送没推出去由宿主记日志（`DesktopHost::push` 对所有插件一样记“push: <插件> <key> 没推出去：……”），插件只把原因交回后台；清单测试直接看清单里的事件（不经事件目录、不用 `Settings`）；“设置说明里写着每条推送的开关”改成桌面端 `plugins::tests::every_push_switch_is_in_her_settings_guide` 对所有插件一起查（`pushes.<id> <名字>`、`<id>（<名字>`）。`Cargo.lock` 回到 HEAD。边界测试加了 `contract_only_plugins_depend_on_nothing_else_of_kotone`：同传只依赖 `kotone-plugin-api`，不许倒回去。说明：ADR 038 D6 和原来的边界测试允许插件依赖 `kotone-core`，英雄联盟、游戏语音、音乐、语音输入都在用（日志、设置说明，语音输入用编排器、历史、注入……），所以没有把“插件只依赖插件契约”写成对所有插件的规则——要的话是一次单独的重构（ADR 043 D1“只经宿主”）。
- **点得穿的透明处**：界面每次报大小时多报“挡鼠标的几块”（`floating_layout` 的 `boxes`：整块内容的每个直接子元素——头像、气泡 / 聊天小窗——按排版位置 `offset*`、四周加 8 px 阴影、裁到整块里；不看出场动画的位移）。后端按那块屏的缩放换成物理像素（纯函数 `floating::place::hit_rects`，有测试），和窗口大小一起用 `SetWindowRgn` 设窗口区域（一样就不再设；没报 = 整个窗口，旧界面照旧）。Windows 按窗口区域判断点的是谁，区域外的点直接落到下面的窗口；鼠标从区域移出去页面收到 pointerleave，进度圈照旧接着走。
- **单格一行折行**：`Floating.svelte` 里只有一格的 `.line` 加 `single`：格子 `white-space: normal`、`overflow-wrap: anywhere`，logo 顶对齐。
- **头像的白框**：`Avatar.svelte` 的 SVG 圈原来叫 `class="ring"`，打包后的样式里 Tailwind 的 `ring` 工具类给它加了一圈 1 px 白色 box-shadow（开发服务器里颜色不同、Playwright 截图里看不出来）。改名 `meter`。
- 英雄联盟：`pushes/import.rs` 加 `MAYHEM_CORE_SETS = 2`，导入和卡片共用；`Mine.svelte` 两处标签；`EnemyBuild.svelte` 没有价钱时用新文案 `plugin_lol_death_me_no_gold`；删了没人用的 `plugin_lol_cs_build_mayhem`（中英）。
- 文档：术语表“推送”一行的例子从三选一（已改成卡片）换成“对面寒冰 | 6 级了，有大招”和同传的一整句；“悬浮窗”两行加上透明的地方点得到游戏（`SetWindowRgn`、`boxes`）。`docs/design/lol-app.md` 加了选英雄、海克斯三选一、你阵亡时、今天的状态四张卡片的布局和这局总结 / 单局详情里的海克斯；“什么时候弹什么”的时机表换成现在的推送，原来的消息提示、覆盖层两节标成“并进了悬浮窗 / 去掉了，留作记录”。
- 真机脚本：`floating_real.py` 加 F4b（F5 之后、小窗高的时候：头像上面那一条下面的窗口不是悬浮窗；把测试窗口挪到悬浮窗后面在那一条上点一下，测试窗口收到、悬浮窗页面没收到、前台没变；头像本身还是悬浮窗）。`auto_card_real.py` 照 ADR 043 改了期望：选人时只有选英雄卡片（和海斗问要不要下载识别模型那张），按 key 找开局战绩，一张卡片一个 key，账本里推送以后只许有念语音推送的合成。

**检查**（WeGame / tcls 开着、没有游戏：正常优先级 `-j 4`，Playwright 2 个 worker）：`cargo fmt --all --check` 干净；`cargo clippy --workspace --all-targets --locked -- -D warnings` 干净；`cargo test --workspace --locked --no-fail-fast` 1800 过、0 挂、35 个忽略（54 个测试程序；新的 3 个：`floating::place::tests::only_the_avatar_and_the_box_beside_it_take_the_mouse`、`plugins::tests::every_push_switch_is_in_her_settings_guide`、边界测试 `contract_only_plugins_depend_on_nothing_else_of_kotone`）；`pnpm -C apps/desktop check` 本体 0 / 0、英雄联盟 App 0 / 0；Playwright 全量（desktop + plugins）252 个全过（本体 196、插件 56；`floating.spec` 新的“一格的一行折行、几格的一行不折、挡鼠标的是头像和气泡”，小窗那条加了“挡鼠标的只有小窗和头像、头像上面那一条不挡”）。全量 Playwright 第一次挂了 34 个，全是共用开发服务器热更新久了以后的老毛病（测试里 `import("/src/lib/stores/…")` 拿到的不是页面用的那一份）；重启开发服务器后这 34 个全过。

**真机**（隔离的 `KOTONE_HOME`，真 DeepSeek 的 Key 只在内存里，输入类工具都拒绝）：`floating_real.py` 33/33（加了 F4b 的 4 条：那一条点下去是桌面 / 测试窗口，测试窗口记到 `MOUSE Left`，悬浮窗页面没收到；`--shots` 32/32，截图里白框没了）；`game_events_real.py` 18/18 两次；`lol_champ_select_real.py` 19/19 两次（第二次海斗导入写 2 套核心三件）；`lol_postgame_real.py` 6/6；`lol_tools_real.py` 45/45；`call_mode_real.py` 32/32；`voice_chat_real.py` 8/8；`interpreter_push_real.py` 9/9；`auto_card_real.py` 13/13（改期望以前 8/13）；`focus_steal_real.py` 28/28；`voice_input_real.py` 42/42、1 个照旧跳过（控制台发送要真的 Steam 游戏）。

- `floating_real.py` 有一次和全量 Playwright 同时跑：开局战绩 40 秒内没推来，F4 看到的已经是收起的窗口（之后两遍都过）——机器忙的时候脚本的等待不够，不是悬浮窗的问题。
- `game_events_real.py` 的 G6（重随以后原地更新）以前挂过一次：那次选英雄卡片第一次推出来用了 6.2 秒（等腾讯 101 / aramhex 的网络数据），假客户端的选人只有 8 秒、第 4 秒重随，读到重随以后的那一轮还没取完数据选人就结束了。这次两遍都是 4.3 秒左右推出来、2 秒后更新成重随以后的英雄。是测试的时间窗太紧，加上卡片要等网络数据才推第一张；逻辑没错。

**还没测 / 留给以后**：

- 真游戏里点头像上面那一条（盖在英雄联盟上）、多显示器 / 不同缩放下的窗口区域。
- 语音推送念出来要用她的声音合成：在线语音（DeepSeek）每句约 0.002 元，`auto_card_real.py` 的账本里看得到；ADR 043 写的“推送不调模型、不花钱”只对模型成立，要不要在术语表 / ADR 里写明“语音推送按在线语音计费（很少）”由主会话定。
- 选英雄卡片第一次推出来要 4–6 秒（等网络数据）；可以先推我方几行、再补“我的英雄”等几块，这一轮没改。
- 子代理核对设计文档时看到的小出入（没改）：今天的状态 / 开局战绩里连胜连败小牌是淡底灰字（共用样式 `.chip.good`，设计文档写的是胜 / 负色字）；今天的状态全屏时没有素材出处那一行；三选一卡片里“左 / 中 / 右”“样本少”是 10 px 中文（设计文档说中文最小 11 px）。
