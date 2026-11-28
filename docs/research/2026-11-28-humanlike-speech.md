# 让她说话更有活人感：调研

2026-11-28。三路调研：DSH 社区的"养鱼"和人格插件、中文拟人机器人项目（MaiBot、N.E.K.O 等）、国外角色扮演 / AI 主播 / 论文。标注：**源码**＝读过代码或原文；**帖子**＝只看了文章或 README。下载的仓库在会话临时目录 `scratchpad\humanlike\{dsh,cn,global}`，重启后可能被清掉，结论都写在这里。

## 结论

- 她现在不像活人，主要不是模型的问题，是**提示词把她写成了调度员**：人格只有两行形容词（`crates/kotone-agent/src/workspace.rs:34`），通话规则是"最多两句、40 字、只说结论"（`crates/kotone-agent/src/prompt.rs` 两个入口那段），没有一句示范，也没说她可以有态度。
- 成熟项目的共识：**示范比形容词管用**、**允许她不说**、**状态改变行为而不是台词**、**套话靠事后检测而不是禁用词表**、**人设别用力过猛**。
- 建议分三批做（第四节）。第一批只改提示词和一个参数，成本最低、效果最大；第二、三批是小机制。错别字、表情包、打字延迟、好感度数值这些流行做法不适合语音，不做（第五节）。

## 一、"养鱼"是什么

