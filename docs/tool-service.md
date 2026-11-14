# 本机 Agent 工具服务（M1）

本页是工具服务的**现行权威手册**：开启方式、发现与认证、v1 协议、已实现工具、捕获语义与限制。字段的最终依据是 `kotone-cli tools list --json` 返回的 schema 与 `crates/kotone-tools` 实现；[演进计划](product-evolution-game-tools.md)中 M2 及以后的工具尚未实现，不属于本 API。

## 1. 定位与边界

工具服务让 Agent（DSH 桌宠插件、脚本、命令行 Agent）连接**已经在运行**的 Kotone 桌面实例，取得窗口信息、前台窗口客户区截图和裁剪图像。Agent 决定何时调用、看图或交给其他模型；Kotone 只提供确定性的本机执行与可追溯结果。

- 桌面进程拥有服务；`kotone-cli tools` 是客户端。客户端直接走 loopback HTTP，**不会再次启动 `kotone.exe`**，因此不会触发单实例回调唤起设置窗口、抢走游戏焦点。
- 服务与 ASR 运行时（启动 / 停止开关）相互独立：运行时 Stopped 时工具照常可用，调用也不会启动 ASR、注册热键或加载模型。
- 服务从不显示、移动或聚焦任何窗口，不发送输入，不读写调用方指定的本地文件。
- 本轮只实现下文 8 个工具。OCR、消息跟踪、通用热键、工作流、模型调用、TTS/弹幕、发送与 DSH 插件均未实现，也不出现在 capabilities 中；调用它们返回 `TOOL_NOT_FOUND`。

## 2. 开启与重启

默认关闭。开关 `toolService.enabled` 只在桌面端启动时读取：

```powershell
# 1. 先从托盘菜单「退出」Kotone（桌面端运行时保存设置会用内存中的旧值覆盖文件）
# 2. 写入开关
kotone-cli config set toolService.enabled true
# 3. 重新启动 Kotone 桌面端，然后验证
kotone-cli tools status
```

关闭同理：退出桌面端 → `config set toolService.enabled false` → 重新启动。没有固定端口、外网监听或其他服务配置项。桌面日志 `~/.kotone/kotone.log` 记录 `tool service: disabled` / `listening on 127.0.0.1:<端口> instance=<ID>` / `failed to start: <原因>`，不记录凭证。

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

## 4. 协议 v1

| 端点 | 用途 |
| --- | --- |
| `GET /v1/capabilities` | 实际实现的工具、输入/输出 JSON Schema、就绪状态、副作用、限制 |
| `GET /v1/status` | 实例、服务、ASR 相位、捕获后端、产物占用 |
| `POST /v1/tools/call` | 调用工具 |

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

失败时 `ok=false`，`error` 至少包含 `code`、`message`、`retryable`，可带 `details`。工具执行结果（含工具错误）使用 HTTP 200；认证失败 401、来源/Host 拒绝 403、未知路径 404、方法不符 405、请求体读取超时 408、请求体过大 413、媒体类型 415、信封不合法 400、服务已关闭 503（`SERVICE_CLOSED`）。`requestId` 可省略（服务生成），只接受 1-128 个字母、数字及 `-_.:`。

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

捕获、裁剪、PNG 编码与 base64 在工作线程执行，不持有 ASR 运行时或 UI 锁。`timeoutMs` 是从服务收到请求起算的**绝对期限**，在三处强制：取得执行名额时（排队耗尽期限 → `TIMEOUT`，`details.stage="queued"`）、工作线程真正开始时（阻塞线程池饱和导致晚开始 → 不执行工具，`not-started`）、以及提交闸门内（`running` / `late-result`）。工作线程只产出“待提交”结果——新产物或释放请求——从不直接修改存储；提交闸门在存储锁内核对“未关闭、未过期”后才插入或释放。因此即使执行器停顿使迟到结果与计时器同时就绪，迟到结果也不会发布；排队超时的 `artifact.release` 之后也不会删除产物。超时调用的工作线程在后台收尾期间仍占用并发名额。

### 错误码

