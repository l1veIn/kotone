# Kotone 术语表

这是产品文案、代码评审和架构说明的权威术语表。代码标识保持原样；中文表述优先使用下表。模型能力清单以 `kotone-stt::model` 为准，配置字段以 Rust 定义为准。

## 界面用语（v2，2026-11-23）

**所有给玩家看的字以这一节为准**：界面（`apps/desktop/messages/zh-CN/*.json`）、气泡、她念的固定的话、托盘、错误提示、设置说明（`settings_guide.rs`）、给模型看的系统提示和工具说明（她会照着这些词跟玩家说话）、`docs/assistant.md`、官网。下面几节的技术术语只用在代码、架构文档和评审里。

英文界面（`messages/en`）照最后一栏翻；玩家嘴里说的命令（退下、停下、可以 / 不要、唤醒词）是中文语音命令，英文界面里照写中文。

规则：一个概念一个说法；“不说”一栏反引号里的词是禁用词，`e2e/terminology.spec.ts` 会在上面这些地方查（注释、日志、测试不查）。调研和取舍见 [2026-11-23 术语统一](research/2026-11-23-terminology.md)。

| 说法 | 意思与边界 | 不说 | 代码名 | English |
| --- | --- | --- | --- | --- |
| 通话 | 玩家用嘴跟她说话的那段时间。按呼叫键、喊唤醒词、点聊天页的呼叫按钮、按快捷指令都会**接通**；通话里她一直在听，用 `speak` 开口回答（一两句、不思考）；说“退下”、再按一次呼叫键、或一阵子没人说话就**挂断**（ADR 033）。“通话里”取代以前“游戏里 / 游戏内”指入口的用法：她怎么回答只看这句话从哪来，和开没开游戏无关 | `游戏内 ·`、`游戏内交给`、`游戏里的思考`、`游戏里的话`、`语音入口`、`语音回合`、`快速模式`、`接着听` | `Entry::Call`、`call.rs`、`assistant.callIdleSeconds` | call (“in a call”, “Call {name}”, “Hang up”) |
| 聊天窗口 | 在 Kotone 窗口的聊天页里打字跟她说话。她写字回答，可以思考 | | `Entry::Chat` | chat window |
| 游戏里 | **只用字面意思**：在游戏窗口里、游戏进程里（“在游戏里按呼叫键”“往游戏里打字”“这个游戏里记下的东西”） | （见“通话”） | | in the game (literal only) |
| 呼叫 / 挂断 | 呼叫 = 接通通话；挂断 = 结束通话 | | `assistant_call`、`call::end` | call / hang up |
| 呼叫键 | 按一下就呼叫；通话中再按 = 挂断。默认 `Ctrl + ~` | `陪玩热键`、`呼叫热键` | `assistant.hotkey` | call key |
| 她 / {名字} | 指角色时用“她”或当前角色的名字（界面里用 `{name}`，默认“琴音”）；“琴音”写死只用在产品名（Kotone 琴音）和默认角色上 | `琴音 Agent`、`陪玩模型`、`陪玩 AI 连接` | `agent.identity.name` | she / {name} |
| 角色 | 她的角色卡：名字、头像、性格、唤醒词；可以切换 | `人设` | `roles`、`role.json` | character |
| 游戏陪玩 | 侧栏分组名（ADR 028 D1，玩家定的），不当成名词指她 | | | Game companion |
| 角色页 | 左侧栏「游戏陪玩」里她那一项（显示她的名字）点开的页面。分页：角色 · 记忆 · 技能 · 唤醒 · 交互 · 权限 · 用量；聊天也在这一页。设置说明里的位置写成“琴音 → 唤醒 → 呼叫键”（第一段是她，界面上换成 `{name}`） | | `AssistantPage`、`AssistantSection` | her page (tabs: Character · Memory · Skills · Wake-up · Interaction · Permissions · Usage) |
| 唤醒 | 角色页的一个分页：呼叫键、通话、快捷指令、语音唤醒、聊天页的呼叫按钮。旧说法“怎么叫她”（2026-11-25 以前它是「设置」的一页） | `设置 → 怎么叫她` | 分页 ID `call` | Wake-up |
| 交互 | 角色页的一个分页：气泡、声音与弹幕（回答方式）；自动卡片和消息提示的开关也在这里（ADR 039 D3、D4）。旧说法“她怎么回答”（以前在「设置」里）。她用的模型不在这里，在聊天页输入框旁 | `设置 → 她怎么回答` | 分页 ID `reply` | Interaction |
| 她用的模型 | 聊天页输入框旁的小菜单：她用哪条 API 连接里的哪个模型（照 DeepSeek Harness 的输入框）。2026-11-21 挪进过设置，2026-11-25 玩家要放回聊天页 | （见“她”） | `assistant.connectionId`、连接的 `model` | Her model |
| 插件 | 装在她身上的一块本事：有自己的页面、工具和开关（英雄联盟、语音输入、网易云音乐……，ADR 038）。照 MCP Apps：她只调用插件的工具或打开它的页面。“应用”在界面里指 Kotone 自己（“桌面应用”），所以插件不叫应用，也不叫 App；浏览器里装的叫“扩展”。每个插件有自己的 logo（原创，不用网易云、拳头这类别人的商标；第三方 MCP 服务器用它自己声明的图标，ADR 039） | | `plugins/<id>`、`kotone-plugin-api` | plugin |
| 第三方插件 | 玩家自己加进来的别人的 MCP 服务器（插件页上粘贴配置，或者从 Claude Desktop、VS Code 导入）。一个服务器就是一个插件，开关、卡片和内置的一样；它的工具名是 `<名字>.<工具>`，结果只当数据（别人写的），只在 Kotone 里用、不转给外部 Agent（MCP 方案第 2 期）。代码里叫远程插件 | | `kotone-remote-plugin`、`KOTONE_HOME/mcp.json` | third-party plugin |
| 信任（第三方插件） | 加第三方插件时的那一次确认：看清它会在你电脑上运行什么程序、或者连哪个网址，要哪些 Key。信任以后，它说只读的工具不再先问；配置改了要重新信任。没标只读的工具照样第一次先问，可以选“总是允许” | | `plugins.<id>.settings.trusted`、`third-party-action`、`third-party-read` | trust |
| 内置浏览器 / 网页 | 聊天页右侧栏的“网页”标签：你自己能上网，她在后台的事里也用它打开网页（ADR 031）。它留在本体，不是插件；“设置 → 浏览器”里有开关（默认开），关掉后右侧栏没有“网页”、她也不再打开网页（ADR 038） | | `browser.enabled`、`browser.*` | built-in browser / Web |
| 扩展 | 内置浏览器里装的 Chrome / Edge 扩展（已解压的扩展文件夹，例如去广告），在“设置 → 浏览器”里加、删、开关。“插件”只指她身上的本事，所以浏览器里的不叫插件 | `浏览器插件` | `browser.extensions` | extension |
| 外部 Agent | 经工具服务 / MCP 接进来的别的 Agent；界面上的“Agent”只指这个 | `AI 助手` | `assistant.route = agent` | external agent |
| 提醒助手 | 只给 0.1.x 升级用户保留的旧页面名 | | `LegacyRemindersPage` | Reminder assistant (0.1.x) |
| 热键 | 全局按键组合的统称（语音输入热键、停下热键、快捷指令的热键）。呼叫键是唯一简称“键”的。「设置 → 热键」一页列出全部，标来源（Kotone / 插件名 / 自己加的 / 固定的 Esc），冲突标红；**热键只在那一页改**，别的页面（唤醒、系统 → 停下、模型 → 语音识别、语音输入）只显示现在的键和“去「设置 → 热键」改”，提醒助手热键照旧在提醒助手页改；游戏配置里的聊天键不是热键、不列在那里，但参与冲突检查（ADR 040） | `快捷键` | `*Hotkey`、`hotkey_table` | hotkey (Settings → Hotkeys) |
| 快捷指令 | 「设置 → 热键」里自己加的、按一下就替你跟她说一句预设的话的热键（按了也会接通通话）。同一处还能加运行自动化的热键（ADR 040 第 3 期）；这些都叫“自己加的” | `快捷消息`、`快捷短语` | `hotkeys.bindings[].say`（旧字段 `assistant.companion.quickMessages` 是镜像） | quick command |
| 后台的事 / 交给后台 | 她交出去慢慢做的复杂的事，聊天里是一张卡片，做完她说一句 | `子代理`、`子会话`、`后台任务` | `agent.start`、`Subagent` | background task / hand off to the background |
| 停下 | 停掉她正在做的一切和全部自动化（不挂断通话）。停止热键、托盘、说“停下”、她操作游戏时按 Esc | `停止所有自动化`、`停止热键` | `stop::stop_everything`、`automationStopHotkey` | Stop (stop everything); “stop hotkey” |
| 停止 / 取消 / 打断 | 停止 = 停一件具体的事（这个自动化、播放、本地语音识别）；取消 = 不做还没做的（对话框、下载、这次录音）；打断 = 她说话时被你打断 | | | stop / cancel / interrupt |
| 权限 | 每个工具是 允许 / 先问我 / 禁止（角色页 → 权限；以前在「设置」里） | `授权`、`设置 → 权限` | `agent.permissions` | permissions: Allow / Ask first / Block |
| 用量 | 今天 / 本月花了多少 token 和钱、命中缓存多少、对话多长整理成摘要（角色页 → 用量；以前在「设置」里） | `设置 → 用量` | `usage.jsonl`、`agent.compactAtTokens` | usage |
| 确认 | 她做某件事前问你一次（“这一步……，可以吗？”），回答是 允许 / 拒绝 / 这段对话都允许；通话里直接说“可以 / 不要” | `授权`、`批准` | `Asker`、`PermissionRequest` | confirmation: Allow / Deny / Allow for this conversation |
| Key | 服务商给你的那串钥匙（“DeepSeek Key”） | `密钥`、`API Key`、`API 密钥`、`凭据` | `apiKey`、`SecretStore` | API key |
| API 连接 | 一个服务商的地址、模型和 Key（设置 → API 连接）；她用哪个连接、哪个模型在聊天页输入框旁的“她用的模型”里选 | `AI 连接`、`API连接` | `Connection`、`connectionId` | API connection |
| 服务商 | DeepSeek、阿里云百炼、火山等 | `提供商`、`厂商` | `provider` | provider |
| 回答方式 | 她的回答和提醒用 语音 / 弹幕 / 两种都用 | `提醒方式`、`播报方式`、`回复方式` | `assistant.defaultDelivery` | reply mode (voice / floating text) |
| 声音 / 语音合成 | 她的声音（音色、语速、音量）；“语音合成”只用在模型分类上 | `TTS`、`朗读` | `tts*`、`SpeechOutput` | voice / text-to-speech |
| 语音识别 | 把说的话变成字（本地 / 在线）。只有一份，在本体：呼叫、语音输入、同传都用它，同一个模型只加载一份（ADR 039 D2）。“语音输入”是另一件事：说的话直接打进游戏聊天框 | `ASR`、`转写`、`听写` | `stt*`、`asr*` | speech recognition |
| 同传 | 插件（ADR 021、038）：她在后台听你指定的程序（游戏、Discord、KOOK……，按程序录音）里队友说的话——不是你的麦克风，也录不到她自己——一句一句翻成你的语言，用弹幕或气泡显示。识别用本体的语音识别：你选的识别听得懂外语（自动判语种的在线识别，或本地 SenseVoice）就用它，否则用本地 SenseVoice（中英日韩粤）；用在线识别时队友的每一句都会发给它识别、每句计费。停止本地语音识别时，用本地识别的同传一起停（ADR 039 实施记录“同传用本体的识别”） | | `plugins/interpreter`、`voice.listen.*`、`voice_listen.rs`、`kotone_stt::listen` | interpreter |
| 对话 | 和她的一段对话（对话列表里的一项）；“聊天”只用在聊天页、聊天窗口、游戏聊天框 | `会话` | `Conversation` | conversation (聊天 = chat) |
| 自动化 | 替你持续或稍后做事；动作是“说一句”或“程序” | `脚本` | `automation.*` | automation |
| 网易云音乐 | 放歌插件的名字（ADR 034、038；2026-11-25 起，以前叫“音乐”）：从左侧栏“插件”的卡片点开，搜歌、每日推荐、私人 FM、我的歌单、播放条。插件 id、配置（`plugins.music`）、她的工具名还是 `music`。“音乐”只说放歌这件事本身（音乐音量、她说话时音乐压低） | | `plugins/music`、`MusicPage` | NetEase Cloud Music |
| 放歌 / 正在放 | Kotone 自己放歌（不用浏览器）：你在网易云音乐的页面里点，或者她用 `music` 工具放；“正在放”是播放条和左侧栏小播放条上的那首 | | `music`、`Player` | play music / Now playing |
| 登录网易云音乐 | 点“登录网易云音乐”打开网易云官方登录页的窗口，用网易云音乐 App 扫码；第一次放歌要登录一次，登录信息交给 Windows 保管。她要放歌而你没登录时，聊天页出这个按钮（她不自己弹窗，免得游戏里抢焦点） | | `music_login_start`、`login.stage = needed` | Log in to NetEase Cloud Music |
| 每日推荐 / 私人 FM | 网易云按你的口味每天给的一组歌 / 一首接一首的电台 | | `daily`、`fm` | Daily picks / Personal FM |
| 我喜欢的音乐 / 红心 | 网易云里你红心过的歌（歌单里排第一）；红心 = 把这首加进去 | | `liked`、`like` | Liked songs / Like |
| 歌单 / 播放列表 | 歌单是网易云里的一组歌；播放列表是正在放的这一串 | | `Playlist`、`queue` | playlist / queue |
| 试听 | 要 VIP 的歌给没会员的账号只有 30–45 秒；她不放试听，改放能完整放的版本并说一声 | | `trial` | preview |
| VIP / 付费 / 无版权 | 放不了的三种：要会员、要单独买（付费专辑）、网易云没有版权；列表里灰掉并标出来 | | `Playable` | VIP / Paid / Unavailable |
| 音质 | 标准 / 极高 / 无损；默认极高（非 VIP 的上限），无损要 VIP | | `music.quality` | Quality: Standard / Very high / Lossless |
| 全部页面 / 恢复默认 | 左侧栏“⋯”打开：页面在组内上移下移、显示隐藏（设置、关于固定）；恢复默认 = 默认顺序、全部显示 | | `ui.sidebar` | All pages / Restore defaults |
| 游戏配置 | 一个游戏的聊天键、频道、词表等（游戏列表页里编辑） | `游戏设置`、`游戏档案` | `GameProfile` | game profile |
| 气泡 / 悬浮窗 / 弹幕 | 气泡只属于她（在听、在想、在说）；悬浮窗只属于语音输入；弹幕是从右往左飘的字 | `对话气泡` | | bubble / overlay / floating text |
| 自动卡片 | 插件看到游戏里的事（例如英雄联盟进选人、加载、结算），自己发进聊天的一张卡片（双方战绩、这局总结）。不叫她、不花钱；同时给她留一句很短的说明，之后的对话里她看得到；通话中不打断她说话。卡片上写明是哪个插件发来的。现在只有内置插件能发（ADR 039 D3）。文档和代码也用这个词 | `事件卡片`、`推送卡片` | `Host::auto_card`、`kotone_agent::auto_card`、`agent.autoCards.enabled` | auto card (“{plugin} posted a card”) |
| 消息提示 | 发自动卡片时，屏幕右下角弹出的小窗：她的头像 + 一句话；点它打开聊天、定位到那张卡片；不抢焦点，几秒后自己消失，游戏独占全屏时不弹；开关在“游戏陪玩 → 角色 → 交互”（ADR 039 D4）。和气泡不同：气泡是她在听、在想、在说，点不了。不叫“通知”（和 Windows 的系统通知撞名），不叫“提醒”（那是过一会儿说一句的提醒） | `右下角弹窗`、`通知弹窗` | `message_popup.rs`、`agent.autoCards.popup` | message pop-up |
| 技能 | 她会的本事包（一个文件夹一个） | `Skill` | `SKILL.md`、`workspace` | skills |
| 本地 / 在线 | 在这台电脑上算 / 发给服务商算 | `离线`、`云端` | `asrLocal` | local / online |

