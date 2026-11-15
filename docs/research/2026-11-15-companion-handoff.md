# 交接：Kotone 陪玩与统一模型管理（2026-11-15）

本文交给下一个会话继续。前半部分是已完成的状态，后半部分是已和用户定下、尚未开工的设计。

## 1. 方向修正：先有完整功能，再开放给 Agent

截至本轮，截图、裁剪、OCR、聊天跟踪等底层能力已作为工具开放给外部 Agent，但 Kotone 自己没有任何面向用户的功能用到它们。用户只装 Kotone 时，用不上 OCR；"队友在说什么"只能靠外部 Agent 现场拼工具。

用户的结论：**每项能力先作为 Kotone 自己的功能完整可用，再整体开放给 Agent。**下面第 4 节的设计就是按这个顺序定的。DSH 插件与桌宠接入放在后面，已完成的部分保持现状。

## 2. 已完成并已合入 main 的工作

分支 `claude/game-tools-m2` 已快进合并到 `main`，未推送远端。

| 提交 | 内容 |
| --- | --- |
| 96efdb7 / c02c471 / f49031f | M2：预处理、本地 / 在线 OCR、聊天跟踪 |
| 00e2b21 | README 写明"为 Agent 而生"，规划全量服务化 |
| 94b51a5 / 7080707 / c190b4c | Agent 底座第一批：应用启停、事件总线、`user.request` / `agent.reply` 反向通道、MCP、Skill |
| 9b09d1d | DSH 插件移出本仓库 |
| 68ec168 | `kotone.exe <子命令>` 兼作 CLI 和 stdio MCP，安装版不再需要单独的 kotone-cli |
| 2d94a8c | 设置了 `KOTONE_HOME` 的隔离实例使用独立的凭据命名空间 `kotone@<KOTONE_HOME>` |
| 2aa8132 | 35 个与设置页对齐的配置类工具，经 `AppHost::invoke` 调用设置页同一批命令函数 |
| 41defcf | 配置类工具与 Agent 配置流程的文档 |

细节见 [Agent 底座第一批](agent-base-batch1-implementation.md)、[第二批](agent-base-batch2-implementation.md) 和 [工具服务手册](../tool-service.md)。

**真机上仍未验证**：`autostart.set`、`elevation.restart`、`models.dir.set`、大模型下载、设置窗口收到 `kotone://settings-changed` 后的刷新效果。

### 独立仓库 `C:\Users\A\Desktop\dsh-game-communicate`

DSH 插件"游戏交流"，git 分支为 main，没有配置远端，未推送。最新提交 c31f813。已包含：

- MCP 启动器：装了 Kotone 时转发给 `kotone.exe mcp`；没装时只提供 `kotone_setup_status` / `kotone_install`；
- 体检；
- 经 minisign 验签的一键静默安装：拒绝低于 0.1.14 的版本，而当前公开的最新版是 0.1.13；
- 设置面板；
- 两个技能 `game-communicate`、`kotone`。

`npm test` 26/26 通过，`test/e2e/run.mjs` 8/8 通过，`test/e2e/setup.mjs` 通过。**未验证**：面板在 DSH 界面里的实际显示、真实安装、DSH 桌面版配置（Electron + `ELECTRON_RUN_AS_NODE`）。

### 未提交的本地文件

`.archify/architecture-kotone-game-communicate-20261115-193552/` 是用 archify 画的 Kotone + 插件架构图，仅存本地，未纳入版本库。

## 3. 用户的长期约定

- **Agent 全量开放，不做分级授权**。风险只靠这几条控制：密钥只写不读、用产物 ID 而不是任意路径、所有权与互斥、如实的回执。
- **"验证完了直接铺开"**：第一批证明可行之后，其余部分不必再逐项确认。
- **截图黑屏不处理**：独占全屏截到黑屏时，提示用户改用无边框窗口即可。
- **不做测试集**：提示词把场景写清楚，交给大模型发挥。那 39 张图已经用过，不能再当留出集，也不能拿旧分数宣称产品成功率。
- **OCR 耗时不等于完整的可播报耗时**，不要混为一谈。

## 4. 已定设计：提醒助手升级为"Kotone 陪玩"

### 4.1 两种入口，同一条流程

- **陪玩热键**：沿用现在的助手热键，按住说话。创建提醒和帮忙打字两类场景必须有用户说的话，否则不知道要提醒什么、打什么字。
- **快捷消息热键**：用户提前配好一句话，例如"帮我看看队友说了什么"，并给它绑定一个热键。按下去等于替用户说了这句话，不经过语音识别，所以更快。快捷消息可以配多条。
- **两种入口后面的流程相同**：
  1. 截前台窗口；
  2. 用用户选定的 OCR 模型识别，结果带每行坐标；
  3. 附上最近的聊天跟踪记录和当前游戏 profile 的说明；
  4. **调一次大模型**，仿照现有的提醒规划（`crates/kotone-ai/src/reminders.rs`，单次请求 + `tool_choice: auto`）。不做多轮 Agent 循环。