| 类别 | 错误码 |
| --- | --- |
| 传输 / 请求 | `UNAUTHORIZED` `FORBIDDEN_ORIGIN` `ROUTE_NOT_FOUND` `METHOD_NOT_ALLOWED` `PAYLOAD_TOO_LARGE` `UNSUPPORTED_MEDIA_TYPE` `INVALID_REQUEST` `REQUEST_TIMEOUT`* |
| 调用 | `TOOL_NOT_FOUND` `INVALID_ARGUMENTS` `UNSUPPORTED_PROTOCOL_VERSION` `BUSY`* `TIMEOUT`* `INTERNAL` `PLATFORM_UNSUPPORTED` `SERVICE_CLOSED` |
| 窗口 / 捕获 | `WINDOW_NOT_FOUND`（未知引用）`WINDOW_GONE`（已关闭）`WINDOW_IDENTITY_CHANGED`（句柄复用/进程更换）`WINDOW_MINIMIZED`* `WINDOW_NOT_VISIBLE`* `WINDOW_NOT_FOREGROUND`* `INVALID_GEOMETRY` `IMAGE_TOO_LARGE` `TARGET_CHANGED`*（捕获期间移动/失焦/最小化）`CAPTURE_FAILED`* |
| 产物 | `ARTIFACT_NOT_FOUND`（从未发放）`ARTIFACT_EXPIRED`（`details.reason` 为 `ttl` 或 `evicted`）`ARTIFACT_RELEASED` |
| 客户端发现 | `SERVICE_NOT_RUNNING` `DESCRIPTOR_INSECURE` `DESCRIPTOR_INVALID` `STALE_DESCRIPTOR` `SERVICE_UNREACHABLE`* `INSTANCE_MISMATCH` `TRANSPORT_ERROR`* `RESPONSE_INVALID` |

带 * 的码 `retryable=true`。网络/连接错误不会被包装成“图像没看清”；全黑图也不等于“已经看懂聊天”。

## 5. 已实现工具

所有工具 `dataEgress` 为 `none`：数据只在本机进程间传递。

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

**窗口引用**：`windowId` 是不透明字符串，只在发放它的实例内有效，绑定窗口句柄 + PID + 进程创建时间（`identityBinding` 说明强度；无权查询创建时间时退化为 `handle+pid`）。每次捕获前重新校验；同一窗口多次列举得到同一 ID。窗口元数据含标题、类名、可执行文件名（不含目录）、PID、前台/可见/最小化/DWM 隐藏标记、客户区与所在显示器的**屏幕物理像素**矩形、DPI 与 `scale = dpi / 96`。

**产物**：不可变 PNG（RGBA8）。元数据含 `width/height`、`byteLength`、PNG 字节的 `sha256`、`createdAtUnixMs`（发布到存储的时刻，TTL 由此起算）/`expiresAtUnixMs`/`ttlMs`、`parentArtifactId`、`rootArtifactId`、`rootTransform`（`rootPoint = offset + scale × point`，M1 只有裁剪，scale 恒为 1）、`rootRect`、采集时刻的 `screenRect`，以及整条裁剪链共享的 `source`（`frameId`、`windowId`、`captureMethod`、采集时客户区屏幕坐标、DPI、`capturedAtUnixMs`、进程名、警告）。`capturedAtUnixMs` 记录捕获后端返回像素的时刻，早于像素校验、PNG 编码与哈希；裁剪沿用根产物的采集时刻，`createdAtUnixMs` 才是各自的发布时刻。不同 `frameId` 的裁剪坐标不能互相套用。

**裁剪**：坐标空间 `artifact-physical-pixels`，以输入产物左上角为原点。负坐标、零宽高、越界或整数溢出直接返回 `INVALID_ARGUMENTS`（`details.reason` 为 `negative-origin` / `empty-rect` / `out-of-bounds` / `overflow`），不截断、不自动修正；输入产物保持不变。释放父产物不影响已生成的子产物。

**存储**：仅在桌面进程内存中，不落盘；服务自带一个有界的空闲维护任务（只持有服务弱引用，关闭时退出），即使没有任何请求也会按周期回收到期产物；超出容量时回收最旧产物；桌面退出即全部丢弃。服务只接受产物 ID，不接受文件路径。

## 6. 捕获后端：GDI 前台可见客户区

当前唯一后端 `captureMethod = "gdi-screen-foreground-client"`：目标必须是前台窗口、未最小化、可见且未被 DWM 隐藏、客户区完整位于一个显示器内，然后用 GDI `BitBlt(SRCCOPY | CAPTUREBLT)` 从屏幕 DC 复制该客户区矩形的**已合成像素**，复制后再次核对前台、位置与尺寸。限制（同时出现在 capabilities 的 `capture.limitations`）：

