# 本机 Agent 工具服务（M1 + M2 + Agent 底座）

本页是工具服务的**现行权威手册**：开启方式、发现与认证、v1 协议、已实现工具、捕获语义与限制，以及 M2 的图像预处理、OCR 后端与聊天消息跟踪，和 Agent 底座的应用生命周期、事件与用户请求反向通道（§10）。字段的最终依据是 `kotone-cli tools list --json` 返回的 schema 与 `crates/kotone-tools`、`crates/kotone-ocr` 实现；[演进计划](product-evolution-game-tools.md)中 M3 及以后的工具尚未实现，不属于本 API。实现与验证记录见 [M1](research/game-tools-m1-implementation.md)、[M2](research/game-tools-m2-implementation.md)、[Agent 底座第一批](research/agent-base-batch1-implementation.md)。

## 1. 定位与边界

工具服务让 Agent（DSH 桌宠插件、脚本、命令行 Agent）连接**已经在运行**的 Kotone 桌面实例，取得窗口信息、前台窗口客户区截图、裁剪与预处理图像、OCR 文字观测，以及跨帧的聊天消息跟踪。Agent 决定何时调用、是否自己看原图、用哪个 OCR 后端、怎样解释结果；Kotone 只提供确定性的本机执行与可追溯结果，不翻译、不摘要、不回复。

- 桌面进程拥有服务；`kotone-cli tools` 是客户端。客户端直接走 loopback HTTP，**不会再次启动 `kotone.exe`**，因此不会触发单实例回调唤起设置窗口、抢走游戏焦点。
- 服务与 ASR 运行时（启动 / 停止开关）相互独立：运行时 Stopped 时工具照常可用，调用也不会启动 ASR、注册热键或加载模型。
- 服务从不移动或聚焦任何窗口，不发送输入，不读写调用方指定的本地文件。唯一的可见输出是 `agent.reply` 的弹幕（不取焦点、鼠标穿透的提醒层）与语音播报。
- 已实现下文 24 个工具（M1 的 8 个 + M2 的 6 个 + Agent 底座的 10 个）。设置、连接、历史、诊断等其余应用能力按[演进计划 §5.5](product-evolution-game-tools.md) 分批开放；通用热键、工作流、模型调用（LLM）与发送尚未实现，不出现在 capabilities 中，调用它们返回 `TOOL_NOT_FOUND`。
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