## 架构文档里的叫法（2026-11-25，ADR 039）

只用在设计文档、ADR 和评审里，讲架构不再用“她”。界面上的字照上一节，指角色照旧用“她 / {name}”。

| 说法 | 意思 | English |
| --- | --- | --- |
| 本体 | Kotone 自己：感官和表达（耳、眼、嘴、手）、对话、自动化、宿主（ADR 038 D2）。语音识别和往游戏聊天里打字也在本体（ADR 039 D2） | core |
| 插件 | 装在本体上的 App（ADR 038），界面上的说法见上一节 | plugin |
| 琴音 | 角色（默认的那个）。讲“谁在听、谁在说、谁在想”时用；玩家可以换角色 | Kotone (the character) |
| 共享资源 / 管家 | 共享资源：全进程（或全屏、全机）只有一份、又有两个以上的用户要用的东西，例如热键、往游戏里输入、麦克风、扬声器、屏幕上的位置和前台、聊天流、语音识别。管家：本体里拥有它的那一处代码，“能不能用、谁先用”的规矩只写在这里，插件经宿主向它要（ADR 042）。界面上不说 | shared resource / owner |
| 命令 / 热键表 | 命令：能绑热键的一件事，`<来源>.<命令 id>`（本体的 `kotone.call`、插件 `plugin.toml` 里 `[[commands]]` 登记的 `voice-input.talk`）。热键表：config.json 的 `hotkeys.bindings`，每条 键 + 命令，或者玩家自己加的（带 `id`、可选 `label`：`say` 跟她说一句、`tool: automation.run` 运行自动化，以后还有调工具、打开页面，ADR 040 第 3 期）；热键的旧字段（`hotkey.key`、`channelCycleHotkey`……、快捷指令 `quickMessages` 前 8 条）是它的镜像（ADR 040 D1、D5、D8）。界面上不说“命令”，行名就是命令的名字；自己加的行名是玩家起的名字，没起就是要做的事 | command / hotkey table |

