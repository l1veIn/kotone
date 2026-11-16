# 任务：前端重构（shadcn-svelte）与「琴音」聊天界面（2026-11-16）

给接手前端重构的 Agent。方向见 [ADR 016](../adr/016-ui-foundation-shadcn.md)；内置 Agent 的背景见 [ADR 014](../adr/014-game-agent-harness.md)、[ADR 015](../adr/015-agent-scope-and-stack.md)、[Agent 交接](2026-11-16-agent-handoff.md)。

## 0. 开工前

- **分支与分工**：用户要求只在 `C:\Users\A\Desktop\kotone` 工作、不建额外 worktree，所以前端重构**直接在 `claude/kotone-agent` 上做**，与同时进行的 Agent 后端（第 5 步）**按文件分工**：
  - 前端只改 `apps/desktop/src/**`、`apps/desktop/e2e/**`、`apps/desktop/package.json`、前端锁文件、前端配置（`vite.config.ts`、`svelte.config.js`、`tsconfig.json`、`components.json`）、`THIRD_PARTY_NOTICES.md` 与新建的来源记录文档；
  - **不改** Rust 代码、`Cargo.*`、`docs/adr/*` 与其他已有文档；需要后端接口时在汇报里提出；
  - 不切换分支、不 `stash` / `reset` / `checkout -- <别人的文件>`；只暂存自己的文件；遇到 `index.lock` 稍等重试；
  - 提交前看 `git diff --cached --stat`，确认只有自己的文件。
- **已定**：外观参照 RambleDesk 支持切换主题、字体大小等；聊天组件暂按“复制 + 记录来源”（见 ADR 016）。
- **约束**（同 [Agent 交接 §6](2026-11-16-agent-handoff.md)）：
  - 每个提交都能独立构建；只暂存自己的改动，不用 `git add .`；`git mv` 会立即进暂存区，提交前看 `git diff --cached --stat`；
  - 不推送、不合并、不发布，除非用户明确说；
  - 不碰别人的未跟踪目录（`.tmp-analysis/`、`.rambledesk/`、`.reasonix/`、`.codegraph/`、`.archify/`、`bash.exe.stackdump`）和储藏；
  - 测试不读真实 `~/.kotone`、凭据库、付费接口；不动 `D:\琴音` 的 0.1.13；
  - 配置兼容：已有字段的值永远不改，新能力用新字段（`docs/development.md`）；
  - 汇报：每一步用简单中文说做了什么、怎么验证的、什么没验证。

## 1. 现状

- **前端**：`apps/desktop`，Svelte 5（runes）+ Tailwind 4 + Vite 6 + TypeScript 5；无组件库、无图标库。
  - 三个窗口入口：设置窗口 `src/routes/settings/`（主要工作量）、悬浮窗 `src/routes/overlay/`、弹幕窗 `src/routes/danmaku/`。**悬浮窗与弹幕窗不改**（透明穿透窗口，保持轻量）。
  - 设置页：`src/routes/settings/Settings.svelte`（左侧导航 `navItems`）+ `pages/*`（通用、文字处理、陪玩、自动化、游戏适配、历史记录、高级、关于）；公共样式 `pages/panel.css`；手写组件在 `src/lib/components/`（`Select`、`Toggle`、`Toasts`、`HotkeyField` 等）。
  - IPC：`src/lib/ipc.ts`、`src/lib/ipc/*`（含浏览器预览用的 `browserMock.ts`）、`src/lib/automation.ts`、`src/lib/assistant.ts`。浏览器里（`isTauri = false`）用预览数据，不调用任何真实功能。
- **检查**：
  - `pnpm -C apps/desktop check`（svelte-check，目前 0 错误）；
  - `pnpm -C apps/desktop test:e2e`（Playwright，19 个文件、58 个用例，`dev:web` 起在 127.0.0.1:17420，用浏览器预览数据与 `@tauri-apps/api/mocks` 模拟 IPC；写法参考 `e2e/assistant-voice.spec.ts`）；
  - 调试构建要嵌入前端：先 `pnpm build:web`，再 `cargo build -p kotone-tauri --features tauri/custom-protocol`。