在线 OCR 另有一个开关（默认未启用，见 [§8](#8-ocr-后端与安装)）：

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
| 事件 / 用户请求 / 宿主 | `EVENT_CURSOR_INVALID` `EVENT_CURSOR_STALE`（实例重启或事件已被淘汰）`EVENT_CURSOR_EXPIRED` `AGENT_REQUEST_NOT_FOUND` `AGENT_REQUEST_EXPIRED` `AGENT_REQUEST_CLAIMED`（`details.claimedBy`）`HOST_ACTION_FAILED`（桌面端执行失败，或本实例没有桌面宿主） |
| 客户端发现 | `SERVICE_NOT_RUNNING` `DESCRIPTOR_INSECURE` `DESCRIPTOR_INVALID` `STALE_DESCRIPTOR` `SERVICE_UNREACHABLE`* `INSTANCE_MISMATCH` `TRANSPORT_ERROR`* `RESPONSE_INVALID` |

带 * 的码 `retryable=true`；† 的 `retryable` 按远端情况逐次给出（429/500/503/504/408 与“服务暂时不可用”为 true；余额不足、文件无法处理、识别超时等为 false）。网络/连接错误不会被包装成“图像没看清”；全黑图也不等于“已经看懂聊天”；OCR 技术失败是错误，不是“没有文字”。

## 5. 已实现工具

除 `ocr.recognize` 外所有工具 `dataEgress` 为 `none`：数据只在本机进程间传递。`ocr.recognize` 为 `backend-dependent`：选用本地后端时不外发，选用在线后端时只上传所选产物的 PNG（见 §8）。

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

两个后端可插拔、互不降级：调用方在 `ocr.recognize` 中必须显式选择 `backend`；本地失败不会改走在线，在线失败也不会改走本地。`ocr.backends` 返回每个后端的真实状态（会读取模型文件做哈希校验、读取凭据库判断 key 是否存在，但不返回 key）。

### 8.1 本地：`local-ppocrv5-korean`

| 项目 | 内容 |
| --- | --- |
| 模型 | PP-OCRv5 mobile：检测 `ch_PP-OCRv5_det_mobile.onnx`（4,819,576 字节，SHA-256 `4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae`）+ 韩文识别 `korean_PP-OCRv5_rec_mobile.onnx`（13,488,748 字节，SHA-256 `cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b`）；字符表内嵌于识别模型 metadata |
| 来源 / 许可 | PaddleOCR PP-OCRv5（Apache-2.0）的 ONNX 导出，RapidAI/RapidOCR v3.10.0（Apache-2.0）在 ModelScope 发布：`https://www.modelscope.cn/models/RapidAI/RapidOCR`（标签 v3.10.0） |
| 运行时 | ONNX Runtime 1.27.1 C API（CPU，intra-op 4 线程、inter-op 1），来自 sherpa-onnx v1.13.5 静态预编译包，与 ASR 共用、**不新增 DLL 或 Python 环境**；没有该运行时的构建报告 `runtime-unavailable` |
| 安装 | `kotone-cli ocr install`（显式下载约 18.3 MB，或 `--from-dir` 离线复制）→ `<模型根>/ocr/ppocrv5-korean-mobile/`（模型根同 ASR：`models.dir` 或 `~/.kotone/models`）。逐文件写临时文件、校验大小与 SHA-256 后原子发布；校验失败不留下任何文件 |
| 就绪检查 | `kotone-cli ocr status` 与 `ocr.backends`：文件存在、大小、SHA-256（按大小+修改时间缓存）；首次识别时把模型读入内存**再校验一次同一份字节**后创建会话，并用空输入自检输出类别数与字符表一致。安装后无需重启桌面端 |
| 语言 | 仅 `ko`（缺省）；其他语言 `OCR_LANGUAGE_UNSUPPORTED`。韩文识别模型对拉丁字母/数字的覆盖以其字符表为限 |
| 选项 | `minConfidence`（0–1，缺省不过滤，被丢弃的行数写入 warnings）、`detLimitSideLen`（32–1536，缺省 736）；其他键 `OCR_OPTION_UNSUPPORTED` |
| 输出 | `line` 粒度、轴对齐行框（DB 检测，输入产物像素）；`confidence` 为识别器对解码字符的最大 softmax 概率均值——是识别器自报分数，**不是**语义或翻译正确率，没有解码字符时为 null |
| 计时 | `timing.ocrMs`，冷启动另有 `modelLoadMs`（`coldStart=true`），以及 `detMs`/`recMs` |

前后处理与 RapidOCR 默认参数对齐（BGR 输入、短边 736 的 32 倍数缩放、DB thresh 0.3 / box 0.5 / unclip 1.6、2×2 膨胀；识别高 48、批宽按最大宽高比、CTC 贪心解码）。已知简化：检测框为连通域的轴对齐外接矩形（适合横排聊天，倾斜文字的框偏大），不使用方向分类器。

### 8.2 在线：`deepseek-ocr`

| 项目 | 内容 |
| --- | --- |
| 启用 | 默认 `not-configured`。用户执行 `config set toolService.ocr.deepseekConnectionId <连接ID>` 并重启桌面端后才可能就绪；Agent 不能通过参数改选连接或端点 |
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

每个事件带递增 `seq` 与全局唯一 `id`（`evt_…`，供去重）。游标形如 `ev1.<实例随机数>.<序号>`，只在发放它的实例内有效。读取时传回上次的游标；实例重启后旧游标是 `EVENT_CURSOR_STALE`，序号已被淘汰是 `EVENT_CURSOR_EXPIRED`——**从不静默跳过事件**，Agent 应从 `from=oldest` 重读并按 ID 去重。`types` 支持精确类型与 `前缀.*`。`waitMs` 长轮询：暂无新事件时最多等待这么久（`event.read` 受本次 `timeoutMs` 约束，`GET /v1/events` 最长 30 秒；CLI 与 MCP 每轮用 25 秒），不占工具执行名额；服务关闭时立即返回 `closed=true`。

**用户请求**（反向通道）。`assistant.route` 设为 `agent` 时（`kotone-cli config set assistant.route agent`，重启生效；默认 `reminders` 保持原有的提醒规划），助手热键语音与文字入口的原文不再交给 Kotone 自己的 AI 连接，而是成为一条 `user.request`：`requestId`（`areq_…`）、`text`（用户原文——对 Kotone 是数据，不是指令）、`source`（`assistant-voice` / `assistant-text` / `tool`）、`foregroundWindow`（提交时的前台窗口，可直接用于截图）、`status`（`pending` → `claimed` → `replied`，或 `expired`）、创建/过期时间、领取者与回复次数。`assistant.submit` 走与文字入口完全相同的路径，供测试和自动化。

Agent 先 `agent.request.claim`（`consumer` 如 `dsh-game-communicate:<pid>` / `claude-code`）再处理：同一领取者重复领取幂等，其他领取者得到 `AGENT_REQUEST_CLAIMED`，过期请求得到 `AGENT_REQUEST_EXPIRED`。多个 Agent 同时在线时由此保证一条请求只被处理一次。

**回复**。`agent.reply` 复用提醒的交付路径：与其他输出互斥排队、播报前等待麦克风空闲（避免把回复当成新的语音指令）、弹幕不取焦点、语音失败时降级为弹幕。默认**受理即返回**（`status=accepted`、`deliveryId`），在后台交付，结果以 `agent.reply.delivered` / `agent.reply.failed` 事件给出——Agent 不必为一条 20 秒的弹幕或一段长语音阻塞，也不会撞上 30 秒调用上限。`wait=true` 时等交付完成再返回回执（受 `timeoutMs` 约束，到期即停止播报）。带 `requestId` 回答对应请求（可多次调用，例如先说“在查，稍等”），请求随即记为 `replied`；不带则是主动通知。服务关闭时丢弃未完成的后台交付（宿主停止播报、收起弹幕）。没有桌面宿主的实例（纯测试服务）直接返回 `HOST_ACTION_FAILED`，不先受理。

**接入方式**：

- **MCP**：`kotone-cli mcp` 是 stdio MCP 服务器。静态工具 `kotone_service_status`、`kotone_launch`、`kotone_quit`、`kotone_capabilities`、`kotone_call`（按 Kotone 工具名调用任意工具）、`kotone_events`；Kotone 运行时再把每个工具映射为 `kotone_<工具名，点换成下划线>`，启动后发送 `notifications/tools/list_changed`。
- **DSH 插件**：独立仓库 `dsh-game-communicate`（与 Kotone 分开发布，类似 dsh-game-daily 之于 BetterGI）挂上上述 MCP 服务器，并把 `user.request` 领取后投进 DSH 里复用的「Kotone」会话。
- **Skill**：[`integrations/skills/kotone/SKILL.md`](../integrations/skills/kotone/SKILL.md) 供只能执行命令行的 Agent 使用。

## 11. 命令行

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
| `integrations/skills/kotone` | Agent Skill（DSH 插件在独立仓库 `dsh-game-communicate`） |
