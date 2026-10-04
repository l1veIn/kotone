# Kotone Windows 发布清单

## 自动门禁

- `pnpm release:verify`：桌面 package、Tauri 配置、桌面 Rust crate 与其 Cargo.lock 条目版本一致，Tag 必须为 `v<version>`；库 crate 有独立版本。
- `pnpm release:test`：更新说明提取、Unreleased 归档与缺失说明门禁通过。
- `pnpm audit --prod`：生产 JavaScript 依赖审计。
- `cargo audit --ignore RUSTSEC-2024-0429`：与 CI 当前例外一致的 Rust 依赖审计；例外变动需同时审查工作流与这里。
- `cargo fmt --all -- --check`：Rust 格式。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：Rust lint 无警告。
- `pnpm check`：Svelte/TypeScript 静态检查无错误和警告。
- `pnpm -C apps/desktop test:e2e`：首次向导、跳过策略、缺失模型与下载失败恢复，以及助手分页、三种模式、语音按钮和弹幕/语音设置。
- `cargo test --workspace --locked`：核心状态机、模型下载、注入、热键和 Tauri 启动参数。
- `pnpm -C apps/desktop tauri build --debug --bundles nsis --config src-tauri/tauri.ci.conf.json`：真实 Windows WebView 壳与 NSIS 安装器编译。
- GitHub [CI](../.github/workflows/ci.yml)：主分支和 PR 门禁，精确工具版本与步骤以工作流为准。

网站有改动时另跑 `pnpm --dir website check` 和 `pnpm --dir website build`，并检查中英文页面；当前桌面 CI 不代替这两项网站验证。构建不等于发布官网。

## 发布候选验收

- 首次安装启动默认进入向导；`--onboarding=always|never|auto` 均符合预期。
- 默认选择英雄联盟配置，模型下载完成后可继续。
- 热键可在目标游戏中触发，预览/发送/取消完整闭环。
- 未下载模型点“启动”会进入可恢复的高级模型页。
- 模型下载断网/超时后保留错误与“重试”，可切换下载源/代理。
- 右键、Ctrl+F、F5、Ctrl+Shift+R、F12 不暴露 WebView 行为。
- 悬浮窗无原生透明阴影；固定位置、拖动记忆、点击穿透均验证。
- 普通设置页不暴露引擎 ID、热键后端、权限探测、下载源等技术项。
- NSIS 安装、覆盖安装、卸载后重新安装均通过。
- 安装后的桌面程序直接启动时不额外打开日志控制台；本地 debug 验证包也使用 Windows GUI 子系统（PE Subsystem = 2）。
- 安装包 SHA-256 已记录到 GitHub Release。
- 当前文档索引、术语、隐私说明和中英文网站与行为一致；旧研究和 ADR 明确标注历史/修订。
- 提醒助手热键、AI 连接失败、语音/弹幕测试、录音互斥、取消和重启错过提醒均按 [助手手册](assistant.md) 验收；真实游戏的无边框与独占全屏分别记录结果。
- 创建提醒右上角的语音按钮显示助手快捷键；未绑定快捷键仍能用按钮，未启动时明确提示先启动。三种助手模式、快速按放、失焦/切页取消和录音笔确认均正确；旧助手独奏配置回到对讲机，普通游戏独奏不变。
- 「停止播报 / 弹幕」在语音、等待麦克风和弹幕阶段都能停止当前输出，停止后不重新出现；后续新提醒仍能投递。
- 弹幕距顶部可精确保存像素值；在不同屏幕尺寸和缩放下保持可见，变化位置不抢焦点、不影响鼠标穿透。
- 弹幕设置与测试区明确说明不支持独占全屏，建议无边框全屏/窗口化或改用语音；检测到独占全屏时显示相同解决办法。

## 发布流程

1. 先在 `CHANGELOG.md` 的 `Unreleased` 中整理变更摘要，再运行
   `pnpm release:bump <版本号>`（或 `--rc` / `--patch` / `--minor` / `--major`）。脚本同步四处
   版本号（package.json / tauri.conf.json / Cargo.toml / Cargo.lock），将 `Unreleased` 归档到
   带日期的版本小节，并保留新的空 `Unreleased`。若此前没有 `Unreleased`，脚本会插入待补充小节；
   发布前须补齐摘要，不能保留空说明或占位文字。
2. 本机执行全部自动门禁并构建 NSIS 安装包。
3. 在干净 Windows 用户环境完成安装/首次向导/卸载冒烟测试。
4. 推送 `v<version>` Tag；`Release Windows` 工作流先创建 Draft，完成构建、签名、清单与
   SHA-256 上传后自动发布稳定版 GitHub Release。
5. 同一 Tag 工作流随即调用 `Publish Release to Tencent COS`，上传版本化安装包、签名和
   校验文件，并在最后更新 COS `releases/latest.json`；RC Tag 保持 Draft/Prerelease，不进入稳定通道。
6. 确认 GitHub 与 COS 工作流成功，下载并核对 SHA-256，再做一次安装包回归。
7. 确认“检查更新”能看到版本号与 CHANGELOG 更新说明。

## 签名与更新策略

- 当前直接分发渠道为腾讯云 COS（中国大陆首选）与 GitHub Release（兜底）+ NSIS
  （按用户安装，不要求管理员权限）。
- Windows 代码签名证书接入后再启用签名门禁；未签名构建可能触发 SmartScreen。
- 自动更新（Tauri updater）已接入：`tauri.conf.json` 的 `plugins.updater` 首先读取
  COS `releases/latest.json`，GitHub 官方地址作为兜底。`bundle.createUpdaterArtifacts`
  已开启；稳定 Tag 只有在全部 GitHub Release 附件上传完成后才自动发布并触发 COS，RC Tag 不会
  更新稳定通道。已发布版本拒绝由 Tag 工作流重建，COS 版本化对象也禁止覆盖。
- COS 发版子账号的对象读写只限 `kotone-1258931230/releases/*`；另有 `coscli cp`
  所需的桶级元数据权限 `HeadBucket / GetBucket / ListMultipartUploads`。仓库 Secrets 为
  `TENCENT_COS_SECRET_ID` / `TENCENT_COS_SECRET_KEY`；不授予删除对象或修改桶配置权限。
- updater 私钥与密码只保管在 GitHub Secrets（`TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）；公钥写在 `tauri.conf.json`。
  **私钥或密码丢失 = 更新通道报废**（已装客户端将永远无法校验后续更新），
  请离线备份；轮换密钥意味着所有用户必须手动重装。