**帖子**：大肥鱼是社区拿 DeepSeek 蓝鲸 logo 二创的角色（Kotone 已有这个角色，`roles.rs:61`）。"养鱼"＝给她写人设，让她接管 QQ / 微信 / 飞书陪聊、刷 B 站、发说说，"很有那种养了一只电子宠物的感觉"（[卡兹克教程](https://www.163.com/dy/article/LAQ13US4055627NS.html)、[虎嗅 11-14](https://www.huxiu.com/article/4905405.html)）。

**源码**：DSH 自己**没有**人格、情绪、好感度；系统提示只有一句 `You are an AI agent powered by DeepSeek Harness.`（`packages/core/system-prompt/src/index.ts`）。活人感全在插件里。按 star 排，最热的是外形（余额挂件、桌宠、皮肤），真正做人格的几个：

| 插件 | ★ | 最值得学的 |
|---|---|---|
| mochi-fish/dsh-fish | 196 | 人格不写形容词，只写事实（"第几天、你说过什么"），代码里用正则禁止"你是一个…的"（`lib/persona.js`）；主动开口只给事实和许可："想不想开口由你……不想说，就什么都别写"，每天最多 2 次、安静满 6 小时（`lib/proactive.js`）；统计短句里"确实""有道理""我记住了"的占比当健康指标（`lib/health.js`） |
| Britneycode/dsh-fish-soul | 53 | 心情改行为不改台词：生气时回得慢、插话少，提示写"体现在快慢长短语气上，别直接说出来"（`lib/behavior.js`）；发送前正则删"作为 AI""希望对你有帮助""综上所述""好问题"和括号动作 |
| Yuuyuko-uu/dsh-feiyu-mood | 278 | 每轮后另调一次模型判断心情；变化够大才注入、一天最多 6 次、只说一遍，"这是这会儿的底子，不是非这样不可"；注释说每轮都带状态会让她"性格单一、抑郁呆板" |
| Yuuyuko-uu/dsh-fish-buddy | 79 | 和 Kotone 最像（桌面 + 语音 + 游戏）：每轮会变的字幕挂在消息末尾、不进系统提示（否则缓存命中跌到 3%）；随机搭话先掷骰子（600 秒一次、命中 0.35、冷却 60–150 秒、8 秒没人说话才算安静），不花 token |
| MX3a48c5/dsh-fish-tone | 37 | 用 10 万条 B 站评论量真人句式密度：真人"不是 X 而是 Y"约每千字 0.06 次，AI 是 5–37 倍；"犹豫是半秒，不是一句""事实不动" |

## 二、中文拟人项目

| 项目 | ★ | 最值得学的 |
|---|---|---|
| MaiBot | 7.1k | **说不说**和**怎么说**分开：规划器"你也可以单纯的沉默"，回复是一个工具，不调就是沉默，篇幅分三档；表达风格"平淡简短，可以参考贴吧……不滥用比喻……视情况省略主语"（`src/config/official_configs.py`）；从聊天里学"当 A 时可以 B"的句式和黑话，**只从用户发言里学** |
| N.E.K.O | 3.3k | 语音陪伴。"不要询问'我可以为你做什么'""不要重复已经说过的片段""真人也会表达自己的兴趣"（`config/prompts/prompts_chara.py`）；**竞技游戏在前台用 terse 语气档**，只做即时反应、起哄吐槽、一句短战术（`main_logic/activity/snapshot.py`）；主动搭话不想说回 `[PASS]`；从她自己的历史回复里挖跨会话重复的说法，判成口癖的进回避列表（`memory/habitual_phrases.py`）；**只改喂回模型的历史里的套话**，不改玩家看到的（`utils/slop_filter.py`），理由是模型会模仿它看到的对话风格 |
| chatluna-character | 82 | 模型每轮自己写一段 `<status>`（心情、状态、近期记忆）带到下一轮，比好感度数值简单 |
| AstrBot Heartflow | 90 | 插不插话按五个维度打分 + "精力值"，每说一次精力 -0.1 |
| nekro-agent | 1.2k | "角色风格影响的是你'如何表达'，而不是'什么是真实的'"（`j2_zh/persona.j2`） |
| KouriChat / WeClone | 3.3k / 18.4k | 分条发送、聊天记录 LoRA 微调——文字聊天用，语音不适合 |

去 AI 味的写作类技能（Humanizer-zh 23.9k、lieflat-less-ai-tone 9.1k）有一条反直觉的实测：**"就 / 很 / 了"和口语连接词人类用得更多，方向是补不是删**；光靠禁用词表会误伤，删完仍有 AI 痕迹。

## 三、国外项目与研究

- **示范对话**（SillyTavern **帖子**）：模型"最可能从第一条消息里学走风格和长度"；Ali:Chat 流派用对话本身表现性格，不写形容词。**Author's Note**："越靠近提示末尾，对下一条回复影响越大"，可每 N 轮插一次。
- **人设漂移**（**源码**，[2402.10962](https://arxiv.org/abs/2402.10962)）：8 轮内明显漂移，原因是注意力衰减——支持把一句风格提醒放在靠后的位置。
- **DeepSeek V4 官方角色扮演指令**（**源码**，DeepSeek 员工发布）：指令追加在第一轮 user 消息末尾，"这是训练时的注入位置，效果最稳定"，放 system 里不如这里。同仓库的用户反馈报告（861 条）里最多的是模板化输出，高频口癖："不是…而是…""这就够了""稳稳接住""总而言之""频繁说谢谢""频繁催促用户睡觉"；"禁止词列表几乎不起作用"；"过于正确：角色鲜有生气、嘴硬"。
- **温度**（**源码**，[参数页](https://api-docs.deepseek.com/quick_start/parameter_settings)）：官方推荐通用对话 1.3。Kotone 通话默认不思考（`kotone-core/src/agent.rs` `DEFAULT_GAME_EFFORT`），但请求里不传温度（服务商默认 1.0）。API 没有 logit_bias，`frequency/presence_penalty` 已不生效，所以 antislop 采样器这类做法用不上。
- **图灵测试**（**源码**，Jones & Bergen [2405.08007](https://arxiv.org/html/2405.08007)、[2503.23674](https://arxiv.org/html/2503.23674v1)）：GPT-4.5 有人设提示 73% 被判为人、没有 36%。获胜提示："very concise and laconic""not very knowledgeable…not afraid to admit""never want to sound like you're forcing it"。被识破的主要原因之一就是 **forcing a persona**——人设用力过猛同样露馅。
- **AI 主播**（**源码**）：airi 的人格提示"Forget about everything as being a helpful assistant"，不知道就说不知道，反对时不用"不过…"软化，不过度附和；Neuro 复刻版"一句左右，对方不接问题就换话题"；Neuro SDK 的游戏事件分 `silent`（只进上下文）和非 silent（她"可能"回应），"不要连续推小事件"。OpenAI 实时语音指南：模型会照搬示例短语，加一条"Variety"规则能修好。
- **啰嗦和谄媚**是 RLHF 的副作用（[2310.03716](https://arxiv.org/abs/2310.03716)、[2310.13548](https://arxiv.org/abs/2310.13548)），要在提示里明确反着写。

## 四、给 Kotone 的建议

先对一下已有的：打断时只记玩家听到的部分（`apps/desktop/src-tauri/src/voice_reply.rs:481`，和 Open-LLM-VTuber 一样）、推送语音分级 + 冷却 + 5 秒过时（`crates/kotone-agent/src/push_voice.rs`，和 Neuro SDK 的思路一样）、每轮会变的放附注不进系统提示（和 dsh-fish-buddy 一样）——这些已经做了，不用动。

### 第一批：只改提示词和一个参数

1. **SOUL 改成示范对话**（SillyTavern、Ali:Chat、dsh-fish）。琴音和大肥鱼各写 8 组左右"玩家说 / 她回"，覆盖：死了、被单杀、赢了、输了、问出装、问版本、闲聊、玩家骂队友。形容词只留一行，加一句"示范只是语气，不要照抄原句"（OpenAI 的 Variety 规则）。玩家自己写的角色不动。
2. **通话规则删"调度员"腔，补"活人"的许可**（N.E.K.O、airi、Jones & Bergen、DeepSeek 反馈报告）。"只说结论"改成：可以先接一句反应再说事，一两个字也行；可以有态度、直接不同意、吐槽；不知道就说不知道；不说谢谢、不客套、不总结升华、不追问、不催玩家休息；少用"不是…而是…"。字数上限保留。
3. **附注末尾加一句风格提醒**（Author's Note、DeepSeek V4 注入位置、8 轮漂移）。只在通话入口加，一句话，比如"用 {角色名} 平时的口气，短"。附注本来就在用户消息里、只追加，不破坏前缀缓存。
4. **通话不思考时传 `temperature: 1.3`**（DeepSeek 官方推荐值）。思考模式下照旧不传。改 `crates/kotone-ai/src/agent/deepseek.rs` 一处，测试 `tests.rs:2339` 要跟着改。

**怎么验收**：真 Key 跑一组固定的 20 个游戏时刻（第 1 条那几类），新旧提示各跑 3 遍。看两样：一是盲听对比；二是数指标——平均字数、问句结尾比例、"谢谢 / 确实 / 不是…而是 / 总之"每千字次数、同一句话跨样本重复率（dsh-fish-tone、dsh-fish 的做法）。

### 第二批：小机制

5. **口癖回避表**（N.E.K.O `habitual_phrases.py`）。每天从她的历史回复里找跨对话反复出现的说法，模型判断是不是口癖，"拿不准选不是"；判成口癖的写进系统提示的一小节。系统提示每段对话开头生成一次，不影响缓存。
6. **喂回模型的历史去套话**（N.E.K.O `slop_filter.py`）。她的回复存进历史时，第二次起出现的套话换掉；玩家看到、听到的不改。在消息存入时做一次，之后前缀不变，缓存照样命中。规则表要写口语客服腔（"好的呀""加油哦""有什么需要随时叫我"），N.E.K.O 那套小说腔对短口语基本不命中。
7. **允许她不说**（MaiBot、dsh-fish、N.E.K.O `[PASS]`）。自动化里 `ask()` 叫她的那一轮，她可以回一个空的 speak 或 `[PASS]`，什么都不出声、不记"好了"。这让"陪看"类自动化（例如击杀 / 被击杀后叫她一下）不用每次都说话，像真人一样有时候沉默。频率和冷却交给自动化本身，和"判断人的事只走自动化"的边界一致。

### 第三批：要多一点设计的

8. **对局心情**（dsh-feiyu-mood、dsh-fish-soul）。从游戏事件里算出几个事实（连败、连胜、刚超神），**变了才**在附注里说一句，写明"是底子，不是非这样不可，别念出来"，影响话多话少和语气，不影响台词。不做好感度数值。
9. **自然提起旧事**（MaiBot、Neuro 复刻、dsh-fish）。记一局里值得记的一两件事（证据是事件或玩家原话），之后在相关的时候提一句"上把那个盖伦……"。记忆要会忘：被反复提起才留下。和现在的 MEMORY.md / 游戏记忆怎么合，需要单独设计。

## 五、不做的

| 做法 | 为什么不做 |
|---|---|
| 错别字、补发更正 | TTS 念不出错字的效果，只会念错 |
| 分条发送、打字延迟 | 语音要尽快开口；切句只用于流式 TTS，已经有了 |
| 表情包、颜文字、星号动作、括号旁白 | 念不出来 |
| 标点改成"？？？"、去句号 | 破坏 TTS 的语调 |
| 好感度数值 | 规则多、玩家看不见，`<status>` 一句话就够 |
| 提示里放禁用词表 | DeepSeek 反馈报告和中文社区都说几乎不起作用，还会误伤；改用第 5、6 条的事后处理 |
| antislop 采样器 | DeepSeek API 没有 logit_bias |
| 用聊天记录 LoRA 微调（WeClone） | 我们用 API，不训练模型 |
| 群聊活跃度打分、随机插话、注意力漂移 | 玩家在打游戏，只会让人分心 |
| 网络用语、口癖高频出现 | 图灵测试里"用力过猛"本身就露馅；低频即可 |

## 来源

- DSH：[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（4b399c39）、[mochi-fish/dsh-fish](https://github.com/mochi-fish/dsh-fish)、[dsh-fish-soul](https://github.com/Britneycode/dsh-fish-soul)、[dsh-feiyu-mood](https://github.com/Yuuyuko-uu/dsh-feiyu-mood)、[dsh-fish-buddy](https://github.com/Yuuyuko-uu/dsh-fish-buddy)、[dsh-fish-tone](https://github.com/MX3a48c5/dsh-fish-tone)
- 中文：[MaiBot](https://github.com/Mai-with-u/MaiBot)、[N.E.K.O](https://github.com/Project-N-E-K-O)、chatluna-character、[AstrBot](https://github.com/AstrBotDevs/AstrBot) + Heartflow、nekro-agent、KouriChat、WeClone、[Humanizer-zh](https://github.com/op7418/Humanizer-zh)、[lieflat-less-ai-tone](https://github.com/larashero3-dotcom/lieflat-less-ai-tone)
- 国外：[Character Card V2](https://github.com/malfoyslastname/character-card-spec-v2/blob/main/spec_v2.md)、[SillyTavern 文档](https://docs.sillytavern.app/usage/core-concepts/authors-note/)、[antislop-sampler](https://github.com/sam-paech/antislop-sampler)、[moeru-ai/airi](https://github.com/moeru-ai/airi)、[Open-LLM-VTuber](https://github.com/Open-LLM-VTuber/Open-LLM-VTuber)、[kimjammer/Neuro](https://github.com/kimjammer/Neuro)、[VedalAI/neuro-sdk](https://github.com/VedalAI/neuro-sdk)、Letta 0.6.0 `memgpt_chat.txt`、[DeepSeek 参数](https://api-docs.deepseek.com/quick_start/parameter_settings)
- 论文：[2405.08007](https://arxiv.org/html/2405.08007)、[2503.23674](https://arxiv.org/html/2503.23674v1)、[2402.10962](https://arxiv.org/abs/2402.10962)、[2310.03716](https://arxiv.org/abs/2310.03716)、[2310.13548](https://arxiv.org/abs/2310.13548)、[2401.01275](https://arxiv.org/abs/2401.01275)
