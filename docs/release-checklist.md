# Kotone Windows 发布清单

## 自动门禁

- `pnpm release:verify`：桌面包、Tauri 配置和 Rust crate 版本一致，Tag 必须为 `v<version>`。
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

1. 更新三个版本号与 `CHANGELOG.md`。
2. 本机执行全部自动门禁并构建 NSIS 安装包。
3. 在干净 Windows 用户环境完成安装/首次向导/卸载冒烟测试。
4. 推送 `v<version>` Tag；`Release Windows` 工作流创建 GitHub Draft Release。
5. 下载工作流产物，核对 SHA-256，并做一次安装包回归。
6. 将 Draft Release 发布为正式版本。

## 签名与更新策略

- 当前直接分发渠道为 GitHub Release + NSIS（按用户安装，不要求管理员权限）。
- Windows 代码签名证书接入后再启用签名门禁；未签名构建可能触发 SmartScreen。
- 自动更新需要单独保管 Tauri updater 私钥。首版不启用 updater，避免在密钥与回滚策略未确定前形成不可维护的更新通道。
