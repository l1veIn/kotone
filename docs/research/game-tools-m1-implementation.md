# 游戏工具 M1 实现与验证记录（2026-11-15）

范围：[演进计划](../product-evolution-game-tools.md) 第 7 节 M1——桌面工具服务与视觉输入。基线为文档提交 `75c846e`，分支 `codex/game-tools-m1`；首次实现提交 `21066e7`，独立评审后的修正见第 7 节。现行用法与协议见 [工具服务手册](../tool-service.md)；本文只记录本轮做了什么、怎样验证、哪些仍未验证。

## 1. 交付内容

| 部分 | 实现 |
| --- | --- |
| 配置 | `toolService.enabled`（默认 false）：core `Settings` 字段与默认值、缺省合并、类型校验；CLI `config set` 白名单（只接受 true/false，写入后提示需退出并重启桌面端）；TS `Settings` 类型与浏览器 mock 默认值 |
| 领域端口 | `kotone-core::tools`：`WindowIdentity`、`PhysicalRect`、`WindowInfo`、`CapturedImage`、`WindowObserver`、`WindowCapturer`、`ToolHost`、`WindowToolError`；core 未新增依赖 |
| Windows 平台 | `window_tools.rs`：EnumWindows 观察（Per-Monitor-V2 线程 DPI 作用域、物理客户区、显示器、DPI、DWM cloaked、进程名只取文件名、进程创建时间）；GDI 前台可见客户区捕获（前后两次校验、单显示器约束、RAII 释放 DC/DIB/选择、`UNIFORM_PIXELS` 与 `EXCLUSIVE_FULLSCREEN_SUSPECTED` 警告）。`tool_host.rs`：会话 ID、进程存活与创建时间、受保护 DACL 的私有目录/文件写入与所有者/ACE 校验。新增 features 均显式写在平台 crate：`Win32_Graphics_Gdi`、`Win32_Graphics_Dwm`、`Win32_UI_HiDpi`、`Win32_System_RemoteDesktop`、`Win32_Security_Authorization` |
| 服务 crate | 新增 `kotone-tools`：v1 信封与错误码、能力目录与 JSON Schema、不可变图像/裁剪/PNG/SHA-256、有界产物存储（TTL、数量/字节回收、墓碑、幂等释放）、调度（请求 ID、期限、并发许可随阻塞任务释放、迟到结果丢弃）、hyper loopback 服务端（每端点 Bearer 认证、Origin/Host 拒绝、64 KiB 请求体、连接数与头读取期限）、描述文件发布/校验、reqwest 客户端（禁用代理、实例 ID 双重核对、响应体上限）。复用工作区已锁定的 hyper/hyper-util/http-body-util/reqwest/png/sha2/base64/getrandom，`Cargo.lock` 只新增本 crate 条目 |
| 桌面 | `tool_service.rs`：setup 中 `assistant::start` 之后按开关启动，`RunEvent::Exit` 同步关闭；不经过 RuntimeManager 的启停；启动尚未完成时退出也会立即关闭；日志不含凭证 |
| CLI | `tools status / list / call / artifact get`；不加载配置、不启动 ASR、不启动 kotone.exe；退出码 0/1/2/3；PNG 由客户端校验 SHA-256 后原子写入 |
| 文档 | 新增 `docs/tool-service.md`；更新 CLI、架构、隐私、术语、索引、开发指南与演进计划状态 |

未实现（保持计划状态，不在 capabilities 中）：OCR 打包与后端、图像预处理、消息跟踪、动态热键、工作流、模型调用、TTS/弹幕工具、回复注入、DSH/MCP 适配；无版本号变更、发布打包或推送。

## 2. 验证命令与结果

