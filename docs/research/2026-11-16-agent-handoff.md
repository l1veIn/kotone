# 交接：内置 Agent（2026-11-16）

上一轮（2026-11-15 ~ 16）把“陪玩”与“自动化”做完并合入 main；方向随后升级为 [ADR 014](../adr/014-game-agent-harness.md)：**Kotone 是游戏场景的 Agent 运行框架**，三层结构（基础能力 → 自动化 → Agent），陪玩改为有聊天界面、能多轮调用全部工具的内置 Agent。本文是下一轮的起点。依据见[游戏陪玩 Agent 现状调研](2026-11-16-game-agent-landscape.md)。

## 1. 现在的状态

- **main** 已包含下面全部工作（本地快进合并，**未推送**；`origin/claude/companion-scripts` 只推到 `4ebe578e`）。
- 下一轮**从 main 新建分支** `claude/kotone-agent`。
- 已有能力（都已真机测过，细节见 [自动化实现记录](2026-11-16-companion-scripts.md)）：
  - **陪玩**（§10，单次 AI 决策：提醒 / 帮忙打字 / 直接回答）、能力自检、快捷指令；
  - **统一模型清单**（ASR / OCR / TTS，`models.*`）、13 个 PP-OCRv5 包、在线 TTS；
  - **自动化**（ADR 013）：
    - 一个调度器管提醒与程序：延迟 / 定时、错过不补、最多一次、绑定目标窗口；
    - 已保存的自动化，带类型的入参（含 `image`）；
    - Luau 沙盒函数：键盘、鼠标、计时、亮度 / 取色、`find_color` / `find_image`（参照 AutoHotkey）/ `find_text`、OCR、语音 / 弹幕；
    - 工具 `automation.run / runs / stop / save / get / list / delete`；
  - **界面**：「自动化」页（进行中 / 已保存 / 记录、编辑器、倒数运行）；陪玩页的「跟琴音说」与快捷指令（可直接运行自动化）；「通用 → 声音与弹幕」；
  - **其他**：配置兼容（新能力只用新字段）、升级弹窗“不再提醒”、“脚本”改名“自动化”。
- 检查：`cargo test --workspace` 714 个、`clippy -D warnings`、`fmt --check`、`pnpm check`、Playwright 62 个，全部通过（合并前最后一次）。

## 2. 用户已定的事（照办）

- **定位与结构**：ADR 014。陪玩 = 内置 Agent；自动化是它的工具之一；Agent 能用全部工具；同一份工具实现同时给内置 Agent 和外部 Agent。
- **权限**：参照现有 Agent 运行框架（Claude Code、Codex、dsh）。每个工具一条“允许 / 询问 / 禁止”规则，加整体模式；默认规则由工具的副作用标记推出。外部 Agent 不加限制。
- **自动化**：
  - 用户可见处叫“自动化”，不叫“脚本”；
  - 借鉴云函数的分层（定义 / 触发 / 执行记录），但最多执行一次、绝不自动重试、错过不补、绑定窗口、人能随时停（ADR 013）；
  - 找色 / 找图参照 AutoHotkey 的 `PixelSearch` / `ImageSearch`。
- **配置兼容**：已有字段的值永远不改；新能力用新字段。0.1.13 读到旧字段里不认识的值会把整个配置当成损坏（规则写在 `docs/development.md`）。
- **其他**：
  - 帮忙打字默认自动发送；
  - 不接 Edge TTS；
  - 模拟键盘本身不算作弊（但要做“按游戏允许自动化输入”的开关）；
  - 升级提示可以“不再提醒”。

## 3. 待用户确认（新一轮开始时先问）

ADR 014 的“待定”一节。**2026-11-16 已确认：五项全部按下表建议执行**（ADR 014 已改为“已确认”）。

| 事项 | 建议 |
| --- | --- |
| 聊天界面放哪 | 「陪玩」页改为聊天窗口，导航名改为「琴音」，设置放到第二个分页 |
| 游戏内语音与聊天窗口是否同一段对话 | 同一段 |
| 空白页的起手提示 | 要 |
| 已保存的自动化给不给 AI 用 | 默认全部（最多 30 个），每个可单独关掉 |
| 帮忙打字的内容是否写进执行记录 | 写进去 |

## 4. 开发顺序（ADR 014）

1. **Agent 循环**（`kotone-ai`）：
   - 多步工具调用、流式输出、步数 / 时长上限、取消、对话存储；
   - 用假模型测试；不调真实付费接口。
2. **工具接入与权限**：
   - 进程内调用工具清单；工具分组，加“查找工具”能力；
   - 规则与模式；询问流程（窗口按钮 / 游戏内语音与弹幕）；
   - 原陪玩动作改成工具，新增内置动作“发到游戏聊天”，进入执行记录；
   - 已保存的自动化作为工具交给模型。
3. **聊天界面**：消息流、工具调用卡片、确认按钮、停止、起手提示、对话列表。
4. **游戏内入口**：
   - 陪玩热键与快捷指令里的“一句话”改走 Agent；
   - 快速模式：工具子集、步数上限、预先附上截图和识别文字。
5. **游戏场景工具**：打开 / 关闭游戏、切换窗口等；按游戏的“允许自动化输入”开关。
6. **文档、Skill、插件**：同步插件仓库 `C:\Users\A\Desktop\dsh-game-communicate` 的技能；它的现有技能还没提到 `automation.*`。

## 5. 代码入口