- **模型从三种动作里选一种**：
  - 创建提醒，沿用现有流程；
  - 帮忙打字：由模型撰写或翻译后打进游戏聊天框，默认不按回车，设置里可以改成自动发送；
  - 直接回答：用语音或弹幕播出。
- **模型可以顺带回报聊天框的位置**，Kotone 记进 profile，下次只识别那块区域。

### 4.2 约束

- **必须有 AI 连接**，用户给的水准下限是 DeepSeek V4.1 Flash 级别。没配连接，或者连接不达标，热键就不工作，只提示用户先去配置，不做降级。
  - 我的建议是保存连接时跑一次能力自检，用户没有反对：先确认工具调用能正常返回，再给一段固定的聊天 OCR 样例，看模型能否挑出队友的话。
- **游戏适配写成提示词和 profile 说明**，不给每个游戏写死规则。profile 需要新增一个可选的自由文本字段，比如"LoL 聊天在左下角，[队伍] 前缀是队友"。
- **OCR 文字一律只当数据**，不当指令。模型可选的动作只有上面三种，没有任何能改配置的工具。
- **在线 OCR 会上传截图**：陪玩热键每按一次都会截图，所以设置页必须写明这一点。

### 4.3 开放给 Agent

同一条流程包装成工具，例如"读聊天"和"帮忙打字"。外部 Agent 调用它们得到的结果，要和用户自己按热键时一致。底层的截图、OCR、跟踪工具保留。

## 5. 已定设计：ASR / OCR / TTS 统一模型管理

### 5.1 形态：照搬现有的 ASR 模型管理

- **可参照的现有实现**：`crates/kotone-stt/src/model.rs` 里的 `ModelInfo`，以及 `models.list / download / activate / delete` 这组工具、设置页的 `ModelsSection.svelte`。
- **本地模型和在线模型放在同一张清单里**：
  - 本地模型带地址、大小和 SHA-256，可以下载、校验、删除、切换；
  - 在线模型带配置字段，通过 `connection` / `secret` 字段绑定 AI 连接，密钥只存在凭据管理器里。
- **加一个类别字段**，值为 `asr | ocr | tts`。设置页按类别分栏，Agent 工具也加类别参数。**现有 ASR 的行为不能变。**
- **从参考项目借的做法**（调研见本文第 6 节）：
  - 语音编号写成 `提供方/声音`；选中的声音找不到时直接报错，不偷偷换成别的；
  - 按"声音 + 语速 + 文本"缓存合成结果，快捷消息的固定回答语尤其用得上；
  - 下载支持断点续传和手动导入；
  - 继续逐文件校验 SHA-256，不像 Sokuji 那样只校验大小。

### 5.2 OCR 首批清单

**本地模型**的来源是 ModelScope `RapidAI/RapidOCR`，发布标签 `v3.10.0`。现在的韩文包就来自这里，见 `crates/kotone-ocr/src/manifest.rs`。

- 地址格式：`https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/<路径>`。
- 下表的大小和 SHA-256 取自 ModelScope 文件接口（`/api/v1/models/RapidAI/RapidOCR/repo/files?Revision=v3.10.0&Recursive=true&Root=onnx/PP-OCRv5`），**写进清单之前必须实际下载复核**。
- 其中韩文识别模型和检测模型的值，与清单里现有的一致。