- **参考实现**：RambleDesk，`C:\Users\A\Desktop\rambledesk\apps\desktop`：
  - shadcn-svelte 配置 `components.json`，基础组件 `src/lib/components/ui/`（alert、badge、button、collapsible、dialog、dropdown-menu、scroll-area、select、separator、skeleton、sonner、switch、tabs、tooltip），样式入口 `src/app.css`；
  - 聊天：`src/lib/agents/chat/`（`SessionTranscript`、`SessionTimeline`、`AgentTurn`、`ToolCallCard`、`ThinkingBlock`、`MessageMarkdown`、`TurnFooter`、`scroll-anchor.ts`、`timeline-window.ts`，说明见同目录 `README.md`）；输入器 `src/lib/agents/composer/`（Tiptap，含输入法与发送键处理）；
  - 来源与许可证：`docs/CODEG_PORTS.md`、`licenses/codeg-APACHE-2.0.txt`、`THIRD_PARTY_NOTICES.md`；
  - 注意：部分组件仍是 Svelte 4 写法（`export let`、`$:`），搬过来时按需改成 runes。

## 2. 第一步：底子

- 初始化 shadcn-svelte，加 bits-ui、tailwind-variants、tailwind-merge、clsx、`@lucide/svelte`；设计变量照 shadcn 命名；
- **外观系统**：参照 RambleDesk `apps/desktop/src/lib/appearance/`（`palettes.css`、`appearancePreferences.ts`、`appearanceRuntime.ts`、`AppearanceSettingsPanel.svelte`；来源见其 `docs/CODEG_PORTS.md`“外观配置”一行），支持切换配色主题、明暗、字体大小（或窗口缩放）、界面字体；Kotone 现在的深色 + 青 / 粉是默认主题。偏好先存在前端（读写失败要能回退默认），只作用于设置窗口；需要随配置存盘时在汇报里提出，由后端加新字段；外观设置入口放在「通用」页；
- 引入第二、三步要用的基础组件；
- **现有页面不改动行为**，`pnpm check` 与全部 Playwright 用例照常通过；
- 验收：依赖与版本记录在提交说明里；`pnpm build:web` 成功；比较构建前后的前端产物体积并写进汇报。

## 3. 第二步：「琴音」聊天页

### 3.1 位置与入口（ADR 014“已确认”）

- 左侧导航「陪玩」改名「琴音」；页面两个分页：**聊天**（默认）与**设置**（现在「陪玩设置」里的 AI 连接、能力自检、热键、快捷指令等，再加 §3.4 的 Agent 设置）。
- 游戏内语音与聊天窗口是**同一段对话**（后端已实现“当前对话”共用）。
- 空白对话显示**起手提示**：做一个自动化、看看队友在说什么、设个提醒、我有哪些自动化（文案草稿在 `src/lib/agent.ts` 的 `starterPrompts`）。
- 现有测试依赖 `assistant-nav-tasks` / `assistant-nav-settings` / `assistant-tasks` / `assistant-request` / 语音按钮 `assistant-voice-button` 等标记；可以改，但要同步改测试。

### 3.2 后端接口（已实现，`apps/desktop/src-tauri/src/agent.rs`）

前端类型草稿：`apps/desktop/src/lib/agent.ts`（与下文一致；可按需重写）。

**命令**

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `agent_send` | `text` | `{status: "started" \| "steered", conversationId}`；正在回答时新消息算**插话** |
| `agent_stop` | — | 是否停了（同时清掉未处理的插话 / 排队） |
| `agent_answer` | `askId`, `answer: "allow-once" \| "allow-for-conversation" \| "deny"` | 是否送达 |
| `agent_state` | — | `{current, running, conversations: [{id, title, createdAtMs, updatedAtMs, messageCount}], notReadyReason, permissions}` |
| `agent_conversation` | `id` | 整段对话 `{id, title, messages, …}` 或 null |
| `agent_new_conversation` | — | 下一条消息开始新对话（正在回答时报错） |
| `agent_select_conversation` | `id` | 切换当前对话（正在回答时报错） |
| `agent_delete_conversation` | `id` | 删除（正在进行的不能删） |

**事件**

- `kotone://agent-event`：`{conversationId, event}`，`event.kind`：
  - `step {step}`：第几次调用模型；
  - `chunk {text}`：回答的流式片段；`thought {text}`：思考片段；
  - `toolCall {call: {id, name, arguments}}`：模型发起工具调用（状态 pending）；
  - `toolCallUpdate {callId, status: in_progress | completed | failed, content}`；
  - `appended {message}`：一条消息已写进对话并存盘（用户、模型、工具结果、状态说明）；
  - `finished {stop, acpStopReason}`：这一轮结束（`stop.kind`：completed / truncated / step-limit / time-limit / cancelled / failed）；
  - `idle`：没有待处理的消息了。
