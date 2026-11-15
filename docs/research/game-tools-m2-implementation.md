# 游戏工具 M2 实现与验证记录（2026-11-15）

范围：[演进计划](../product-evolution-game-tools.md) M2——图像预处理、OCR 与聊天消息跟踪的产品化。基线 `3a2ab7a`（M1 完成），分支 `claude/game-tools-m2`。现行用法、schema 与语义见 [工具服务手册](../tool-service.md) §7–§9；本文只记录做了什么、怎样验证、哪些仍未验证。

M2 不包含翻译、摘要、自动回复、LLM 调用、工作流、TTS/弹幕、通用热键（M3）与发送（M4）；这些工具不在 capabilities 中，调用返回 `TOOL_NOT_FOUND`。

## 1. 交付内容

| 部分 | 实现 |
| --- | --- |
| 领域端口 | `kotone-core::ocr`：`OcrBackend`（`id/kind/info/recognize`）、`OcrObservation`（文字、`line/block/page` 粒度、可空框、可空置信度、块内行、版面类别）、`OcrError`（未就绪/语言/选项/取消/期限/远端/传输/响应无效/引擎）、`OcrCancel`（可注册中断钩子）。core 未新增依赖 |
| 本地 OCR | 新 crate `kotone-ocr`：PP-OCRv5 mobile（韩文识别 + 多语言检测）纯 Rust 前后处理（BGR、短边 736、DB 后处理、CTC 解码，参数对齐 RapidOCR 3.10.0）；`ort.rs` 直接调用 ONNX Runtime 1.27.1 C API（sherpa-onnx v1.13.5 静态预编译包内已有，函数表下标取自官方 v1.27.1 头文件，运行时不支持 API 27 时整体报告不可用）；`onnx_meta.rs` 从模型 metadata 读取内嵌字符表；推理可经 `RunOptionsSetTerminate` 中断。不依赖实验 `.tmp-analysis/vendor`、Python 或新的 DLL |
| 模型安装 | `manifest.rs` 固定来源（ModelScope RapidAI/RapidOCR 标签 v3.10.0）、许可（Apache-2.0）、版本、大小与 SHA-256；`install.rs` 下载或 `--from-dir` 复制到临时文件、校验后原子发布，失败不留文件；就绪检查逐文件核对存在/大小/哈希（按大小+mtime 缓存），加载时对内存中同一份字节再校验，并以空输入自检输出类别数 |
| 在线 OCR | `deepseek.rs`：复用连接目录与系统凭据库；只有 `toolService.ocr.deepseekConnectionId` 指定连接才就绪；multipart 只含 `file`（所选产物 PNG）与 `purpose`；处理 HTTP 状态错误、HTTP 200 内的 `error` 对象、`finish_reason=length` 截断、空结果、结构无效；`<｜ref｜>/<｜box｜>` 0–1000 坐标解析为 block 观测，无标记时逐行无几何；置信度恒为 null；在独立的单线程运行时中执行，可被取消或期限中止 |
| 工具服务 | `kotone-tools`：6 个新工具与真实 schema；`preprocess.rs`（预设与 5 种操作、nearest/linear/cubic）；产物 `rootTransform` 改为可累积小数比例（整数仍以 JSON 整数输出，兼容 M1），新增 `derivation`；`ocr_results.rs`（驻留 OCR 结果，提交闸门与关闭线性化）；`tracking.rs`（聊天跟踪）；按本地/在线划分的 OCR 名额；每次调用的取消令牌（期限、关闭时取消）；新错误码 |
| 配置 | `toolService.ocr.deepseekConnectionId`（默认 None）：core 字段、round-trip 测试；CLI `config set` 白名单（必须是已有连接 ID，空字符串清除）；设置页按补丁合并保存，不会丢失该字段 |
| 桌面 | `tool_service.rs` 组装本地后端（模型目录随 `models.dir`）与在线后端（读取内存设置 + `SharedState.connections` 解析器）；`ocr-local` 默认 feature。未修改 `lib.rs` |
| CLI | `ocr status`、`ocr install`；`tools` 子命令无需改动即可调用新工具 |
| 文档 | 工具服务手册（§5 工具表、§7 预处理、§8 OCR 后端与安装、§9 聊天跟踪、错误码与上限）、CLI、架构、隐私、术语、索引、开发指南、第三方声明、演进计划状态 |

`Cargo.lock` 只新增内部 crate `kotone-ocr` 的条目及既有 crate 的依赖边；没有新的外部 crate（`wiremock`、`png`、`reqwest` 等均为工作区已锁定版本）。

## 2. 关键语义（摘要）

