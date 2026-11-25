# Kotone 开发指南

当前架构见 [architecture.md](architecture.md)，统一用语见 [glossary.md](glossary.md)。早期设计和阶段验收已归档至 [development-history.md](archive/development-history.md)，其中的路线图和 API 草案不再作为当前规范。

## 环境与启动

完整桌面构建面向 Windows 10/11 x64，需要 rustup（Rust 版本钉在根目录的 `rust-toolchain.toml`，进仓库目录自动用它，不用再写 `+版本号`）、MSVC / Windows C++ Build Tools、WebView2，以及根 `package.json` 指定的 pnpm。先安装依赖，再从仓库根运行：

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

三条写测试的约定（2026-11-25，CI 提速时定的）：

- **Playwright**：CI 上 4 个 worker、`fullyParallel`，本地默认 1 个，要快加 `--workers=4`；所以 spec 之间不能共享状态（固定端口、同一个文件）。截图一律走 `e2e/screenshot.cts`：CI 上不截，`KOTONE_E2E_SHOTS=1` 强制截、`=0` 强制不截。
- **kotone-cli 的集成测试**写成 `crates/kotone-cli/tests/it/` 下的子模块（在 `it/main.rs` 里 `mod`），不要在 `tests/` 下再加文件：每多一个文件就多链接一个大测试程序。
- **要一个“没人听的端口”**：绑一个不 `listen` 的 socket，测完再放；不要绑了马上放掉再连（Windows 上连没人听的端口要重试约 2 秒，这期间同进程别的测试可能正好绑上它）。

### 推送前

**推送到 main 之前跑 `pnpm prepush`**，最后一行是“可以推送”才推。它把上面的检查按 CI 的样子跑一遍（`scripts/prepush.mjs`）：

- 每次都跑：`cargo fmt --check`、`release:verify` / `release:test`、`pnpm check`、`cargo clippy … -D warnings`、`cargo test --workspace --locked --no-fail-fast`；
- 相对 `origin/main` 改了前端（`apps/desktop/`、`plugins/`，以及 spec 直接读的几个 Rust 文件和文档）才跑 Playwright（4 个 worker、`KOTONE_E2E_SHOTS=0`，不覆盖 `target/plugin-shots/` 里的审查截图）；改了 `Cargo.lock` / `pnpm-lock.yaml` 才跑两种依赖审计。`--all` 全跑，`--dry-run` 只列出要跑什么；
- 每一步开跑前都查游戏进程（含地下城与勇士和腾讯 ACE 反作弊的进程），跑着的时候也每 15 秒查一次：查到就把自己和之后的子进程降到低优先级、cargo 用 `-j 2`、Playwright 1 个 worker，正按正常设置跑的那一步停掉、按游戏模式重跑；`target/` 里的 `kotone.exe` 在跑（玩家在用 `pnpm dev` 测）就不跑。`TMP` / `TEMP` 设到 `target/tmp`，每一步的日志在 `target/prepush/`；
- 一步失败也接着跑后面的，最后一起列出没过的步骤。

这是文档里的规矩，不是 git hook：推送的多半是 Agent，全量要好几分钟，做成 hook 每次推送（包括只改文档的）都要干等。

受控窗口的前台像素测试会短暂把测试窗口切到前台，默认忽略，需要交互式桌面时单独运行：`cargo test -p kotone-platform-windows --test native_capture -- --ignored --test-threads=1`。

两条构建与真机测试的环境事实（2026-11-16 自动化真机测试时确认）：

- **C++ 依赖统一用静态运行时 `/MT`**：sherpa-onnx 预编译库是 `static-MT`，`.cargo/config.toml` 的 `[env]` 因此给 MSVC 目标设了 `CXXFLAGS_x86_64_pc_windows_msvc=/MT`，否则 cc 以 `/MD` 编译的 C++（如 `kotone-script` 用的 Luau）会和 sherpa 的 `libcpmt` 冲突（LNK2005 / LNK2038）。改过这个设置后需 `cargo clean -p luau0-src -p mlua-sys -p mlua` 让 C++ 重新编译。
- **从本仓库目录启动的 exe 以低完整性级别运行**：仓库根目录带 `Mandatory Label\Low Mandatory Level`（OI/CI 继承，供工具沙盒写入），所以 `target/debug/kotone.exe` 是 Low IL。Windows 的 UIPI 会**静默丢弃**它向普通（Medium）窗口发送的模拟输入——`SendInput` 仍返回成功。需要真实按键 / 打字的真机测试，先把 exe 复制到仓库外（例如 `%LOCALAPPDATA%\Temp\kotone-script-e2e\`）再启动；已安装的 Kotone 不受影响。不要改仓库目录的完整性标签。

GitHub 上（精确命令以工作流文件和 [发布清单](release-checklist.md) 为准）：

- [ci.yml](../.github/workflows/ci.yml)：推 main、PR、手动时跑，三个 job 并行——Rust lint（fmt、clippy）、Rust test（全量）、Frontend（release 校验、`pnpm check`、`pnpm build:web`、Playwright）。只改 `docs/`、`website/`、`*.md` 的推送不跑（测试会读的术语表、助手手册、官网中文文案、`SKILL.md` 除外）。CI 里 `CARGO_BUILD_JOBS` 覆盖本机 `.cargo/config.toml` 的 `jobs = 1`（clippy 4、test 2）；Rust 缓存只在 main 上保存。
- [audit.yml](../.github/workflows/audit.yml)：依赖审计，锁文件变了时跑，另外每天定时一次；忽略的公告写在 `.cargo/audit.toml`。
- [installer.yml](../.github/workflows/installer.yml)：debug NSIS 安装包，只在打包相关文件（Tauri 配置、`installer/`、图标、`Cargo.toml` / 锁文件、打进安装包的许可证文件等）变了时打。
- [release.yml](../.github/workflows/release.yml)：推 `v*` tag 时打正式安装包。依赖审计、版本校验每次都跑；这个提交在 ci.yml 上三个 job 都绿了就不再跑一遍测试（CI 还在跑就等它，最多 25 分钟），查不到就照旧全跑。
- 升级 Rust：单独一个提交只改 `rust-toolchain.toml` 的版本号，本机 `pnpm prepush` 过了再推。

`pnpm build` 生成 release NSIS 安装包；用户配置与模型不会因为构建而迁移。

## 文档和发布

行为变化在同一变更中更新当前手册。领域名词补 [术语表](glossary.md)，跨模块/数据边界变化补 [架构](architecture.md) 和 [隐私政策](../PRIVACY.md)，重要取舍新增 ADR。旧 ADR 保留历史正文，在开头标明修订或替代，并链接当前依据。

版本与发布说明通过仓库脚本维护，按 [release-checklist.md](release-checklist.md) 走完整发布验收。研究报告记录核实日期和固定来源，不把测量样本成绩写成所有设备的性能承诺。
