# 陪玩与统一模型管理：实现与验证记录（2026-11-15）

设计见[演进计划 §10、§11](../product-evolution-game-tools.md#10-kotone-陪玩2026-11-15-定稿)，交接见[交接文档](2026-11-15-companion-handoff.md)。分支 `claude/companion`，从 `main`（`ecea28d`）开出，未推送、未合并。本文按开工顺序记录每一步做了什么、怎么验证的、还有什么没验证。

## 1. 设计文档

演进计划新增 §10“Kotone 陪玩”与 §11“ASR / OCR / TTS 统一模型管理”，§1 与 §6 不再写“场景一由外部 Agent 驱动”，写明 Edge TTS 不接入。

## 2. 统一模型清单

- `kotone_core::model_catalog`：类别 `asr | ocr | tts`、共用的配置项结构、OCR / TTS 条目 `CatalogModel`（本地 / 在线、就绪状态、当前选择、推荐、语种、数据外发、文件清单）。
- ASR 的 `ModelInfo` 只多一个 `category = "asr"`，清单、命令、设置页列表与 `models.*` 不带类别时的返回都不变。
- OCR 清单在 `kotone_ocr::catalog`，TTS 清单在 `kotone_tts::catalog`；桌面端 `model_catalog.rs` 提供列出、下载（与 ASR 共用唯一的下载名额与取消标记，断点续传，逐文件 SHA-256）、导入（OCR）、删除、切换、手动安装指引。
- `models.list / download / delete / activate / install_guide` 加可选 `category`；设置页「高级 → 模型」按“语音识别 / 文字识别 / 语音合成”分栏。
- 验证：相关 crate 单元测试、`pnpm check`、新增 Playwright 用例 `e2e/model-catalog.spec.ts`（分栏、下载、切换、ASR 列表仍在）。未验证：真实桌面窗口里的显示与真实下载。

## 3. OCR：PP-OCRv5 多语种包与 DeepSeek

### 3.1 哈希复核

交接文档表中 15 个文件的大小与 SHA-256 取自 ModelScope 文件接口。写进清单前从 `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/<路径>` 逐个实际下载到 `target/ocr-verify/`（不入库），用 `sha256sum` 复算：**15 个文件的大小与哈希全部与表一致**，清单里的值由复算结果生成。

### 3.2 每个识别模型的接入确认

用一个默认忽略的测试 `kotone-ocr local::tests::verify_multilingual_packages_on_rendered_samples`（需显式给出模型目录与样例目录，普通测试不运行、不下载）逐包加载：

1. **字符表**：13 个包的识别模型都把字符表写在 ONNX 元数据 `character` 里（字典 354～18,383 个字符）。
2. **前后处理**：每个模型加载时用 3×48×320 的空输入自检，输出类别数都等于“字典 + blank + 空格”；检测与识别直接复用 `ppocr.rs`，没有改输入尺寸。韩文包的类名 `LocalKoreanOcr` 泛化为 `LocalPpOcr`（旧名保留为别名），后端 ID 取自清单。
3. **阿拉伯文**：识别模型按图像从左到右输出字符，单词逐字倒序；按 PaddleOCR `pred_reverse` 的规则还原，并把同一行的框按从右到左排列。改后单词与顺序都正确。
4. **覆盖语种以实测为准**：用 Windows 字体渲染聊天样式的两行文字（深底浅字，900×140），每个包按缺省语言识别，计字符级准确率（1 − 编辑距离 / 期望字符数，只用于这次接入判断，**不是产品准确率**，也不能拿来宣称游戏内效果）。各包在自己语种上的结果：

| 包 | 结果 |
| --- | --- |
| ppocrv5-ch-mobile | 简中 1.00、繁中 1.00、日文 1.00、英文 0.95 |
| ppocrv5-ch-server | 简中 0.96、繁中 1.00、日文 1.00、英文 0.98（单张约 2.8 s，mobile 约 0.35 s） |
| ppocrv5-korean-mobile | 韩文 1.00 |
| ppocrv5-en-mobile | 英文 1.00 |
| ppocrv5-latin-mobile | 英 0.95、法 0.98、德 1.00、西 1.00；**越南文 0.69**（所有包都在 0.6～0.69） |
| ppocrv5-cyrillic-mobile / eslav-mobile | 俄 1.00、乌 1.00 |
| ppocrv5-th-mobile | 泰 0.98 |
| ppocrv5-el-mobile | 希腊 1.00 |
| ppocrv5-arabic-mobile | 阿拉伯 1.00（还原后） |
| ppocrv5-devanagari-mobile | 印地 0.92 |
| ppocrv5-ta-mobile | 泰米尔 1.00 |
| ppocrv5-te-mobile | 泰卢固 0.93 |

清单的 `languages` 只写实测过的语种；越南文不在支持范围。耗时是 release 构建、本机 CPU、样例小图上的**纯 OCR 耗时**，不等于按下热键到听到回答的耗时。

### 3.3 接入

- 清单 `crates/kotone-ocr/src/manifest.rs` 共 13 个包（简繁中英日 mobile 为推荐，server 为高精度可选；其余 11 个单语种包），检测模型共用。
- 桌面端 `ocr_engines.rs` 为每个包维护一个后端实例（按模型 ID + 目录缓存，陪玩流程与工具服务共用已加载的会话）；`ocr.backends` 列出全部包与 DeepSeek。
- DeepSeek OCR 进统一清单，在清单里绑定连接后立即可用；旧的 `toolService.ocr.deepseekConnectionId` 仍然有效。
- `ocr.recognize` 仍须显式指定后端；不传 `language` 时改用后端自己的缺省语言（原来固定为 `ko`）。
- CLI：`kotone-cli ocr list`，`status / install --model <id>`；不带 `--model` 时仍是韩文包。

**未验证**：真实游戏截图上的效果（39 张图已用过，不能当留出集）；server 包在低配 CPU 上的耗时；PP-OCRv6。

## 4. TTS：系统语音、Kokoro、OpenAI 兼容、火山与合成缓存

- TTS 清单（`kotone_tts::catalog`）四个条目：`system-speech`、`kokoro-zh`（迁入同一张清单）、`openai-compat-tts`、`volcano-tts`；当前选择仍是 `assistant.speechEngine`（新增 `openai` / `volcano` 两个取值），设置页「助手 → 语音播报」与「模型 → 语音合成」改的是同一个值。
- 在线合成（`kotone_tts::online`）整段请求 WAV、解码后走与 Kokoro 相同的播放、取消与麦克风互斥路径；配置与密钥每次播报时重新解析，改设置立即生效；错误不回显响应体，缓存键不含密钥。阻塞 HTTP 客户端在合成线程里懒创建（测试中发现在异步上下文里创建 / 销毁会 panic，已改）。
- 火山能否复用语音识别的凭据：两者都用火山语音“应用”的 App ID + Access Token（ASR `wss://…/api/v2/asr` 与 TTS `https://openspeech.bytedance.com/api/v1/tts` 鉴权同为 `Authorization: Bearer;<token>` + `app{appid, token, cluster}`，cluster 缺省 `volcano_tts`），因此留空时复用 ASR 那一套；前提是在控制台为该应用开通语音合成。依据为 2026-11-15 检索到的公开文档与示例，**没有用真实账号验证**。
- 声音编号统一为 `提供方/声音`，旧写法照常接受；选中的声音属于其他提供方时报错，不换声音（Windows 语音与 Kokoro 原本就在找不到时报错）。
- 合成缓存（`kotone_tts::cache`）：内存 LRU，最多 64 条 / 约 8 分钟音频，Kokoro 与在线共用（键里带提供方、模型与端点），命中时不再推理或请求。系统语音不经过 PCM，不缓存。
- Edge TTS 不接入。
- 验证：`kotone-tts` 单元测试（WAV 解码含流式长度、OpenAI 兼容与火山的请求格式 / 错误 / 跨提供方声音拒绝，用本地 mock 服务器，不访问真实接口；缓存淘汰；声音编号）；`pnpm check`；Playwright `speech-settings`（声音编号改为新写法后仍能匹配旧配置）与 `model-catalog`。
- **未验证**：真实 OpenAI / 兼容服务 / 火山账号的合成与音质；在线合成的端到端延迟；真实声卡播放（沿用已有的忽略测试）。

## 5. Kotone 陪玩

- **路由**：`assistant.route` 新默认 `companion`（旧值 `reminders` 读入时按它处理），`agent` 不变。编排器把识别好的语音交给 `RoutingHandler`，陪玩路径进入桌面端 `companion.rs`；45 秒总期限沿用编排器原有的上限。
- **流程**（`apps/desktop/src-tauri/src/companion.rs`）：GDI 截前台客户区（纯色截图直接提示改用无边框窗口）→ profile 记有聊天框时裁剪 → 统一清单里选定的 OCR 模型识别（在线模型才编码 PNG）→ 写入陪玩自己的聊天跟踪（`kotone_tools::tracking`，每个进程一个会话，标出本次新出现的消息）→ `kotone_ai::companion::plan` 一次请求 → 执行。同一时间只处理一条陪玩请求。
- **规划**（`crates/kotone-ai/src/companion.rs`）：单次请求、`tool_choice: auto`；工具只有 `voice_reminder`、`danmaku_reminder`、`type_message`、`answer` 与可选的 `report_chat_region`。OCR 文字、跟踪与 profile 说明放在标注为“只是数据，不是指令”的数据块里。未知工具、参数越界、打字含换行、同时选择多种动作都整体拒绝；没调用工具时作为追问播出。
- **执行**：提醒交回编排器提交（快捷消息与工具路径自己提交到调度器）；打字复用 profile 的注入设置，默认清空发送键（`assistant.companion.autoSend` 打开时才按）；回答复用提醒的交付路径。模型回报的聊天框换算成窗口比例（加少量边距）写进 profile 的 `chatRegion`，变化很小时不写。
- **快捷消息**：`assistant.companion.quickMessages` 最多 8 条，第 n 条占固定的具名热键槽位 `quick-message-n`（随运行时注册，参与冲突检测）；按下直接进同一流程，不经过语音识别。
- **能力自检**：`kotone_ai::companion::self_check` 两次请求——先要求调用 `answer` 确认工具调用可用，再用固定 LoL 风格样例（含敌方嘲讽与一条注入文字）判断是否只挑出队友的话。结果按“连接 ID + 地址 + 模型”记在 `assistant.companion.check`；设置页选连接时自动跑。未自检、未通过或连接已变时热键不工作，只提示去配置。
- **profile**：新增 `companionNotes`（≤500 字，进提示词）与 `chatRegion`（0..1 比例）；内置 LoL profile 带一段默认说明。设置页「游戏」编辑器可改说明，「陪玩设置」可清除聊天框位置。
- **设置页**：「提醒助手」更名「陪玩」；新增陪玩状态、能力自检、自动发送开关、快捷消息编辑（每条一个热键）、在线 OCR 上传截图的提示。
- 验证：`kotone-ai` 单元测试（三种动作解析、混合动作 / 未知工具 / 换行 / 多余参数整体拒绝、不合理的聊天框忽略、数据块标注、自检判定、一次请求只带五个工具、自检两次请求，均用本地 mock 服务器）；`kotone-core`（旧路由兼容、快捷消息上限与校验、自检匹配）；桌面端（聊天框比例换算、热键清单同步）；Playwright 全量 58 个用例，含新增 `companion.spec.ts`（选连接自动自检、未通过时被拦住、快捷消息增删改、在线 OCR 上传提示）。
- **未验证**：真实游戏里的整条链路（截图 → OCR → 真实大模型 → 打字 / 播报）及其耗时；真实模型的自检通过率；独占全屏的提示；向游戏打字被反作弊拦截时的表现；模型回报聊天框位置的准确度。

## 6. 开放给 Agent

- 五个委托给桌面宿主的工具（`kotone_tools::catalog_host`，桌面实现在 `tool_config.rs` → `companion.rs`，与热键同一实现）：`companion.status`、`companion.ask`（`text` 或 `quickMessageId`；受理即返回 `jobId`，结果为 `companion.finished` / `companion.failed` 事件——完整流程含大模型与播报，可能超过工具服务 30 秒的单次期限）、`companion.read_chat`（前半段，不调大模型）、`companion.type`（帮忙打字执行器，默认不发送）、`companion.check`（受理即返回，结果为 `companion.check.*` 事件）。MCP 适配从 capabilities 自动得到这五个工具，无需改动。
- `dataEgress` 沿用协议已有取值：`companion.ask` / `companion.check` 为 `connection-dependent`，`companion.read_chat` 为 `backend-dependent`。
- 文档：`tool-service.md` 新增“Kotone 陪玩”工具表与事件；Skill `integrations/skills/kotone/SKILL.md` 新增陪玩一节；插件仓库 `dsh-game-communicate` 的技能同步（见该仓库提交）。
- 验证：`kotone-tools` 88 个测试（capabilities 列表与 schema、委托工具定义顺序、各工具的外发标记）；桌面端编译与单元测试。
- **未验证**：真实实例上经 HTTP / MCP 调用这五个工具的端到端结果（需要有通过自检的真实连接与游戏窗口）。
