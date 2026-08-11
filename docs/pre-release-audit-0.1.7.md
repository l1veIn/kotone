# Kotone 0.1.7 发布前审计

日期：2026-08-11
审计分支：`codex/pre-release-readiness`

## 结论

已发现的主流程高风险问题已修复并补回归测试。当前没有已知的
P0/P1 业务阻断项，可进入正式版发布流程。

## 主流程修复

- 发送与重试：确认、错误重试与重发改为原子争用发送权，避免重复注入；
  浮窗重试保留原 profile、频道、焦点目标和历史语义。
- 运行时快照：启动时锁定已预热的引擎、模型和识别参数；设置变更只会
  标记「需重启」，不会在已运行会话中偷换配置。停止时卸载真正已启动的引擎。
- STT 预热：`warmup` 使用真实 `SessionConfig`，线程数、语言、热词及权重与会话一致。
- 音频稳定性：生产 PCM 改为有界队列，满载时中止并显示错误；`push_audio`
  失败不再被吞掉；单次收音限制为 10 分钟，防止评测/历史/非流式缓冲无界增长。
- 全屏保护：后端在设置窗口隐藏时仍监控活动游戏；独占全屏时拦截浮窗
  显示、隐藏已显示浮窗，并持久提示切换为无边框或窗口模式。

## 持久化与输入安全

- 新增 `SettingsRepository`，将「写盘 → 外部资源切换 → 内存发布」串行事务化；
  热键、模型、目录迁移失败时回滚磁盘和资源。
- 损坏/不可读配置先备份为 `config.json.corrupt[.N]`，再恢复默认值并向用户显示一次诊断。
- 历史记录按目录串行化，解决并发 append/delete/trim 丢失更新；封顶缓存减少全量扫描。
- `.kprofile`/热词导入增加包、条目、JSON、图标、热词和字段上限；图标校验扩展名、
  文件签名和尺寸，阻断 ZIP bomb 与本地资源耗尽。
- 模型下载设置 15s 连接、30s 读取和 4h 总超时；活动状态由 RAII 释放。
  已有文件必须同时通过 size + SHA-256，清单 URL 固定到不可变提交。

## 结构与风格

- 将全屏/UIPI 兼容逻辑从 Tauri `lib.rs` 抽出为 `compatibility.rs`。
- 设置页重复的 patch/store/toast 流程收敛为 `patchSettings`。
- 浮窗使用独立的最小 Tauri capability，不再继承主窗口的 dialog/updater/process/
  global-shortcut 权限。
- Rust workspace 已全量 `rustfmt`，并修至 `clippy --workspace --all-targets -D warnings` 通过；
  CI 新增 fmt 和严格 Clippy 门禁。

### 仍偏大的文件

`src-tauri/lib.rs`、`orchestrator.rs`、`AdvancedPage.svelte`、`ipc.ts`、`model.rs` 和 CLI `main.rs`
仍是后续重构候选。它们目前内部的职责边界和测试覆盖足以支撑本次发布；
发布前继续大面积搬迁会引入不必要的回归面。建议正式版后分别按 IPC domain、
session/send/history actor、模型列表/调优/诊断 UI 以及 manifest/download/storage 进一步拆分。

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `cargo test --workspace --locked --no-fail-fast -j 1`：331 passed，5 ignored（真机/公网手动项）。
- `pnpm -C apps/desktop check`：0 errors / 0 warnings。
- `pnpm -C apps/desktop test:e2e`：10 passed。
- Web 生产构建：通过。
- CI 等价的 Tauri debug/NSIS bundle：通过，已生成安装包。
- `pnpm audit --prod`：无已知漏洞。
- `cargo audit`：无未允许漏洞；`event-listener` 已升级到修复版 5.4.2。

## 已知非阻断项

- sherpa-onnx 上游 Windows 预编译静态库在 debug/test 链接时会输出 `LNK4098` CRT 混用警告；
  当前全部本地测试与 release 构建均能完成链接。切换上游 shared 包会改变 DLL 分发模型，
  不在本次发布前冒险切换。
- 本地执行正式 Tauri bundle 时已生成 exe 和 NSIS 安装包，随后因本机未配置
  `TAURI_SIGNING_PRIVATE_KEY` 停在更新签名阶段；CI/发布环境需提供该 secret。
- RustSec 剩余均为已允许的上游维护性告警：GTK3/glib 只存在于 Tauri 的非 Windows
  依赖树；`unic-*` 仍由 Tauri 的 URL pattern 解析链引入，当前没有可直接替换的应用层依赖。