环境：Windows 11 Pro 10.0.26200，rustc/cargo 1.98.0，pnpm 11.16.0，Node 24.18.1。为避免与原工作区共享构建产物，所有 cargo 命令使用本 worktree 的 `CARGO_TARGET_DIR`，并以 `SHERPA_ONNX_LIB_DIR` 只读复用原工作区已解压的 sherpa-onnx 预编译库（不下载模型、不修改原工作区文件）。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过（全部测试二进制 0 失败；既有 ignored 测试保持忽略） |
| `cargo test -p kotone-tools` | 31 通过：图像/裁剪/PNG/哈希、产物存储、协议，以及 15 个真实 loopback HTTP 端到端测试 |
| `cargo test -p kotone-cli` | 单元 23 通过；`tests/tools_cli.rs` 2 通过（真实 CLI 二进制） |
| `cargo test -p kotone-platform-windows` | 单元 45 通过（含私有 ACL、Everyone 授权被拒、会话/进程身份）；`tests/native_capture.rs` 默认 1 通过 |
| `cargo test -p kotone-platform-windows --test native_capture -- --ignored --test-threads=1` | 1 通过（受控窗口前台像素与完整链路，见第 3 节） |
| `cargo test -p kotone-core`（workspace 中） | 单元 218 通过；新增 `toolService` 默认关闭、缺省合并、round-trip、类型错误用例与物理矩形包含/溢出用例 |
| `cargo build -p kotone-tauri` | 通过（Windows 桌面壳编译） |
| `pnpm install --frozen-lockfile` / `pnpm check` | 通过（svelte-check 308 文件 0 错误 0 警告） |
| `pnpm release:verify` / `pnpm release:test` | 通过（v0.1.13 未变；6/6） |
| `pnpm -C apps/desktop test:e2e` | **未能运行**：见第 5 节环境阻塞 |

自动化覆盖要点：

- **认证与传输**：5 个端点/路径在无凭证和错误凭证时均 401（未知路径也先认证，再 404/405）；Origin 与非 loopback Host 403；>64 KiB 413；非 JSON / 信封多余字段（如 `path`）400；`text/plain` 415；`timeoutMs` 0 与 30001 拒绝；requestId 回显与格式校验。
- **能力真实性**：capabilities 的工具名与 `IMPLEMENTED_TOOLS` 完全一致，全部 `status=implemented`、`dataEgress=none`；12 个路线图工具（`ocr.recognize`、`chat.track.read`、`hotkey.register`、`tts.speak`、`chat.send` 等）不在清单中且返回 `TOOL_NOT_FOUND`；每个工具的真实结果都按其声明的 outputSchema 校验；后端不可用时 `ready=false` 并给出原因。
- **ASR Stopped**：status 报告 `runtimePhase=stopped`、`requiredForTools=false`，同一服务完成列举→捕获→裁剪→下载。
- **裁剪与谱系**：二级裁剪的父/根 ID、`rootTransform`、`rootRect`、`screenRect` 精确；PNG 解码后逐像素等于原图偏移位置；原图哈希不变；负数/零宽高/越界/i64 溢出/小数/1e30/缺字段/额外字段均拒绝且不产生产物。
- **TTL/容量/释放**：到期前 1 ms 可读、到期即 `ARTIFACT_EXPIRED(ttl)`；容量回收 `ARTIFACT_EXPIRED(evicted)` 并在结果中返回 `evictedArtifactIds`；重复释放幂等；从未发放的 ID 与路径字符串都是 `ARTIFACT_NOT_FOUND`。
- **窗口失败**：非前台、最小化、句柄复用、关闭、跨实例 ID、像素超限分别返回独立错误码，失败路径不产出像素。
- **期限与并发**：捕获延迟 600 ms、期限 100 ms 时立即 `TIMEOUT`；后台仍占名额使后续调用 `BUSY`；后台完成后产物计数仍为 0。
- **发现**：其他会话看不到；会话不符、进程已退出、PID 复用、非 loopback 地址、协议版本不符、权限不安全、凭证被替换（401→`STALE_DESCRIPTOR`）、实例 ID 不符、端口无服务（`SERVICE_UNREACHABLE`）均被识别；退出只删除本实例的描述文件；仍在运行的外部实例不被覆盖，退出后可替换。
- **CLI**：真实二进制经私有描述文件连接，覆盖 status/list（JSON 与文本）、call（文件与 stdin 参数）、artifact get（像素与 SHA-256 核对、拒绝覆盖、`--force`）、工具错误退出码 1、本地参数错误 2、服务退出后 3；所有输出中不出现凭证；对已退出进程的描述文件报告 `STALE_DESCRIPTOR`，被授予 Everyone 读取的描述文件报告 `DESCRIPTOR_INSECURE`。

