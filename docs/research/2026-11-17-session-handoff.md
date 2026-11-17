# 交接：琴音陪玩 0.2.0 分支与下一步 computer use（2026-11-17）

给下一个会话。先读这一份，再读 [ADR 019](../adr/019-game-computer-use.md)（下一步要做的事）。

## 1. 分支状态

- 分支 `claude/kotone-agent`，比 `main` 多 60 个提交，**已推送到 origin，没有合并、没有打标签**。
- 发布 0.2.0 只有在用户明确说“合并并发布”之后才做：快进合并到 main、推 main、打并推送 `v0.2.0` 标签（推标签会触发 GitHub Release、COS 同步和自动更新）。发布当天把 `CHANGELOG.md` 里 `## 0.2.0 — 2026-11-16` 的日期改成当天。
- 工作区里有不属于我们的东西，**不要动、不要提交**：`.archify/`、`.codegraph/`、`.rambledesk/`、`.reasonix/`、`.tmp-analysis/`、`bash.exe.stackdump`，以及只有换行符差异的 `apps/desktop/src-tauri/Cargo.toml`。还有一个 stash `codex-preserve-before-single-folder-consolidation`，也不要动。

## 2. 本轮完成的工作（都已推送）

按时间顺序，详细见各提交说明与 CHANGELOG 0.2.0 段。

| 提交 | 内容 |
|---|---|
| 721469a4 … 2de93eef | 29 家 AI 提供商预置；设置窗口 Harness 风格重做；“琴音陪玩”改名；琴音能配置 Kotone（设置说明、生效方式、`app.restart`） |
| 209c6a5b、cdd7167b、cb60e922、f5a464f3 | 语音唤醒（ADR 017，sherpa-onnx 关键词检测，“琴音琴音 / 你好琴音”）及卡死修复（悬浮窗在 `with_webview` 回调里发事件会锁死主线程） |
| bdde9cc8 | DeepSeek 语音合成，琴音用 DeepSeek 时默认用它 |
| 82fb2151、c40923d0 | 语音追问 `reply.ask`、语音确认权限；语音入口能查找工具 |
| 59c4ebc5 | 琴音的对话气泡（ADR 018）：右下角独立窗口，鼠标穿透、不抢焦点；语音会话不再用语音输入的胶囊悬浮窗；回答默认不走弹幕 |
| 72999419 | 语音入口与聊天窗口同一套工具（打开游戏、下载 / 启用模型变成常用工具）；输入框里切换权限（仅可查看 / 先问我 / 完全权限）和模型；唤醒应答预合成 + 裁静音（2 秒 → 0.5 秒） |
| 39f68cef | **去掉能力自检**：选好连接、有密钥就能用；“测试连接”照 DeepSeek Harness 读服务商的模型列表（`GET /models`，不花 token），没有列表接口时发一次短对话；模型菜单显示服务商真实的模型 |
| 068ccab8 | **检测已安装的游戏**（`kotone_platform_windows::games`，工具 `game.installed`）：Steam、英雄联盟国服 / Riot、米哈游、Epic、WeGame，只读注册表和清单；`game.launch` 没填启动路径时用检测结果，也能直接给 `installedId`；启动路径支持“带引号的程序 + 参数”并在程序目录运行；游戏适配页“自动检测” |
| （本提交） | ADR 019 computer use 设计 + 本交接文档 |

## 3. 下一步：computer use（ADR 019）

用户已确认方向，并要求**照搬开源项目、不自己造轮子**。ADR 019 写清了每个部分照抄谁：

- 动作清单：Anthropic `computer_toolset_20260801`；
- 批量执行与安全交互：dsh-computer-use-win；
- 截图编号：UFO² / OmniParser；
- 游戏循环与提示词：Cradle、GameVerse、UI-TARS。

实施分 7 步，见 ADR 019 末尾。另有 4 个问题等用户确认（竞技对局默认不动、停止键、步数预算、边缘发光提示）。**开工前先把这 4 个问题问用户，或按 ADR 里的推荐做并在汇报里说明。**

关键事实（已核实）：

- 琴音现在**没有直接的键鼠工具**，只能经 `automation.run` 写 Luau 程序。
- `apps/desktop/src-tauri/src/automation.rs` 的 `DesktopHost` 已有窗口绑定、窗口比例坐标、只截游戏客户区、前台等待、松开按住的键、纯色画面判独占全屏：抽出来复用。
- `crates/kotone-platform-windows/src/inject.rs` 的模拟输入 `dwExtraInfo` 是 0，要补标记。
- `hotkey_ll.rs` 的钩子已区分 `LLKHF_INJECTED` / `LLMHF_INJECTED`。
- DeepSeek 能看图；`kotone_ai::agent::Attachment::Image` 已支持图片附件。