## 游戏陪玩的核心概念（ADR 028）

界面文案以这一节为准。[ADR 028](adr/028-game-companion-interaction.md) 有完整的流程和界面结构。

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 呼叫 / Call | `assistant_call`, `call_hotkey()`, `HK_ASSISTANT`, `call.rs` | 接通和角色的通话（ADR 033，见上面“界面用语”）。四种触发：呼叫键、唤醒词、聊天页的呼叫按钮、快捷指令。听和答都在**气泡**里。语音识别没准备好时，呼叫按钮先启动本地识别再听；呼叫键只在气泡里提示。 |
| 呼叫键 / Call key | `assistant.hotkey`；0.1.x 升级用户为 `assistant.companionBindings.hotkey` | 按一下就呼叫，通话中再按 = 挂断。新用户默认 `Ctrl + ~`，升级用户默认空。不再叫“陪玩热键”“呼叫热键”。 |
| 语音输入 / Voice input | `hotkey`, `Orchestrator`（`purpose = input`） | 说话，原话打进游戏聊天框。**不经过 Agent**。状态显示在**悬浮窗**里。是一个插件（`plugins/voice-input`，ADR 038）：从左侧栏「插件 → 语音输入」打开它的页面，分页 输入 / 文字处理 / 历史（分页 ID `general` / `processing` / `history`）；关掉插件时语音输入热键、悬浮窗一起停。语音识别和往游戏聊天里打字属于本体，语音输入借用它们；她替你在游戏聊天里发言（`game_chat.send`）在本体，关掉语音输入不影响；每个游戏的聊天键留在游戏配置（ADR 039）。 |
| 语音输入热键 / Voice input hotkey | `hotkey.key` | 按下说话的那个键（原“录制热键”）。 |
| 停下 / Stop | `stop::stop_everything`, `automationStopHotkey` | 停止角色正在做的事和所有自动化。四种方式：她操作游戏时按 Esc、停止热键、说“停下”、托盘「停下」。不取消语音输入的录音（那是 Esc）。 |
| 启动 / Start | `runtime::start`, `runtimeToggleHotkey`, `ui.autoStart` | **只和本地语音识别有关**：把本地识别模型加载进内存。本地识别有 未启动 / 准备中 / 已启动 / 出问题了 四种状态；在线识别打开 Kotone 就就绪，界面上看不到“启动”。 |
| 气泡 / Bubble | `bubble.rs`, `assistant.bubble` | 只属于角色：她在听、在想、在说什么。关掉后只用声音和弹幕，不借用悬浮窗。 |
| 悬浮窗 / Overlay | `overlay.rs`, `overlay.*` | 只属于语音输入：识别进度、频道、没准备好的提示。 |
| 屏幕角落分配 / Screen corner | `screen_corner.rs` | 摆在屏幕角上的小窗从角往外排成一列：悬浮窗（选了角上的预设或拖到角上）贴着角、从不被挪，气泡在它外面，消息提示最外面；谁没显示就不占地方，后出来的里面的窗口叠上外面的，外面的让开（ADR 039 实施记录“右下角排成一列”）。弹幕、插件覆盖层不占角落。只在代码和文档里用，界面上不说。 |
| 打开软件时显示 / Start page | `ui.startPage` | `voiceInput` = 语音输入；空或其它任何值 = 聊天（默认）。 |
| 主会话 / Main conversation | `agent.rs` `drive`，`kotone_agent::catalog::main_offers`（`ToolEntry::main`） | 玩家唯一看得到的对话：语音和打字共用一段、共用一小套马上出结果的工具（ADR 029 第二期；每个工具登记时自己标给不给主会话，MCP 方案第 1.5 期）。工具超过 3 秒没回来就转到后台（`kotone_agent::background`）。界面上就叫“对话”。 |
| 后台的事 / Background task | `agent.start`、`Subagent`、`kotone_agent::subagents`（代码与 ADR 里叫“子代理”） | 主会话交出去的复杂、慢、多步的事。玩家看到的是主会话里的一张卡片（“写喝药脚本 · 进行中”），做完由主会话说一句；不进对话列表，不能点进去聊。界面和文案不说“子代理”“子会话”。旧的“子会话”（ADR 022）已经并进来，这个词不再用。 |