- 得到的是“屏幕上该矩形里看到的像素”：覆盖在其上的置顶窗口、通知以及 Kotone 弹幕这类分层（`WS_EX_LAYERED`）叠加层也会出现在图中；不是窗口内容隔离捕获，不恢复被遮挡内容。受控原生测试验证了分层叠加层被合入；在验证所用的 Windows 11 上不加 CAPTUREBLT 时同样被合入，CAPTUREBLT 用于覆盖不会默认合入分层窗口的系统。CAPTUREBLT 在部分系统上可能让鼠标指针短暂闪烁，指针本身不会出现在图中。
- 后台、遮挡、最小化、隐藏窗口不能捕获。非前台目标返回 `WINDOW_NOT_FOREGROUND`——服务不会替调用方切换焦点。Agent 通常在用户正在游戏时调用；在终端里手动运行 CLI 时前台是终端本身。
- 客户区跨显示器或部分超出屏幕时返回 `INVALID_GEOMETRY`，不会退化为截取整个桌面。
- 独占全屏、硬件叠加层或受保护内容可能得到黑图或旧帧：全部像素相同会附带 `UNIFORM_PIXELS` 警告，系统报告 Direct3D 独占全屏时附带 `EXCLUSIVE_FULLSCREEN_SUSPECTED`。对 LOL 等真实游戏的可用性尚未验证（见[实现记录](research/game-tools-m1-implementation.md)）。
- 坐标在线程级 Per-Monitor-V2 DPI 上下文中读取（物理像素），进入后核实线程确实处于 per-monitor 感知；切换失败且线程本身也不是 per-monitor 感知时，`window.*` 与 `screen.capture` 返回 `PLATFORM_UNSUPPORTED`，不会以未经验证的坐标冒充物理像素。调用结束恢复原上下文；每个 DC/位图都由守卫释放。

## 7. 命令行

```text
kotone-cli tools status [--json]
kotone-cli tools list [--json]
kotone-cli tools call <tool> [--args-file <json文件|->] [--timeout-ms N] [--request-id ID]
kotone-cli tools artifact get <artifact-id> --output <png文件> [--force]
```

- `call` 把完整信封以 JSON 打印到标准输出；`--args-file -` 从标准输入读取；参数文件必须是 JSON 对象（≤64 KiB，允许 UTF-8 BOM）。
- `artifact get` 由客户端把 PNG 写到 `--output`：先核对长度与 SHA-256，把内容完整写入同目录的独占临时文件，再一次性发布。未加 `--force` 时用不替换的原子改名（`MoveFileExW` 不带 `REPLACE_EXISTING`），目标在任何时刻已存在都会失败——并发导出只有一个成功，不会覆盖其他进程刚创建的文件；`--force` 才原子替换已存在的文件。失败时临时文件被删除。标准输出是去掉 base64 内容、附 `output.path/bytesWritten` 的信封。
- 退出码：`0` 成功；`1` 工具返回 `ok=false`；`2` 本地用法或文件错误；`3` 发现或连接失败（此时信封的 `instanceId` 为 null）。

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
```

## 8. 隐私

截图、窗口标题与进程名只在本机返回给持有会话凭证的本地客户端；服务不上传、不落盘产物，诊断事件不记录它们。客户端写出的 PNG 由调用方保存和清理。Agent 若把图片交给在线模型，那是 Agent 侧的数据外发，见 [PRIVACY.md](../PRIVACY.md)。

## 9. 实现位置

| 位置 | 内容 |
| --- | --- |
| `crates/kotone-core/src/tools.rs` | 窗口观察 / 捕获 / 宿主端口与类型 |
| `crates/kotone-platform-windows/src/window_tools.rs` | EnumWindows 观察与 GDI 前台客户区捕获 |
| `crates/kotone-platform-windows/src/tool_host.rs` | 会话 ID、进程身份、私有 DACL 写入与校验 |
| `crates/kotone-tools` | 协议、能力目录、产物存储、调度、HTTP 服务端、发现、客户端 |
| `apps/desktop/src-tauri/src/tool_service.rs` | 桌面 setup 启动、`RunEvent::Exit` 关闭 |
| `crates/kotone-cli/src/tools.rs` | `tools` 子命令 |
