# 游戏陪玩 Agent 现状调研（2026-11-16）

为 [ADR 014](../adr/014-game-agent-harness.md)（Kotone 定位为游戏场景的 Agent 运行框架）提供依据。资料来自 2026-11-16 的网络检索，未逐一安装试用；星数、活跃度未核实。

## 1. 结论

“游戏陪玩 Agent”这个概念已经有人在做，但分成互不重叠的几块：

- 平台与硬件厂商做的是**问答、攻略、教练**，基本不替玩家动手；
- 国内商业产品主打**情感陪伴与建议**；
- 开源项目多停在**看与说**（聊天、虚拟形象、语音）；
- 研究型 Agent 让模型**自己玩**（像素进、键鼠出），离普通玩家很远。

空着的一块是：**替玩家在游戏里动手做事、但始终由玩家指挥**的 Agent——游戏场景的工具（读屏、输入、聊天、语音输出）+ 有护栏的自动化 + 内置 Agent + 对外部 Agent 开放全部工具。这正是 Kotone 已有能力的延伸。

## 2. 现有产品与项目

### 2.1 平台 / 硬件厂商（闭源）

| 产品 | 形态 | 要点 |
| --- | --- | --- |
| 微软 Gaming Copilot | Xbox Game Bar（Win+G），手机 Xbox App，2026 年上 Xbox 主机 | 打字或按键说话提问；语音模式可带上游戏截图；攻略、推荐（基于账号数据）；“Mini Mode” 悬浮持续对话。2025-09 在 PC 上线测试版 |
| NVIDIA Project G-Assist | NVIDIA App，本地运行（RTX） | 本地小模型 + **插件**：每个插件用 manifest 声明函数与参数（等同工具调用），官方开源 Python / C++ 插件样例（Twitch、Discord、Nanoleaf、IFTTT 等），有插件构建器与应用内插件下载。侧重系统 / 显卡 / 外设控制 |
| 雷蛇 Project AVA | CES 2025 概念（电竞教练）→ 2026 桌面伴侣（全息形象、PC 视觉模式） | 赛前建议、局内提示、赛后分析；**明确定位为教练，不做自动游玩**；2026 下半年发售（预订定金，未核实正式日期） |

### 2.2 国内商业产品

逗逗 AI、OBeeBee、Hakko AI 等：悬浮在桌面边缘读屏幕，夸操作、接梗、卡关时给语音战术指导；不绑定单个游戏。逗逗 AI 表示下一步把陪伴扩展到看剧、购物、网课等数字生活场景。

### 2.3 开源陪伴项目

| 项目 | 要点 |
| --- | --- |
| AiGameCompanion（Wintersta7e，及 swang666 的分支） | 透明置顶、空闲时鼠标穿透的游戏内悬浮窗，Ctrl+Shift+G 呼出，本地语音，多模型，MIT |
| airi | 自托管、本地优先的陪伴，实时语音，Live2D / VRM，可以玩游戏 |
| super-agent-party | “自托管 Neuro-sama + 工具”：Live2D、语音、游戏互动；主 Agent 可用 MCP、Skills、A2A、HTTP 等工具 |
| Open-LLM-VTuber | 语音驱动的 Live2D VTuber 框架（可打断、本地 LLM / TTS） |
| Win AI Buddy | Windows 桌面应用，看游戏画面、听语音，用 Gemini Live 实时指导 |
| OpenGameMate | 自称本地优先的 AI 游戏伙伴，路线图从语音陪伴走向完整 Agent（第二次检索未找到更多资料） |
| Wonderfon/game-companion | 检索摘要称其能把 Codex / Claude Code / WorkBuddy 变成陪玩伙伴；**专门检索未找到，未核实** |

共同点：以聊天、陪伴、观察为主；会动手的很少，有工具的（super-agent-party）是通用工具而非游戏场景工具。

### 2.4 研究型“自己玩”的 Agent

| 项目 | 要点 |
| --- | --- |
| Cradle（智源 BAAI，开源，约 2.2k 星） | 通用计算机控制：截图进、键鼠出，玩过 RDR2 等；效果强依赖底座模型与针对环境的配置 / 技能 |
| UI-TARS / Game-TARS（字节） | 原生 GUI Agent 模型；Game-TARS 用统一的键鼠动作空间做大规模预训练 |
| Lumine（2025-11） | 在原神里训练的 3D 开放世界通用 Agent，Qwen2-VL-7B，5 Hz 看画面、30 Hz 输出键鼠，可零样本迁移到鸣潮；未见公开代码 |
| GamingAgent（lmgame-org）、Orak（Krafton） | 游戏 Agent 与评测基准 |