## 3. 原生检查（实际执行）

只使用测试自行创建的置顶、不进任务栏的 160×100 弹出窗口（左半红、右半蓝），没有捕获或读取任何用户窗口内容，没有向任何窗口发送输入。

| 检查 | 结果 |
| --- | --- |
| 不激活显示时，EnumWindows 列出测试窗口；PID/进程创建时间/可见性/客户区 160×100 物理像素/显示器/DPI 正确 | 通过；本机 DPI 96，显示器 2656×1220 |
| 非前台目标连续 10 次捕获均为 `NotForeground`，且测试窗口从未成为前台 | 通过 |
| PID 或进程创建时间不符 → `IdentityChanged` | 通过 |
| `SW_SHOWMINNOACTIVE` 后 `minimized=true`、客户区为 null、捕获 `Minimized`；失败路径 GDI 对象数不变 | 通过 |
| 关闭后 inspect 与捕获均为 `Gone` | 通过 |
| （ignored）切到前台后直接调用 GDI 后端 25 次：尺寸与客户区一致，左右边界与分界线像素精确为红/蓝，无 `UNIFORM_PIXELS`；前后 GDI 对象数一致 | 通过（运行时测试窗口短暂成为前台，结束后尝试还原原前台窗口） |
| （ignored）真实后端 → ToolService → loopback HTTP → 客户端：window.list → screen.capture（`captureMethod=gdi-screen-foreground-client`）→ 右半裁剪全为蓝色 → 最小化 `WINDOW_MINIMIZED` → 关闭 `WINDOW_GONE` | 通过 |

## 4. 捕获语义与已知限制

- 仅 GDI 前台可见客户区（`SRCCOPY | CAPTUREBLT`）：复制屏幕上该矩形的已合成像素；覆盖在上面的置顶窗口/通知/分层叠加层（如 Kotone 弹幕）会出现在图中；不支持后台、遮挡、最小化、隐藏窗口；非前台直接报错，不代为切换焦点；跨显示器或部分出屏拒绝，不退化为整桌面截图。
- 独占全屏、硬件叠加层或受保护 GPU 内容可能黑屏或旧帧，只能通过 `UNIFORM_PIXELS` / `EXCLUSIVE_FULLSCREEN_SUSPECTED` 警告提示，不能断言内容可读。
- 产物只在内存中，默认 TTL 300 秒、最多 32 个/256 MiB，空闲维护每 5 秒回收到期产物；`artifact.get` 以 base64 传输，单图最大约 16.7 M 像素。
- 描述文件对当前用户的任何进程可读——这是“当前用户可用”的设计边界，开启服务即表示同一用户下运行的软件都能截取前台客户区。
- 桌面端运行时用设置页保存其他设置，会把内存中的 `toolService.enabled` 一并写回；因此手册要求先退出桌面端再用 CLI 修改开关。本轮未增加设置页开关。

## 5. 未执行或未验证

