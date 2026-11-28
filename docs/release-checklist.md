# Kotone Windows 发布清单

## 自动门禁

- `pnpm release:verify`：桌面 package、Tauri 配置、桌面 Rust crate 与其 Cargo.lock 条目版本一致，Tag 必须为 `v<version>`；库 crate 有独立版本。
- `pnpm release:test`：更新说明提取、Unreleased 归档与缺失说明门禁通过。
- `pnpm audit --prod`：生产 JavaScript 依赖审计。
- `cargo audit`：Rust 依赖审计；忽略的公告和理由只写在 `.cargo/audit.toml`。
- `cargo fmt --all -- --check`：Rust 格式。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：Rust lint 无警告。
- `pnpm check`：Svelte/TypeScript 静态检查无错误和警告。
- `pnpm -C apps/desktop test:e2e`：首次向导、跳过策略、缺失模型与下载失败恢复，以及助手分页、三种模式、语音按钮和弹幕/语音设置。
- `cargo test --workspace --locked`：核心状态机、模型下载、注入、热键和 Tauri 启动参数。
- `pnpm -C apps/desktop tauri build --debug --bundles nsis --config src-tauri/tauri.ci.conf.json`：真实 Windows WebView 壳与 NSIS 安装器编译（GitHub 上由 [installer.yml](../.github/workflows/installer.yml) 在打包相关文件变了时跑）。
- GitHub [CI](../.github/workflows/ci.yml)：主分支和 PR 门禁（Rust lint、Rust test、Frontend 三个 job），精确步骤以工作流为准；依赖审计在 [audit.yml](../.github/workflows/audit.yml)（锁文件变了 + 每天一次）。Rust 版本钉在 `rust-toolchain.toml`。
- 发版（[release.yml](../.github/workflows/release.yml)）：审计和版本校验每次都跑；tag 指向的提交在 CI 上三个 job 都绿了，就不再跑 `pnpm check`、Playwright、`cargo test`（CI 还在跑就等它跑完，最多 25 分钟；查不到就全跑）。

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
- 提醒（她安排的、「自动化」里的“X 秒之后提醒我”、0.1.x `reminders.json` 导入的）、AI 连接失败、语音/弹幕测试、录音互斥、取消和重启错过提醒均按 [助手手册](assistant.md) 验收；真实游戏的无边框与独占全屏分别记录结果。
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
2. 本机执行全部自动门禁并构建 NSIS 安装包。用**正式版**程序跑一遍内置浏览器的真机脚本：
   `KOTONE_EXE=target/release/kotone.exe python -X utf8 scripts/manual-e2e/browser_real.py`，以及调试版的
   `browser_probe_real.py --minimized`（主窗口最小化时建浏览器；rc1 就是在这个状态下没建起来）。
3. 在干净 Windows 用户环境完成安装/首次向导/卸载冒烟测试。

### 0.1.x 维护版

0.1.14 可独立验收。一份代表性的合成旧配置即可复测 0.1.13 覆盖到 0.1.14，检查原录音热键、识别设置、语音输入和回退入口。多份真实配置与跨到 0.2.0 的完整实装验证按 ADR 027 在放量前完成。

1. 经确认后，从带旧通道护栏的维护分支推送稳定版 Tag。`Release Windows` 先创建 Draft，构建并上传安装包、更新签名、清单与 SHA-256，随后发布 GitHub Release。
2. `Publish Release to Tencent COS` 上传版本化安装包、签名和校验文件，最后更新 COS `releases/latest.json`。
3. 下载并核对 SHA-256，做一次安装包回归。确认 COS 和 GitHub 固定地址都返回该 0.1.x 版本，客户端能查到维护版说明。

### 0.2.0 过渡版

1. 先完成 [ADR 027](adr/027-0.2-upgrade-transition.md) 的验证并取得玩家确认。当前代码已是 0.2.0，日志也已有这一节。确认发布时把待发布条目合入现有小节并更新日期，保留回退链接。确认默认分支的手动 COS 发布入口也有旧通道护栏，再推送 Tag。0.2 的 Tag 工作流只生成草稿和签名制品，公开 GitHub Release 时显式设置 `make_latest: "false"`，旧通道继续使用自己的 `latest.json`。
2. 核对 GitHub 草稿的 SHA-256 和更新签名。先将版本化安装包、签名和校验文件上传 COS，再验证公开包 URL 与下载校验和。
3. 用 `node scripts/stage-transition-update.mjs <签名清单> <输出目录> <确认的比例>` 离线生成 `latest-0.2.json`。它带短版更新说明。收到该档放量批准后，才更新 COS 的专用地址，并为清单设置 `Cache-Control: no-store, no-cache, must-revalidate`。
4. 用无缓存请求分别检查 COS 专用清单、COS 旧清单与 GitHub 旧清单。两个固定旧地址须继续提供 0.1.x。用 0.1.14 检查到 0.2.0 的弹窗说明，确认完整日志指向旧版下载，且 0.1.14 关于页的回退链接可用。

## 签名与更新策略

- 当前直接分发渠道为腾讯云 COS（中国大陆首选）与 GitHub Release（兜底）+ NSIS
  （按用户安装，不要求管理员权限）。
- Windows 代码签名证书接入后再启用签名门禁；未签名构建可能触发 SmartScreen。
- 0.1.x 先读 COS `releases/latest.json`，GitHub 固定地址为兜底。0.1.14 在维护版之后检查 COS 专用通道并在本机判断灰度，0.2.0 使用专用通道。只有稳定 0.1.x 允许自动更新旧通道。RC 保留为草稿，不进入稳定更新。已发布版本拒绝由 Tag 工作流重建，COS 版本化对象也禁止覆盖。
- COS 发版子账号的对象读写只限 `kotone-1258931230/releases/*`；另有 `coscli cp`
  所需的桶级元数据权限 `HeadBucket / GetBucket / ListMultipartUploads`。仓库 Secrets 为
  `TENCENT_COS_SECRET_ID` / `TENCENT_COS_SECRET_KEY`；不授予删除对象或修改桶配置权限。
- updater 私钥与密码只保管在 GitHub Secrets（`TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）；公钥写在 `tauri.conf.json`。
  **私钥或密码丢失 = 更新通道报废**（已装客户端将永远无法校验后续更新），
  请离线备份；轮换密钥意味着所有用户必须手动重装。
