# Kotone 开发指南

当前架构见 [architecture.md](architecture.md)，统一用语见 [glossary.md](glossary.md)。早期设计和阶段验收已归档至 [development-history.md](archive/development-history.md)，其中的路线图和 API 草案不再作为当前规范。

## 环境与启动

完整桌面构建面向 Windows 10/11 x64，需要 Rust stable/MSVC、Windows C++ Build Tools、WebView2，以及根 `package.json` 指定的 pnpm。先安装依赖，再从仓库根运行：

```powershell
pnpm install --frozen-lockfile
pnpm dev
```

`pnpm dev:web` 启动浏览器 mock UI，适合快速调整页面；它不连接真实热键、音频、注入或 Windows 语音。`cargo build -p kotone-cli` 构建无 Tauri 命令前端。桌面和 CLI 默认带 sherpa 与 VAD；真实 ASR 模型按需下载，不应为了普通单元测试下载大模型。

仓库存在 `.codegraph/` 时，理解或定位代码先用 `codegraph explore "问题或符号"`；不为完成任务自行建立索引。不要扫描用户配置、录音或密钥目录来代替测试夹具。

## 修改入口

| 需求 | 入口 |
| --- | --- |
| 会话状态、取消、历史、配置 | `crates/kotone-core/src` 与 `crates/kotone-core/tests` |
| 模型与识别适配器 | `crates/kotone-stt/src/model.rs`、`sherpa_runtime.rs`、各适配器 |
| Windows 采音、热键、注入、语音 | `crates/kotone-platform-windows/src` |
| AI 服务和凭据 | `crates/kotone-ai/src` |
| 文本处理器 | `crates/kotone-postprocess/src` |
| 桌面组合与 IPC | `apps/desktop/src-tauri/src`；前端 `lib/ipc.ts` 命令封装、`lib/ipc/contracts.ts` 类型、`lib/ipc/browserMock.ts` 浏览器实现、`lib/assistant.ts` 助手命令 |
| 设置页面、悬浮窗、浏览器测试 | `apps/desktop/src/routes`、`apps/desktop/e2e` |
| 命令行与评测目标解析 | `crates/kotone-cli/src/main.rs`、`crates/kotone-stt/src/eval.rs` |
| 本机 Agent 工具服务 | 端口 `crates/kotone-core/src/tools.rs`、`ocr.rs`；协议/存储/预处理/跟踪/服务端/客户端 `crates/kotone-tools`；OCR 后端与模型安装 `crates/kotone-ocr`；Windows 实现 `kotone-platform-windows/src/window_tools.rs`、`tool_host.rs`；桌面生命周期 `apps/desktop/src-tauri/src/tool_service.rs`；CLI `crates/kotone-cli/src/tools.rs`、`ocr.rs` |
| 连外部 MCP 服务器（客户端） | `crates/kotone-mcp`（rmcp 上包一层：子进程 stdio、Streamable HTTP）；子进程的 Job Object `kotone-platform-windows/src/job.rs`；测试服务器 `kotone_mcp::testing`、`crates/kotone-mcp/examples/stdio_server.rs` |

**配置兼容（2026-11-16 起）**：用户可能不升级，或升级后又装回旧版。旧版本读到某个字段里不认识的值时，会把整份 `config.json` 当作损坏、改名备份并恢复默认（0.1.13 即如此）。因此：已有字段只写旧版本认识的值，不扩展它的取值范围、不改变写法；新能力一律用新字段（旧版本会忽略未知字段）。例如在线语音写 `assistant.ttsModelId` 而不是给 `assistant.speechEngine` 加新值；声音编号在界面与接口上是 `提供方/声音`，落盘仍是 `kokoro:zf_001` 与不带前缀的 Windows 语音；陪玩路由落盘仍是 `reminders`。代价：旧版本保存配置时会丢掉它不认识的新字段。`kotone-core` 有对应的回归测试。

修改接口时同时检查 Rust、IPC 类型、Svelte 消费者和浏览器 mock。新增能力通过领域端口注入；core 不能反向依赖平台、STT 或网络实现。锁保护区内避免网络/音频设备/模型加载等阻塞操作，异步结果交付前再次核对会话代际。

**文件组织**：新功能尽量放新文件；一个文件只管一件事；按职责拆，不按行数拆（例如桌面端 `agent/` 按“入口与附注 / 跑一轮 / 语音收尾 / 工具宿主 / 后台任务 / 转达 / 界面命令”分文件，`lib.rs` 只留组装，命令放在各自的 `*_cmd.rs`）。

## 验证

先运行覆盖改动的测试，再执行适用的集成门禁。普通测试使用临时目录、假引擎和假注入器，不发送到真实游戏窗口，不读取个人语料。真实模型、音频回路、SAPI 播报和游戏测试单独标明前置条件及人工结果；编译成功不能代替真机验收。

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm check
pnpm -C apps/desktop test:e2e
pnpm release:verify
pnpm release:test
```

受控窗口的前台像素测试会短暂把测试窗口切到前台，默认忽略，需要交互式桌面时单独运行：`cargo test -p kotone-platform-windows --test native_capture -- --ignored --test-threads=1`。

两条构建与真机测试的环境事实（2026-11-16 自动化真机测试时确认）：

- **C++ 依赖统一用静态运行时 `/MT`**：sherpa-onnx 预编译库是 `static-MT`，`.cargo/config.toml` 的 `[env]` 因此给 MSVC 目标设了 `CXXFLAGS_x86_64_pc_windows_msvc=/MT`，否则 cc 以 `/MD` 编译的 C++（如 `kotone-script` 用的 Luau）会和 sherpa 的 `libcpmt` 冲突（LNK2005 / LNK2038）。改过这个设置后需 `cargo clean -p luau0-src -p mlua-sys -p mlua` 让 C++ 重新编译。
- **从本仓库目录启动的 exe 以低完整性级别运行**：仓库根目录带 `Mandatory Label\Low Mandatory Level`（OI/CI 继承，供工具沙盒写入），所以 `target/debug/kotone.exe` 是 Low IL。Windows 的 UIPI 会**静默丢弃**它向普通（Medium）窗口发送的模拟输入——`SendInput` 仍返回成功。需要真实按键 / 打字的真机测试，先把 exe 复制到仓库外（例如 `%LOCALAPPDATA%\Temp\kotone-script-e2e\`）再启动；已安装的 Kotone 不受影响。不要改仓库目录的完整性标签。

CI 还执行依赖审计与 debug NSIS 构建；精确命令以 [ci.yml](../.github/workflows/ci.yml) 和 [发布清单](release-checklist.md) 为准。`pnpm build` 生成 release NSIS 安装包；用户配置与模型不会因为构建而迁移。

## 文档和发布

行为变化在同一变更中更新当前手册。领域名词补 [术语表](glossary.md)，跨模块/数据边界变化补 [架构](architecture.md) 和 [隐私政策](../PRIVACY.md)，重要取舍新增 ADR。旧 ADR 保留历史正文，在开头标明修订或替代，并链接当前依据。

版本与发布说明通过仓库脚本维护，按 [release-checklist.md](release-checklist.md) 走完整发布验收。研究报告记录核实日期和固定来源，不把测量样本成绩写成所有设备的性能承诺。
