# ADR 043：插件形态与推送——插件是 App、事件和内置推送、工具；推送有卡片、一行、语音三种样子；覆盖层改成悬浮窗

状态：Accepted（2026-11-26，玩家定的方向；细节由主会话照讨论记录定）。部分修订 [ADR 039](039-desktop-companion-and-game-apps.md)（D3 的去重和“前面一个字不改”的说法、D4 消息提示、D5 覆盖层那一条）和 [ADR 038](038-kotone-and-apps.md) D5（覆盖层是插件页面的第三个显示位置）。

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
- 插件的推送不调模型、不花钱。给琴音的那句话（`note`）照 ADR 039 D3：作为一条隐藏的说明追加在对话末尾。说明要少给，只在关键时刻带（例如选人锁定、选人结束），中间的变化只更新卡片。

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
- 文字长度由宿主截：一行最多 3 格，每格最多 12 个字；语音每个说法最多 20 个字；`note` 照旧 160 字。
- 聊天里的记录：`Message::Notice` 带推送的数据（插件、插件名、`trigger`、`key`，以及卡片 / 一行 / 说了哪句）。旧对话里带 `card` 的 Notice 照旧显示（读旧写新）。
- 界面事件：新的一条推送、一条推送更新了、一条推送结束了，各一个（具体名字实现时定，写进实施记录）。悬浮窗和聊天页都听它。

- **有多深远**：给内置插件的接口是双向门（一起编译）；给第三方仍然是单向门，这次照旧不给（ADR 039 D3）。Notice 里的数据格式要能读旧的（原则 19）。
- **什么事实会让我们改主意**：玩家常关掉语音推送（嫌吵）；20 秒的间隔让要紧的提醒说不出来。

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