## 语音输入与运行时

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 运行时 / Runtime | `RuntimePhase`, `RuntimeStatus` | 应用级 stopped → starting → running → stopping 生命周期；包含模型预热、热键和资源释放。不是一句话的识别状态。界面上只在本地识别时以“启动”出现；`RuntimeStatus.asrLocal` 区分本地 / 在线，`error` 是最近一次启动失败的原因。 |
| 识别会话 / Session | `Orchestrator`, `OrchestratorState`, `SttSession` | 一次采音、识别、处理和交付；`OrchestratorState` 表示业务阶段，`SttSession` 是识别端口实例。取消后的迟到结果不能继续交付。 |
| 交互模式 / Interaction mode | `InteractionMode` | 决定怎样开始、结束以及直接发送或预览。包括对讲机、录音笔、说一句就走、独奏模式。`HotkeyMode` 仅描述按住/点按，不等于完整交互模式。 |
| 识别引擎 / STT engine | `SttEngine`, `EngineRegistry`, `engineId` | 执行识别的适配器/运行循环，例如 `sherpa-streaming`、`sherpa-offline`。不是模型家族；旧 `sherpa-onnx-*` ID 只是兼容别名。 |
| 模型 / Model | `ModelInfo`, `MultiFileModel`, `modelId`, `ModelRecipe` | 具体权重或在线服务条目及其加载配方。X-ASR、SenseVoice、FunASR-Nano 属模型；多个模型可以共用同一引擎。 |
| 流式与本地 / Streaming and local | `ModelIo`, `EngineCapabilities` | `ModelIo::Offline` 表示非流式，`EngineCapabilities.offline` 表示无需远程服务。两者不得混用。“在线 transducer”常指流式解码，不代表上传音频。 |
| 游戏配置 / Game profile | `GameProfile`, `ChatChannel` | 前台程序匹配、聊天打开/发送按键、频道、前缀和热词等游戏适配数据。不是 AI 连接，也不是全局设置。 |
| 发到游戏 / Send to game | `SendToGame`, `GameChat`, `ChatRequest`, `ChatOutcome`, `game_chat.send` | “把这段话发到当前游戏的聊天”唯一的入口（ADR 041）：语音输入、她、外部 Agent、自动化的 `chat()` 都经它。按前台检测到的游戏挑游戏配置和频道，等修饰键松开，一行发一条，结果说走了哪条路、哪个频道、发了几行、没发完的原因。不重试、不补发。 |
| 发法 / Chat provider | `[[send_to_game]]`, `SendToGameSpec`, `ChatProvider`, `Plugin::send_to_game`, `route: client` | 游戏插件给某个游戏提供的“发到游戏”的另一种做法（ADR 041 D3）：插件在 `plugin.toml` 里登记“前台是这些窗口时交给我”，入口照前台挑，没有就是本体的通用发法（按键）。英雄联盟登记的是客户端：选英雄、房间里经客户端的聊天接口发，不按键、不拿输入锁。插件接手以后失败也不退回按键。界面上不出现这个词，玩家看到的是插件的开关（“在客户端里发聊天”）和回执里的“经客户端的聊天接口发”。只给内置插件。 |
| 输入锁 / Input lock | `InputLock`, `InputUser` | 往游戏里按键的人共用的一把锁（ADR 041 D4）：一条发言、一次 `game.act`、按键程序的每一下各拿一次，后来的排队，最多 5 秒（或调用方的期限）。实例只有宿主一份。不是 ADR 032 的挡键（那个挡的是玩家的键，输入锁排的是 Kotone 自己的几路）。 |
| 连接 / Connection | `Connection`, `connectionId`, `ResolvedConnection` | 可复用的服务地址、协议、模型与凭据引用。凭据由独立存储管理，不嵌入 pipeline、profile 导出或诊断包。 |
| 后处理流水线 / Post-processing pipeline | `PostProcessPipeline`, `PipelineStepConfig`, `TextProcessor` | 有序处理步骤；每步消费上一步输出。处理器是能力定义，步骤是带配置和失败策略的实例。`required` 阻止失败交付；`best-effort` 保留上一文本继续。 |
| 识别原文与待交付文本 / Recognized and ready text | `RecognizedText`, `ReadyText` | 原文来自 STT；待交付文本经过后处理后进入只读预览/注入。处理失败重试处理阶段，发送失败重试现成文本。评测使用识别原文。 |
| 评测目标 / Evaluation target | `EvalTarget` | `engineId + modelId + sessionConfig` 的明确快照。目标指纹区分模型与配置，源指纹区分录音与人工标注；旧无身份结果不能当作当前缓存。 |
| 语音活动检测 / Voice activity detection | `Vad`, `VadConfig` | 识别语音区间、静音判停和音频裁剪；不是语音转文字模型。`vadSilenceMs` 是交互判停等待，`vad.*` 是检测器内部参数。 |