## 3. 对 Kotone 的启示

1. **定位**：类比 Claude Code / Codex / dsh 是“写代码场景的 Agent 运行框架”（文件、终端工具 + Agent 循环 + 权限），Kotone 做“打游戏场景的 Agent 运行框架”：屏幕感知、键鼠输入、聊天、语音与弹幕、游戏配置等工具 + 自动化运行层 + 内置 Agent + 对外开放。
2. **差异化**（相对大厂免费、预装的助手）：能动手（自动化、帮忙打字）而不只给建议；多人游戏沟通（语音转文字发到聊天框、翻译）；中文游戏与国内环境；用户自选模型；工具对外部 Agent 开放（Kotone 已经给 dsh 开放工具）。
3. **借鉴**：
   - G-Assist 的插件 manifest（函数 + 参数声明）与 Kotone 的工具清单、已保存自动化的入参声明是同一种东西，以后可以让用户或第三方扩展工具；
   - Gaming Copilot 的“按键说话 + 截图 + 悬浮持续对话（Mini Mode）”是游戏内交互的成熟形态；
   - Cradle 一类说明：让模型逐帧操作很重，Kotone 的做法（模型写一次程序、程序毫秒级执行；需要理解画面时再回到 Agent）更适合玩家实时使用。
4. **风险**：
   - 反作弊与游戏规则：键鼠自动化的尺度因游戏而异（雷蛇刻意回避“自动游玩”）。用户的判断是模拟键盘本身不算作弊；但 Agent 能自己动手之前，要做好按游戏的“允许自动化输入”开关与明确提示；
   - 大厂产品免费且预装，只做问答 / 攻略没有优势，必须在“能动手、开放、中文沟通”上做深。

## 4. 资料来源

- [ai-companion · GitHub Topics](https://github.com/topics/ai-companion)、[game-companion · GitHub Topics](https://github.com/topics/game-companion?l=python)
- [swang666/AiGameCompanion](https://github.com/swang666/AiGameCompanion)、[heshengtao/super-agent-party](https://github.com/heshengtao/super-agent-party)、[DasterProkio/awesome-ai-companion](https://github.com/DasterProkio/awesome-ai-companion)、[OpenGameMate](https://opengamebot.ai/)
- Gaming Copilot：[Generación Xbox](https://en.generacionxbox.com/gaming-copilot-arrives-at-xbox-game-bar-this-is-how-the-ai-that-helps-you-while-you-play-works-now-available-in-spain-and-latin-america/)、[GamingBolt](https://gamingbolt.com/gaming-copilot-is-coming-to-xbox-series-x-s-in-2026)、[BetaNews](https://betanews.com/2025/08/07/microsoft-brings-ai-to-the-game-bar-with-gaming-copilot/)
- G-Assist：[NVIDIA/g-assist](https://github.com/NVIDIA/g-assist)、[NVIDIA Blog：Plug-In Builder](https://blogs.nvidia.com/blog/rtx-ai-garage-g-assist-plugin-builder)、[NVIDIA：G-Assist](https://www.nvidia.com/en-us/geforce/news/g-assist-ai-companion-for-rtx-ai-pcs/)
- Project AVA：[Razer](https://www.razer.com/mena-en/concepts/project-ava)、[PCWorld](https://www.pcworld.com/article/2570095/razers-project-ava-ai-esports-coach-could-make-you-an-esports-superstar.html)、[Hypebeast](https://hypebeast.com/2026/1/razer-project-ava-ai-companion-3d-hologram-kira-zane-faker-sao-release-info)
- 研究：[Cradle](https://arxiv.org/pdf/2403.03186)、[Lumine](https://arxiv.org/abs/2511.08892v1)、[Game-TARS](https://arxiv.deeppaper.ai/papers/2510.23691v1)、[lmgame-org/GamingAgent](https://github.com/lmgame-org/GamingAgent)
- 国内行业报道：[投资界](https://news.pedaily.cn/202607/566964.shtml)、[36氪](https://www.36kr.com/p/3916178328039048)