- **坐标**：子产物点 p 对应父产物点 `offset + p / scale`，根变换逐级复合。测试覆盖“3 倍放大 → 裁剪 → 2 倍放大”与 1.5 倍非整数比例；OCR 框以输入产物像素与根坐标同时返回。
- **不降级**：`ocr.recognize` 必须显式指定后端；本地失败时在线后端调用次数不变（测试断言）。
- **几何与置信度诚实**：没有框 → `geometry:"none"`、`box/rootBox=null`；整块框 → `block` 且块内行无坐标；没有置信度 → null；空结果 → 成功 + `empty:true` + 警告；技术失败 → 错误码。
- **跟踪**：可见集与 `newSince` 独立；同帧同内容幂等；改字同 ID 加 revision；相同文字可为新消息，无法区分时返回 `uncertainty`；滚动、续行候选、裁切、ROI/窗口变化、乱序、空观测、代际与游标失效的定义见手册 §9。匹配同时要求整行与说话者前缀之后的正文相似，避免同一说话者的不同短消息被误判为修订。
- **生命周期**：OCR 名额在期限内于进入工作线程前获取；名额随工作线程释放；期限/关闭取消在途推理；OCR 结果与跟踪变更只在锁内提交闸门写入。

## 3. 验证命令与结果

环境：Windows 11 Pro 10.0.26200，AMD Ryzen 7 9800X3D（8 核 16 线程），rustc/cargo 1.98.0，Node 24.18.1，pnpm 11.16.0。所有命令在主工作区运行（未建 worktree），`SHERPA_ONNX_LIB_DIR` 未设置，复用 `target/sherpa-onnx-prebuilt` 已解压的 v1.13.5 预编译包。

**环境限制**：本会话的命令沙箱不允许新编译的测试二进制在 `%TEMP%` 下创建目录（`tempfile::tempdir()` 返回拒绝访问，M1 已有的测试同样失败；M1 记录的 esbuild 现象同源）。因此测试以 `TMP=TEMP=<仓库>\target\tmp`（构建输出目录，已被 gitignore）运行；测试代码本身仍只用 `tempfile` 临时目录。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 本分支文件全部通过；唯一差异在 `apps/desktop/src-tauri/src/lib.rs`，来自工作区中他人未提交的修改（不属于本提交，未格式化、未暂存） |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过（编译覆盖桌面壳 `kotone-tauri`、CLI 与全部测试目标） |
| `cargo test -p kotone-tools --locked` | 73 通过（M1 原有用例 + 7 个 M2 真实 HTTP 端到端、17 个跟踪单元、4 个预处理、1 个 OCR 结果存储、1 个变换复合）；连续 5 次全部通过 |
| `cargo test -p kotone-ocr --features onnx --locked` | 25 通过（含 ONNX Runtime 函数表握手与损坏模型拒绝），真实模型冒烟 1 个默认忽略 |
| `cargo test -p kotone-ocr --locked`（无 onnx） | 24 通过：本地后端报告 `runtime-unavailable`，不推理 |
| `cargo test -p kotone-cli --test tools_cli --test tools_cli_m2 --locked` | 4 通过、1 忽略（真实 CLI 二进制 + 私有描述文件 + loopback HTTP） |
| `cargo test -p kotone-cli --bin kotone-cli --locked -- config_set tools::` | 16 通过（含 `toolService.ocr.deepseekConnectionId` 白名单） |
| `cargo test -p kotone-core --lib --locked -- ocr:: tool_service default_values_match_doc` | 4 通过 |
| `cargo test -p kotone-platform-windows --test native_capture --locked` | 1 通过、1 忽略（仅为 `ServiceDeps` 新字段更新，捕获路径未改） |
| 独立构建检查 | TBD_INDEPENDENT |

自动化覆盖要点（均经真实 HTTP 服务；CLI 部分经真实 `kotone-cli` 二进制）：