- `kotone://agent-ask`：`{askId, conversationId, request: {callId, tool, title, reason, arguments}}`，需要用户确认；按钮放在 `callId` 对应的工具卡片上。
- `kotone://agent-ask-resolved`：`{askId, conversationId, answer}`（回答了、停止了或超时）。

**消息格式**（存盘格式，字段 camelCase）

- `{role: "user", text, attachments?, source?, atMs}`：`source` 为 `chat`（聊天窗口）、`assistant-text`（旧文字入口）、`voice` / `quick-command` / `tool`（游戏内）、`automation`（**系统消息**：由对话发起的自动化结束后送回的结果，不是玩家说的话，界面应显示成系统卡片而不是用户气泡）。`attachments` 里的 `data` 是附带的数据（如游戏内按热键时识别的屏幕文字），`image` 存盘后只留标签；
- `{role: "assistant", text, toolCalls?, reasoning?, usage?, atMs}`：`usage` 为本次调用的 token 用量（可能没有）；
- `{role: "tool", callId, name, content, isError, untrusted, atMs}`：`untrusted` 表示内容来自屏幕 / 游戏聊天；
- `{role: "notice", text, atMs}`：状态说明（已停止、出错……），不发给模型。

### 3.3 界面要点

- 消息流：用户气泡、回答（Markdown，流式）、思考折叠、工具卡片（中文名见 `toolLabel`；参数与结果默认折叠；状态 pending / 执行中 / 完成 / 失败；`saved.*` 是“运行已保存的自动化”）、状态说明、自动化结果卡片；
- 工具卡片上的确认按钮：允许 / 本段对话都允许 / 拒绝；
- 输入框：Enter 发送、Shift+Enter 换行、输入法组字时不发送；回答中显示“停止”；回答中仍可发送（插话）；
- 语音输入按钮（现在的 `AssistantVoiceInput`）放进输入框；
- 对话列表：新对话、切换、删除；
- 没有就绪时（`notReadyReason`）给出原因和“前往设置”；
- 每轮页脚显示 token 用量（有就显示，没有不显示，不估算）；
- 浏览器预览：给一段示例对话（`agent.ts` 里有草稿），发送时明确提示“仅预览”，不发出任何外部请求。

### 3.4 Agent 设置（设置分页）

`config.json` 的 `agent` 段（`crates/kotone-core/src/agent.rs`）：

- `permissions.mode`：`default` / `allow-all` / `read-only`；`permissions.rules`：按工具名的 `allow` / `ask` / `deny`；
- `hiddenAutomations`：不交给 AI 的已保存自动化 id（默认全部交给，最多 30 个）。开关建议放在「自动化 → 已保存」每一项上；
- `inGame`：陪玩热键与快捷指令交给琴音（默认开；关闭时用旧的单次陪玩）。

修改方式：
- 权限：`agent_set_permissions({permissions: {mode, rules}})`，整体替换（设置补丁是递归合并的，删不掉单条规则，所以不要用 `update_settings` 改 `rules`）；
- 隐藏某个自动化：`agent_set_automation_hidden({id, hidden})`，返回新的隐藏列表；
- `inGame`：`update_settings({agent: {inGame}})`。

### 3.5 验收

- `pnpm check` 0 错误；新增聊天页的 Playwright 用例：用 IPC 模拟发出 `agent-event` / `agent-ask`，验证流式文字、工具卡片状态、确认按钮调用 `agent_answer` 的参数、停止、插话、起手提示、对话列表；原有用例全部更新并通过；
- 来源记录与许可证文件就位；
- 隔离环境下真机看一次（`KOTONE_HOME` 与 `WEBVIEW2_USER_DATA_FOLDER` 放在 `target/` 下；脚本见 `scripts/manual-e2e/`）。没有真实 AI 连接时，至少确认未就绪提示、对话列表与预览正确；有用户同意的连接时再试一次完整对话。

## 4. 第三步：其他页面逐页迁移

（**先不做**：第一、二步完成并汇报、用户看过之后再开始。）

- 一页一个（或几个）提交，行为不变，测试同步更新；
- 手写组件（`Select`、`Toggle`、`Toasts` 等）换成 shadcn 对应组件后删除旧组件；`panel.css` 随最后一页迁完删除；
- 热键录制（`HotkeyField`、`hotkeyCapture.ts`）、首次启动向导、更新弹窗这些交互细节逐项对照旧行为，Playwright 已覆盖的不能回退。

## 5. 不在本任务内

- 后端的游戏内语音确认、切换到游戏窗口等（ADR 014 第 4、5 步的剩余部分，由 Agent 分支继续）；
- 悬浮窗与弹幕窗；
- 多语言。