- **真实桌面进程端到端**：没有启动本分支构建的 `kotone.exe`（会读写真实 `~/.kotone` 用户配置，并可能与用户正在运行的实例冲突），因此“开启开关→重启桌面→CLI 连接→退出删除描述文件”的完整 GUI 生命周期、`asr_phase` 读取真实 RuntimeManager、桌面日志行，只有编译与同一 `RunningToolService` 代码路径的自动化测试作为证据，尚需人工检查。
- **LOL 或其他真实游戏**：未对 LOL 客户端或对局做任何捕获；不能据本轮测试宣称可截取 LOL 聊天。窗口化/无边框/独占全屏三种模式下的像素是否有效、坐标是否与游戏 UI 缩放一致，都需要人工验收。
- **非 100% 缩放与多显示器**：本机为 96 DPI 单显示器；125%/150% 缩放、混合 DPI 多显示器、负坐标副屏的物理坐标与单显示器约束未实测。
- **提权组合**：桌面端以管理员运行、CLI 未提权（或反之）时的描述文件所有者校验与进程身份查询未实测；代码允许 Administrators 作为所有者。
- **远程桌面/多会话隔离**：会话 ID 分离只由假宿主测试覆盖，未在真实第二会话验证。
- **Playwright e2e**：`pnpm -C apps/desktop test:e2e` 两次都因 Vite 预构建时 esbuild.exe 无法写文件失败（`Failed to write to output file … Access is denied`）。排查：Windows 受控文件夹访问为关闭；同一会话中 node 可以写入相同目录，但 esbuild.exe（本 worktree 与原工作区的同一硬链接文件，SHA-256 相同）向 worktree 和 `%TEMP%` 写入都被拒绝，PowerShell 中同样复现，判断为本会话对该可执行文件的环境限制而非代码问题。前端改动仅为 `Settings` 类型新增 `toolService` 字段与 mock 默认值，`pnpm check` 已通过；e2e 需在正常开发环境或 CI 中补跑。
- 既有的 `kotone-cli` 单元测试（如 `listen_session_wav_mock_engine_exits_0`）沿用仓库原有行为，会使用默认 `~/.kotone` 位置；本轮新增测试全部使用临时目录，未读取或输出用户配置、历史、录音或密钥。

## 6. 人工验收建议

1. 托盘退出 Kotone → `kotone-cli config set toolService.enabled true` → 启动本分支桌面端 → `kotone-cli tools status`（ASR 保持停止）应显示运行中、`stopped`。
2. 打开一个普通窗口并置于前台，由另一个不抢焦点的触发方式（如计划任务或延时脚本）运行 `tools call window.active` 与 `screen.capture`，确认设置窗口未被唤起、前台未改变、截图内容正确。
3. 在 LOL 窗口化、无边框、全屏模式下分别重复第 2 步，记录是否出现 `UNIFORM_PIXELS` 及聊天区域坐标是否与 `clientRect`/DPI 一致。
4. 托盘退出 Kotone，确认 `~/.kotone/run/tool-service-s<会话>.json` 被删除，`tools status` 退出码为 3。

## 7. 评审修正（第二次提交）

独立评审确认了 8 类正确性问题（均为 M1 原有契约：期限、如实捕获、有界产物、关闭），本轮全部按“修复 + 能复现原缺陷的回归测试”处理，没有改为文档限制。