| 路径 | 字节 | SHA-256 | 用途 |
| --- | --- | --- | --- |
| det/ch_PP-OCRv5_det_mobile.onnx | 4819576 | 4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae | 所有语种共用的检测模型 |
| det/ch_PP-OCRv5_det_server.onnx | 88118768 | 0f8846b1d4bba223a2a2f9d9b44022fbc22cc019051a602b41a7fda9667e4cad | 高精度检测，可选 |
| rec/ch_PP-OCRv5_rec_mobile.onnx | 16631306 | 5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5 | 简中、繁中、英文、日文（按 PaddleOCR 官方说明），建议默认推荐 |
| rec/ch_PP-OCRv5_rec_server.onnx | 84577022 | e09385400eaaaef34ceff54aeb7c4f0f1fe014c27fa8b9905d4709b65746562a | 同上，更准，CPU 上更慢 |
| rec/korean_PP-OCRv5_rec_mobile.onnx | 13488748 | cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b | 韩文，现有的包 |
| rec/en_PP-OCRv5_rec_mobile.onnx | 7872351 | c3461add59bb4323ecba96a492ab75e06dda42467c9e3d0c18db5d1d21924be8 | 英文 |
| rec/latin_PP-OCRv5_rec_mobile.onnx | 7904513 | b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a | 拉丁字母语言 |
| rec/cyrillic_PP-OCRv5_rec_mobile.onnx | 8074092 | 90f761b4bfcce0c8c561c0cb5c887b0971d3ec01c32164bdf7374a35b0982711 | 西里尔字母 |
| rec/eslav_PP-OCRv5_rec_mobile.onnx | 7911802 | 08705d6721849b1347d26187f15a5e362c431963a2a62bfff4feac578c489aab | 东斯拉夫语言 |
| rec/th_PP-OCRv5_rec_mobile.onnx | 7915294 | de541dd83161c241ff426f7ecfd602a0ba77d686cf3ab9a6c255ea82fd08006e | 泰文 |
| rec/el_PP-OCRv5_rec_mobile.onnx | 7832350 | b4368bccd557123c702b7549fee6cd1e94b581337d1c9b65310f109131542b7f | 希腊文 |
| rec/arabic_PP-OCRv5_rec_mobile.onnx | 8023828 | c1192e632d0baa9146ae5b756a0e635e3dc63c1733737ebfd1629e87144e9295 | 阿拉伯文 |
| rec/devanagari_PP-OCRv5_rec_mobile.onnx | 7940361 | d6f0a906580e3fa6b324a318718f1f31f268b6ea8ef985f91c2012a37f52c91e | 天城文 |
| rec/ta_PP-OCRv5_rec_mobile.onnx | 7909926 | a42448808b7dea87597336f12438935f40353f1949e8360acd9e06b4da21bfe1 | 泰米尔文 |
| rec/te_PP-OCRv5_rec_mobile.onnx | 7923102 | a3690451b50028a09a3316a1274f7c05728151ea3f8fd392696397a7fefcbd92 | 泰卢固文 |

**每个识别模型接入前要逐一确认**：

1. 字符表是否像韩文模型那样写在 ONNX 元数据（`character`）里；
2. 输入尺寸和前后处理能否直接复用 `crates/kotone-ocr/src/ppocr.rs` 与 `local.rs`。现有的类名 `LocalKoreanOcr` 需要泛化。
3. 覆盖哪些语言以实测为准。

**两条注意**：

- **PaddlePaddle 官方在 ModelScope 上发布的是 Paddle 原生格式**（`.pdiparams`），ONNX Runtime 加载不了，不要选。
- **PP-OCRv6 暂不使用**。RapidOCR 的 master 分支已有 `onnx/PP-OCRv6`，检测和识别各有 tiny / small / medium 三档，2026 年 6 月提交，但还没进任何发布标签，支持的语种和前后处理都未核实。

**在线 OCR**：把现有的 DeepSeek OCR（`crates/kotone-ocr/src/deepseek.rs`）迁进统一模型清单，作为一条在线条目。

### 5.3 TTS 首批清单

- **本地**：
  - 现有的 Kokoro 中文，约 147 MB，见 `crates/kotone-tts/src/engine.rs` 和 [本地 TTS 研究](../research-tts.md)；
  - Windows 系统语音，迁进同一张清单；
  - 只放中文就够了：外语聊天会先由大模型翻译，再播报。
- **在线**：
  - OpenAI 兼容的语音合成接口；
  - 火山语音合成，能否复用 ASR 那边的火山连接，接入前要核实。
- **待用户决定：Edge TTS 是否接入。**我建议暂不接入，理由如下：
  - 它是逆向出来的非官方接口，客户端令牌写死在代码里；
  - Sokuji 源码注释称其服务地址 2026 年 10 月刚换过一次。

  用户如果想要，就作为可选项加入，并标明"非官方，可能失效"。

## 6. 参考项目调研摘要

**dsh-pet**（<https://github.com/PC2005-cloud/dsh-pet>，MIT）：

- 语音引擎由 DSH 宿主提供，插件只定义注册接口，形如 `voice.register({id, name, voices, format: mp3|wav, timeoutMs, synthesize({text, voice, rate, signal}) → Uint8Array})`；
- 整段合成，不做流式；
- 按"带提供方前缀的声音编号 + 语速 + 文本"缓存；
- 选中的声音解析不到就报 `bad-request`，不降级；
- 默认引擎是在线 Edge 语音，没有本地模型管理。
- 只读了文档，没读宿主侧的 `src/host/voice/` 源码。