- **预处理/坐标**：最近邻 3 倍放大后在 (30,15) 裁剪 12×9，逐像素等于原图 (10+x/3, 5+y/3)；`rootTransform`、`rootRect`、`screenRect`（含客户区屏幕偏移）精确；二级放大比例相乘；1.5 倍线性放大后裁剪得到小数偏移；原图哈希不变；`preset=none` 不创建产物；二选一、空列表、越界比例、超像素、未知操作、多余 `path` 字段、未知预设均拒绝。
- **OCR（假后端）**：行框经“裁剪 + 3 倍放大”映射到根坐标；null 置信度保持 null；block 与无几何观测；空结果；`OCR_BACKEND_NOT_FOUND/NOT_READY/LANGUAGE_UNSUPPORTED/OPTION_UNSUPPORTED/FAILED/TRANSPORT_ERROR`；路径字符串作为产物 ID 被拒绝；本地失败不调用在线后端；`ocr.backends` 的外发声明与就绪；所有结果按 capabilities 声明的 outputSchema 校验。
- **DeepSeek（真实后端对接本地 wiremock）**：block 坐标换算；HTTP 200 内 `error` → `OCR_REMOTE_ERROR`（不可重试）；截断警告；429 可重试；空结果；每个请求带 `Bearer` key、恰好两个表单字段、包含所选产物 PNG 的原始字节且不含窗口标题；未配置时 `not-configured` 且不发请求。后端单元测试另覆盖端点推导/https 约束、取消与期限在慢响应时 2 秒内返回。
- **期限/名额/关闭**：协作后端超时后收到取消、结果不发布；不响应取消的后端超时后同名额的第二次调用 `BUSY`（`details.lane`）且从未开始，工作线程真正结束后才能再次调用；关闭时在途识别被取消并返回 `SERVICE_CLOSED`，OCR 结果与会话均为 0；跟踪提交停在时钟闸门处、期限过后放行 → `TIMEOUT(stage=late-result)` 且不创建会话。
- **跟踪**：首帧、改字修订（同 ID、revision 2、`previousText`）、同一说话者重复同一句为新消息、滚动保持 ID、`visible` 返回已读旧消息、`newSince` 只返回新增/修订、空 OCR 不清空不推进游标、外部视觉观测（产物像素框映射到根坐标、来源标注、ROI 变化警告）、旧帧完全重放幂等、乱序帧拒绝且状态不变、窗口变化拒绝、reset 清理计数与旧游标 `CURSOR_STALE`、参数/会话/游标/代际错误码；单元测试另覆盖无几何重复文字的不确定性、向上翻看、翻回后与历史重新对齐（不重复计为新消息）、裁切修订、续行候选、频道/说话者证据、指令样式文字原样保存、事件容量导致 `CURSOR_EXPIRED`、会话容量回收、空闲 TTL、关闭、来自未来的游标、同帧先空后有内容不被强制更正。
- **安装**：缺失 → `not-installed`；同大小篡改 → `invalid`；截断 → 大小不符；下载内容哈希不符或超长 → 失败；CLI `ocr install --from-dir` 篡改文件 → 退出码 1 且模型目录为空；`ocr status` 报告运行时版本、许可与哈希。

## 4. 真实本地韩文 OCR 冒烟（实际执行）

- **模型**：用产品入口 `kotone-cli ocr install --model-dir target\ocr-smoke\ppocrv5-korean-mobile` 从 ModelScope 官方地址下载（隔离目录，不是用户模型目录）。首次下载返回 HTTP 403——ModelScope CDN 拒绝没有 User-Agent 的请求；改为发送通用 `kotone-ocr/<版本>` UA 后成功，两文件 SHA-256 与清单一致（`4d97c44a…78ae`、`cd6e2ea5…73c9b`），与实验目录中 RapidOCR 下载的文件哈希也一致。
- **输入**：不使用 39 张实验图或任何用户截图。用 PowerShell System.Drawing 以 Windows 自带 Malgun Gothic 15px 在 650×200 深色底上绘制 4 行合成韩文聊天（`[팀] 테스트용사 (아리): 안녕하세요 반갑습니다`、`[전체] 플레이어둘 (가렌): 미드 먼저 갑니다`、`[팀] 테스트용사 (아리): 용 타이밍 3분 남았어요`、`오늘 저녁에 한 판 더 해요`），PNG SHA-256 `0c033903…3f4b4d`。
- **路径**：`kotone-cli ocr status` → 假窗口返回该图像像素 → 真实 HTTP 服务 + 真实 `LocalKoreanOcr` → CLI `tools call window.active / screen.capture / ocr.recognize`（冷、热、热）→ `chat.track.ingest` → `image.preprocess(upscale-2x-cubic)` → `ocr.recognize`（`cargo test -p kotone-cli --test tools_cli_m2 -- --ignored`，模型目录与输入通过环境变量显式给出）。

结果（4 行全部检出，行框为原图物理像素）：

| 行 | 识别原文 | 识别器分数 |
| --- | --- | --- |
| 1 | `[팀] 테스트용사(아리):안녕하세요 반갑습니다` | 0.986 |
| 2 | `[전체] 플레이어둘 (가렌): 미드 먼저 갑니다` | 0.965 |
| 3 | `[팀] 테스트용사 (아리):용 타이밍 3분 남았어요` | 0.962 |
| 4 | `오늘 저녁에 한 판 더 해요` | 0.963 |

字符全部正确，部分行丢失或增加了空格（OCR 常见现象，跟踪匹配因此忽略空白）。2 倍放大后识别结果相同（第 3 行空格恢复），各行 `rootBox` 映射回原图后与 1 倍结果相差不超过 1 像素（如 `x 13,y 21,w 318,h 24` 对 `x 13,y 21,w 318.5,h 23.5`），验证了放大链的坐标映射。