## 提醒能力

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 提醒助手 / Reminder assistant | `assistant`, `voice_reminder`, `danmaku_reminder` | 面向屏幕前用户本人，通过独立热键或文字入口让 AI 提出受限的提醒计划；不向第三人喊话或注入游戏。语音入口要等语音识别准备好（本地识别要先启动），文字入口不依赖。0.2 起只有 0.1.x 升级用户保留（「提醒助手」页）。 |
| 助手交互模式 / Assistant interaction mode | `AssistantInteractionMode`, `assistant.interactionMode` | 与游戏输入独立的对讲机、录音笔、说一句就走三种模式，默认对讲机，热键默认未绑定；录音笔确认提交给助手，其余模式按各自结束规则提交，不走游戏发送出口。旧助手独奏配置兼容为对讲机。 |
| 提醒 / Reminder | `ReminderIntent`, `Reminder`, `ReminderScheduler` | 一次性的内容、到期时间、输出方式和状态；输出采用简短正文，不附称呼或开场白。本机保存，应用未退出时调度，停止语音运行时不停止既有提醒；不等于系统级闹钟。 |
| 语音播报 / Text to speech (TTS) | `SpeechOutput`, `SpeechRequest`, `SpeechVoice` | 文本转成可听语音，默认 Windows SAPI 声音，无需下载 Kotone TTS 模型；Kokoro 中文模型可选、手动下载。与 STT、提示音分开。 |
| 自动化 / Automation | `automation.*`、`kotone-script` | 替用户持续或稍后做事：**定义**（名字、入参、动作）+ **触发**（立即 / 延迟 / 指定时刻 / 热键）+ **执行记录**。动作是“播报”或“程序”（沙盒 Luau）。提醒是“延迟 + 播报”的自动化（ADR-013）。用户可见处不叫“脚本”。 |
| 执行记录 / Automation run | `automation.runs` | 一次执行：入参、目标窗口、到期时间、状态（等待中 / 运行中 / 完成 / 失败 / 取消 / 错过）、日志。最多执行一次，不自动重试，错过不补。 |
| 漂浮弹幕 / Danmaku | `DeliveryKind::Danmaku`, `DanmakuPresentation`, `ReminderDelivery` | 到期时从右向左滑动的单行桌面文字，可带小表情前缀；窗口透明、无底板、点击穿透。速度以 CSS 像素/秒定义，按实际条宽计算时长；不是聊天室消息，不能保证覆盖独占全屏游戏。 |