| 评审项 | 修正 | 回归测试（文件::测试） |
| --- | --- | --- |
| 1. DPI 切换失败仍标注物理像素 | `DpiScope::enter()` 检查 `SetThreadDpiAwarenessContext` 返回值，并用 `GetThreadDpiAwarenessContext`/`GetAwarenessFromDpiAwarenessContext` 核实线程为 per-monitor 感知；否则 `Unsupported` → `PLATFORM_UNSUPPORTED`，所有 `window.*`/`screen.capture` 都经过该检查 | `window_tools::tests::dpi_scope_requires_verified_per_monitor_context`（判定逻辑四种组合）、`verified_scope_is_per_monitor_and_restores_previous_context`（原生：作用域内为 per-monitor，离开恢复） |
| 2. 分层叠加层声明与 SRCCOPY 不符 | BitBlt 改为 `SRCCOPY \| CAPTUREBLT`；能力限制改写为“含分层叠加层（原生验证）”，并说明 CAPTUREBLT 可能使指针闪烁 | `native_capture::foreground_capture_exact_pixels_full_chain_and_no_gdi_leak`（ignored，已运行）新增：不激活的 `WS_EX_LAYERED` 置顶绿色窗口盖住目标客户区一角，捕获中该区域为绿色、区外仍为红/蓝、目标保持前台；移除后恢复红色 |
| 3. 执行器停顿时 `timeout_at` 接受迟到结果 | 绝对期限三道闸：取得名额后（`queued`）、工作线程入口（`not-started`）、提交闸门（存储锁内核对，`late-result`） | `tests_races::late_worker_result_after_executor_stall_is_not_published`：current-thread 运行时，期限 30ms、捕获 60ms，阻塞唯一运行时线程 150ms；断言 `TIMEOUT` 且 `stage=late-result`、驻留产物为 0 |
| 4. 排队超时的 release 之后仍删除产物 | 工作线程只解析参数，返回 `Prepared::Release`；释放只在提交闸门内执行；工作线程入口拒绝已过期工作 | `tests_races::queued_work_that_times_out_before_starting_has_no_side_effects`：`max_blocking_threads(1)` 被占满，release 与 capture 各以 20ms 超时；恢复后产物仍在、捕获后端从未被调用 |
| 5. 空闲时到期产物不回收 | `ToolService::spawn_maintenance`：单个任务、固定周期（默认 5s）、只持有弱引用，关闭信号或服务释放即退出；`RunningToolService` 持有并在关闭时中止/等待 | `tests_races::idle_maintenance_reclaims_expired_artifacts_without_reads`（只用不触发清理的 `resident_artifacts()` 观测，到期后无任何读请求即回收到 (0,0)，关闭后任务退出）；对照 `without_maintenance_expired_artifacts_stay_resident_until_touched` 证明观测接口本身不清理 |
| 6. capturedAt 晚于 PNG 编码 | 在捕获后端返回处立即取时钟，早于像素校验/编码/哈希；`createdAt` 为发布时刻；裁剪沿用根产物的 `source.capturedAtUnixMs` | `tests_races::captured_at_is_the_capture_boundary_and_survives_crops`：编码前测试缝把注入时钟推进 5s，断言 capturedAt 不含这 5s、createdAt 含；再推进 7s 裁剪，capturedAt 不变 |
| 7. 关闭与同步提交不可线性化 | 关闭标志移入产物存储的同一把锁：`close()` 在锁内置关闭并清空，`insert_gated`/`release_gated` 在锁内先核对关闭与期限再变更；`shutdown_now`/`shutdown` 先关闭服务（撤销准入与提交），再删描述文件、停 HTTP 与维护；HTTP 处理在服务关闭后对所有端点返回 503 `SERVICE_CLOSED` | `tests_races::shutdown_linearizes_against_a_paused_commit`：`GatedClock` 把提交停在插入前的时钟读取处，另一 OS 线程完成 `shutdown_now`+drop；放行后旧请求得到 `SERVICE_CLOSED`、存储保持 (0,0)、新请求被拒、维护任务退出；`artifacts::tests::commit_gate_rejects_after_close_and_after_deadline`；`tests_http::shutdown_removes_only_owned_descriptor` 改为断言关闭后任何响应都不是成功、端口最终不可达 |
| 8a. 整连接 120s 超时截断在途请求 | 去掉整连接超时：请求头读取（含 keep-alive 空闲）5s、请求体读取 10s（超时 408 `REQUEST_TIMEOUT`）；连接寿命到期调用 hyper `graceful_shutdown`（空闲连接立即关闭、在途请求完成），之后 60s 硬上限 | `tests_races::connection_lifetime_does_not_cut_an_in_flight_request`（手写 HTTP/1.1，同一连接寿命 300ms，空闲 200ms 后开始 500ms 捕获，完整 200 响应后连接被关闭）、`idle_keep_alive_connection_is_closed_at_lifetime`、`header_and_body_read_limits_still_apply` |
| 8b. CLI 无 `--force` 时 exists()+rename 竞态覆盖 | 内容先写入独占（`create_new`）临时文件；无 `--force` 用 `kotone_platform_windows::fs_publish::rename_no_replace`（`MoveFileExW` 不带 `REPLACE_EXISTING`）原子不覆盖发布；`--force` 用 `std::fs::rename` 替换；任何失败删除临时文件 | `tools::tests::concurrent_exports_without_force_publish_exactly_one_file`（12 线程同时导出：恰一个成功、内容属于胜者、无残留 `.part`）、`file_created_after_any_check_is_never_replaced_without_force`、`fs_publish::tests::refuses_existing_target_and_moves_otherwise` |