| 要做的事 | 入口 |
| --- | --- |
| 现在的单次规划（要被 Agent 循环取代） | `crates/kotone-ai/src/companion.rs`：`tools_schema`、`plan`、`parse_plan`、`self_check`、`CompanionAction` |
| OpenAI 兼容请求 | `crates/kotone-ai/src/transport.rs` `chat_json`；连接与密钥在 `connections.rs`、`secrets.rs` |
| 桌面端陪玩流程 | `apps/desktop/src-tauri/src/companion.rs`：`handle_assistant`、`run`、`quick`、`tell`、`ocr_backend` |
| 工具清单与说明 | `crates/kotone-tools/src/catalog.rs`（基础工具 `tool_specs`）、`catalog_host.rs`（桌面委托工具，`Def` 里有 `side_effects`） |
| 工具执行 | `crates/kotone-tools/src/service.rs` `ToolService::call` / `capabilities`；桌面委托工具在 `apps/desktop/src-tauri/src/tool_config.rs` |
| **注意** | `ToolService` 现在只在“工具服务”开启时随 HTTP 服务创建（`apps/desktop/src-tauri/src/tool_service.rs` `start`）。内置 Agent 需要它常驻：要把服务对象与对外监听拆开 |
| 自动化 | core：`crates/kotone-core/src/automation.rs`（调度器）、`automation/library.rs`（定义与入参）；引擎：`crates/kotone-script`；桌面：`apps/desktop/src-tauri/src/automation.rs`（执行器、`run_saved`、Tauri 命令） |
| 输出（语音 / 弹幕） | `apps/desktop/src-tauri/src/assistant.rs` `deliver_text` |
| 前端 | `apps/desktop/src/routes/settings/pages/`：`AssistantPage`、`assistant/*`、`AutomationPage`、`automation/*`、共用样式 `panel.css`；IPC 在 `src/lib/automation.ts`、`src/lib/assistant.ts`、`src/lib/ipc/*` |

## 6. 工作约束（照旧，另加本轮新增）

- **分支与提交**：
  - 只在本目录工作，从 main 新建分支，不建额外 worktree；
  - 只暂存自己的改动，不用 `git add .`；
  - **每个提交都要能独立构建**。注意 `git mv` 会立即进暂存区，提交时 `git commit` 带走的是整个暂存区；上一轮因此有一个提交在单独签出时前端编译不过，合并前已拆开重做；
  - 推送、合并、发布要用户明确说。
- **不要碰的**：别人的未跟踪目录 `.tmp-analysis/`、`.rambledesk/`、`.reasonix/`、`.codegraph/`、`.archify/`、`bash.exe.stackdump`；储藏 `codex-preserve-before-single-folder-consolidation`。
- **测试隔离**：
  - 测试不读真实 `~/.kotone`、凭据库、付费接口，不自动下载大模型；
  - 冒烟用隔离的 `KOTONE_HOME` 与 `WEBVIEW2_USER_DATA_FOLDER`（放在 `target/` 下）；
  - 不运行 `autostart.set` / `elevation.restart` / 真实安装器；
  - 不动 `D:\琴音` 的 0.1.13 与 WebView2 用户数据目录。
- **cargo**：
  - `TMP=TEMP=C:\Users\A\Desktop\kotone\target\tmp`，关闭沙箱；
  - `.cargo/config.toml` 的 `CXXFLAGS_x86_64_pc_windows_msvc=/MT` 不要动（Luau 要和 sherpa-onnx 的静态运行库一致；改了要 `cargo clean -p luau0-src -p mlua-sys -p mlua`）。
- **编辑**：
  - rustfmt 只对自己的文件（`cargo fmt --all -- --check` 可以用来确认整体）；
  - 含反斜杠或中文的复杂替换，用编辑工具或 `python -I` 脚本，按原文件的换行符写回。
- **前端**：
  - `$state` 里的对象是代理，`structuredClone` 会抛错，用 `$state.snapshot`；
  - 调试构建要嵌入前端：先 `pnpm build:web`，再 `cargo build -p kotone-tauri --features tauri/custom-protocol`。
- **真机测试**：用 `scripts/manual-e2e/`（见其 README），要点：
  - 仓库根目录带低完整性标签：exe 要复制到仓库外运行，否则模拟输入被静默丢弃；
  - Kotone 的热键钩子忽略模拟按键，热键只能人工按；
  - 隔离实例的设置窗口会抢前台，用 `fg.ps1` 切回测试窗口；
  - 用 UI 自动化可以点 WebView2 界面；
  - `python -I` 会忽略 `PYTHONIOENCODING`，要加 `-X utf8`。
- **汇报**：每一步用简单中文说做了什么、怎么验证的、什么没验证。

## 7. 已知未验证 / 不足

- 真实游戏里：反作弊、游戏内输入法、截图（独占全屏）、`SendInput` 鼠标（锁定鼠标、读原始输入的游戏）、多显示器与不同缩放下的坐标换算。
- 物理按键：停止自动化热键、快捷指令热键；托盘「停止所有自动化」的点击。
- 弹幕与语音的实际观感 / 听感。
- 播报类执行记录没有记来源定义（`automationId` 只在程序动作里）。
- 模板图片不会自动清理。
- Kotone 崩溃（不是正常退出）时，自动化按住的键不会松开。
- OCR 检测器最多放大 2 倍：改变了全部本地 OCR 的预处理，对真实游戏聊天截图的影响未验证。
