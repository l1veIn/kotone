# 本机 Agent 工具服务（M1 + M2 + Agent 底座 + 配置与引导）

本页是工具服务的**现行权威手册**：开启方式、发现与认证、v1 协议、已实现工具、捕获语义与限制，以及 M2 的图像预处理、OCR 后端与聊天消息跟踪，和 Agent 底座的应用生命周期、事件与用户请求反向通道（§10），以及与设置页对齐的配置类工具（§5“配置与首次引导”）。字段的最终依据是 `kotone-cli tools list --json` 返回的 schema 与 `crates/kotone-tools`、`crates/kotone-ocr` 实现；[演进计划](product-evolution-game-tools.md)中 M3 及以后的工具尚未实现，不属于本 API。实现与验证记录见 [M1](research/game-tools-m1-implementation.md)、[M2](research/game-tools-m2-implementation.md)、[Agent 底座第一批](research/agent-base-batch1-implementation.md)、[第二批](research/agent-base-batch2-implementation.md)。

## 1. 定位与边界

工具服务让 Agent（DSH 桌宠插件、脚本、命令行 Agent）连接**已经在运行**的 Kotone 桌面实例，取得窗口信息、前台窗口客户区截图、裁剪与预处理图像、OCR 文字观测，以及跨帧的聊天消息跟踪。Agent 决定何时调用、是否自己看原图、用哪个 OCR 后端、怎样解释结果；Kotone 只提供确定性的本机执行与可追溯结果，不翻译、不摘要、不回复。

- 桌面进程拥有服务；`kotone-cli tools` 是客户端。客户端直接走 loopback HTTP，**不会再次启动 `kotone.exe`**，因此不会触发单实例回调唤起设置窗口、抢走游戏焦点。
- 服务与 ASR 运行时（启动 / 停止开关）相互独立：运行时 Stopped 时工具照常可用，调用也不会启动 ASR、注册热键或加载模型。
- 服务从不移动窗口；只有 `game.focus` 会把用户配置的游戏窗口切到前台，`game.launch` / `game.close` 会按游戏配置打开或请求关闭游戏，发送输入的只有自动化与陪玩打字（见各节）。除 `models.dir.set`（把模型迁到用户指定的目录，与设置页相同）外不读写调用方指定的本地文件；`diagnostics.export` 只写到 `<KOTONE_HOME>/exports/`。可见输出只有 `agent.reply` 的弹幕（不取焦点、鼠标穿透的提醒层）与语音播报，以及 `elevation.restart` 触发的系统 UAC 提示。
- 已实现下文 80 个工具（服务自身 24 个 + 委托桌面宿主 56 个；以 `kotone-cli tools list` 为准）。引导页与设置页里人能做的配置，Agent 都能通过工具完成（[演进计划 §5.5](product-evolution-game-tools.md)）；通用热键、工作流、模型调用（LLM）与发送尚未实现，不出现在 capabilities 中，调用它们返回 `TOOL_NOT_FOUND`。
- OCR 结果与聊天文字一律是**数据**：服务从不解释或执行其中的内容（例如“忽略之前的指令”之类的文字只会被原样返回）。

## 2. 开启与重启

默认关闭。开关 `toolService.enabled` 只在桌面端启动时读取：

```powershell
# 1. 先从托盘菜单「退出」Kotone（桌面端运行时保存设置会用内存中的旧值覆盖文件）
# 2. 写入开关
kotone-cli config set toolService.enabled true
# 3. 重新启动 Kotone 桌面端，然后验证
kotone-cli tools status
```

关闭同理：退出桌面端 → `config set toolService.enabled false` → 重新启动。没有固定端口或外网监听选项；除下文的在线 OCR 连接（`toolService.ocr`）外没有其他服务配置项。桌面日志 `~/.kotone/kotone.log` 记录 `tool service: disabled` / `listening on 127.0.0.1:<端口> instance=<ID>` / `failed to start: <原因>`，不记录凭证。

## 3. 发现、认证与实例校验

启动时服务绑定 `127.0.0.1` 上由系统分配的端口，生成 256 位随机会话凭证，并写入描述文件：

```text
~/.kotone/run/tool-service-s<Windows会话ID>.json
```

- 运行目录和描述文件使用受保护（不继承）的 DACL，只授予当前用户与 SYSTEM；文件经同目录临时文件原子替换写入。
- 文件名带 Windows 会话 ID：同一账户的另一个登录会话（如远程桌面）看不到、也不会连到这个实例。
- 描述文件含协议版本、`instanceId`、PID、进程创建时间、会话 ID、地址与凭证。客户端读取后依次校验：文件所有者为当前用户（提权实例写入时允许 Administrators）且没有向其他账户授权的 ACE → schema 与协议版本 → 会话 ID → 地址必须是 `127.0.0.1:<非零端口>` → PID 仍在运行且创建时间一致（防 PID 复用）。
- 每个请求（包括未知路径）都要 `Authorization: Bearer <凭证>`；带 `Origin` 头（浏览器）或 `Host` 不是 `127.0.0.1:<端口>`/`localhost:<端口>` 的请求返回 403。每个响应都带 `x-kotone-instance-id`，客户端核对它和信封里的 `instanceId` 与描述文件一致。
- 凭证不出现在命令行参数、status/capabilities 结果、CLI 输出或日志中；客户端不使用系统/环境代理。
- 退出（`RunEvent::Exit`）时按顺序：① 在产物存储锁内关闭提交闸门并丢弃全部内存产物——此后任何在途请求都不能再发布产物或执行释放，新请求返回 `SERVICE_CLOSED`；② 删除描述文件（仅当文件中的 `instanceId` 仍是本实例）；③ 停止 HTTP 接受循环与空闲维护任务。已经在工作线程中运行的原生捕获可以自行完成并释放资源，但结果在提交闸门被丢弃。若另一个仍在运行的进程已发布描述文件，新实例拒绝覆盖并不启动服务；过期的描述文件会被替换。

`KOTONE_TOOL_SERVICE_DIR` 环境变量可覆盖运行目录，仅供自动化测试；正常使用不需要设置。