**Sokuji**（<https://github.com/kizuna-ai-lab/sokuji>）：

- 清单是单一的 `src/lib/local-inference/modelManifest.ts`，用 `type` 字段区分 asr / tts 等，在线条目用 `isCloudModel` 标记；
- 本地模型的文件只记录大小，没有哈希；
- `ModelManager.ts` 负责下载：连接超时 30 秒、停滞超时 60 秒、重试 3 次、断点续传，存进 IndexedDB，并支持手动导入；
- TTS 本地模型以 sherpa-onnx WASM 包为主，包括 Piper、VITS、Matcha 等；
- Edge TTS 实现在 `src/lib/edge-tts/`。
- 早先的对照文档见 [research-sokuji.md](../research-sokuji.md)。

## 7. 建议开工顺序

1. **设计文档**：把第 4、5 节正式写进 [product-evolution-game-tools.md](../product-evolution-game-tools.md)，替换原来"场景一由外部 Agent 驱动"的写法，同时更新 `docs/README.md`。
2. **统一模型清单**：先改通用结构、工具参数和设置页，保证 ASR 行为和测试不变。
3. **OCR 接入**：本地多语种包，加上迁进来的 DeepSeek OCR；设置页可以选择当前 OCR 模型。
4. **TTS 接入**：迁入系统语音和 Kokoro，新增 OpenAI 兼容和火山两个在线选项，加上合成缓存。Edge TTS 看用户的决定。
5. **陪玩**：
   - 助手路由从只能"提醒"升级为"提醒 / 打字 / 回答"三选一；
   - 新增快捷消息的配置和热键；
   - profile 新增说明字段；
   - 实现 AI 连接的能力自检；
   - 完善各种失败情况下的回话。
6. **工具对齐**：把陪玩流程开放成工具，更新 `tool-service.md`、Skill，以及插件仓库里的技能。

## 8. 代码入口

- **助手路由**：`crates/kotone-core/src/assistant.rs` 里的 `AssistantRoute`，目前只有 `Reminders` / `Agent`。
- **桌面端助手**：`apps/desktop/src-tauri/src/assistant.rs`、`assistant_input.rs`、`hotkey.rs`。
- **提醒规划**：`crates/kotone-ai/src/reminders.rs`。
- **AI 传输**：`crates/kotone-ai/src/transport.rs`。
- **OCR**：
  - 端口：`crates/kotone-core/src/ocr.rs`，即 `OcrBackend`；
  - 实现：`crates/kotone-ocr/src/`，包括 manifest、install、local、ppocr、deepseek。
- **TTS**：`crates/kotone-tts/src/`，包括 lib、engine、player。
- **游戏 profile**：`crates/kotone-core/src/profile.rs`。
- **工具服务**：
  - 工具目录：`crates/kotone-tools/src/catalog.rs`、`catalog_host.rs`；
  - 宿主侧：`apps/desktop/src-tauri/src/tool_config.rs`。
- **设置页**：
  - `apps/desktop/src/routes/settings/pages/assistant/`；
  - `advanced/ModelsSection.svelte`；
  - `advanced/ConnectionsSection.svelte`；
  - `GamePage.svelte`。

## 9. 工作约束（照旧）

- **分支与提交**：
  - 只在当前目录工作，从 main 新建分支，不建额外的 worktree；
  - 只暂存自己的改动，不用 `git add .`；
  - 每个提交都要能独立构建；
  - 不自行推送、合并或发布。
- **别人的未跟踪目录不要碰**：`.tmp-analysis/`、`.rambledesk/`、`.reasonix/`、`.codegraph/`、`bash.exe.stackdump`。
- **测试与冒烟的隔离**：
  - 普通测试不读取真实的 `~/.kotone`、凭据管理器或付费 API，不自动下载大模型；
  - 模型冒烟使用显式的隔离缓存；
  - 冒烟里不运行 `autostart.set`、`elevation.restart` 或真实的安装器；
  - 不动用户安装在 `D:\琴音` 的 0.1.13 版，也不动 WebView2 用户数据目录。
- **运行 cargo 测试**：
  - 设置 `TMP=TEMP=C:\Users\A\Desktop\kotone\target\tmp`，并关闭沙箱；
  - 原生进程只能写本仓库目录，端到端测试的工作目录放在 `target/` 下。
- **编辑注意**：
  - rustfmt 只对自己改过的文件运行，先跑 `--check`；
  - 工作区文件是 CRLF 换行；
  - 含反斜杠的文本用编辑工具写，不要用 Bash heredoc。
- **代码检索**：仓库有 `.codegraph/` 索引，先用 CodeGraph，不要自己重建索引。