服务端计时（`timing`，毫秒；`ocrMs` 仅为识别阶段，**不是**完整可播报耗时，不含截图 API、翻译、TTS 或播放）：

| 构建 | 轮次 | ocrMs | modelLoadMs | detMs | recMs | CLI 往返 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| debug | 冷 | 860 | 369 | 373 | 117 | 883 |
| debug | 热 | 397 | — | 308 | 87 | 420 |
| debug | 热 | 407 | — | 314 | 93 | 430 |
| release | 冷 | 390 | 122 | 205 | 61 | 412 |
| release | 热 | 199 | — | 155 | 43 | 219 |
| release | 热 | 179 | — | 143 | 36 | 204 |
| release | 2 倍放大图（热） | 197 | — | 138 | 59 | — |

“CLI 往返”是测试进程测得的 `kotone-cli tools call` 子进程墙钟时间（含进程启动、描述文件校验与 HTTP）。release 为 `cargo test --release` 构建的同一测试；debug 构建中纯 Rust 前后处理未优化，耗时偏高。冷启动指桌面实例内首次识别（含读取并校验 18 MB 权重、创建两个 ONNX 会话与自检），不含 `kotone-cli ocr install` 下载（本次两个文件下载约 1.2 秒，取决于网络）。

这只是一张合成图的功能冒烟：说明本地链路真实可运行、坐标与计时可观测；不代表游戏截图上的识别率，也不能与研究报告中的 39 图实验结果比较或据此宣称产品成功率。39 张图已参与实验，不作为留出集。

## 5. 回归测试能捕获的缺陷（变异检查）

临时破坏单个保证、运行对应测试、随后恢复源码（脚本逐项执行并还原，未提交任何变异）：

| 临时破坏 | 对应测试 | 结果 |
| --- | --- | --- |
| OCR 名额在工作线程结束前释放 | `tests_m2::ocr_timeout_cancels_and_lane_is_held_until_the_worker_really_ends` | FAILED（捕获） |
| 跟踪提交闸门不检查期限 | `tests_m2::late_ingest_commit_does_not_mutate_tracking_state` | FAILED（捕获） |
| 修订匹配不检查正文相似度 | `tracking::tests::same_speaker_short_messages_with_long_shared_prefix_are_not_revisions` | FAILED（捕获）；最初选用的 `repeated_identical_text_from_same_speaker_can_be_new` 没能捕获（其夹具整行相似度已低于阈值），因此补充了这个专门用例 |
| 空观测清空可见集 | `tracking::tests::empty_observation_keeps_visible_set_and_cursor` | FAILED（捕获） |
| 关闭时不取消在途 OCR | `tests_m2::shutdown_cancels_inflight_ocr_and_rejects_its_result` | FAILED（捕获） |

## 6. 未执行或未验证

- **真实桌面 GUI 生命周期**：未启动本分支的 `kotone.exe`（会读写真实 `~/.kotone`、可能与用户实例冲突）。桌面 `tool_service.rs` 的后端组装只有编译与 clippy 证据；`SharedState` 设置读取、`models.dir` 自定义目录、设置页保存后 `toolService.ocr` 保留需人工检查。
- **真实 DeepSeek 在线 OCR**：普通测试只对接本地 mock，未调用付费 API、未读取 Keyring；真实接口的响应形态、坐标标记在聊天截图上的实际出现频率、计费与延迟未在本轮验证。接口行为依据 2026-11-15 抓取的官方文档。
- **游戏截图上的 OCR 质量**：只做了合成图冒烟；没有在 LOL 截图、不同 UI 缩放、字号、颜色、半透明背景上评估；没有新的留出集。
- **长时间运行**：模型会话常驻（约 18 MB 权重 + 运行时内存）未做内存曲线；多次加载/卸载、并发压力未测。
- **ONNX Runtime 共享**：本地 OCR 与 ASR 共用 sherpa-onnx 静态链接的同一运行时与进程级 Env；ASR 运行中同时执行 OCR 的 CPU 争用与延迟影响未测。
- **跟踪启发式**：对齐、相似度阈值与续行/裁切判断只由确定性夹具验证；真实游戏中淡出、系统消息穿插、极快刷屏、多行换行等情况下的误合并/误拆分率未知，`uncertainty` 的覆盖面也未经实测。
- **Playwright e2e / `pnpm check`**：前端未改动，本轮未运行（M1 记录的 esbuild 环境限制仍在）。
- **`cargo test --workspace`**：未整体运行；按 crate 运行了与改动相关的全部测试目标。