在线 OCR 默认未启用（见 [§8](#8-ocr-后端与安装)）。推荐在设置页「高级 → 模型 → 文字识别」给 DeepSeek OCR 选一条连接（立即生效）；旧的命令行开关仍然有效：

```powershell
# 先从托盘退出 Kotone；连接（含 API key）需已在设置页的“连接”中添加
kotone-cli config set toolService.ocr.deepseekConnectionId <连接ID>
# 清除：kotone-cli config set toolService.ocr.deepseekConnectionId ""
```

## 4. 协议 v1

| 端点 | 用途 |
| --- | --- |
| `GET /v1/capabilities` | 实际实现的工具、输入/输出 JSON Schema、就绪状态、副作用、限制 |
| `GET /v1/status` | 实例、服务、ASR 相位、捕获后端、产物占用 |
| `POST /v1/tools/call` | 调用工具 |
| `GET /v1/events` | 读取事件（与 `event.read` 相同；查询参数 `cursor` / `from` / `types`（逗号分隔）/ `limit` / `waitMs`），长轮询不占执行名额 |
| `GET /v1/resources/read?uri=` | 读一个 MCP App 资源（插件做成独立 HTML 的页面，ADR 039 D5）：结果是 MCP `resources/read` 的样子 `{contents: [{uri, mimeType: "text/html;profile=mcp-app", text, _meta.ui}]}`；有哪些在 capabilities 的 `resources` 里（插件或页面要的数据开关关着时不列）。`kotone.exe mcp` 的 `resources/list` / `resources/read` 走这里 |

请求体（`Content-Type: application/json`，不超过 64 KiB，不接受额外字段）：

```json
{ "requestId": "agent-42", "tool": "image.crop",
  "arguments": { "artifactId": "art_…", "rect": { "x": 15, "y": 210, "width": 650, "height": 200 } },
  "timeoutMs": 3000 }
```

所有端点返回同一信封：

```json
{ "protocol": "kotone.tools", "protocolVersion": 1, "requestId": "agent-42",
  "instanceId": "inst_…", "ok": true, "tool": "image.crop",
  "result": { "artifactId": "art_…", "parentArtifactId": "art_…", "width": 650, "height": 200, "...": "..." },
  "timing": { "totalMs": 12, "queueMs": 0, "cropMs": 1, "encodeMs": 9 } }
```

失败时 `ok=false`，`error` 至少包含 `code`、`message`、`retryable`，可带 `details`。工具执行结果（含工具错误）使用 HTTP 200；认证失败 401、来源/Host 拒绝 403、未知路径 404、方法不符 405、请求体读取超时 408、请求体过大 413（超出上限后服务端最多再读取并丢弃 1 MiB 再回复，使正常客户端收到完整信封；更大的声明长度立即拒绝，客户端可能看到连接重置）、媒体类型 415、信封不合法 400、服务已关闭 503（`SERVICE_CLOSED`）。`requestId` 可省略（服务生成），只接受 1-128 个字母、数字及 `-_.:`。

### 执行上限（以 capabilities 的 `limits` 为准）

| 项目 | 默认 |
| --- | --- |
| 请求体 | 64 KiB |
| `timeoutMs` | 缺省 5000，允许 1-30000 |
| 并发执行 | 4（超时调用在后台收尾期间仍占名额；期限内拿不到名额返回 `BUSY`） |
| 连接 | 最多 16 个；请求头读取（也约束 keep-alive 两次请求间的空闲）5 秒；请求体读取 10 秒；连接寿命 120 秒，到期后优雅关闭——空闲连接立即关闭，在途请求照常完成（仍受自身 `timeoutMs` 约束），之后最多再等 60 秒应对不读响应的客户端 |
| 单图像素 | 16,777,216 |
| 产物 | 最多 32 个、共 256 MiB（RGBA + PNG），TTL 缺省 300 秒，允许 1-1800 秒；空闲维护每 5 秒回收到期产物（无需读请求） |
| `window.list` | 每次最多 256 项 |
| OCR 执行名额 | 本地 1、在线 2（与上面的并发执行名额叠加；在期限内拿不到 → `BUSY`，`details.lane`） |
| 驻留 OCR 结果 | 最多 64 个，各 600 秒（供 `chat.track.ingest` 引用；过期/回收 → `OCR_RESULT_EXPIRED`） |
| 预处理 | 最多 8 个操作；缩放 0.25–4；输出不超过单图像素上限 |
| 事件 | 总线保留最近 1000 个；同时长轮询的等待者最多 8 个（不占执行名额，满时 `BUSY`）；单次最多返回 500 个 |
| 用户请求 | 有效期 600 秒，最多保留 64 个；文字 ≤4000 字符 |
| Agent 回复 | 文字 ≤2000 字符；已受理、尚未交付完的回复最多 8 条（满时 `BUSY`） |
| 聊天跟踪 | 会话 16 个（满时回收最久未用的会话，结果 `evictedSessions`）；每会话消息 500、事件 2000、帧记录 32、每条保留 8 个旧修订；每次观测最多 64 行、每行 300 字符；会话空闲 30 分钟回收 |

捕获、裁剪、预处理、OCR 推理、PNG 编码与 base64 在工作线程执行，不持有 ASR 运行时或 UI 锁。`timeoutMs` 是从服务收到请求起算的**绝对期限**，在三处强制：取得执行名额时（排队耗尽期限 → `TIMEOUT`，`details.stage="queued"`）、工作线程真正开始时（阻塞线程池饱和导致晚开始 → 不执行工具，`not-started`）、以及提交闸门内（`running` / `late-result`）。工作线程只产出“待提交”结果——新产物或释放请求——从不直接修改存储；提交闸门在存储锁内核对“未关闭、未过期”后才插入或释放。因此即使执行器停顿使迟到结果与计时器同时就绪，迟到结果也不会发布；排队超时的 `artifact.release` 之后也不会删除产物。超时调用的工作线程在后台收尾期间仍占用并发名额。

M2 沿用同一套闸门：OCR 名额在期限内于进入工作线程**之前**获取（排队耗尽期限的调用从不开始推理）；每次调用带一个取消令牌，期限到达或服务关闭时取消——本地推理通过 ONNX Runtime `RunOptionsSetTerminate` 中断，在线请求的连接被丢弃（远端识别仍会继续并计费）。名额与执行许可都随工作线程一起释放，即后端不响应取消时，名额要等工作线程真正结束。OCR 结果只在提交闸门内（结果存储锁内核对关闭/期限）发布；聊天跟踪的变更（ingest/reset）在跟踪器锁内核对关闭与期限后才执行，迟到的 ingest 不改变任何会话。

### 错误码

| 类别 | 错误码 |
| --- | --- |
| 传输 / 请求 | `UNAUTHORIZED` `FORBIDDEN_ORIGIN` `ROUTE_NOT_FOUND` `METHOD_NOT_ALLOWED` `PAYLOAD_TOO_LARGE` `UNSUPPORTED_MEDIA_TYPE` `INVALID_REQUEST` `REQUEST_TIMEOUT`* |
| 调用 | `TOOL_NOT_FOUND` `INVALID_ARGUMENTS` `UNSUPPORTED_PROTOCOL_VERSION` `BUSY`* `TIMEOUT`* `INTERNAL` `PLATFORM_UNSUPPORTED` `SERVICE_CLOSED` |
| 窗口 / 捕获 | `WINDOW_NOT_FOUND`（未知引用）`WINDOW_GONE`（已关闭）`WINDOW_IDENTITY_CHANGED`（句柄复用/进程更换）`WINDOW_MINIMIZED`* `WINDOW_NOT_VISIBLE`* `WINDOW_NOT_FOREGROUND`* `INVALID_GEOMETRY` `IMAGE_TOO_LARGE` `TARGET_CHANGED`*（捕获期间移动/失焦/最小化）`CAPTURE_FAILED`* |
| 产物 | `ARTIFACT_NOT_FOUND`（从未发放）`ARTIFACT_EXPIRED`（`details.reason` 为 `ttl` 或 `evicted`）`ARTIFACT_RELEASED` |
| OCR | `OCR_BACKEND_NOT_FOUND` `OCR_BACKEND_NOT_READY`（`details.reason`：`not-installed` / `invalid` / `runtime-unavailable` / `not-configured` / `credentials-missing` / `credentials-error`）`OCR_LANGUAGE_UNSUPPORTED` `OCR_OPTION_UNSUPPORTED` `OCR_FAILED`（本地推理失败）`OCR_REMOTE_ERROR`†（远端 HTTP 错误或 HTTP 200 内的 `error` 对象；`details.httpStatus/errorType`）`OCR_TRANSPORT_ERROR`*（连接/读取失败）`OCR_RESPONSE_INVALID` `OCR_RESULT_NOT_FOUND` `OCR_RESULT_EXPIRED` |
| 聊天跟踪 | `CHAT_TRACK_SESSION_NOT_FOUND`（`details.reason`：`unknown` / `expired`）`CHAT_TRACK_GENERATION_MISMATCH` `CHAT_TRACK_OUT_OF_ORDER` `CHAT_TRACK_SOURCE_CHANGED` `CHAT_TRACK_CURSOR_INVALID` `CHAT_TRACK_CURSOR_STALE` `CHAT_TRACK_CURSOR_EXPIRED` |
| 事件 / 用户请求 / 宿主 | `EVENT_CURSOR_INVALID` `EVENT_CURSOR_STALE`（实例重启或事件已被淘汰）`EVENT_CURSOR_EXPIRED` `AGENT_REQUEST_NOT_FOUND` `AGENT_REQUEST_EXPIRED` `AGENT_REQUEST_CLAIMED`（`details.claimedBy`）`HOST_ACTION_FAILED`（桌面端执行失败，或本实例没有桌面宿主） `RESOURCE_NOT_FOUND`（`GET /v1/resources/read`：没有这个资源，或插件 / 数据开关关着） |
| 客户端发现 | `SERVICE_NOT_RUNNING` `DESCRIPTOR_INSECURE` `DESCRIPTOR_INVALID` `STALE_DESCRIPTOR` `SERVICE_UNREACHABLE`* `INSTANCE_MISMATCH` `TRANSPORT_ERROR`* `RESPONSE_INVALID` |

带 * 的码 `retryable=true`；† 的 `retryable` 按远端情况逐次给出（429/500/503/504/408 与“服务暂时不可用”为 true；余额不足、文件无法处理、识别超时等为 false）。网络/连接错误不会被包装成“图像没看清”；全黑图也不等于“已经看懂聊天”；OCR 技术失败是错误，不是“没有文字”。

## 5. 已实现工具

除 `ocr.recognize`、`postprocess.test` 与陪玩的 `companion.ask` / `companion.read_chat` / `companion.check` 外所有工具 `dataEgress` 为 `none`：数据只在本机进程间传递（模型下载只从下载源取文件，不上传用户数据）。`ocr.recognize` 为 `backend-dependent`：选用本地后端时不外发，选用在线后端时只上传所选产物的 PNG（见 §8）。`postprocess.test` 为 `connection-dependent`：流程里的在线步骤会把样例文本发给其配置的连接。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `system.capabilities` | 可选 `protocolVersion` | 与 `GET /v1/capabilities` 相同 |
| `system.status` | 无 | `asr.runtimePhase`（stopped/starting/running/stopping）、`asr.requiredForTools=false`、产物计数；不含凭证 |
| `window.list` | 可选 `processName`（可执行文件名，大小写不敏感精确匹配）、`titleContains`、`includeUntitled`（默认 false）、`limit` | `windows[]`、`matched`、`truncated` |
| `window.active` | 无 | `window`，或 `window=null` + `reason="NO_FOREGROUND_WINDOW"` |
| `screen.capture` | `windowId`、可选 `ttlMs` | 捕获产物元数据 + `window` + `evictedArtifactIds` |
| `image.crop` | `artifactId`、`rect{x,y,width,height}`、可选 `ttlMs` | 新产物元数据（含父产物、根变换） |
| `artifact.get` | `artifactId`、可选 `includeContent`（默认 true） | 元数据 + `content{encoding:"base64",mimeType:"image/png",data}` |
| `artifact.release` | `artifactId` | `status`：`released` / `already-released` / `already-expired` / `already-evicted` |
| `image.preprocess` | `artifactId`、`preset` 或 `operations`（二选一）、可选 `ttlMs` | 新产物元数据 + `created`、`preprocess{preset,operations,scaleX,scaleY}`、`evictedArtifactIds`（§7） |
| `ocr.backends` | 无 | `backends[]`：`id/kind/ready/state/reason/languages/defaultLanguage/dataEgress/optionsSchema/details`，`fallback="none"`（§8） |
| `ocr.recognize` | `artifactId`、`backend`、可选 `language`、`options` | `ocrResultId`、原文 `text`、`observations[]`、`backend{id,kind,dataEgress,modelVersion}`、`frameId`、`rootTransform`、`empty/truncated/coldStart`、`warnings`（§8） |
| `chat.track.ingest` | `session`、`observation`（`{type:"ocrResult",ocrResultId}` 或 `{type:"external",artifactId,producer,lines[]}`）、可选 `generation`、`onSourceChange` | 可见消息集（稳定 ID、revision、证据、不确定性）、本次 `changes`、`cursor`、`warnings`（§9） |
| `chat.track.read` | `session`、`mode`（`visible` / `newSince`）、`newSince` 可选 `cursor`、`limit` | `visible`：`messages[]`；`newSince`：`changes[]`、`hasMore`；均含新 `cursor`、`generation`、观测时间、`stale`（§9） |
| `chat.track.reset` | `session`、`reason`（`new-match` / `reconnect` / `roi-changed` / `window-changed` / `manual`） | `previousGeneration`、`generation`、`cleared{messages,events,frames}`、新 `cursor`（§9） |
| `app.status` | 无 | 实例、版本、PID、运行时长、`app{host,launch,assistantRoute,kotoneHome}`、ASR 运行时状态、事件头、待处理请求数（§10） |
| `app.quit` | 无 | 先返回响应，再关闭服务、删除描述文件并退出（等同托盘“退出”） |
| `runtime.status` / `runtime.start` / `runtime.stop` | 无 | ASR 运行时 `phase`、`restartNeeded`、引擎与模型；start 预热模型可能需数秒，给足 `timeoutMs` |
| `event.read` | 可选 `cursor` 或 `from`（`oldest`/`latest`）、`types`、`limit`、`waitMs` | `events[]{seq,id,type,atUnixMs,data}`、新 `cursor`、`hasMore`、`headSeq`、`oldestSeq`、`closed`（§10） |
| `agent.requests` | 可选 `status`、`limit` | `requests[]`（新的在前）、`matched`、`truncated` |
| `agent.request.claim` | `requestId`、`consumer`（1-64 字符） | `request`、`newlyClaimed`（同一领取者重复调用为 false） |
| `agent.reply` | `text`、可选 `requestId`、`outputs`（`speech`/`danmaku`，缺省用助手默认提醒方式）、`wait` | `status`（`accepted` / `delivered`）、`deliveryId`、`receipt`（仅 `wait=true`）、`request`、`warnings`（§10） |
| `assistant.submit` | `text` | 与用户在助手文字入口提交相同的结果 |

### 自动化（委托桌面宿主，2026-11-16）

大模型或 Agent 写的一小段 Luau 程序（自动化）在沙盒里对游戏窗口执行（[演进计划 §12](product-evolution-game-tools.md#12-自动化大模型写程序沙盒执行2026-11-16-定方向)、[ADR 013](adr/013-automation-domain-model.md)）。调用时的**前台窗口**就是目标，先切到游戏。提醒也是自动化（“延迟 + 播报”），和程序共用一个调度器与执行记录。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `automation.run` | `id` + 可选 `args`（运行已保存的自动化），或 `source`（临时 Luau，≤8000 字）+ 可选 `name`；可选 `delaySeconds`（≤604800）或 `at`（Unix 毫秒） | 先按声明校验入参、检查语法（错误为 `INVALID_ARGUMENTS`）；播报类自动化不需要前台窗口。立即运行时同时最多 3 个（否则 `BUSY`）；带延迟时存为等待中（Kotone 重启后仍在）。返回 `{status:"started"/"scheduled", id, target, dueAtUnixMs}` |
| `automation.runs` | 无 | `running`、`pending`、`maxRunning`、`runs[]`：执行记录（含提醒），未结束的按到期时间在前，其后已结束的新的在前，最多 50 条 |
| `automation.save` | 可选 `id`（覆盖）、`name`（≤40）、可选 `description`（≤200）、可选 `params[]`（≤16）、`action`（`{kind:"program", source}`、`{kind:"speak", text, delivery}` 或 `{kind:"prompt", text}`）、可选 `schedule`、可选 `on`（游戏事件触发，ADR 036：`event[]`、`game`、`match`、`for`、`mode`、`oncePerMatch`、`cooldownSeconds`、`enabled`，事件名见 `game.state`） | 保存的自动化（补全 `label`、规范化缺省值）；`createdBy` 为 `agent`。程序先检查语法；`prompt`（“让她做”：触发时把这句话交给她跑一轮，每次一次 AI 调用，按事件触发时每局最多 3 次）不能有参数；按事件触发时和事件 `data` 同名的参数自动填上。`on` 里的事件名按合并后的事件目录校验（插件关着时它登记的也认得）；触发里的事现在不会发生时（例如英雄联盟插件关着）照常保存，结果多一项 `triggerNotes`（“英雄联盟插件关着，这个触发不会发生；到「插件」里打开以后照常触发”）。发布 `automation.saved` |
| `automation.get` | `id` | 一个已保存的自动化（含源码） |
| `automation.list` | 无 | `automations[]`（按名字排序，省略源码）、`max`（200） |
| `automation.delete` | `id` | `deleted`；已安排的执行保存的是快照，照常执行（要取消用 `automation.stop`）。发布 `automation.deleted` |
| `automation.stop` | 可选 `id` | 有 `id`：取消等待中的或停止运行中的那一条（提醒也可以）。没有 `id`：停止全部运行中的程序并取消全部等待中的程序，**等待中的提醒保留**。返回 `stopped`（涉及条数）；按住的键随即松开 |

**入参**（`params[]`）：每项 `name`（程序里的 `args.<name>`，小写字母开头，只含小写字母、数字、下划线）、可选 `label` / `description` / `default`（没有缺省值 = 必填），`type` 是 `number`（`min`、`max`、`integer`）、`text`（`maxLength` ≤200，不能换行）、`boolean`、`choice`（`options`）、`key`（键名）、`region`（`{x, y, w, h}` 窗口比例，可直接交给 `brightness` / `ocr`）、`color`（`{r, g, b}`）、`image`（`find_image` 的模板：提交 `{png: base64}`，例如 `screen.capture` → `image.crop` → `artifact.get` 拿到的 PNG，≤512×512、≤512 KB；保存为 `~/.kotone/automation-images/<sha256>.png`，参数里换成 `{image: <sha256>}`，执行记录只存引用）。运行时多给、少给、类型或范围不对都是 `INVALID_ARGUMENTS`，且不留执行记录。程序里 `args` 是只读表。已保存的自动化存在 `~/.kotone/automations.json`。

**执行记录**：`id`、`sessionId`（发起方）、`action`（`{kind:"speak", text, delivery}` 或 `{kind:"program", name, source, automationId, args, target:{label,…}}`）、`createdAtMs`、`dueAtMs`、`startedAtMs`、`finishedAtMs`、`status`（`pending` / `running` / `finished` / `failed` / `cancelled` / `missed`）、`message`（失败、停止或错过的原因，如“游戏窗口已关闭”“停止自动化热键”）、`report`（程序：`outcome` 为 `finished` / `stopped` / `limit` / `error`，`keys`、`durationMs`、`log[]`）。每条执行进入终态时发布 `automation.finished`，内容即这条记录（省略 `source`）。

**调度语义**：最多执行一次，**绝不自动重试**；到期超过 120 秒（Kotone 没开、休眠）记为 `missed`，不补执行；Kotone 退出时仍在运行的记录下次启动记为 `failed`；程序在**安排时**记下目标窗口，到点时窗口已关闭则 `failed`，不会改投当时的前台窗口；到点时已有 3 个程序在跑则这次 `failed`，不排队。

**白名单函数**（坐标一律是游戏窗口客户区比例 0..1，换分辨率仍可用）：

- 键盘：`key(名)` / `key_down` / `key_up`（键名如 `Q`、`Enter`、`Ctrl+Q`、`F1`）、`type_text(文字)`；
- 游戏聊天：`chat(文字, {channel="team"/"all"})`——经和 `game_chat.send` 同一个“发到游戏”的入口（ADR 041）：用游戏配置里的聊天键和频道、等修饰键松开、和别的输入排队，一行发一条。往聊天里发话用它，不要自己按回车；
- 鼠标：`click(x, y, {button="left"/"right"/"middle", count=1..3})`、`mouse_move(x, y)`、`mouse_down(键)` / `mouse_up(键)`（拖动 = 移动、按下、移动、松开）、`scroll(格数)`（正数向上，±20 以内）；
- 找色 / 找图 / 找字（参照 AutoHotkey 的 `PixelSearch` / `ImageSearch`）：
  - `find_color(区域, 颜色, {variation=0, largest=false, min_pixels=1})` → `x, y, 像素数` 或 `nil`：每个通道与目标相差不超过 `variation`（0 = 完全相同）。缺省按从上到下、从左到右返回**第一个**匹配像素；`largest=true` 返回**最大一块相连区域**的中心，适合拿去点击。颜色写 `{r=…, g=…, b=…}` 或 `"#RRGGBB"`；
  - `find_image(区域, args.<image 参数>, {variation=0, trans=颜色})` → 中心 `x, y`、左上 `x, y` 或 `nil`：按模板原尺寸逐像素比较（不缩放，游戏分辨率或界面缩放变了要重新截模板），`trans` 颜色的模板像素不参与比较（AHK 的 `*TransN`），返回第一个匹配。与 AHK 不同：先返回中心，便于直接 `click`；
  - `find_text(区域, 文字)` → 文字中心 `x, y` 或 `nil`：对区域做一次 OCR（陪玩选定的模型），跨行、忽略空白与大小写；只匹配一行的一部分时按字符比例估计横向位置；
- 其他：`wait(ms)`、`every(ms, fn, {for_ms, times})`（固定节拍、不补拍，`fn` 返回 `false` 结束）、`wait_until(条件, {poll_ms=200, stable_frames=1, timeout_ms})` → 是否满足、`now()`；`region(x, y, w, h)`（窗口客户区比例 0..1）、`brightness(区域)`（0..255）、`color_at(x, y)` → r, g, b、`ocr(区域)` → 只能 `:has(文字)` / `:count()` / `:find(文字)`（位置）的对象（拿不到原文）；`say(文字)`（语音，等播完）、`toast(文字)`（弹幕，不等）、`log(文字)`、`game_focused()`、`stop()`。没有文件、网络、进程、配置。

**护栏**：Luau 沙盒（无 `loadstring` / `require` / `io` / `os.execute`，内存 16 MB）；总时长 30 分钟、执行步数上限（死循环会被终止）、最多 10000 次输入（按键与鼠标动作）、两次输入至少 50 ms；输入与截图只在启动时的游戏窗口处于前台时进行，切走即暂停、切回继续（不补按）；窗口关闭则结束；自动化结束（含出错、超限、停止、Kotone 退出）时松开所有按住的键和鼠标键。鼠标移动用 `SendInput` 的绝对坐标（游戏读原始输入也能收到），换算偏差超过 2 像素时用 `SetCursorPos` 校正。停止方式：`automation.stop`、托盘「停止所有自动化」、**停止自动化热键**（`automationStopHotkey`，应用级，默认 `Ctrl+Shift+F12`，Kotone 停止时也有效）。`say` 与 `toast` 走提醒的输出队列：一条弹幕滚完之前，后面的语音会等它。

### 游戏（委托桌面宿主，2026-11-16）

游戏 = 一个有 `processNames` 的游戏配置（profile）；通用配置匹配任何进程，不算“在运行”。这些工具从不操作 Kotone 自己的窗口。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `game.list` | 无 | `games[]`：`id`、`name`、`processNames`、`running`、`foreground`、`windows[]`（`title`、`process`、`minimized`、`foreground`，最多 5 个）、`allowAutomationInput`、`canLaunch`。窗口标题来自别的程序，按数据对待 |
| `game.focus` | `profileId`，或 `processName` / `titleContains`（至少一个） | 把匹配的窗口切到前台（最小化的先还原），之后 `automation.run`、`companion.read_chat`、打字都作用于它。找不到或系统不允许切换时报错，不猜。副作用 `focuses-window` |
| `game.launch` | `profileId` 或 `installedId` | 用游戏配置里的**启动路径**（程序、快捷方式、`steam://` 之类的启动器链接，或带引号的程序加参数）打开；没填时用本机检测到的对应游戏（`game.installed`）；也可以直接给已安装游戏的 ID。有反作弊的游戏走平台启动器，部分启动器会让 Windows 弹管理员授权。已在运行时什么都不做（`alreadyRunning: true`）。立即返回，窗口出现可能要一会儿（用 `game.list` 看）。副作用 `launches-program` |
| `game.installed` | `query`（可选） | 本机装了哪些游戏：只读注册表和启动器清单、不扫全盘（Steam、英雄联盟国服 / Riot、米哈游 HoYoPlay、Epic、WeGame），每项 `id`、`name`、`source`、`installDir`、`launch`（可填进启动路径的命令）、`processNames`、`needsAdmin`；给 `query` 时按名字筛选，并在系统“卸载程序”列表里按名字搜（可能含非游戏）。目录已不存在的丢弃 |
| `game.state` | `game`（可选，游戏配置 ID） | 现在在玩什么（ADR 036 游戏中心）：每个在跑的游戏的阶段 `phase`（stopped / running / menu / queue / pre_match / loading / in_match / post_match；没有适配器的游戏一直是 running）、`nativePhase`（游戏自己的叫法，如英雄联盟的 `InProgress`）、`focused`、`runningSeconds`、`matchSeconds`（不在一局里为 null）、`attrs`（各游戏自己的状态，如 `lol.gameTime`）、最近的事 `recent [{type, secondsAgo, gameTime, data}]`；还有事件目录 `events`：每个适配器能给哪些事和它们的 `data` 字段（`generic` 的 `game.*` 所有有进程名的游戏配置都有；游戏专用的由带适配器的插件在自己的 `plugin.toml` 里登记，例如英雄联盟插件的 `lol.*`，插件或它要的数据开关关着时不列，只有一句 `off` 说为什么），自动化的 `on` 用这些名字。只读 |
| `game.close` | `profileId` | 给这个游戏的每个窗口发普通的关闭请求（相当于点窗口的 ×，游戏可能弹确认），从不强制结束进程。返回收到请求的窗口数。副作用 `closes-program` |
| `game.look` | `zoom`（可选，`[x0, y0, x1, y1]`） | 看前台游戏（[ADR 019](adr/019-game-computer-use.md)）：只截游戏窗口客户区，按 Anthropic computer use 的规则缩放（长边 ≤ 1280、约 115 万像素，不放大），文字识别出的每一行框出来并编号（OmniParser 的标签样式）。返回 `observationId`、`screenshot {width, height}`（`game.act` 的坐标系）、`window {title, process, width, height}`、`elements [{id, text, box}]`、`image {mimeType: image/jpeg, base64, width, height}`；没有可用的文字识别模型时照样给截图，附 `ocr` 说明。`zoom` 是上一张完整截图里的区域，从原分辨率裁剪放大，元素和之后的坐标仍用完整截图的坐标系。前台是 Kotone 自己、独占全屏（纯色画面）时报错。结果是屏幕数据（不可信）。副作用 `captures-screen` |
| `game.act` | `observationId`、`actions`（1–20 步） | 一步一步操作游戏。`observationId` 必须是最近一次 `game.look` 的；动作名与参数照抄 Anthropic computer use：`left_click` / `right_click` / `middle_click` / `double_click` / `triple_click`（`coordinate [x, y]` 或 `element` 编号，`text` = 点击时按住的修饰键）、`left_click_drag`（`start_coordinate`、`coordinate`）、`mouse_move`、`left_mouse_down` / `left_mouse_up`、`scroll`（`scroll_direction`、`scroll_amount`）、`type`（`text`，50 字一组）、`key`（`text` 如 `i`、`ctrl+s`、`Return`，`repeat`）、`hold_key` / `wait`（`duration` 秒 ≤ 10，一次合计 ≤ 20），游戏补充 `key_down` / `key_up`、`mouse_move_by`（`dx`、`dy` 相对移动，转视角）。规则照抄 dsh-computer-use-win `batch`：先校验整份计划；按顺序执行，第一处失败就停，后面的记 `Not executed: an earlier action failed.`；每步前检查停止（操作期间玩家按 Esc——这个 Esc 被拦下、不传给游戏；停止热键；语音“停下”；本轮停止；玩家别的键鼠输入**不**会让它停）、游戏在前台（最多等 5 秒）、窗口尺寸没变；点被别的程序窗口盖住时不点。结果 `{sent, total, results [{index, action, status sent/failed/stopped/not_executed, message}], stopReason?（cancelled / window_changed / window_closed / not_foreground / point_covered / action_failed）, hint, look}`：只说动作已发出，结尾自动再看一眼（玩家叫停时不看）。游戏配置不允许自动化输入或“让琴音操作”为不允许、自动化程序正在运行时拒绝。期限：调用时传 `timeoutMs`（建议 30000）。副作用 `sends-game-input` |

### 游戏语音（游戏语音插件，2026-11-17，[ADR 020](adr/020-voice-into-game-chat.md)；2026-11-24 搬进插件，ADR 038）

琴音用虚拟麦克风 VB-CABLE（VB-Audio 的捐赠软件，https://vb-cable.com）替玩家在游戏语音里说话。由 `plugins/game-voice` 提供，工具名没变；插件关着（缺省）时这些工具不出现。开关是 `plugins.game-voice.enabled`（开着 = 转发开着），自己的设置在 `plugins.game-voice.settings`：`includeMic`、`micDeviceId`、`micGain`（`settings.update`）。旧配置的 `voiceChat.*` 读配置时自动搬过来。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `voice_chat.status` | 无 | `driverInstalled`、`installing`、`devices {cableInput, cableOutput}`、`enabled` / `includeMic` / `micDeviceId` / `micGain`、`running`（转发在跑）、`error`、`windowsDefaultMic` / `windowsCommunicationsMic`、`windowsDefaultIsCable`、`canRestoreDefaultMic`、`ttsCanSpeak`（系统语音送不进游戏）、`attribution` |
| `voice_chat.install` | 无 | 自动安装 VB-CABLE：从官网下载 `VBCABLE_Driver_Pack46.zip`（固定 SHA-256），解压到 `<KOTONE_HOME>/drivers/vbcable/`，以管理员身份静默运行 `VBCABLE_Setup_x64.exe -h -i`（Windows 弹一次授权），等设备出现。返回 `installed`、`needsReboot`；已装时 `alreadyInstalled`。副作用 `installs-driver`、`shows-uac-prompt` |
| `voice_chat.default_mic` | `use`：`cable` / `restore` | 把 Windows 默认录音设备与默认通信设备设成 CABLE Output（先记下原来的），或改回。改的是系统设置，别的程序也会用它。副作用 `writes-system-audio` |
| `voice_chat.say` | `text`（≤ 200 字） | 用当前语音合成一次，送进虚拟麦克风，玩家自己也听得到。要求游戏语音在运行、语音引擎不是系统语音。副作用 `speaks-to-teammates` |
| `voice_chat.test` | 无 | 往 CABLE Input 放 1 秒 440 Hz 测试音，同时从 CABLE Output 录回来；`ok`、`peak`、`note`。在游戏语音里时队友会听到一声。副作用 `speaks-to-teammates` |

**让琴音操作**：游戏配置的 `computerUse`：`ask`（缺省，内置 Agent 每段对话先问一次）/ `allow`（一直允许）/ `deny`。哪些游戏能操作由玩家判断，Kotone 不单独区分竞技游戏。外部 Agent 调 `game.act` 不经过询问（权限由它们自己的运行框架管），但游戏配置的拒绝同样生效。Kotone 发出的模拟输入在 `dwExtraInfo` 带标记 `0x4B544E49`（"KTNI"）：`game.act` 自己按的 Esc 照常传给游戏，只有玩家按的 Esc 才算停止。

**允许自动化输入**：游戏配置的 `allowAutomationInput`（缺省允许）为 `false` 时，`automation.run` 对这个游戏的程序执行会被拒绝（安排时检查，带延迟的到点时再检查一次）；玩家自己的语音打字不受影响。启动路径存在游戏配置的 `launchPath`。

### 插件（委托桌面宿主，2026-11-24，[ADR 038](adr/038-kotone-and-apps.md)）

插件（`plugins/<id>/`）的工具和本体的工具一起列在 `system.capabilities` 里，调用方式一样（MCP 里同样映射成 `kotone_<名字>`）。只列**开着的**插件、而且工具要的数据开关都开着的；开关在设置 `plugins.<id>`（`enabled`、`sources.<开关>`）。每个插件工具的说明多几个字段（照 MCP Apps）：

- `plugin`：哪个插件；
- `untrustedResult`：结果里有别人写的字（玩家名、聊天），只当数据；
- `_meta.ui.visibility`：`["model", "app"]` = 给 Agent 也给插件自己的页面，`["app"]` = 只给页面用（翻页、详情、搜索）；
- `_meta.ui.resourceUri`：结果用哪个页面显示（`ui://kotone/<插件>/<页面>`）。

结果照 MCP 的工具结果：`{content, structuredContent?, _meta?: {ui: {resourceUri, params}}}`。`content` 是给 Agent 的（要短）；`structuredContent` 是给页面的数据。插件关着时调用返回 `TOOL_NOT_FOUND`。

| 插件 | 工具 | 参数 | 结果要点 |
| --- | --- | --- | --- |
| 英雄联盟（`lol`，数据开关 `client` 读客户端数据） | `lol.match` | 无 | 这局现在的样子（模式、时间、比分、10 个人的英雄 / 技能 / KDA / 装备、对面谁闪没好、谁没带闪）。只读本机的局内接口 |
| | `lol.spell` | `champion`、`spell`（缺省闪现）、`ago`、`clear` | 记下谁交了召唤师技能，返回几点好和一句能发到聊天框的 `chat` |
| | `lol.scout` | `player`（可选） | 这局 10 个人最近 20 局的战绩、开黑、突破口。进加载界面就在后台查。页面 `ui://kotone/lol/match`（这局） |
| | `lol.player` | `player`（这局里的英雄名或玩家名，或“名字#编号”）或 `puuid`；都不写 = 自己 | 一个人最近 20 局的战绩、段位、常玩英雄、每一局。页面 `ui://kotone/lol/player`（战绩） |
| | `lol.game`（只给页面） | `gameId` | 一局的详情：两边 10 个人的 KDA、伤害、经济、补刀、装备、海克斯。页面 `ui://kotone/lol/game` |
| 英雄联盟（数据开关 `stats` 用网上的统计） | `lol.champion` | `champion`（名字、称号或外号）、`mode`（`rift` / `mayhem`，不写看这局）、`lane` | 峡谷：强度、胜率、克制、出装、符文、加点、召唤师技能（腾讯 101）；海斗：英雄榜名次、搭档、最好的海克斯（aramhex-data，CC BY-NC 4.0，要署名）。数据开关 `opgg` 开着时加 OP.GG 的分析。页面 `ui://kotone/lol/champion` |
| | `lol.tier` | `mode`、`lane` | 峡谷各路 / 海斗的强度榜。页面 `ui://kotone/lol/tier` |
| 英雄联盟（数据开关 `control` 操作客户端，**默认关**，要 `client` 也开着） | `lol.client` | `action`：`accept` / `decline` / `pick` / `ban` / `hover`；`champion` | 照 LeagueAkari 经客户端的本机接口接受 / 拒绝对局、选（并锁定）/ 禁 / 亮出英雄。副作用 `controls-game-client`（她默认先问玩家）。有封号风险（LeagueAkari #523） |
| | `lol.champions`（只给页面） | 无 | 全部英雄的 id 和名字（设置里排自动选 / 禁的顺序） |

**打开页面**：`ui.open {page, params?}` 在 Kotone 窗口里打开一个页面：插件的页面（上面各工具的 `resourceUri`，例如 `ui://kotone/lol/player` 带 `{puuid}`）在聊天旁边的右侧栏；`ui://kotone/settings/<分页>`（models、connections、sound、tuning、appearance、system、browser）打开设置页；`plugins` 打开左侧栏的「插件」；`call`、`reply`、`permissions`、`usage` 打开角色页的唤醒、交互、权限、用量（2026-11-25 从设置搬过去，地址没变）。不会把窗口拉到前面。**不打断玩家正在看的**（2026-11-25，ADR 039 实施记录“ui.open 不打断玩家”）：玩家在聊天页、右侧栏空着（收着、停在“卡片”、“网页”里没开网页）或显示的就是这个插件时直接显示（同一个插件的下一页往上叠，“返回”回到刚才那页），设置页在聊天页时直接跳过去；别的时候不切页、不换掉，放在右侧栏“卡片”最上面、窗口里出一个提示，玩家点“打开”才打开。同一个页面（地址 + 参数）反复打开只算一次。结果：`opened`（已经显示了）、`status`（`shown` / `waiting` = 放在右侧栏等玩家点 / `sent` = 窗口没回话）、`uri`、`title`、`note`（照着告诉玩家的话）。她和外部 Agent 同一条规矩；外部 Agent 调它照旧不经 Kotone 询问（和上面的 `game.act` 一样，权限由它们自己的运行框架管，ADR 014 第 5 节）。页面不存在、插件或它要的数据开关关着时报 `INVALID_ARGUMENTS`。

### Kotone 陪玩（委托桌面宿主，2026-11-15）

与用户按陪玩热键 / 快捷消息热键走的是**同一条流程、同一个实现**（[演进计划 §10](product-evolution-game-tools.md#10-kotone-陪玩2026-11-15-定稿)、[陪玩说明](assistant.md)）：截前台窗口 → 统一清单里选定的 OCR 模型 → 陪玩自己的聊天跟踪 + profile 说明 → 一次大模型调用，从提醒 / 帮忙打字 / 回答中选一种并执行。底层的截图、OCR、跟踪工具照常保留。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `companion.status` | 无 | `ready`、`reason`（缺什么：AI 连接与密钥、OCR 模型）、`check`、`ocr{modelId,state{dataEgress…}}`、`autoSend`、`quickMessages[]`、当前 profile 的 `notes` 与 `chatRegion` |
| `companion.ask` | `text` 或 `quickMessageId`（二选一） | 受理即返回 `{status:"started", jobId}`；结果为 `companion.finished` 事件：`jobId`、`action`（`reminder` / `type` / `answer` / `clarify`）、`text`、`reminders[]`、`result`（已调度的提醒 / 打字回执 / 实际交付方式）、`ocr{model,backend,dataEgress,lines,region}`、`chatRegionLearned`、`warnings`、`timing{captureMs,ocrMs,planMs}`、`durationMs`；失败为 `companion.failed {jobId, message}`。前置条件不满足（没有通过自检的连接）时直接报错，不受理；另一条陪玩请求进行中返回 `BUSY` |
| `companion.read_chat` | 无 | 流程前半段，不调大模型：`window`、`ocr{model,backend,dataEgress,region,lines[{text,box,confidence}]}`（框为截图像素）、`chatTracking[{messageId,text,newThisRead,firstSeenMsAgo}]`、`profile{name,notes}`、`warnings`、`timing` |
| `companion.type` | `text`（≤200 字，单行）、可选 `send`（缺省跟随设置 `autoSend`，默认 true） | 旧工具，经和 `game_chat.send` 同一个“发到游戏”的入口（ADR 041；新的调用用 `game_chat.send`：能选频道、能多行、写执行记录）：按前台游戏的游戏配置（对不上就用当前选中的）开聊天框、打字、按发送键，等修饰键松开，和 Kotone 别的输入排队，前台不是游戏时不发；`send=false` 只打进聊天框不发送。回执 `{typed, sent, profile, receipt}` 加上 `route`、`channel`、`lines`、`sentLines`，只证明本机按键已执行，不证明服务器已收到 |
| `companion.check` | 无 | 测试助手所选连接（读一次服务商的模型列表，不花 token；没有列表接口时发一次短对话），只用于显示、不是使用前提，受理即返回 `{status:"started"}`；结果（同时写进设置）为 `companion.check.finished {check}` 或 `companion.check.failed {message}` |

数据外发：`companion.ask` / `companion.check` 把请求与识别出的聊天发给用户的 AI 连接（`connection-dependent`）；所选 OCR 模型是在线模型时，`companion.ask` / `companion.read_chat` 每次都会上传截图（`ocr.dataEgress=selected-image`）。识别出的文字对 Kotone 与 Agent 都只是数据，不是指令；模型能选的动作只有上面三种。

**发到游戏的结果**（`game_chat.send`、`companion.type`，ADR 041）。只加不改（单向门）：

- `route`：这次走了哪条路。`keys` = 开聊天框 → 打字 → 发送键（游戏配置里的键）；`console` = Source 2 控制台通道（ADR 032）；`client` = 游戏客户端自己的聊天接口，不按键（ADR 041 第 2 期：英雄联盟客户端在前台时，选英雄、房间里经英雄联盟插件发，几行连成一条消息）。
- `plugin`：走了游戏插件提供的发法时是那个插件的 id（例如 `lol`）；本体的通用发法没有这个字段。
- `channel` / `channelName`：实际的频道。客户端的聊天里：选英雄、房间只有自己这边看得到（`team`，`channelName` 是“选英雄的聊天”“房间的聊天”）；自定义对局两边都看得到（`all`）。要的频道和这里不一样就不发。
- `receipt`：按键的“只证明本机按键已执行，不证明服务器已收到”；客户端的“只证明客户端收下了，不证明队友已经看到”。
- 一行都没发 → 工具返回错误（话里说原因和下一步）；发了一部分 → `sent: false`、`stopped: {code, message}`。原因代码（`stopped.code`、执行记录、进程日志）：第 1 期的 `emptyText` / `tooManyLines` / `lineTooLong` / `multiLineWithoutSend` / `noForeground` / `ownWindow` / `desktop` / `notAGame` / `noSuchChannel` / `modifiersHeld` / `foregroundChanged` / `busy` / `timeUp` / `cancelled` / `inputBlocked` / `needsElevation` / `injectFailed`，第 2 期加 `noChatHere`（这里现在没有能发的聊天：不在房间或选英雄里、聊天还没连上、在对局里、在结算）、`clientUnavailable`（连不上游戏客户端）、`clientRejected`（客户端回了错误，没发）、`clientNoReply`（客户端没在期限内回话：**可能已经发出去了**，不要重发）、`providerOff`（前台是插件登记的窗口，但插件或它要的开关关着：替玩家发言的不往那个窗口里按键）、`noDraftHere`（这里经客户端发，没法 `send: false` 只打进聊天框）。
- 客户端的发法不重试、失败也不退回按键；只有插件关着时，玩家自己的语音输入才照旧按键。

### 配置与首次引导（委托桌面宿主）

下列 35 个工具让 Agent 完成引导页与设置页里的全部配置。它们由服务检查“参数是 JSON 对象”后交给桌面宿主，调用的是与设置页**同一个**实现（校验、原子落盘、热键重注册、运行时状态推送完全一致），参数按工具严格解析（未知字段、类型或取值不合法 → `INVALID_ARGUMENTS`；已有下载进行中 → `BUSY`；执行失败 → `HOST_ACTION_FAILED`）。没有桌面宿主的实例里它们 `ready=false`。改动设置后，打开着的设置窗口会自动刷新。

| 工具 | 参数 | 结果要点 |
| --- | --- | --- |
| `settings.get` | 无 | 与设置页相同的完整设置（camelCase）；连接条目不含 API key |
| `settings.describe` | `query?`（关键词） | `fields[]`：每个用户可改的设置的点分路径、中文名、在设置页的位置、含义、可选值和生效方式（`immediate` / `restart-voice` / `restart-app` / `next-launch`）；`dedicated[]`：要用专用工具改、或不能由 Agent 改的设置。说明表与真实设置由测试保证一一对应 |
| `settings.update` | `patch`（部分设置，深合并） | `settings`（更新后）、`runtime`（含 `restartNeeded`）、`effects`（`changed` 改动的路径、`restartVoice`、`restartApp`、`nextLaunch`、给用户的 `summary`）。立即生效：热键（运行时运行中重注册）、助手去向与提醒方式、悬浮窗、音效等；换引擎/模型使 `restartNeeded=true`。`connections` 与 `models.dir` 不接受（用下面的专用工具）；热键改成和别的热键、Esc、游戏配置的聊天键一样的键不接受（先用 `hotkeys.conflicts` 查，ADR 040）；`toolService.*` 下次启动生效。完成首次引导：`{"ui": {"firstRunCompleted": true}}` |
| `audio.devices` | 无 | `devices[]`（标出默认与虚拟声卡）、`selected`；选择用 `settings.update {"audioDeviceId": …}` |
| `stt.engines` | 无 | `engines[]`（可用性）、`current` |
| `models.list` | 可选 `category`（`asr` 缺省 / `ocr` / `tts` / `search`） | `models[]`（本地/在线、大小、是否已下载、配置项、API key 页面）、`dir{dir,isDefault}`、`active`、`downloading`。`asr` 的返回与加入类别前相同（`active{engine,modelId}`）；`ocr` / `tts` 多 `category`，条目另含 `ready`/`state`/`reason`、`active`、`recommended`、`languages`、`dataEgress`（`selected-image` = 每次识别上传截图）、`files[]`，`active{modelId}` |
| `models.download` | `id`、可选 `category` | 受理即返回 `status=started`；进度 `model.download.progress`（至多每秒一次），结果 `model.download.finished` / `model.download.failed`（取消时 `cancelled=true`；OCR / TTS 的事件带 `category`）。镜像优先、断点续传、逐文件 SHA-256、先查磁盘空间；全应用（三个类别）同时只有一个下载 |
| `models.download.cancel` | 无 | `requested`（是否有下载在进行）；保留临时文件以便续传 |
| `models.delete` | `id`、可选 `category` | `wasActive`（删的是活动模型时回退默认；TTS 删除 Kokoro 时改回系统语音） |
| `models.activate` | `modelId`；`asr` 还需 `engineId`；可选 `category` | `asr`：运行时状态（运行中会 `restartNeeded`，`runtime.start` 重启生效）。`ocr`：陪玩流程所用模型（`ocr.recognize` 仍须显式指定后端）；`tts`：播报引擎；`search`：琴音 `web.search` / `web.fetch` 用的联网搜索（`deepseek-search` / `off`，ADR 026）。本地模型须先下载，在线模型可先选后配；返回 `{category, activeModelId}` |
| `models.dir.set` | `dir`（空 = 默认 `~/.kotone/models`） | `dir`、`moved[]`、`failed[]`；运行时运行中或下载中拒绝。选目录前先问用户 |
| `models.install_guide` | `id`、可选 `category` | 自动下载失败时的手动安装指引（OCR 包在设置页「导入」时仍逐文件校验） |

在线模型的配置项（例如 DeepSeek OCR 绑定的 API 连接）用 `settings.update {"modelConfigs": {"<模型 id>": {"connection": "<连接 id>"}}}` 写入；密钥只在凭据管理器，用 `connections.upsert` 写。
| `speech.status` | 无 | `package`（Kokoro 语音包：installed/downloading/字节/阶段/错误）、当前语音引擎的 `voices[]`（编号为 `提供方/声音`，如 `system/…`、`kokoro/zf_001`、`openai/alloy`、`volcano/BV700_streaming`、`deepseek/stella`）、`voicesError`。默认“自动”：琴音的连接是 DeepSeek 时用 DeepSeek 语音（`deepseek-tts`），否则系统语音（`settings.update {"assistant": {"ttsModelId": ""}}` 回到自动）。语音引擎也可用 `models.activate {"category": "tts"}` 明确选择（`deepseek-tts` / `system-speech` / `kokoro-zh` / `openai-compat-tts` / `volcano-tts`），在线引擎的连接、模型与声音写 `settings.update {"modelConfigs": {…}}` |
| `speech.model.download` | 无 | `status=started`（或已安装时 `installed`）；结果 `speech.model.download.finished` / `failed`；与模型下载共用唯一名额 |
| `wake.status` | 无 | 语音唤醒：`state`（`off` / `waitingRuntime` 语音识别没在运行 / `modelMissing` / `listening` / `error`，出错带 `message`）、`modelReady`、`downloading`、`detections`（本次启动以来唤醒次数）。开关与唤醒词等写 `settings.update {"assistant": {"wake": {…}}}`（见 `settings.describe "唤醒"`）；只在语音识别运行时监听，开启后麦克风常开（本机检测） |
| `wake.model.download` | 无 | 下载唤醒模型（官方整包约 33 MB，只保留约 5.5 MB；逐文件 SHA-256），完成或失败才返回（给足 `timeoutMs`）；进度 `wake.model.progress`（至多每秒一次）；返回同 `wake.status` |
| `connections.presets` | 无 | 内置在线连接预设 |
| `connections.list` | 无 | `connections[]{id,displayName,kind,provider,baseUrl,model,hasApiKey}`——**API key 只写不读** |
| `connections.upsert` | `connection`、可选 `apiKey` | 更新后的 `connections[]`；key 存入系统凭据库，不回显、不进日志；省略则保留原 key |
| `connections.delete` | `id` | 更新后的 `connections[]`（同时删除其 key） |
| `profiles.list` | 无 | `profiles[]`、`activeProfileId`；激活用 `settings.update {"activeProfileId": …}` |
| `profiles.save` / `profiles.delete` | `profile` / `id` | 保存（内置 profile 记住被删的内置热词）/ 删除（内置恢复出厂，导入的永久删除） |
| `hotkeys.status` | 无 | 热键注册状态（失败原因，如被其他程序占用）；`bindings[]`：全部热键（本体的、插件的、快捷指令、固定的 Esc），每项有键、绑的命令（`command`，如 `kotone.call`、`voice-input.talk`）、来源、字段、在哪改（`page`）和 `conflicts`（冲突的 id，可能指 `gameKeys[]` 里游戏配置的聊天键、控制台触发键；ADR 040） |
| `hotkeys.conflicts` | `key`、`except`（正在改的那一项的 id，可省） | `conflicts[]`：已经占着这个键的热键、Esc、游戏配置的聊天键和控制台触发键（有就别用这个键，ADR 040 D6）；`warnings[]`：与常见游戏键位的冲突提示 |
| `input.check` | 无 | 低级键盘钩子与模拟输入（SendInput）自检，不需要先启动运行时 |
| `elevation.status` | 无 | 是否已提权、能否提权、前台游戏是否以管理员运行 / 独占全屏 |
| `elevation.restart` | 无 | 先返回 `status=restarting`，再弹系统 UAC：用户同意后本实例退出、新的提权实例接替（新 `instanceId`，用 `app status` 重连）；拒绝则发布 `elevation.restart.failed`，本实例继续运行 |
| `autostart.status` / `autostart.set` | 无 / `enabled` | 开机自启（真实的 HKCU Run 项） |
| `history.list` | 可选 `limit`（1-1000，默认 50） | `records[]`（新的在前）、`total`、`truncated`——内容是用户说过的话，属于隐私数据 |
| `history.delete` / `history.clear` | `sessionId`+`ts` / 无 | 删除单条（幂等）/ 清空（含录音）；历史页自动刷新 |
| `diagnostics.export` | 可选 `window`（`24h`/`7d`/`all`） | 脱敏诊断 ZIP 的 `path`（`<KOTONE_HOME>/exports/`），不含识别文字、音频与热词 |
| `diagnostics.clear` | 可选 `stopRecording` | 清除本地流程事件与 `kotone.log` |
| `app.resources` | 无 | CPU 与内存占用 |
| `usage.summary` | 无 | 用量与费用（`<KOTONE_HOME>/usage.jsonl`）：今天、本月、按用途（对话 / 压缩 / 复盘 / 语音合成 / 语音识别 / 文字识别）、按模型、最近 30 天；token、命中缓存的 token 与命中率、字数、音频时长、张数、按币种的估算费用（DeepSeek 用人民币，分高峰 / 空闲）。估算值，以服务商账单为准 |
| `roles.list` / `roles.get` | 无 / `id` | 可切换的角色：id、名字、介绍、头像文件、声音（`provider/voice`，空 = 全局）、唤醒词及其拼音检查、是否当前；`get` 另带性格全文（ADR 025） |
| `roles.switch` | `id` | 换角色：名字、头像、性格、声音、唤醒词一起换，下一句话开始新对话（记忆共用）；琴音正在回答时拒绝。副作用 `writes-settings` |
| `roles.create` / `roles.update` | `name` 等 / `id`+`patch` | 新建 / 修改角色：`name`、`summary`、`voice`、`wake {text, readings}`（3–8 个汉字，多音字的读音）、`greeting`、`soul`；头像只能由用户在设置页上传 |
| `roles.delete` | `id` | 删除自己建的角色（内置的改回默认）；删的是当前角色就换回琴音 |
| `game.recap` | 可选 `game`（游戏配置 id）、`sinceMinutes`（默认 60） | 现在就复盘：最近这段时间琴音的对话、工具、提醒、自动化 → 聊天里一条复盘 + 合并进这个游戏的记忆。Kotone 在游戏进程退出时也会自己做（`agent.matchRecap`）。调一次用户的 AI 连接；没发生什么就跳过 |
| `app.restart` | 无 | 先返回 `status=restarting`，再走正常退出流程后重启（同一用户，不弹 UAC；新 `instanceId`）。用于 `effects.restartApp` 不为空时 |
| `postprocess.processors` | 无 | 已注册的文字处理模块与配置 schema |
| `postprocess.test` | `text`、可选 `pipeline` | 逐步输入输出与耗时；不发送、不记录 |

典型的首次引导（替代引导页）：`models.list` → 问用户模型放哪（`models.dir.set`）→ `models.download` 并等 `model.download.finished` → `models.activate` → `audio.devices` 选麦克风 → `hotkeys.conflicts` 后用 `settings.update` 设热键与交互模式 → `profiles.list` 选游戏 → 需要在线能力时 `connections.presets` / `connections.upsert`（向用户要 key）→ `input.check`、`elevation.status`（游戏以管理员运行时 `elevation.restart`）→ `runtime.start` → `settings.update {"ui": {"firstRunCompleted": true}}`。

**窗口引用**：`windowId` 是不透明字符串，只在发放它的实例内有效，绑定窗口句柄 + PID + 进程创建时间（`identityBinding` 说明强度；无权查询创建时间时退化为 `handle+pid`）。每次捕获前重新校验；同一窗口多次列举得到同一 ID。窗口元数据含标题、类名、可执行文件名（不含目录）、PID、前台/可见/最小化/DWM 隐藏标记、客户区与所在显示器的**屏幕物理像素**矩形、DPI 与 `scale = dpi / 96`。

**产物**：不可变 PNG（RGBA8）。元数据含 `kind`（`capture` / `crop` / `preprocess`）、`width/height`、`byteLength`、PNG 字节的 `sha256`、`createdAtUnixMs`（发布到存储的时刻，TTL 由此起算）/`expiresAtUnixMs`/`ttlMs`、`parentArtifactId`、`rootArtifactId`、`rootTransform`（`rootPoint = offset + scale × point`，像素边界坐标；裁剪只平移，预处理放大 N 倍后 scale 乘 1/N）、`rootRect`、采集时刻的 `screenRect`、`derivation`（捕获为 null；裁剪 `{type:"crop",rect}`；预处理 `{type:"preprocess",preset,operations,scaleX,scaleY}`），以及整条派生链共享的 `source`（`frameId`、`windowId`、`captureMethod`、采集时客户区屏幕坐标、DPI、`capturedAtUnixMs`、进程名、警告）。`capturedAtUnixMs` 记录捕获后端返回像素的时刻，早于像素校验、PNG 编码与哈希；裁剪沿用根产物的采集时刻，`createdAtUnixMs` 才是各自的发布时刻。不同 `frameId` 的裁剪坐标不能互相套用。`offsetX/offsetY` 与 `rootRect`/`screenRect` 为整数时以 JSON 整数输出（与 M1 一致），放大后再裁剪等情况可为小数（保留 3 位）；`scaleX/scaleY` 始终是小数。

**裁剪**：坐标空间 `artifact-physical-pixels`，以输入产物左上角为原点。负坐标、零宽高、越界或整数溢出直接返回 `INVALID_ARGUMENTS`（`details.reason` 为 `negative-origin` / `empty-rect` / `out-of-bounds` / `overflow`），不截断、不自动修正；输入产物保持不变。释放父产物不影响已生成的子产物。

**存储**：仅在桌面进程内存中，不落盘；服务自带一个有界的空闲维护任务（只持有服务弱引用，关闭时退出），即使没有任何请求也会按周期回收到期产物；超出容量时回收最旧产物；桌面退出即全部丢弃。服务只接受产物 ID，不接受文件路径。

## 6. 捕获后端：GDI 前台可见客户区

当前唯一后端 `captureMethod = "gdi-screen-foreground-client"`：目标必须是前台窗口、未最小化、可见且未被 DWM 隐藏、客户区完整位于一个显示器内，然后用 GDI `BitBlt(SRCCOPY | CAPTUREBLT)` 从屏幕 DC 复制该客户区矩形的**已合成像素**，复制后再次核对前台、位置与尺寸。限制（同时出现在 capabilities 的 `capture.limitations`）：

- 得到的是“屏幕上该矩形里看到的像素”：覆盖在其上的置顶窗口、通知以及 Kotone 弹幕这类分层（`WS_EX_LAYERED`）叠加层也会出现在图中；不是窗口内容隔离捕获，不恢复被遮挡内容。受控原生测试验证了分层叠加层被合入；在验证所用的 Windows 11 上不加 CAPTUREBLT 时同样被合入，CAPTUREBLT 用于覆盖不会默认合入分层窗口的系统。CAPTUREBLT 在部分系统上可能让鼠标指针短暂闪烁，指针本身不会出现在图中。
- 后台、遮挡、最小化、隐藏窗口不能捕获。非前台目标返回 `WINDOW_NOT_FOREGROUND`——服务不会替调用方切换焦点。Agent 通常在用户正在游戏时调用；在终端里手动运行 CLI 时前台是终端本身。
- 客户区跨显示器或部分超出屏幕时返回 `INVALID_GEOMETRY`，不会退化为截取整个桌面。
- 独占全屏、硬件叠加层或受保护内容可能得到黑图或旧帧：全部像素相同会附带 `UNIFORM_PIXELS` 警告，系统报告 Direct3D 独占全屏时附带 `EXCLUSIVE_FULLSCREEN_SUSPECTED`。对 LOL 等真实游戏的可用性尚未验证（见[实现记录](research/game-tools-m1-implementation.md)）。
- 坐标在线程级 Per-Monitor-V2 DPI 上下文中读取（物理像素），进入后核实线程确实处于 per-monitor 感知；切换失败且线程本身也不是 per-monitor 感知时，`window.*` 与 `screen.capture` 返回 `PLATFORM_UNSUPPORTED`，不会以未经验证的坐标冒充物理像素。调用结束恢复原上下文；每个 DC/位图都由守卫释放。

## 7. 图像预处理（`image.preprocess`）

预处理是可选工具：Agent 可以只用原图或裁剪图，也可以对比原图、放大图与 OCR 结果。每次调用必须明确给出 `preset` 或 `operations` 之一（同时给出或都缺省 → `INVALID_ARGUMENTS`），结果总会返回展开后的实际操作列表。

| 预设 | 展开为 |
| --- | --- |
| `none` | 不处理：不创建新产物，原样返回输入产物（`created=false`） |
| `upscale-2x-cubic` / `upscale-3x-cubic` | `[{op:"scale",factor:2/3,filter:"cubic"}]` |
| `grayscale` | `[{op:"grayscale"}]` |
| `gray-autocontrast-2x` | `[grayscale, {op:"autoContrast",clipPercent:1}, {op:"scale",factor:2,filter:"cubic"}]` |

| 操作 | 参数 | 说明 |
| --- | --- | --- |
| `scale` | `factor` 0.25–4，`filter` `nearest`/`linear`/`cubic`（缺省 cubic；OpenCV 语义，cubic a=-0.75） | 输出尺寸 `round(输入×factor)`；**实际比例** = 输出/输入，分别记入 `preprocess.scaleX/scaleY` 与 `rootTransform` |
| `grayscale` | — | BT.601 亮度写入 RGB |
| `invert` | — | RGB 取反 |
| `autoContrast` | `clipPercent` 0–10 | 按亮度直方图两端各裁掉该比例后线性拉伸 |
| `threshold` | `value` 0–255 | 亮度 ≥ value 为白，否则黑 |

最多 8 个操作；输出超过单图像素上限、比例越界、未知操作或多余字段（如 `path`）均 `INVALID_ARGUMENTS`，不截断。输入产物不变；预处理产物与裁剪产物一样可继续裁剪、预处理或 OCR，所有坐标经 `rootTransform` 精确映射回原始捕获（例如 3 倍放大后在 `(30,60)` 裁剪，再在裁剪图 `(6,3)` 处的框，对应原图 `(12,21)`）。

## 8. OCR 后端与安装

OCR 模型与 ASR、TTS 同在统一模型清单（`models.list {"category": "ocr"}`，设置页「高级 → 模型 → 文字识别」）：每个本地包和 DeepSeek 在线条目各对应一个后端，清单条目的 `engineId` 就是 `ocr.backends` / `ocr.recognize` 的后端 ID。后端可插拔、互不降级：调用方在 `ocr.recognize` 中必须显式选择 `backend`；本地失败不会改走在线，在线失败也不会改走本地。`ocr.backends` 返回每个后端的真实状态（会读取模型文件做哈希校验、读取凭据库判断 key 是否存在，但不返回 key）。清单里“当前 OCR 模型”（`models.activate {"category": "ocr"}`，配置 `ocr.activeModelId`；为空时取第一个已安装的本地包，不会自动选在线）只决定陪玩流程用哪个，不改变 `ocr.recognize` 必须显式指定后端的约定。

### 8.1 本地：PP-OCRv5 各语种包

| 模型 ID | 后端 ID | 语言（`language`，首个为缺省） | 大小 |
| --- | --- | --- | --- |
| `ppocrv5-ch-mobile`（推荐） | `local-ppocrv5-ch` | `zh` `zh-Hant` `ja` `en` | 21.5 MB |
| `ppocrv5-ch-server` | `local-ppocrv5-ch-server` | `zh` `zh-Hant` `ja` `en` | 172.7 MB |
| `ppocrv5-korean-mobile` | `local-ppocrv5-korean` | `ko` | 18.3 MB |
| `ppocrv5-en-mobile` | `local-ppocrv5-en` | `en` | 12.7 MB |
| `ppocrv5-latin-mobile` | `local-ppocrv5-latin` | `en` `fr` `de` `es` | 12.7 MB |
| `ppocrv5-cyrillic-mobile` | `local-ppocrv5-cyrillic` | `ru` `uk` | 12.9 MB |
| `ppocrv5-eslav-mobile` | `local-ppocrv5-eslav` | `ru` `uk` | 12.7 MB |
| `ppocrv5-th-mobile` | `local-ppocrv5-th` | `th` | 12.7 MB |
| `ppocrv5-el-mobile` | `local-ppocrv5-el` | `el` | 12.7 MB |
| `ppocrv5-arabic-mobile` | `local-ppocrv5-arabic` | `ar`（按从右往左的阅读顺序还原，一个词一个框） | 12.8 MB |
| `ppocrv5-devanagari-mobile` | `local-ppocrv5-devanagari` | `hi` | 12.8 MB |
| `ppocrv5-ta-mobile` | `local-ppocrv5-ta` | `ta` | 12.7 MB |
| `ppocrv5-te-mobile` | `local-ppocrv5-te` | `te` | 12.7 MB |

语言列只写 2026-11-15 用渲染的聊天样例实测过的语种（结果见[陪玩与统一模型管理实现记录](research/2026-11-15-companion-implementation.md)）；越南文在所有包上都识别不好，不在支持范围。

| 项目 | 内容 |
| --- | --- |
| 模型 | 每个包 = 检测（`ch_PP-OCRv5_det_mobile.onnx`，server 包用 `ch_PP-OCRv5_det_server.onnx`）+ 该语种识别模型；全部文件的大小与 SHA-256 写在 `crates/kotone-ocr/src/manifest.rs`，写入前已实际下载复核。字符表内嵌于识别模型 metadata `character` |
| 来源 / 许可 | PaddleOCR PP-OCRv5（Apache-2.0）的 ONNX 导出，RapidAI/RapidOCR v3.10.0（Apache-2.0）在 ModelScope 发布：`https://www.modelscope.cn/models/RapidAI/RapidOCR`（标签 v3.10.0） |
| 运行时 | ONNX Runtime 1.27.1 C API（CPU，intra-op 4 线程、inter-op 1），来自 sherpa-onnx v1.13.5 静态预编译包，与 ASR 共用、**不新增 DLL 或 Python 环境**；没有该运行时的构建报告 `runtime-unavailable` |
| 安装 | 设置页「模型 → 文字识别」、`models.download {"category": "ocr", "id": …}`（镜像优先、断点续传，与 ASR 共用下载名额）或 `kotone-cli ocr install --model <id>`（`--from-dir` 离线复制；不带 `--model` 为韩文包）→ `<模型根>/ocr/<模型 ID>/`（模型根同 ASR：`models.dir` 或 `~/.kotone/models`）。逐文件校验大小与 SHA-256 后才发布；校验失败不留下任何文件。设置页「导入」从用户选的文件夹复制并同样校验 |
| 就绪检查 | `kotone-cli ocr status` 与 `ocr.backends`：文件存在、大小、SHA-256（按大小+修改时间缓存）；首次识别时把模型读入内存**再校验一次同一份字节**后创建会话，并用空输入自检输出类别数与字符表一致。安装后无需重启桌面端 |
| 语言 | 见上表，不传时用包的缺省语言；其他语言 `OCR_LANGUAGE_UNSUPPORTED`。各识别模型对拉丁字母 / 数字的覆盖以其字符表为限 |
| 选项 | `minConfidence`（0–1，缺省不过滤，被丢弃的行数写入 warnings）、`detLimitSideLen`（32–1536，缺省 736）；其他键 `OCR_OPTION_UNSUPPORTED` |
| 输出 | `line` 粒度、轴对齐行框（DB 检测，输入产物像素）；`confidence` 为识别器对解码字符的最大 softmax 概率均值——是识别器自报分数，**不是**语义或翻译正确率，没有解码字符时为 null |
| 计时 | `timing.ocrMs`，冷启动另有 `modelLoadMs`（`coldStart=true`），以及 `detMs`/`recMs` |

前后处理与 RapidOCR 默认参数对齐（BGR 输入、短边 736 的 32 倍数缩放——但相对原图**最多放大 2 倍**，避免小裁剪图里的大字被放得过大而碎成几段（2026-11-16 真机发现，见[自动化实现记录](research/2026-11-16-companion-scripts.md)）、DB thresh 0.3 / box 0.5 / unclip 1.6、2×2 膨胀；识别高 48、批宽按最大宽高比、CTC 贪心解码）。已知简化：检测框为连通域的轴对齐外接矩形（适合横排聊天，倾斜文字的框偏大），不使用方向分类器。

### 8.2 在线：`deepseek-ocr`

| 项目 | 内容 |
| --- | --- |
| 启用 | 默认 `not-configured`。在统一模型清单里给 `deepseek-ocr` 绑定一条连接（设置页「模型 → 文字识别」，或 `settings.update {"modelConfigs": {"deepseek-ocr": {"connection": "<连接ID>"}}}`）后立即可用；旧的 `toolService.ocr.deepseekConnectionId` 仍然有效（清单配置优先）。`ocr.recognize` 不能通过参数改选连接或端点 |
| 凭据 | 复用连接目录（`base_url`）与系统凭据库（API key）；缺 key → `credentials-missing`，读取失败 → `credentials-error` |
| 端点 | 由连接地址推导 `…/v1/tools/ocr`（去掉末尾 `/v1` 后追加）；只接受 https（测试用 loopback http 除外） |
| 外发 | 只上传所选产物的 PNG 字节与 `purpose` 字段（multipart 两个字段）；不上传窗口标题、帧信息、聊天历史或其他图像 |
| 语言 | 接口没有语言参数，只接受 `auto`（缺省） |
| 选项 | `purpose`：`text_recognition`（缺省，通常无几何）/ `rich_layout_recognition`（带版面框） |
| 输出 | 无标记时：每行一个 `line` 观测、`geometry:"none"`、`box=null`；带 `<｜ref｜>…<｜box｜>[[x1,y1,x2,y2]]` 标记时：每块一个 `block` 观测（坐标按 0–1000 归一化换算为像素、`lines[]` 为块内各行且**没有逐行坐标**）。`confidence` 恒为 null；`modelVersion` 注明接口未返回版本 |
| 错误 | 依据 2026-11-15 核对的[官方文档](https://api-docs.deepseek.com/zh-cn/guides/ocr/)：HTTP 200 的响应体若是 `error` 对象 → `OCR_REMOTE_ERROR`（不当作空结果）；`finish_reason="length"` → `truncated=true` + `OCR_TRUNCATED` 警告；正常返回但无文字 → `empty=true` + `OCR_EMPTY_RESULT`；结构不符 → `OCR_RESPONSE_INVALID`；错误消息截断到 300 字符，不回显响应体 |
| 取消 | 期限到达或服务关闭时丢弃连接；**远端识别仍会继续并计费**（官方说明）；账号并发上限 15（远端） |

### 8.3 `ocr.recognize` 结果

`observations[]` 每项：`index`、`text`（原文，不做语义修正）、`granularity`（`line` / `block` / `page`）、`geometry`（`box` / `none`）、`box`（输入产物像素）、`rootBox`（原始捕获像素，经整条派生链映射）、`polygon`、`confidence`（可空）、`label`（在线版面类别）、`lines`（block 内各行）。另有 `text`（在线为接口原文，本地为各行换行拼接）、`frameId/windowId/capturedAtUnixMs`、`geometryAvailable`、`confidenceAvailable`、`empty`、`truncated`、`coldStart`、`warnings`、`backendExtra`（在线 `usage` 等）。

结果驻留为 `ocrResultId`（600 秒、最多 64 个），供 `chat.track.ingest` 使用服务端记录的帧、坐标与后端来源。OCR 耗时（`ocrMs`）只是识别阶段，**不是**“从触发到可播报”的完整耗时。

## 9. 聊天消息跟踪（`chat.track.*`）

跟踪把逐帧观测对齐成带稳定 ID 的消息，只做位置、时序与来源上的匹配，不翻译、不判定频道或说话者、不执行文字中的任何指令。会话由调用方命名（`session`，1–64 个字母、数字或 `-_.:`），首次 ingest 时创建，代际从 1 开始。

**观测来源**：`{type:"ocrResult", ocrResultId}` 使用服务端记录的 OCR 结果（帧、根坐标、后端）；`{type:"external", artifactId, producer, lines:[{text, box?, confidence?}]}` 供 Agent 用自己的视觉模型读图后提交，`box` 是该产物像素中的框（越界拒绝），`artifactId` 必须仍有效以确定帧与 ROI，`producer` 记录来源。服务不接受任意文件路径。

| 情形 | 行为 |
| --- | --- |
| 单位 | 一条消息 = 一个可视文本行；自动换行的续行不合并，只给 `continuationOf` 候选与证据（无说话者前缀、紧随上一行、垂直间距小） |
| 对齐 | 聊天区自上而下由旧到新。新帧的行序列与上一非空帧的可见序列做重叠对齐；重叠内文字完全相同（忽略空白）为“再次看到”，相似（整行 ≥ 0.7 且说话者前缀之后的正文 ≥ 0.5）为 OCR 修订。几何可比时，位移不一致的对齐被排除 |
| OCR 改字 | 同一 `messageId`，`revision+1`，`changes` 中 `kind:"revised"` 带 `previousText`，`previousRevisions` 保留旧文 |
| 重复发送 | 不按文本去重：重叠之后多出的行是新消息，即使与旧消息文字相同；存在同样自洽的另一种对齐时（如连续相同文字、无几何）返回 `uncertainty`（`possible-duplicate` / `possible-repeat`） |
| 滚动 | 整体上移：旧消息保持 ID，底部新出现的行 `origin:"new"`；顶部消失的消息只是不再 `visible`，历史与 ID 保留。向上翻看在顶部出现的未知行 `origin:"scrolled-into-view"`（不是新发送）；翻看后再出现的、与会话历史中紧邻消息文字相同的行重新对齐到原 ID（`REMATCHED_FROM_HISTORY` 警告），不产生新增事件。首帧的行 `origin:"first-observation"` |
| 裁切 | 行框贴住 ROI 上/下边缘（2px 内）标记 `clipped:"top"/"bottom"`；之后完整出现时作为修订（`reason:"clipping-resolved"`） |
| ROI/窗口尺寸变化 | 位置证据不可用：只按文字顺序对齐，`geometryComparable=false` 并给 `ROI_CHANGED` 警告；建议 Agent 在有意更换 ROI 时 `reset`（`roi-changed`） |
| 窗口/进程变化 | 默认拒绝（`CHAT_TRACK_SOURCE_CHANGED`，状态不变）；`onSourceChange:"new-generation"` 时开启新代际（结果 `implicitReset`） |
| 同帧重复提交 | 同一 `frameId` 且内容相同 → `replay:true` 返回原结果，不改状态、不推进游标；同一**最新**帧的不同内容 → 按位置更正（`correction:true`，`reason:"same-frame-correction"`） |
| 乱序 | 采集时刻早于已接受最新帧的观测 → `CHAT_TRACK_OUT_OF_ORDER`，不改变状态 |
| 空观测 | 没有任何行时不清空可见集、不推进游标：可见集保持上一非空帧，`stale:true` + `EMPTY_OBSERVATION` 警告（无法区分聊天淡出与识别失败）。OCR 技术失败根本不会产生可 ingest 的结果 |
| 新对局 / 重连 | 由 Agent 调用 `chat.track.reset`（`new-match` / `reconnect`）：代际 +1，清空消息、事件与帧记录，新代际不复用旧 ID；旧游标 → `CHAT_TRACK_CURSOR_STALE` |
| 频道 / 说话者 | `channel.value` 与 `speaker.value` 恒为 `unknown`；`candidates` 只是证据：行首 `[팀]`/`[전체]`/`[파티]`（及英/中对应）映射为 `team`/`all`/`party`/`other`，冒号前文字作为说话者候选。Agent 自行判定 |
| 置信度 | 透传来源的值，没有就是 null，不是语义正确率 |

**读取**：`mode:"visible"` 返回当前可见的全部消息（已经读过的旧消息仍然返回），并给出 `visibleAsOfFrameId/visibleObservedAtUnixMs`、`latestFrameId/latestObservedAtUnixMs` 与 `stale`；读取不改变任何状态。`mode:"newSince"` 返回游标之后的 `added`/`revised` 事件（缺省从本代际开头），附当前修订与可见性、`hasMore` 和下一游标。游标（`ct1.<会话随机数>.<代际>.<序号>`）只在本会话本代际有效：代际变化或会话被回收后重建 → `CHAT_TRACK_CURSOR_STALE`；游标之后的事件已被容量回收 → `CHAT_TRACK_CURSOR_EXPIRED`（用 `visible` 重新同步）；格式错误或来自未来 → `CHAT_TRACK_CURSOR_INVALID`。错误从不伪装成空列表。

典型流程（每次用户询问时主动触发）：

```text
window.active → screen.capture → image.crop（聊天 ROI）→ [image.preprocess] → ocr.recognize
  → chat.track.ingest{ocrResult} → chat.track.read{visible}（翻译当前聊天）
                                  或 chat.track.read{newSince, cursor}（只看新增）
```

## 10. 应用生命周期、事件与用户请求（Agent 底座）

Agent 从“打开 exe”开始就能驱动 Kotone，并能收到用户通过 Kotone 发给它的话。

**生命周期**。`kotone-cli app launch` 以 `--background` 启动 `kotone.exe`：只驻留托盘，不打开设置窗口、不抢焦点；进程完全脱离调用方（`bInheritHandles=FALSE`、独立进程组），调用方捕获输出用的管道不会泄漏进长驻进程。等到工具服务就绪（实测约 0.5 秒）才返回；已在运行则直接返回现有实例，第二次后台启动也不会唤起窗口。工具服务未开启时 `--enable-tool-service` 先写入 `toolService.enabled=true`。`app.quit`（或 `kotone-cli app quit`）等同托盘“退出”：先送出响应，再发布 `app.stopping`、关闭服务、删除描述文件，CLI 等到进程真正退出才返回。`KOTONE_HOME` 环境变量把 Kotone 数据目录（配置、日志、描述文件）整体换到别处，供隔离运行与测试；桌面端与 CLI 都读取它。

**事件**（Kotone → Agent）。服务内有一条有界事件总线：

| 类型 | 何时 | `data` 要点 |
| --- | --- | --- |
| `app.started` / `app.stopping` | 服务启动 / 关闭 | `instanceId`（started 另有 `pid`） |
| `runtime.phase` | ASR 运行时状态变化 | 与 `runtime.status` 相同 |
| `user.request` | 用户经 Kotone 发给 Agent 的请求 | 完整请求（见下） |
| `agent.request.claimed` / `agent.request.expired` | 请求被领取 / 过期 | `requestId`（claimed 另有 `consumer`） |
| `agent.reply.delivered` / `agent.reply.failed` | 回复交付完成 / 失败 | `deliveryId`、`requestId`、`text`、`receipt` 或 `code/message`、`deliverMs` |
| `model.download.progress` / `.finished` / `.failed` | `models.download` 的进度（至多每秒一次）与结果 | `id`、`downloadedBytes/totalBytes`；结果含 `durationMs`，失败含 `cancelled`、`message` |
| `wake.model.progress` | `wake.model.download` 的进度（至多每秒一次） | `downloadedBytes`、`totalBytes` |
| `companion.finished` / `companion.failed` | `companion.ask` 的结果 | 见上表；`jobId` 对应受理时的返回 |
| `automation.finished` | 一条执行（程序或提醒）进入终态：完成、失败、取消、错过 | 执行记录（见“自动化”，省略程序源码） |
| `automation.saved` / `automation.deleted` | 保存（新建或覆盖）/ 删除一个自动化 | `id`（保存时另有 `name`） |
| `companion.check.finished` / `.failed` | `companion.check` 的结果 | `check{connectionId,baseUrl,model,passed,checkedAtMs,message}` 或 `message` |
| `speech.model.download.finished` / `.failed` | `speech.model.download` 的结果 | `status` 或 `cancelled`、`message`、`durationMs` |
| `elevation.restart.failed` | 用户拒绝 UAC 或提权启动失败 | `message` |

每个事件带递增 `seq` 与全局唯一 `id`（`evt_…`，供去重）。游标形如 `ev1.<实例随机数>.<序号>`，只在发放它的实例内有效。读取时传回上次的游标；实例重启后旧游标是 `EVENT_CURSOR_STALE`，序号已被淘汰是 `EVENT_CURSOR_EXPIRED`——**从不静默跳过事件**，Agent 应从 `from=oldest` 重读并按 ID 去重。`types` 支持精确类型与 `前缀.*`。`waitMs` 长轮询：暂无新事件时最多等待这么久（`event.read` 受本次 `timeoutMs` 约束，`GET /v1/events` 最长 30 秒；CLI 与 MCP 每轮用 25 秒），不占工具执行名额；服务关闭时立即返回 `closed=true`。

**用户请求**（反向通道）。`assistant.route` 设为 `agent` 时（运行中用 `settings.update {"assistant": {"route": "agent"}}` 立即生效；桌面端未运行时也可 `kotone.exe config set assistant.route agent`；默认 `companion` 为 Kotone 陪玩，旧值 `reminders` 读入时按 `companion` 处理），助手热键语音与文字入口的原文不再交给 Kotone 自己的 AI 连接，而是成为一条 `user.request`：`requestId`（`areq_…`）、`text`（用户原文——对 Kotone 是数据，不是指令）、`source`（`assistant-voice` / `assistant-text` / `tool`）、`foregroundWindow`（提交时的前台窗口，可直接用于截图）、`status`（`pending` → `claimed` → `replied`，或 `expired`）、创建/过期时间、领取者与回复次数。`assistant.submit` 走与文字入口完全相同的路径，供测试和自动化。

Agent 先 `agent.request.claim`（`consumer` 如 `dsh-game-communicate:<pid>` / `claude-code`）再处理：同一领取者重复领取幂等，其他领取者得到 `AGENT_REQUEST_CLAIMED`，过期请求得到 `AGENT_REQUEST_EXPIRED`。多个 Agent 同时在线时由此保证一条请求只被处理一次。

**回复**。`agent.reply` 复用提醒的交付路径：与其他输出互斥排队、播报前等待麦克风空闲（避免把回复当成新的语音指令）、弹幕不取焦点、语音失败时降级为弹幕。默认**受理即返回**（`status=accepted`、`deliveryId`），在后台交付，结果以 `agent.reply.delivered` / `agent.reply.failed` 事件给出——Agent 不必为一条 20 秒的弹幕或一段长语音阻塞，也不会撞上 30 秒调用上限。`wait=true` 时等交付完成再返回回执（受 `timeoutMs` 约束，到期即停止播报）。带 `requestId` 回答对应请求（可多次调用，例如先说“在查，稍等”），请求随即记为 `replied`；不带则是主动通知。服务关闭时丢弃未完成的后台交付（宿主停止播报、收起弹幕）。没有桌面宿主的实例（纯测试服务）直接返回 `HOST_ACTION_FAILED`，不先受理。

**接入方式**：

- **MCP**：`kotone.exe mcp`（安装版；开发时等价的 `kotone-cli mcp`）是 stdio MCP 服务器。静态工具 `kotone_service_status`、`kotone_launch`、`kotone_quit`、`kotone_capabilities`、`kotone_call`（按 Kotone 工具名调用任意工具）、`kotone_events`；Kotone 运行时再把每个工具映射为 `kotone_<工具名，点换成下划线>`，启动后发送 `notifications/tools/list_changed`。
- **DSH 插件**：独立仓库 `dsh-game-communicate`（与 Kotone 分开发布，类似 dsh-game-daily 之于 BetterGI）挂上上述 MCP 服务器，并把 `user.request` 领取后投进 DSH 里复用的「Kotone」会话。
- **Skill**：[`integrations/skills/kotone/SKILL.md`](../integrations/skills/kotone/SKILL.md) 供只能执行命令行的 Agent 使用。

## 11. 命令行

安装版的 `kotone.exe` 接受下列全部子命令（`kotone.exe tools call …`、`kotone.exe mcp` 等），经管道调用时与 `kotone-cli` 行为一致；终端手动使用请用 `kotone-cli`。见 [CLI 参考](cli.md)。

```text
kotone-cli tools status [--json]
kotone-cli tools list [--json]
kotone-cli tools call <tool> [--args-file <json文件|->] [--timeout-ms N] [--request-id ID]
kotone-cli tools artifact get <artifact-id> --output <png文件> [--force]
kotone-cli tools events [--cursor C | --from oldest|latest] [--types T1,T2] [--wait-ms N] [--follow]
kotone-cli app status [--json]
kotone-cli app launch [--exe <kotone.exe>] [--wait-ms N] [--enable-tool-service] [--foreground]
kotone-cli app quit [--wait-ms N]
kotone-cli mcp
kotone-cli ocr status [--model-dir <目录>] [--json]
kotone-cli ocr install [--model-dir <目录>] [--from-dir <目录>] [--json]
```

- `call` 把完整信封以 JSON 打印到标准输出；`--args-file -` 从标准输入读取；参数文件必须是 JSON 对象（≤64 KiB，允许 UTF-8 BOM）。
- `artifact get` 由客户端把 PNG 写到 `--output`：先核对长度与 SHA-256，把内容完整写入同目录的独占临时文件，再一次性发布。未加 `--force` 时用不替换的原子改名（`MoveFileExW` 不带 `REPLACE_EXISTING`），目标在任何时刻已存在都会失败——并发导出只有一个成功，不会覆盖其他进程刚创建的文件；`--force` 才原子替换已存在的文件。失败时临时文件被删除。标准输出是去掉 base64 内容、附 `output.path/bytesWritten` 的信封。
- 退出码：`0` 成功；`1` 工具返回 `ok=false`；`2` 本地用法或文件错误；`3` 发现或连接失败（此时信封的 `instanceId` 为 null）。
- `tools events --follow` 持续长轮询，每行输出一个事件 JSON（JSON Lines）；游标失效、服务停止或连接失败时退出（退出码同上），由调用方从 `--from oldest` 重新开始。
- `app launch` 找 `kotone.exe` 的顺序：`--exe`、`KOTONE_EXE`、与 `kotone-cli` 同目录；显式给出的路径不存在时报错，不悄悄改用别的文件。`app status` 未运行时退出码 1；`app quit` 等进程退出才返回，超时退出码 1。
- `ocr status` / `ocr install` 不连接桌面实例：在本地检查/安装模型（退出码 0 就绪或安装成功，1 未就绪或安装失败，2 用法错误）。`--model-dir` 指向模型包目录本身；`KOTONE_OCR_MODEL_DIR` 环境变量同样可覆盖（隔离冒烟用）。

典型流程（PowerShell）：

```powershell
kotone-cli tools call window.active > active.json
$win = (Get-Content active.json | ConvertFrom-Json).result.window.windowId
@{ windowId = $win } | ConvertTo-Json | Set-Content -Encoding utf8 cap.json
kotone-cli tools call screen.capture --args-file cap.json > shot.json
$art = (Get-Content shot.json | ConvertFrom-Json).result.artifactId
@{ artifactId = $art; rect = @{ x = 15; y = 210; width = 650; height = 200 } } | ConvertTo-Json | Set-Content -Encoding utf8 crop.json
kotone-cli tools call image.crop --args-file crop.json > crop-result.json
kotone-cli tools artifact get ((Get-Content crop-result.json | ConvertFrom-Json).result.artifactId) --output chat.png
$crop = (Get-Content crop-result.json | ConvertFrom-Json).result.artifactId
@{ artifactId = $crop; backend = "local-ppocrv5-korean" } | ConvertTo-Json | Set-Content -Encoding utf8 ocr.json
kotone-cli tools call ocr.recognize --args-file ocr.json --timeout-ms 10000 > ocr-result.json
$ocrId = (Get-Content ocr-result.json | ConvertFrom-Json).result.ocrResultId
@{ session = "lol-team"; observation = @{ type = "ocrResult"; ocrResultId = $ocrId } } | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 ingest.json
kotone-cli tools call chat.track.ingest --args-file ingest.json
'{"session":"lol-team","mode":"visible"}' | kotone-cli tools call chat.track.read --args-file -
```

## 12. 隐私

截图、窗口标题与进程名只在本机返回给持有会话凭证的本地客户端；服务不落盘产物、OCR 结果或跟踪状态（全部在内存中，TTL/容量回收，桌面退出即丢弃），诊断事件不记录它们。客户端写出的 PNG 由调用方保存和清理。

配置类工具：`connections.upsert` 的 API key 只写——存入系统凭据库，任何工具结果、事件与日志里都不出现；设置了 `KOTONE_HOME` 的隔离实例使用独立的凭据库命名空间（`kotone@<目录>`），读不到也写不进用户主实例的 key。`history.list` 返回用户说过的话，只交给本机持有会话凭证的客户端。

用户请求的原文、Agent 回复与事件只在内存中保留（有界、TTL），不写入识别历史或诊断；`assistant.route=agent` 时用户对助手说的话交给本机连接的 Agent，由 Agent 决定是否再发往其使用的模型服务。

只有选用在线 OCR 后端时，Kotone 才会把**所选产物的 PNG** 上传到用户配置的 DeepSeek 连接；该后端默认未启用，需用户在配置中指定连接。本地后端完全离线；模型只在用户显式运行 `kotone-cli ocr install` 时下载。Agent 若自行把图片交给其他在线模型，那是 Agent 侧的数据外发，见 [PRIVACY.md](../PRIVACY.md)。

## 13. 实现位置

| 位置 | 内容 |
| --- | --- |
| `crates/kotone-core/src/tools.rs` | 窗口观察 / 捕获 / 宿主端口与类型 |
| `crates/kotone-core/src/ocr.rs` | OCR 后端端口、观测/粒度/可空置信度、失败分类、取消令牌 |
| `crates/kotone-ocr` | 本地 PP-OCRv5（`local.rs`、`ppocr.rs`、`ort.rs` ONNX Runtime C API、`onnx_meta.rs`）、模型清单与安装（`manifest.rs`、`install.rs`）、DeepSeek 在线（`deepseek.rs`） |
| `crates/kotone-tools/src/preprocess.rs`、`tracking.rs`、`ocr_results.rs` | 预处理操作、聊天跟踪、驻留 OCR 结果 |
| `crates/kotone-platform-windows/src/window_tools.rs` | EnumWindows 观察与 GDI 前台客户区捕获 |
| `crates/kotone-platform-windows/src/tool_host.rs` | 会话 ID、进程身份、私有 DACL 写入与校验 |
| `crates/kotone-tools` | 协议、能力目录、产物存储、调度、HTTP 服务端、发现、客户端 |
| `apps/desktop/src-tauri/src/tool_service.rs` | 桌面 setup 启动、`RunEvent::Exit` 关闭 |
| `crates/kotone-cli/src/tools.rs`、`ocr.rs` | `tools` 子命令；`ocr status/install` |
| `crates/kotone-tools/src/events.rs`、`agent_requests.rs`、`host.rs`、`service_app.rs` | 事件总线与游标、用户请求状态机、桌面宿主端口、应用级工具 |
| `apps/desktop/src-tauri/src/tool_service.rs`（`DesktopHost`）、`assistant.rs`（`RoutingHandler`、`deliver_text`） | 宿主实现：运行时启停、退出、回复交付、助手路由 |
| `crates/kotone-platform-windows/src/detached.rs` | 不继承句柄的脱离式进程启动 |
| `crates/kotone-cli/src/app.rs`、`mcp.rs` | `app status/launch/quit`；stdio MCP 服务器 |
| `crates/kotone-tools/src/catalog_host.rs` | 配置类工具目录与 schema（`HOST_TOOLS`） |
| `apps/desktop/src-tauri/src/tool_config.rs` | 配置类工具的桌面实现：调用设置页同一批命令函数、后台下载与事件、设置窗口刷新 |
| `integrations/skills/kotone` | Agent Skill（DSH 插件在独立仓库 `dsh-game-communicate`） |
