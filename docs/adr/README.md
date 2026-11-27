# 架构决策记录

ADR 保留当时的上下文和选择，状态决定其适用性。当前架构与术语分别见 [architecture.md](../architecture.md) 和 [glossary.md](../glossary.md)。修订状态不表示原始方案从未实现；表示后续实现已改变其中部分约束。

| ADR | 状态 | 当前说明 |
| --- | --- | --- |
| [001 workspace 布局](001-workspace-crate-layout.md) | Amended / 已修订 | core 与实现层依赖方向保留；目录和 crate 数量已扩展 |
| [002 apps/crates 归位](002-apps-crates-monorepo-layout.md) | Accepted / 已采纳 | 目录原则仍适用 |
| [003 whisper sidecar](003-whisper-sidecar-engine.md) | Superseded / 已替代 | sidecar 已删除，当前使用 sherpa 配方与可选远程适配器 |
| [004 sherpa 流式](004-sherpa-onnx-streaming-engine.md) | Amended / 已修订 | sherpa 绑定保留，模型/默认 feature/引擎身份已变 |
| [005 评测](005-eval-recording-replay.md) | Amended / 已修订 | 录档默认关闭，目标与缓存遵循 ADR-011 |
| [006 交互模式](006-session-lifecycle-interaction-modes.md) | Amended / 已修订 | 只读预览保留；当前四预设含 solo |
| [007 VAD](007-vad-one-shot-mode.md) | Amended / 已修订 | 默认特性、内置模型和兜底已落地 |
| [008 聊天频道](008-chat-channels.md) | Accepted / 已采纳 | 频道属于游戏配置 |
| [009 后处理](009-post-processing-pipeline.md) | Amended / 已修订 | 连接、密钥、schema、在线处理与双文本已实现，共享AI层抽出 |
| [010 提示音](010-sound-feedback.md) | Accepted / 已采纳 | 已随 0.1.9 发布；与 TTS 不是同一能力 |
| [011 评测身份](011-eval-target-identity.md) | Accepted / 已采纳 | 明确模型与配置、源指纹和独占评测资源 |
| [012 提醒助手](012-reminder-assistant.md) | Amended / 已修订 | 独立入口、受限 AI 工具、系统语音和弹幕保留；调度并入自动化（ADR-013） |
| [013 自动化领域模型](013-automation-domain-model.md) | Accepted / 已采纳 | 定义 / 触发 / 执行记录三层，一个调度器，提醒 = 延迟 + 播报；最多一次、错过不补 |
| [014 游戏场景 Agent 运行框架](014-game-agent-harness.md) | Accepted / 已采纳 | 基础能力 → 自动化 → Agent 三层；内置 Agent（聊天窗口 + 游戏内语音），权限为允许 / 询问 / 禁止；取代陪玩“一次决策”设计 |
| [015 内置 Agent 的完成度与技术栈](015-agent-scope-and-stack.md) | Accepted / 已采纳 | 第一版对标 pi 的精简完整度、游戏相关层做深；Rust 单进程，不带 Node；模型通信层用 rig-core，循环自己写；边界用 MCP / SKILL.md / ACP 式事件 |
| [016 前端引入 shadcn-svelte](016-ui-foundation-shadcn.md) | Accepted / 已采纳 | 分三步：底子 → 「琴音」聊天页（复用 RambleDesk 的聊天实现）→ 其他页面逐页迁移；悬浮窗与弹幕窗不改 |
| [017 语音唤醒与执行中的语音接话](017-voice-wake.md) | Accepted / 已采纳 | sherpa-onnx 关键词检测（“琴音琴音”），模型按需下载；唤醒后与陪玩热键同一条收音与琴音流程；应答“嗯？/我在”，玩家开口即打断；1.5 秒先接一句；默认关 |
| [018 琴音的对话气泡](018-companion-bubble.md) | Accepted / 已采纳 | 语音和琴音说话时用屏幕角落的独立气泡窗（头像 + 状态 + 一句话），不再借用语音输入的悬浮窗；整窗鼠标穿透、不抢焦点；回答默认不走弹幕 |
| [019 琴音在游戏里看屏幕、动鼠标](019-game-computer-use.md) | Accepted / 已实现（2026-11-17，与设计的出入见文末“实施记录”） | 底层与自动化共用；新增 `game.look`（截图 + 编号文字元素）与 `game.act`（批量动作）；动作清单照抄 Anthropic，批量与安全照抄 dsh-computer-use-win，编号照抄 UFO² / OmniParser，游戏循环照抄 Cradle / GameVerse / UI-TARS |
| [020 替玩家在游戏语音里说话](020-voice-into-game-chat.md) | Accepted / 已采纳 | 虚拟麦克风 VB-CABLE（一键安装）、混音、改 Windows 默认麦克风；`voice_chat.*` 工具 |
| [021 听懂说外语的队友](021-hear-teammates.md) | Accepted / 已采纳 | 只录指定程序的声音、本地切句识别，交给“同传”子会话翻译 |
| [022 会话、记忆与 SOUL](022-sessions-memory-soul.md) | Accepted / 已采纳 | 子会话（照 OpenClaw）、工作区（SOUL / USER / MEMORY / 游戏记忆 / Skill，照 Hermes） |
| [023 界面转向陪玩 Agent](023-ui-toward-companion-agent.md) | Accepted / 已采纳 | 侧栏分“语音输入 / 游戏陪玩”，「琴音」页分聊天、角色、记忆、技能、设置 |
| [024 前缀缓存、上下文压缩与用量账本](024-prefix-cache-compaction-usage.md) | Accepted / 已采纳 | 照 DeepSeek Harness：系统提示与工具清单一段对话不变、会变的写进每轮附注、只追加；照 Goose 压缩（请求形状照 dsh，整理也命中缓存）；usage.jsonl 记 token、命中、费用 |
| [025 角色、自定义唤醒词与对局复盘](025-roles-wake-words-match-recap.md) | Accepted / 已采纳 | 角色卡照 Character Card V2；唤醒词照 sherpa-onnx text2token 转拼音并校验；游戏进程退出时自动复盘写进游戏记忆（陪玩特有，自己设计） |
| [026 不要通用读写执行；按 DeepSeek 最佳实践对齐](026-no-generic-rwx-deepseek-best-practices.md) | Accepted / 已采纳 | 琴音没有 read / write / bash（陪玩特有；第 1 条由 030 修订）；联网搜索用 DeepSeek 搜索与抓取接口做成 `web.search` / `web.fetch`；DeepSeek 连接用自己的客户端：`latest_reminder`、`title`、`safety_check`、最后一块报错、内容审核拒绝 |
| [027 0.1.x 到 0.2.0 过渡](027-0.2-upgrade-transition.md) | 0.1.14 已发布（2026-11-19）；0.2.0 的发布与放量待玩家确认 | 旧用户优先保留语音输入，独立通道分桶，维护版升级前备份 |
| [028 按“游戏陪玩”重新设计交互流程和界面](028-game-companion-interaction.md) | Accepted / 已采纳 | 打开就是聊天页，一个 DeepSeek Key 打开全部功能；语音输入是子板块、不经过 Agent；“启动”只属于本地语音识别；「琴音 → 设置」并入「设置」；老用户可以一切照旧 |
| [029 像打电话一样和她说话：快速主会话 + 完整子代理](029-voice-call-and-background-tasks.md) | Accepted / 已确认，实现中 | 语音回合不思考、短回答、说话那一刻截屏、预生成等待短句、答完接着听 8 秒、按键打断、边生成边念；复杂的事交给玩家看不到的子代理（照 Codex / OpenCode / Goose），做完由主会话说一句 |
| [030 子代理可以执行命令、读写文件](030-subagent-shell-and-files.md) | Accepted / 已确认 | 主会话仍然没有；子代理有 `shell.run` / `file.*`，只能由玩家派；读过外面的东西（屏幕、聊天、网页、外部文件）就先问一句（照 Meta Agents Rule of Two）；删除类先问、模拟键鼠不让做（照 Codex execpolicy）；改文件先备份 |
| [031 内置浏览器与浏览器操作](031-built-in-browser.md) | Accepted / 已实现（随 0.2.0-rc.1；2026-11-21 改成完整浏览器：多标签、地址栏、扩展） | 后台任务在 Kotone 隐藏的 WebView2 里操作网页：快照和点击照 dsh-ego-browser（调试协议、节点编号、不抢前台），安全外壳照 dsh-browser 和 xylt369 url-guard；程序内调用、不开端口、不驱动玩家的 Chrome；登录由玩家扫码；第一个用处是点歌 |
| [032 往游戏聊天里发字，不被玩家的按键打断](032-game-chat-send-reliability.md) | Accepted / 已实现（第一、三期 0.2.0-rc.3，CS2 内置配置 rc.5）；真机验证大多等玩家 | 发送的约 220ms 里用已有的低层钩子吞掉玩家的按下，不替玩家补按任何键；新增 CS2 内置配置；Dota 2 / CS2 可由玩家打开“不开聊天框直接发”（写 `say_team` 到 cfg 再按一个绑定键，照 cs2-chat-translator）；英雄联盟局内没有官方写接口 |
| [033 呼叫就是通话：一直在听、开口就能打断、说“退下”挂断](033-call-mode.md) | Accepted / 已实现（0.2.0-rc.4） | rc3 真人测试反馈：呼叫键 / 唤醒 = 接通通话，一直在听；回声消除照 orb（扬声器回采作参考 + 纯 Rust 的 sonora AEC3）；她说话时开口先压低、识别出真话再停（空的、“嗯”、她自己的回声不算）；“退下 / 安静”随时挂断，30 秒没人说话挂断；用嘴问过的确认之后整句说“可以 / 不要”也算回答；修订 ADR 029 第五节 |
| [034 放歌收敛成工具：先只做网易云，Kotone 自己放](034-music-tools.md) | Accepted / 已实现，登录后的部分待真机验证 | 浏览器放歌慢、意外多：主会话的 `music` 换成真正的播放器（搜歌即放、每日推荐、我喜欢的、FM、暂停切歌）；网易云走非官方接口（ncmapi，照 api-enhanced）+ 扫码登录，Cookie 存凭据库；Rust 边下边放，她说话时压低；`MusicProvider` 留给以后的 QQ 音乐；删掉 `music-web` 技能和“放歌用哪个网站” |
| [035 英雄联盟工具：这局的玩家、战绩和召唤师技能](035-lol-tools.md) | Accepted / 实现中（随机英雄的模式不看英雄熟练度；不开游戏的真模型测试 `lol_tools_real.py`） | 一组 `lol.*` 工具：`lol.match`（官方局内接口：英雄、召唤师技能、KDA、装备）、`lol.spell`（玩家说了才记交的技能）、`lol.scout`（客户端本地接口查 10 人战绩，加载界面预查，工具算“突破口”、她说成信息）；只读、不请求服务器、不反匿名；接口读不到冷却（国服实测），玩家说了才记、照 LeagueAkari 手动计时；全部模式的战绩、隐藏战绩照读并标出来；默认开，界面提示风险（Riot 25.17 限制冷却计时、LeagueAkari 声明不支持国服） |
| [036 游戏状态和事件：所有游戏共用一层，自动化可以按游戏事件触发](036-game-state-and-events.md) | Accepted / 一期已实现（2026-11-23，真客户端待测）；二期（CS2 / Dota 2 的 GSI、外部 Agent 订阅事件）未开始 | 状态和事件分开（照 Overwolf）；事件信封照 CloudEvents，`game.*` 通用、`<游戏>.*` 专用；所有游戏共用的阶段（stopped / running / menu / queue / pre_match / loading / in_match / post_match，原生值保留）；每个游戏一个适配器，先声明能给什么、现在能不能收到；游戏中心合并分发，复盘和预查改成订阅；自动化加 `on`（照 Home Assistant / EventBridge）和第三种动作“让她做”（每次触发一次 AI 调用，内置模板默认关）；游戏启动自动切到它的配置；CS2 / Dota 2 的 GSI 文件默认写（二期） |
| [038 Kotone 是什么：她，加上装在她身上的插件（照 MCP Apps）](038-kotone-and-apps.md) | Accepted（方向）；部分由 039 修订 / D7 第 1–4 期做了（英雄联盟、网易云音乐、游戏语音、同传、语音输入是插件），第 5 期按键程序停在“只搬不改”，第 6 期提前做了（英雄联盟 App 经 `kotone.exe mcp` 对外、Kotone 里显示第三方 MCP App，039 D9）；进度见[插件重构计划](../plugin-refactor.md)第九节 | 一句话：一个陪你打游戏的角色，听得见、看得见、说得出、记得住，本事都装在插件里，人和外部 Agent 用同一套；本体 = 她、感官和表达（含读聊天）、自动化、宿主；插件 = 工具 + 自己的页面（MCP Apps：给她的文字和给页面的数据分开，只给页面用的工具不进她的清单），每个插件一个 `plugins/<id>/` 目录；英雄联盟、语音输入、游戏语音、同传、音乐、按键程序、浏览器 |
| [039 桌面陪伴 Agent，差异在游戏：输入归本体、自动卡片、英雄联盟做成独立的 MCP App](039-desktop-companion-and-game-apps.md) | Accepted（方向，2026-11-25）/ 实现中 | 定位：桌面陪伴 Agent，听、看、说是同类都有的，差异在游戏场景；文档讲架构用“本体 / 插件 / 琴音”；语音识别和往游戏聊天里打字归本体（`game_chat.send` 回到本体、名字不变，聊天键留在游戏配置）；内置插件能不叫模型往聊天里发自动卡片（附一段短文字、只追加），同时右下角弹消息提示（不抢焦点）；英雄联盟做成独立的 MCP 服务器 + App（对标 LeagueAkari、AramHex，视觉原创，素材不进安装包）；陪玩设置搬进“游戏陪玩 → 角色”，选模型回到聊天页；插件各有 logo，“音乐”改名“网易云音乐”；MCP 第 5 期提前；部分修订 038、029 |
| [040 快捷键归本体：插件只登记命令，一张表、一个地方改](040-hotkeys-belong-to-core.md) | Accepted（方向，2026-11-25）/ 第 1、3 期做完；第 2 期第 1–3 步做完，第 4 步（删主键位）等真机和真人按 | 照 VS Code、Obsidian、Raycast：插件登记命令（按一下 / 按住；插件动作、调工具、开页面），不抢键；“设置 → 快捷键”一页列出全部、标来源、只在这里改；默认键只有呼叫键、停下热键、语音输入的“说话”，第三方不带默认键，老用户已有的绑定不收回；快捷指令的 8 格并进表；冲突只在后端查一次，含游戏聊天键、F23、Esc；给有副作用的工具绑键时问权限 |
| [041 发到游戏：入口和通用发法归本体，每个游戏的发法归游戏插件](041-send-to-game.md) | Accepted（方向，2026-11-25）/ 第 1–3 期已实现；真机脚本（`send_to_game_e2e.py`、`lol_tools_real.py` 的 C1–C8）和真客户端待跑 | 细化 039 D2：`game_chat.send` 一个入口（加频道、多行、结果说清楚），通用的“回车—Unicode—回车”在本体，游戏插件可以提供自己游戏的发法（英雄联盟选人走客户端聊天接口，照 LeagueAkari），宿主按前台的游戏挑；打字、`game.act`、按键程序共用一把输入锁，入口里统一等修饰键松开；不用自动化中转；和琴音说话时转不转给队友不管（玩家：按键说话） |
| [042 共享资源归本体：每样一个管家](042-shared-resources-belong-to-core.md) | Proposed / 提议（2026-11-25）；第 0 期做了，麦克风之前的一小步（系统默认是 CABLE 时改用真麦克风）也做了 | 总则（040、041 的共同做法）：每样共享资源由本体里的一个管家拥有，插件只声明要什么、经宿主拿，规矩只写在管家里一次，设置只在一处，边界写成测试；已照做的七样：热键、发到游戏和输入锁、前台和打扰（`attention.rs`）、屏幕角落、她正忙时插进来（`when_free.rs`）、谁在出声的第一步、语音识别；排期按玩家感觉的轻重：麦克风 → 声音焦点（照 Android AudioFocus）→ 屏幕 → 聊天流，设置跟着收（“声音”一页、“提示和覆盖层”一处） |
| [043 插件形态与推送：插件 = App + 事件和内置推送 + 工具；覆盖层改成悬浮窗](043-plugin-shape-and-pushes.md) | Accepted（方向，2026-11-26）/ A 轮实现中 | 插件说“是什么”、琴音说“意味着什么”（去掉突破口）；推送有卡片 / 一行 / 语音三种样子，不调模型；同一个 key 再推一次就是更新（发给模型的只追加，通知里的卡片数据可以改）；清单声明推送、插件页每条一个开关、宿主执行；英雄联盟两个覆盖层改成选人卡和一行三列 + 语音；悬浮窗分气泡 / 小窗，不抢焦点；风险说明每个插件一个弹窗 |
| [044 默认以管理员身份运行；Kotone 打开的东西一律降回普通权限](044-run-as-admin-by-default.md) | Accepted（2026-11-26）/ 实现中 | 国服英雄联盟是管理员权限，普通权限的 Kotone 收不到按键、发不进字；默认每次启动点一次 UAC，第一次先解释；她跑的命令、第三方 MCP、链接、文件夹、游戏一律以普通权限启动；不做静默提权 |
| [045 自动化只剩程序；说一句、推送、交给她都是程序里的函数](045-automation-is-programs.md) | Accepted（2026-11-27）/ 已实现，待真机（见文末“实现情况”） | 玩家：“都做成函数，固有配置就可以轻松一些”。动作只剩程序，旧的播报 / 让她做读进来转成程序；`say(text, { toast })`、`toast`、`push`（记进聊天、悬浮窗一行、不叫模型）、`ask`（交给她，每次一次 AI 调用，事件触发每局 3 次，话里不能带事件填进来的值），不写就安静，热键成功不再弹幕；按源码判断用不用游戏窗口，只有用到的才绑窗口、占 3 个名额、看 `mode`、▶ 先倒数；停下照旧不取消等待中的提醒；`delay_seconds` 对所有自动化生效；新建先选模板；她不运行会叫她的自动化。修订 013、036 D6 / D8、043 D3、040 第 3 期 P2 |
