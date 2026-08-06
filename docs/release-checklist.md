# Kotone Windows 发布清单

## 自动门禁

- `pnpm release:verify`：桌面包、Tauri 配置、Rust crate 与 Cargo.lock 版本一致，Tag 必须为 `v<version>`。
- `pnpm check`：Svelte/TypeScript 静态检查无错误和警告。
- `pnpm -C apps/desktop test:e2e`：首次向导、跳过策略、缺失模型与下载失败恢复。
- `cargo test --workspace --locked`：核心状态机、模型下载、注入、热键和 Tauri 启动参数。
- `pnpm -C apps/desktop tauri build --debug --no-bundle`：真实 Windows WebView 壳编译。
- GitHub `CI`：每次主分支和 PR 重跑上述门禁。

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
- 安装包 SHA-256 已记录到 GitHub Release。

## 发布流程

1. `pnpm release:bump <版本号>`（或 `--rc` / `--patch` / `--minor` / `--major`）同步四处
   版本号（package.json / tauri.conf.json / Cargo.toml / Cargo.lock）并预置
   `CHANGELOG.md` 小节标题；随后在标题下补充变更摘要。
2. 本机执行全部自动门禁并构建 NSIS 安装包。
3. 在干净 Windows 用户环境完成安装/首次向导/卸载冒烟测试。
4. 推送 `v<version>` Tag；`Release Windows` 工作流创建 GitHub Draft Release。
5. 下载工作流产物，核对 SHA-256，并做一次安装包回归。
6. 将 Draft Release 发布为正式版本。

## 签名与更新策略

- 当前直接分发渠道为 GitHub Release + NSIS（按用户安装，不要求管理员权限）。
- Windows 代码签名证书接入后再启用签名门禁；未签名构建可能触发 SmartScreen。
- 自动更新（Tauri updater）已接入：`tauri.conf.json` 的 `plugins.updater` 指向
  `https://github.com/l1veIn/kotone/releases/latest/download/latest.json`，
  `bundle.createUpdaterArtifacts` 已开启，`Release Windows` 工作流会随 Release
  自动上传签名后的更新包与 `latest.json`。
- updater 私钥与密码只保管在 GitHub Secrets（`TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）；公钥写在 `tauri.conf.json`。
  **私钥或密码丢失 = 更新通道报废**（已装客户端将永远无法校验后续更新），
  请离线备份；轮换密钥意味着所有用户必须手动重装。