## 本机工具服务

| 中文 / English | 代码名 | 含义与边界 |
| --- | --- | --- |
| 工具服务 / Tool service | `kotone-tools`, `ToolService`, `toolService.enabled` | 桌面实例托管、默认关闭的 loopback HTTP 服务，供 Agent 调用确定性的本机工具；独立于 ASR 运行时。不是 MCP/DSH 插件，也不自行规划任务。 |
| 描述文件 / Descriptor | `Descriptor`, `tool-service-s<会话>.json` | 当前用户、当前 Windows 会话私有的发现文件，含协议版本、实例 ID、PID、地址与会话凭证；客户端校验权限、会话与进程身份后才连接。 |
| 能力清单 / Capabilities | `system.capabilities`, `GET /v1/capabilities` | 本实例真实实现的工具及其输入/输出 schema、就绪状态与限制；路线图工具不在其中。 |
| 窗口引用 / Window reference | `windowId`, `WindowIdentity` | 不透明 ID，绑定窗口句柄、PID 与进程创建时间，仅在发放它的实例内有效；捕获前重新校验。 |
| 前台可见客户区捕获 / Foreground visible client capture | `gdi-screen-foreground-client` | 目标为前台时复制其客户区内屏幕已合成的物理像素；覆盖其上的窗口会被一并捕获，不支持后台/最小化/隐藏窗口，独占全屏可能得到黑图。 |
| 产物 / Artifact | `artifactId`, `ArtifactStore` | 不可变的内存 PNG 图像及元数据（SHA-256、尺寸、TTL、父/根产物、到根图的变换）；裁剪生成新产物，原图不变。 |
| 帧 / Frame | `frameId` | 一次采集的标识；同一捕获的裁剪/预处理链共享，不同帧的坐标不可混用。 |
| 预处理 / Preprocess | `image.preprocess`, `preset`, `operations` | 显式的缩放/灰度/反色/自动对比度/阈值操作，生成新产物并记录实际比例；`preset=none` 不处理。不是 OCR 的强制前置步骤。 |
| 根变换 / Root transform | `rootTransform` | `rootPoint = offset + scale × point`：把任意派生产物（含放大后再裁剪）中的像素映射回原始捕获的物理像素。 |
| OCR 后端 / OCR backend | `OcrBackend`, `local-ppocrv5-korean`, `deepseek-ocr` | 可插拔的文字识别实现，显式选择、互不降级；本地不外发，在线只上传所选图像且需用户指定连接。 |
| 文字观测 / Observation | `observations[]`, `granularity` | OCR 返回的一段原文及其几何：`line` 行框、`block` 整块框（块内行无坐标）、`page`/无几何；置信度可空，不是语义正确率。 |
| 跟踪会话 / Tracking session | `chat.track.*`, `session` | 调用方命名的聊天跟踪状态：消息、事件、帧记录；有容量与空闲 TTL。 |
| 消息 ID 与修订 / Message ID, revision | `messageId`, `revision` | 一个可视文本行在代际内的稳定身份；OCR 改字保持 ID 并 revision+1；同文重复发送可以是新 ID。 |
| 代际 / Generation | `generation`, `chat.track.reset` | 新对局、重连、换 ROI 等边界；重置后清空消息与事件，旧游标失效。 |
| 游标 / Cursor | `cursor`, `newSince` | 绑定会话与代际的事件位置，只用于读取其后的新增与修订；`visible` 读取与游标无关。 |
| 事件总线 / Event bus | `EventBus`, `event.read`, `GET /v1/events` | 工具服务内 Kotone → Agent 的有界事件流（`user.request`、`agent.*`、`runtime.phase`、`app.*`）；游标 `ev1.<实例>.<序号>` 只在本实例有效，失效报错而不跳过。与聊天跟踪的游标是两回事。 |
| 用户请求 / User request | `user.request`, `AgentRequests`, `areq_…` | 用户经 Kotone（`assistant.route=agent` 时的助手热键语音或文字入口）发给外部 Agent 的原文；pending → claimed → replied / expired，十分钟有效。原文对 Kotone 是数据。 |
| 助手去向 / Assistant route | `AssistantRoute`, `assistant.route` | 助手入口把请求交给谁：`reminders`（默认，Kotone 自己的 AI 连接规划提醒）或 `agent`（发布 `user.request`）。 |
| 领取 / Claim | `agent.request.claim`, `consumer` | Agent 处理用户请求前声明归属；同一领取者幂等，其他领取者被拒绝，保证多个 Agent 在线时一条请求只处理一次。 |
| Agent 回复 / Agent reply | `agent.reply`, `deliveryId` | Agent 经 Kotone 对用户说话（语音/弹幕），复用提醒的交付路径；默认受理即返回，交付结果以 `agent.reply.delivered/failed` 事件给出。 |
| 桌面宿主 / App host | `AppHost`, `DesktopHost` | 工具服务调用应用级能力（启停运行时、退出、交付回复、助手提交）的端口；`kotone-tools` 不依赖 Tauri。 |
| 后台启动 / Background launch | `--background`, `kotone-cli app launch` | 只驻留托盘、不打开设置窗口、不抢焦点的启动方式；进程不继承调用方句柄。 |

## 四种交互模式

下表描述游戏语音输入的四种模式；助手仅复用前三种的采集规则，提交对象是提醒助手。助手录音笔预览后确认创建请求，其余两种模式结束后提交。

| 产品名称 | 配置值 | 开始 | 结束与交付 |
| --- | --- | --- | --- |
| 对讲机 | `push-to-talk` | 按住热键 | 松手结束，转写/处理成功后直接发送 |
| 录音笔 | `dictation` | 点按热键 | 再点停止，显示只读预览，再确认发送 |
| 说一句就走 | `one-shot` | 点按热键 | VAD 静音判停后发送；热键可强制结束 |
| 独奏模式 | `solo` | 点按热键 | 每段判停后发送并继续监听，再点停止 |

文档中的“确认发送”不意味着可编辑预览。首字延迟也不等于用户等待时间：评测的全速回放不模拟真实说话节奏，模型预热不包含在采音后的计时里。