**回归测试确实能捕获原缺陷**（变异检查：临时还原单个修复，运行对应测试，随后恢复）：

| 临时还原的修复 | 结果 |
| --- | --- |
| 提交闸门不检查期限 | `late_worker_result_after_executor_stall_is_not_published` FAILED |
| 插入不检查关闭标志 | `shutdown_linearizes_against_a_paused_commit` FAILED |
| 工作线程入口不检查期限 | `queued_work_that_times_out_before_starting_has_no_side_effects` FAILED |
| 维护任务不回收 | `idle_maintenance_reclaims_expired_artifacts_without_reads` FAILED |
| capturedAt 在编码后取值 | `captured_at_is_the_capture_boundary_and_survives_crops` FAILED |
| 寿命到期直接断开连接 | `connection_lifetime_does_not_cut_an_in_flight_request` FAILED |
| 去掉 CAPTUREBLT（原生 ignored 测试） | **仍通过**：本机 Windows 11 DWM 下分层窗口不加 CAPTUREBLT 也被合入；原生测试验证了能力声明，但不能在本机单独证明 CAPTUREBLT 的作用 |

**本轮实际运行的命令与结果**（`CARGO_TARGET_DIR` 为本 worktree，`SHERPA_ONNX_LIB_DIR` 只读复用原工作区预编译库）：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过（编译覆盖全部目标，未执行遗留测试） |
| `cargo test -p kotone-tools` | 42 通过；整套连续重复 15 次全部通过（修复 `shutdown_removes_only_owned_descriptor` 的偶发失败后） |
| `cargo test -p kotone-cli --bin kotone-cli -- tools::` | 4 通过（只运行 tools 模块；未运行读取 `~/.kotone` 的遗留 CLI 测试） |
| `cargo test -p kotone-cli --test tools_cli` | 2 通过（真实 CLI 二进制 + 临时目录） |
| `cargo test -p kotone-platform-windows --lib -- fs_publish window_tools tool_host` | 8 通过 |
| `cargo test -p kotone-platform-windows --test native_capture` | 默认 1 通过（受控窗口，不需焦点） |
| `cargo test -p kotone-platform-windows --test native_capture -- --ignored --test-threads=1` | 1 通过（受控窗口短暂前台，含分层叠加层检查与完整 HTTP 链路） |
| `cargo test -p kotone-core --lib -- tools:: tool_service default_values_match_doc` | 3 通过 |

偶发失败说明：第一次 10 次重复中 `shutdown_removes_only_owned_descriptor` 失败 1 次。原因是 `shutdown_now` 同步中止接受循环是协作式的，关闭后极短时间内仍可能接受连接，而 `GET /v1/status` 当时没有检查关闭状态。修正为服务关闭后所有端点返回 503 `SERVICE_CLOSED`，测试改为断言“关闭后没有任何成功响应，且监听端口最终不可达”，不再假设关闭瞬间端口已消失。

本轮未重新运行：`cargo test --workspace`（含会读取默认 `~/.kotone` 的遗留 CLI 测试，按要求不执行）、`pnpm check`（前端未改动，沿用第 2 节结果）、`cargo build -p kotone-tauri` 单独构建（由 workspace clippy 编译覆盖，桌面 `tool_service.rs` 未改，`shutdown_now` 新顺序由 `RunningToolService` 测试覆盖）。Playwright e2e 仍受第 5 节环境限制。

工作区中 `crates/kotone-cli/src/main.rs` 出现了一处不是本轮实现的改动（遗留 `listen` 测试的配置隔离），推测来自并行的评审/修复流程；本次修正提交未包含它，保留在工作区待其所有者处理。

**仍未验证**（与第 5 节相同，修正没有改变）：真实桌面 GUI 生命周期、LOL 任一显示模式、125%/150% 与混合 DPI 多显示器（本机 96 DPI 单显示器）、提权组合、多会话隔离、不默认合入分层窗口的旧系统上 CAPTUREBLT 的实际作用、CAPTUREBLT 指针闪烁是否出现。