## 4. 仍在等用户决定的事

- UI 的 5 个选择：侧栏对话列表、确认卡位置、默认页（推荐“琴音陪玩”）、发送按钮文字、760 px 抽屉断点。
- ADR 017 第 7 条“30 分钟没说话算新话题”（会改变热键的行为）。
- 插件仓库 `C:\Users\A\Desktop\dsh-game-communicate` 的 `claude/kotone-agent` 分支要不要合并、推送。
- 琴音默认权限：现在保持“先问我”；用户说过“权限直接给最高”，已改为在输入框一键切换。如果用户要默认就是完全权限，改 `kotone_core::agent::Mode` 的默认值。
- 对话气泡第二版（鼠标移上去可点、跟着语音逐句显示）。

## 5. 怎么测（踩过的坑都在这）

- **隔离**：测试用 `KOTONE_HOME=target/agent-smoke/home`、`WEBVIEW2_USER_DATA_FOLDER=target/agent-smoke/webview`。不碰真实的 `~/.kotone`、凭据库、付费接口，也不碰用户装的 0.1.13（`D:\琴音`）。可以只读用户的 `~/.kotone/kotone.log` 和对话记录来排查问题。
- **单实例**：用户自己的 Kotone（`pnpm dev`，`target/debug/kotone.exe`）开着时，测试实例起不来，exe 也被锁住无法重新编译。**不要杀用户的进程**，请用户关掉。
- **调试版要开发服务器**：`target/debug/kotone.exe` 的界面从 `http://127.0.0.1:17420` 加载，真机测界面前先在 `apps/desktop` 跑 `npx vite dev --port 17420 --strictPort`，测完关掉。
- **模拟 AI**：`target/agent-smoke/mock_openai.py`（端口 18765）。
  - `MOCK_SCRIPT="工具名:json|…"` 按脚本依次调用工具，用完给出最终回答；
  - `/v1/tools/tts` 模拟 DeepSeek 语音（`MOCK_TTS_DELAY_S` 控制延迟，首尾各带 0.5 秒静音）；
  - 请求记录在 `requests.jsonl`、`tts_requests.jsonl`。
- **虚拟麦克风**：VB-Audio Cable。往 “CABLE Input” 播放音频，Kotone 的 `audioDeviceId` 设成 “CABLE Output (VB-Audio Virtual Cable)”。
  - `target/agent-smoke/wake_e2e.py`：唤醒；
  - `target/agent-smoke/ask_e2e.py`：追问和语音确认；
  - `target/agent-smoke/bubble_e2e.py`：气泡的显示、穿透、焦点，带截图。
  - 这几个脚本的参数是 16 kHz 录音样本所在的目录（在会话的 scratchpad 里，scratchpad 被清空后要重新准备）。
- **屏幕缩放 150%**：用 Python 截屏和读窗口位置前，先调用 `SetProcessDpiAwarenessContext(-4)`，否则坐标会对不上。
- **WebView 调试**：启动前设 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`，再用 Playwright 的 `chromium.connectOverCDP` 连上，就能看页面内容、调用 `window.__TAURI_INTERNALS__.invoke`。
- **卡死排查**：用 cdb 非侵入地抓线程栈（见 memory 里的 kotone-work-discipline）。
- **Bash 的 heredoc 会吃掉反斜杠、把 `\x00` 变成真的 NUL 字节**：改带反斜杠的代码用 Edit 工具，或者把 Python 脚本写成文件再执行。
- **编译**：`export TMP/TEMP=<仓库>\target\tmp`，命令要关掉沙盒才能运行。
- **检查**（每次提交前跑）：
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked`
  - 在 `apps/desktop` 下：`pnpm check`、`npx playwright test`
  
  第一次跑 Playwright 冷启动时，偶尔会因为开发服务器预构建依赖超时，重跑即可。

## 6. 还没在真机上验证的

- 真的打开一个游戏（`game.launch`），包括国服 LoL 的管理员授权；
- Riot 外服、Epic、WeGame 其他游戏的检测（这台电脑没装）；
- 真实服务商的“测试连接”和模型列表；
- 真实 DeepSeek 语音的应答延迟；
- 对话气泡在多显示器、独占全屏和真实游戏画面下的表现。

## 7. 工作约定（用户定的，一直有效）

- 每个提交都能独立构建，只暂存自己的改动（提交时写明文件路径）；
- 分支可以推送，合并、打标签、发布要等用户明确说；
- 每一步用简单中文说清楚做了什么、怎么验证的、什么没验证；
- 只在这个目录工作，不开额外的 worktree；不编造、不记录 API key。
