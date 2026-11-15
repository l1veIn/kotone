# Kotone 产品演进：面向 Agent 的游戏场景工具基础设施

状态：演进计划。M1（桌面工具服务与视觉输入）与 M2（图像预处理、OCR、聊天消息跟踪）已实现，现行协议、工具 schema 与限制以[工具服务手册](tool-service.md)为准，验证记录见[M1 实现记录](research/game-tools-m1-implementation.md)与[M2 实现记录](research/game-tools-m2-implementation.md)；M3 及以后仍是计划，尚非 API。日期：2026-11-15。现有能力基线：`124cbf8`。本文与[本轮研究报告](research/2026-11-15-lol-chat-ocr-report.md)配套；当前行为仍以[架构](architecture.md)、[CLI 手册](cli.md)和实现为准。

## 1. 产品定位与首个场景

总方向（2026-11-15 确认，见 README「为 Agent 而生」）：功能完整的软件必然复杂，复杂软件应由 Agent 使用而不是让人学习，软件最终都会成为基础设施。Kotone 的目标是让人能在界面里做的每件事，Agent 都能通过工具完成，并配套 Skill / 桌宠插件让 Agent 从启动 Kotone 开始完整驱动它。全量服务化清单见 [§5.5](#55-全量服务化清单2026-11-15-修订取代上文-m3m4-表)。

Kotone 固定定位为工具。Agent 决定何时调用、调用哪些能力、采用何种模型和如何解释结果；DSH 桌宠负责用户对话、人格、意图与上下文。Kotone 提供本机资源访问、确定性执行和可追溯结果。参考 BetterGI 的[命令行入口](https://www.bettergi.com/feats/command/introduce.html)，目标是让 Agent 操作同一个 Kotone 桌面实例，而不是另造一个桌宠。

优先围绕 LOL 聊天闭环：

1. “大肥鱼，我队友在说什么？”——玩家主动触发，截图、裁剪，Agent 可自行看原图，也可取 OCR 结果，解释后通过 TTS 或弹幕告知。
2. “帮我告诉队友不要浪。”——Agent 生成回复，Kotone 按指定窗口、频道与文本执行发送。
3. 韩服聊天——读取韩文、解释中文，必要时让 Agent 拟定韩文回复；翻译与游戏语义由模型处理。
4. 用户没有桌宠——可选用 Kotone 的语音入口及无会话状态的模型调用执行同一条已配置流程。

第一版采用主动触发，不默认持续截图、持续上传或自动回复。读取当前可见内容与只看新增消息是两个独立请求，不能因为已经读过就让“翻译当前聊天”漏掉仍然可见的旧消息。

## 2. Kotone 与 DSH / Agent 的分工

| 工作 | Kotone | DSH / Agent |
| --- | --- | --- |
| 用户意图、对话记忆、人格 | 接收明确工具参数 | 负责理解和维护 |
| 窗口与原始图像 | 定位、截图、裁剪、坐标与来源记录 | 决定目标窗口/区域，可用多模态直接读图 |
| OCR | 可插拔本地/在线后端，统一文字、框、来源、耗时 | 按需调用、比较 OCR 与原图，判断是否可信 |
| 消息跟踪 | 保存观测、稳定 ID、修订、可见集与新增游标 | 解释省略、缩写、指代及对局含义 |
| 频道和发言者 | 返回前缀、颜色、位置等证据和候选，允许 unknown | 综合上下文判定；不能把候选当作已证实阵营 |
| 翻译、摘要和回复 | 可选的无会话模型工具；不自行规划 | 默认由 Agent 编排和解释 |
| 热键与触发链 | 注册、冲突报告、事件、执行预先配置的流程 | 根据用户要求配置绑定和流程 |
| 发送、TTS、弹幕 | 执行并报告真实交付结果 | 决定内容、目标、时机与输出方式 |

模型并非必须回到桌宠主会话。两条路径都应成立：Agent 的独立短任务处理；或 Kotone 调用用户配置的无会话 LLM 节点就地处理。外部 Agent 调用需关联 `jobId`，不隐式把截图塞进繁忙的主对话。

## 3. 已有能力与真实缺口

以下以已提交代码 `124cbf8` 为准。当前工作区另有未提交的 runtime、热键和悬浮窗修复，不计为本方案已交付能力。

| 领域 | 当前已有 | 仍需增加 |
| --- | --- | --- |
| 桌面运行时 | ASR 生命周期、会话编排、Tauri IPC | 独立于 ASR 的外部工具服务 |
| 命令行 | send/listen/config/devices/eval 等独立 CLI | 连接正在运行的桌面实例的客户端 |
| 热键 | 平台具名按下/松开事件、固定内置动作 | Agent 可用的注册/注销、所有权、冲突和租约 |
| 模型连接 | kotone-ai 连接、凭据、聊天请求；后处理和提醒消费者 | 通用且可发现的无会话模型调用端口 |
| ASR | 本地与在线引擎、助手语音输入 | 按用途/所有权隔离的通用输入工具 |
| 输出 | 提醒场景的 SAPI/Kokoro、弹幕、取消和资源互斥 | 任意文本交付的统一 API 与回执 |
| 窗口/截图/裁剪 | 前台窗口与注入相关端口 | 窗口枚举、坐标、截图、图像产物服务 |
| OCR/消息跟踪 | 只有研究脚本和实验数据 | 产品级后端、模型安装、观测与去重状态 |
| 工作流/插件 | 现有内部领域流程 | 可配置触发链、执行状态、DSH 适配插件 |

M1 之后，上表中“独立于 ASR 的外部工具服务”“连接正在运行的桌面实例的客户端”以及“窗口枚举、坐标、截图、图像产物服务”已由[工具服务手册](tool-service.md)覆盖；截图仅限 GDI 前台可见客户区，其余缺口不变。

现有 CLI 是 core 的另一消费者，并不是桌面遥控器。新的 `tools` 命令直接连接服务；不得通过再次运行桌面 exe 发送每条命令，避免 single-instance 回调唤起设置窗口并抢走游戏焦点。

## 4. 统一契约

### 4.1 传输、发现与能力

首个传输采用本机 loopback HTTP。桌面进程拥有服务，CLI/DSH 插件是客户端，领域层不依赖 HTTP/Tauri。

- 默认关闭，通过新增配置 `toolService.enabled` 显式开启；M1 可通过配置命令设置并重启桌面生效。
- 绑定 `127.0.0.1` 的可用端口，不监听外网。实例启动生成会话凭证，所有入口校验凭证。
- 在用户私有运行目录写 descriptor，至少包含协议版本、实例 ID、PID、地址、凭证；限制为当前用户访问。客户端读取它，不硬编码固定端口、不在参数/普通日志打印凭证。
- 退出仅清理自己拥有的 descriptor/产物；旧 descriptor、实例代际不匹配或连接失败返回明确错误，不误连其他进程。
- `GET /v1/capabilities`、`GET /v1/status`、`POST /v1/tools/call`；CLI 使用同一协议，不另写一套业务逻辑。
- capabilities 返回实际实现的工具及 schema、版本、就绪条件、数据外发/副作用属性。路线图里的工具不冒充已实现；未知工具返回 `TOOL_NOT_FOUND`。

已实现的 CLI 入口（参数与退出码见[工具服务手册](tool-service.md#10-命令行)）：

```text
kotone-cli tools status
kotone-cli tools list --json
kotone-cli tools call <tool-name> --args-file <json-file>
kotone-cli tools artifact get <artifact-id> --output <png-file>
kotone-cli ocr status | ocr install        # M2：本地 OCR 模型就绪检查与显式安装
```

原有 `send/listen/...` 行为继续由现行手册定义。本节以下的契约草案保留为设计依据；实际字段（如信封中的 `protocol/protocolVersion`、产物的 `rootTransform/screenRect`）以手册和 capabilities 为准。

### 4.2 请求与结果

```json
{
  "requestId": "req-123",
  "tool": "image.crop",
  "arguments": {
    "artifactId": "img-source",
    "rect": { "x": 15, "y": 210, "width": 650, "height": 200 }
  },
  "timeoutMs": 3000
}
```

```json
{
  "requestId": "req-123",
  "instanceId": "desktop-instance",
  "ok": true,
  "result": {
    "artifactId": "img-chat",
    "parentArtifactId": "img-source",
    "width": 650,
    "height": 200,
    "mimeType": "image/png",
    "coordinateSpace": "artifact-physical-pixels"
  },
  "timing": { "totalMs": 12 }
}
```

错误使用同一信封，至少带 `code/message/retryable`。区分未看清、无聊天、后端不可用、超时、取消、窗口失效和交付失败；网络错误不伪装成“图像没看清”。不向调用方返回内部堆栈、凭证或不必要的用户路径。

### 4.3 图像、坐标与窗口身份

原图只读。截图、裁剪与预处理都产生新 `artifactId`，携带 SHA-256、尺寸、创建/过期时间、父产物和从当前图映射回原图的变换。截图另带窗口引用、客户区屏幕位置、DPI、捕获方法和可见性条件。`frameId` 标识一次采集，不能拿旧图的裁剪坐标套在新窗口上。

裁剪矩形以输入产物的物理像素为准，左上为原点。屏幕坐标、窗口客户区坐标、Tauri 逻辑像素不可混用。只接受有效产物 ID；HTTP 不开放任意本地文件路径读写。CLI 的 `--output` 仅由客户端写到用户指定路径。

窗口引用应绑定实例、HWND/PID 及有效性信息。发送和截图前重新验证，避免 HWND 复用或进程更换。默认不抢焦点。M1 若使用 GDI，明确标记为前台可见客户区像素捕获，不宣称隔离窗口内容、遮挡恢复、最小化或独占 GPU 捕获；不支持的情况返回错误或有依据的警告，不悄悄改截整个桌面。全黑图不等于已经看懂聊天。

### 4.4 执行与计时

M1 为有界短请求；后续长任务使用 `jobId` 和 queued/running/succeeded/failed/cancelled 状态，提供查询、取消、截止时间和进度事件。取消后迟到结果不能继续发送或播报。阻塞截图/编码/推理进入工作线程，不能持有运行时锁执行。

时延分别记录 capture/crop/preprocess/ocr/model/delivery；“完整可播报文本耗时”不含 TTS 合成/播放。实际体验还需记录触发至开始听到/看到内容的时延，不能拿前一指标替代。

## 5. 计划中的服务暴露列表

工具名为 v1 目标契约；表中阶段表示计划顺序。M1 的 8 个工具与 M2 的 6 个工具已实现（见[工具服务手册](tool-service.md#5-已实现工具)）；M3 及以后的工具当前不可调用，也不出现在 capabilities 中。每个工具均继承统一错误、超时、来源与计时契约。

### M1：桌面工具服务与视觉输入

| 工具 | 主要输入 | 主要输出 |
| --- | --- | --- |
| `system.capabilities` | 可选协议版本 | 实际工具、schema、平台、就绪条件、版本 |
| `system.status` | 无 | 桌面实例、服务状态、ASR phase；不包含密钥 |
| `window.list` | 可选进程/标题过滤、数量上限 | 窗口引用、进程、可见性、前台标记、物理客户区/DPI |
| `window.active` | 无 | 当前前台窗口引用及元数据，或明确无目标 |
| `screen.capture` | windowId | 原图 artifact、frameId、坐标/捕获方法/时间/警告 |
| `image.crop` | artifactId、rect | 新图、父图、尺寸、原图映射、哈希 |
| `artifact.get` | artifactId | 元数据与 PNG 内容，或过期/无效错误 |
| `artifact.release` | artifactId | 幂等释放回执 |

### M2：OCR、预处理与消息跟踪

实现状态：已实现（[M2 实现记录](research/game-tools-m2-implementation.md)）。本地后端为不依赖 Python 的 Rust 实现（PP-OCRv5 韩文 mobile，复用 sherpa-onnx 已链接的 ONNX Runtime），模型经 `kotone-cli ocr install` 显式安装与哈希校验；在线 DeepSeek 后端需用户显式指定连接。下表与下文保留为设计依据，实际字段以手册与 capabilities 为准。

| 工具 | 主要输入 | 主要输出 |
| --- | --- | --- |
| `image.preprocess` | artifactId、明确 preset/参数 | 新图与处理/坐标变换记录，保留原图 |
| `ocr.backends` | 无 | 已接入后端、安装/凭据/语言/设备状态 |
| `ocr.recognize` | artifactId、backend、language、参数 | 原文、行/框、可空置信度、原图坐标、后端/版本、耗时、警告 |
| `chat.track.ingest` | trackingSession、frameId、OCR/外部视觉观测及来源 | 可见消息候选、稳定ID、修订、频道/说话者证据、重复不确定性 |
| `chat.track.read` | session、visible/newSince、可选 cursor | 当前可见集或新增变化、游标、修订，不擅自做翻译 |
| `chat.track.reset` | session、原因 | 新代际与清理回执 |

OCR 后端可选本地与在线。实验中的 RapidOCR + 韩文 PP-OCRv5 CPU 环境尚未完成产品安装、许可核对、权重校验、升级和打包，不能直接把 `.tmp-analysis/vendor` 纳入产品。在线后端复用连接/凭据设施，明确只外发所选图像；按照 [DeepSeek OCR 契约](https://api-docs.deepseek.com/zh-cn/guides/ocr/)解析状态和响应体错误，不能仅用 HTTP 200 判成功。

置信度无法提供时返回 null，不能造 1.0 或把其当语义正确率。框可为行框或整块框，需标注粒度；不可凭模型分行数量伪造逐行坐标。预处理是可选工具，Agent 可比较原图、裁剪、放大和 OCR，不能把单个有损预设强制成唯一入口。

跟踪使用时序、位置、文本相似和来源共同匹配；同一人重复发送同样一句话仍可能是新消息。OCR 修订用同 ID 的 revision；不确定重复显式返回。滚动、窗口/ROI变化、重连、新对局须定义重置或新代际。频道无法证实时返回 unknown，LOL 的 `[팀]`、`[전체]` 等前缀和颜色只是证据。

### M3：触发链、模型与输出

> 本表与下方 M4 表保留为早期设计依据，已被 [§5.5 全量服务化清单](#55-全量服务化清单2026-11-15-修订取代上文-m3m4-表) 取代。

| 工具 | 主要输入 | 主要输出 |
| --- | --- | --- |
| `hotkey.register` / `hotkey.unregister` | owner、chord、事件/流程绑定、租约 | bindingId、冲突/释放状态 |
| `hotkey.list` | 可选 owner | 实际绑定与内置冲突信息 |
| `workflow.put` | 明确节点/参数映射/绑定、版本 | workflowId、校验结果 |
| `workflow.run` / `workflow.cancel` | workflowId、输入 / jobId | jobId / 取消回执 |
| `job.get` / `event.subscribe` | jobId / cursor与事件过滤 | 状态、节点结果、事件与计时 |
| `llm.complete` | 已配置连接、文本/可选图像引用、模型参数、deadline | 无会话最终文本/结构结果、usage、失败类型 |
| `tts.speak` / `tts.cancel` | text、声音、owner / deliveryId | deliveryId、实际开始/完成/取消/失败 |
| `overlay.push` / `overlay.dismiss` | text、样式、owner / deliveryId | 弹幕交付回执及不支持显示模式原因 |

触发链执行 Agent 已配置的确定性节点与参数映射，不自行规划，不执行任意 shell/JavaScript 字符串。第一版可限定线性链，并提供失败分支、截止时间、重复触发策略（忙时合并/拒绝/取消旧任务）和逐节点取消。不能把“保存成功”当作“已执行”。通用热键复用现有管理器；不得覆写 CapsLock、应用启停键或其他绑定，也不另开相互竞争的钩子。

模型节点有两种处理器：Kotone 的无会话调用；或 DSH 插件承接独立任务并回传同一 job。外部节点不携带对话主会话的隐式依赖。Kotone 可复用 `kotone-ai`，不维护第二套密钥、人格或长期记忆。TTS/弹幕复用现有输出互斥与取消规则，文字翻译成功不等于已经听到。

### M4：回复、语音与适配层

| 工具 | 主要输入 | 主要输出 |
| --- | --- | --- |
| `chat.send` | windowId、channel、text、发送意图、幂等键、可选 preview | 校验/预览或真实发送回执，准确标注可验证范围 |
| `asr.start` / `asr.stop` / `asr.cancel` | owner、purpose、引擎/设备 / sessionId | 所属会话、partial/final、取消/失败事件 |
| `asr.transcribe` | 音频产物、后端/语言、deadline | final文本、来源、耗时 |
| `runtime.start` / `runtime.stop` | 明确动作及期望代际 | 状态和失败阶段；不用 toggle 表达自动化意图 |
| DSH / MCP 适配 | 工具描述、认证连接、产物与事件协议 | DSH 可调用工具及任务回传；复用底层服务 |

发送前校验目标、前台与频道，发送阶段不自动重试，幂等键阻止重复发送。输入注入回执只能证明本地执行到哪一步；没有可验证聊天回显时不能声称服务器已送达。发送内容由用户请求及 Agent 决定，不从翻译结果自动延伸成回复。已有授权可持续作用于配置好的触发链，工具不要求用户每次重复确认。

ASR 是可选输入基础能力，保持用途与所有权隔离，不让桌宠会话接管游戏语音会话。优先复用已有本地/在线 ASR，不以改造语音输入阻塞视觉工具交付。

### 5.5 全量服务化清单（2026-11-15 修订，取代上文 M3/M4 表）

状态：计划，尚非 API。工具名为目标契约，实现时以 capabilities 为准。

#### 5.5.1 原则

| 原则 | 含义 |
| --- | --- |
| 界面对等 | 设置页、托盘、热键、CLI 能做的事都有对应工具；不因“敏感”而隐藏能力 |
| 分级授权代替隐藏 | 每个工具声明所需级别：`read`（状态）、`local`（本地副作用：播报、弹幕、提醒、启停）、`user-content`（历史原文、录音、截图）、`egress`（数据外发：在线识别/AI/OCR）、`input`（向其他窗口注入输入）、`admin`（设置、凭据、模型、提权、自启）。授权由用户一次性配置、持续生效（可选“全部授予”），capabilities 返回每个工具的级别与是否已授权，未授权返回 `PERMISSION_DENIED{level}`，不逐次弹确认 |
| 凭据只写不读 | Agent 可以新建连接、写入或替换 API key，但任何工具都不返回 key |
| 内容走产物，不收任意路径 | 图像、音频、文本、压缩包以产物上传/下载（新增 `artifact.put` 与音频/文件产物类型），CLI 负责读写本地文件；只有配置 Kotone 自身存储位置的 `admin` 工具（模型目录、历史目录）接受目录参数 |
| 所有权与互斥 | 麦克风、播报、热键、悬浮窗与用户本人的语音输入共享：会话带 owner/purpose，冲突时返回明确错误或按策略排队，不抢占用户正在进行的会话 |
| 真实回执与长任务 | 结果只报告可验证的执行阶段；下载、回放、录音、播报等使用 `jobId`、取消、截止时间与事件 |

#### 5.5.2 应用生命周期（Agent 从“打开 exe”开始）

工具服务只在桌面实例运行时存在，因此启动由客户端完成：

| 入口 | 作用 |
| --- | --- |
| `kotone-cli app status` | 是否在运行、实例/版本、工具服务是否开启、描述文件是否有效 |
| `kotone-cli app launch [--wait]` | 以后台模式启动 `kotone.exe`（只驻留托盘，不打开设置窗口、不抢焦点），等待描述文件就绪后返回实例信息；已在运行时直接返回现有实例，不唤起窗口。工具服务未开启时明确报错，或在显式 `--enable-tool-service` 时先写配置再启动 |
| `app.quit` / `kotone-cli app quit` | 等价于托盘“退出”：优雅结束（`RunEvent::Exit`），工具服务按 M1 顺序关闭；无响应时 CLI 可按实例 PID 结束并报告 |
| `app.restart` | 退出后以相同模式重启，返回新实例 ID（旧会话凭证随之失效） |
| `runtime.status` / `runtime.start` / `runtime.stop` | ASR 运行时的显式启停，带期望代际，不提供 toggle |

需要的配套改动：桌面端 `--background` 启动参数；单实例回调对后台参数不显示窗口；`kotone-cli` 在实例未运行时可以写配置（已有）。

#### 5.5.3 反向通道：Kotone 主动通知 Agent

Agent 调用 Kotone 是“拉”；很多场景需要 Kotone 先说话——用户按了绑定的热键、对 Kotone 说了一句要交给 Agent 的话、提醒到期、跟踪的聊天出现新消息、长任务完成。这是一项独立的基础服务：

| 组成 | 作用 |
| --- | --- |
| 事件总线 | `GET /v1/events?cursor=`（SSE；不支持时长轮询）与 `event.read`：类型化事件、单调游标、有界保留、至少一次送达、事件 ID 去重；游标过期返回错误而不是静默跳过 |
| `user.request` 事件 | 用户经 Kotone 发给 Agent 的请求：助手热键语音（如“大肥鱼，我队友在说啥？”）、文字入口、或绑定到“转交 Agent”的热键；携带原文、来源、前台窗口引用与可选截图产物，带 `requestId` |
| `agent.reply` | Agent 对某个 `requestId` 的回应，由 Kotone 按用户偏好交付（播报/弹幕/发送前预览），并产生交付回执事件 |
| 其他事件 | `hotkey.triggered`、`chat.track.changed`（订阅的会话出现新增/修订）、`reminder.due`、`job.*`、`delivery.*`（播报/弹幕回执）、`runtime.*`、`app.*`、`asr.partial/final` |
| 推送接收端（可选） | `agent.sinks.register`：注册 loopback webhook 等接收端，由 Kotone 主动推送，供不方便长连接的插件使用；带租约，过期自动注销 |

Kotone 侧只提供与传输无关的事件与回应协议；把事件变成 Agent 对话里的一条消息由插件完成（DSH / pi 等插件 API 的能力需在适配时核对，这里不做假设）。Agent 未运行时事件按保留上限暂存，不由 Kotone 启动 Agent。

#### 5.5.4 服务清单

已实现（M1/M2）：`system.capabilities/status`、`window.list/active`、`screen.capture`、`image.crop/preprocess`、`artifact.get/release`、`ocr.backends/recognize`、`chat.track.ingest/read/reset`。

| 领域 | 工具 | 级别 | 复用来源 |
| --- | --- | --- | --- |
| 应用与系统 | `app.status` `app.quit` `app.restart`；`runtime.status/start/stop`；`system.resources`；`system.elevation.get` / `system.elevation.restart`；`system.autostart.get/set` | read / local / admin | `get_runtime_status`、`start/stop_runtime`、`get_resource_usage`、`get_elevation_status`、`restart_as_admin`、`get/set_autostart_enabled` |
| 设置 | `settings.get`（完整、不含密钥）`settings.update`（补丁，与设置页相同的校验与即时生效/需重启标记）`settings.warnings` | read / admin | `get_settings`、`update_settings`、`get_settings_load_warning` |
| 连接与凭据 | `connections.presets/list`、`connections.upsert`（key 只写）、`connections.delete`、`connections.test` | read / admin | `list_connection_presets`、`list_connections`、`upsert/delete_connection` |
| 模型 | `models.list`、`models.download`（job）`models.cancel`、`models.delete`、`models.setActive`、`models.dir.get/set`；`speech.models.status/download`；`ocr.models.install`（job） | read / admin | `list_models`、`download_model`、`delete_model`、`set_active_model`、`get/set_models_dir`、speech_models、`kotone-ocr::install` |
| 语音输入 | `audio.devices`、`audio.setDevice`；`asr.engines`、`asr.setEngine`、`asr.engineOptions`；`audio.capture`（音频产物）；`asr.transcribe`（音频产物→文本）；`asr.start/stop/cancel`（带 owner 的实时会话）；`session.confirm/cancel`（现有预览会话）；`input.checkEnvironment` | read / local / user-content / egress | `list_audio_devices`、`set_audio_device`、`list_stt_engines`、`set_stt_engine`、`get_engine_options`、Orchestrator、`confirm_send`、`cancel_session`、`check_input_environment` |
| 文本与模型调用 | `text.processors`、`text.process`（流水线/处理器序列）、`llm.complete`（无会话） | read / egress | `list_post_processors`、`test_post_processing`、`kotone-ai` |
| 游戏适配 | `profile.list/get/save/delete`、`profile.import/export`（JSON 内容或产物）、`profile.detect`、`profile.icon`；`hotwords.import/export`、`blocklist.export` | read / admin | `list/save/delete_profile`、`import/export_profile`、`get_profile_icon`、热词与屏蔽词导入导出、CLI `profile detect` |
| 发送 | `chat.send`（窗口/profile/频道/文本、预览或发送、幂等键；回执只证明本地执行阶段）、`session.simulateSend` | input | `Injector`、`simulate_send` |
| 热键 | `hotkey.status`、`hotkey.conflicts`、`hotkey.capture/cancelCapture`（录入下一次按键）、`hotkey.register/unregister`（owner + 租约，不覆盖内置绑定） | read / admin | `get_hotkey_status`、`detect_hotkey_conflicts`、`start/cancel_hotkey_capture`、`HotkeyManager` |
| 输出 | `tts.voices`、`tts.speak/cancel`；`overlay.push/dismiss`（弹幕）；`sfx.play`；`overlay.status`（识别悬浮窗健康/状态）、`overlay.savePosition` | read / local | `list_speech_voices`、`preview_reminder`、`stop_speech`、danmaku、`preview_sfx`、`get_overlay_health/state`、`save_overlay_position` |
| 提醒与助手 | `reminder.create`（结构化，不经 LLM）、`reminder.list/cancel`、`assistant.submit`（现有 LLM 规划路径）、`assistant.cancel` | local / egress | `kotone-core::reminders`、`list/cancel_reminder`、`assistant_submit_text`、`cancel_assistant_request` |
| 历史 | `history.list`、`history.get`、`history.audio`（音频产物）、`history.delete`、`history.clear`、`history.dir.get` | user-content / admin | `get_history`、`read_history_audio`、`delete_history_record`、`clear_history` |
| 诊断与评测 | `diagnostics.export`（压缩包产物）、`diagnostics.clear`；`eval.list`、`eval.replay`（job）、`eval.label`、`eval.report` | read / admin | `export/clear_diagnostics`、CLI `eval` |
| 视觉与 OCR | 已实现（见上）；补 `artifact.put` 上传图像 | — | M1/M2 |
| 编排与事件 | `job.get/cancel/list`、`event.read`（+ SSE）、`agent.reply`、`agent.sinks.register/unregister`、`workflow.put/run/cancel/list` | read / local | 新增 |

#### 5.5.5 Agent 侧交付物

| 交付物 | 内容 |
| --- | --- |
| Skill | `SKILL.md` + 示例：何时用 Kotone、从 `app status/launch` 到常见流程（读聊天、翻译后播报、提醒）、授权与失败处理；工具参考由 capabilities 生成，不手抄 schema |
| 桌宠 / Agent 插件（DSH、pi 等） | 薄客户端：发现与认证、工具调用、订阅事件并转成 Agent 消息、`agent.reply` 回传；业务逻辑留在 Kotone |
| MCP 适配（可选） | `kotone-cli mcp`（stdio）把 capabilities 映射为 MCP 工具、事件映射为通知，覆盖支持 MCP 的通用 Agent |

#### 5.5.6 建议顺序

1. **底座**：后台启动与 `app.*`/CLI `app` 命令；事件总线、`user.request`/`agent.reply`；`job.*`；`artifact.put` 与音频/文件产物；分级授权与 `PERMISSION_DENIED`；Skill 初版。
2. **只读面与管理面**：设置、连接（key 只写）、模型、profile、历史、诊断、热键状态等经典业务的查询与管理。
3. **输出与文本**：`tts.*`、`overlay.push`、`reminder.*`、`text.process`、`llm.complete`。
4. **语音输入**：`audio.capture`、`asr.transcribe`、`asr.*` 会话（owner 隔离与麦克风互斥）。
5. **输入与编排**：`chat.send`、`hotkey.register`、`workflow.*`。
6. **插件**：DSH / pi 插件与 MCP 适配，接入反向通道。

## 6. 两种主动触发链

Agent 直接控制：

```text
用户询问 -> window.active -> screen.capture -> image.crop
          -> Agent自行看图，按需ocr.recognize/消息跟踪
          -> Agent翻译/摘要 -> tts.speak 或 overlay.push
```

用户配置热键后，无需每次回到桌宠主会话：

```text
已注册热键 -> 已保存流程 -> 截图/裁剪 -> OCR
          -> Kotone无会话LLM 或 DSH独立任务 -> TTS/弹幕
```

两者都保留原图供复核。确定可读部分先解释；关键字形、主客体或动作方向不确定时说明局部不清。没有可靠核心才整体“没看清”；服务超时则报告读取失败。39图对照说明更换 OCR 并未解决语义误判，不能把 OCR 后端分数或模型自报置信度作为自动发送依据。

## 7. 下一轮实现：M1 的具体边界与验收

交付目标：**Agent 通过 CLI 连接已运行的 Kotone 桌面实例，获取一个有效窗口的原始截图和正确裁剪的图像；ASR 停止时仍可使用，全程不抢焦点。**

实现状态：已实现，手册见 [tool-service.md](tool-service.md)，命令、测试结果与未验证项见 [M1 实现记录](research/game-tools-m1-implementation.md)。受控测试窗口的原生捕获已验证；对真实 LOL 客户端/对局的捕获尚未验证，不能据自动化测试宣称可用。

### 实现范围

1. 增加工具领域类型/端口、产物存储、请求/错误/能力 schema；具体 crate 划分可按现有依赖结构安排，core 不依赖 Windows/HTTP/Tauri。
2. Windows 层实现窗口观察和前台可见客户区捕获，独立声明 GDI/HiDpi 等所需 features；检查目标失效、非前台、最小化与坐标范围，记录支持边界。
3. 桌面 `setup` 共享状态就绪后启动可选服务，`RunEvent::Exit` 关闭；不借用 ASR start/stop，不另启桌面实例。
4. loopback 服务、实例发现、会话凭证、请求/图像/并发/产物容量上限、TTL 和超时；原图不覆盖，释放幂等。
5. CLI 增加 `tools` 客户端、结构化输出、参数文件和产物导出，保留原命令兼容。
6. 同步现行 CLI/架构/隐私/术语文档，新增权威工具服务手册；本文仍作为演进依据，不取代实际 schema。

`toolService.enabled` 需要同步新增 Settings 字段、默认值/校验及 CLI `config set` 白名单。只写文档或服务读取逻辑不足以交付可用的开启路径。

本轮不实现 OCR 产品打包、翻译模型策略、消息跟踪、动态热键、工作流、任意文本输出 API 或 DSH 插件。它们只保留上述目标契约；服务能力清单不得将其列为可用。

### 验收证据

- 真实传输：临时目录与假平台后端启动实际 HTTP 服务，CLI 走完整协议而非仅测函数；覆盖认证、错误/超时、失效 descriptor 和实际能力发现。
- 图像：固定像素夹具覆盖裁剪内容/尺寸、越界/负数/溢出/空矩形、父图与原图映射、哈希、TTL/容量回收；过期或释放的产物不可读取。
- Windows：受控测试窗口完成定位→原图→裁剪并核对像素，关闭/最小化/目标改变后返回明确错误；原生句柄资源正确释放。
- 生命周期：ASR Stopped 时接口可用；调用不唤起设置、不改变前台、不注册第二套热键；退出清理仅影响本实例资源。
- 集成：相关 crate 的测试、Windows 编译、`cargo fmt --all -- --check`，适用的 clippy/workspace tests 和 `pnpm check`。大模型下载、真实游戏输入或付费 API 不是普通测试前置条件。
- 如环境限制真实桌面/游戏验收，必须明确记录未验证项；mock 和 HTTP 200 不作为 LOL 捕获成功证据。

## 8. 后续推进与研究门槛

M2 先把可安装、可取消、可观测的 OCR 与消息观测产品化。保留本地/在线可替换，基准按原生整图、统一行图等前端分开；后端比较必须同输入/筛选/模型参数，全部尝试计入分母。39图已用于对比，未来改策略后应另取未参与调参的数据验证。

M3 打通快捷键→已配置流程→文本输出的闭环，再适配 DSH 独立任务；同时量测触发至实际开始播报的耗时。M4 增加可靠回复、语音输入和插件分发。模型/网络/坐标/焦点失败都需要清晰结果，不通过自启动或无限重试掩盖。

默认只在本机持有临时图像，关闭/释放/TTL 清理；诊断仅记白名单元数据。使用在线 OCR/视觉模型时，在所选连接和流程中清楚表达图像/文字会外发；密钥复用现有凭据层。不得把实验 `.env`、诊断原件、用户截图、视频、Python vendor 或模型权重提交到产品仓库。

## 9. Claude Code 执行约定

先提交本报告、本文、文档索引及小型统计证据，再以该文档提交创建独立 `codex/` 分支/worktree，实现第7节 M1。当前主工作区的未提交修复保留原样，不通过 stash/reset/clean 或覆盖文件带入任务。

Claude Code 应先阅读文档和仓库约定；存在 `.codegraph/` 时先用 CodeGraph，新 worktree 未索引时按普通方式读代码，不自行建立索引。完成实现、相关验证、文档更新和范围内提交，报告 commit、测试结果、捕获语义及未验证的真机条件。实现分支交付与主工作区合入是两件事；有并行未提交修改时，保留可审查分支，不擅自覆盖它们。
