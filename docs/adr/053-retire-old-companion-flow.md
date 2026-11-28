# ADR 053：旧的单次陪玩退掉；`companion.type` 不再有

状态：Accepted（2026-11-28）/ 已实现（单元测试、浏览器预览的端到端测试），待真机。

修订：[ADR 014](014-game-agent-harness.md) §4（`agent.inGame`）、[ADR 017](017-voice-wake.md)、[ADR 023](023-ui-toward-companion-agent.md)、[ADR 032](032-game-chat-send-reliability.md) 实施记录第 13 条、[ADR 040](040-hotkeys-belong-to-core.md)（快捷指令经 `companion.ask` 触发）、[ADR 041](041-send-to-game.md)（入口的用户）；[docs/product-evolution-game-tools.md](../product-evolution-game-tools.md) §10 那条流程到此为止。

## 背景

玩家：

> “旧流程可以去掉了”

0.1.x 末期的“Kotone 陪玩”是一条单次流程：按陪玩热键说一句（或按快捷消息热键）→ 截前台窗口、OCR、聊天跟踪、profile 说明 → 一次大模型调用（`kotone_ai::companion`，受限工具 `voice_reminder` / `danmaku_reminder` / `type_message` / `answer` / `report_chat_region`）→ 提醒、帮忙打字、回答三选一，顺带记住聊天框的位置。

ADR 014 §4 以后呼叫默认交给她（`agent.inGame = true`），旧流程只在两处还活着：

- 「琴音 → 权限」里的「通话交给她」关掉时（`agent.inGame = false`），呼叫、唤醒词、快捷指令、`assistant.submit` 走它；
- 选了“转交外部 Agent”（`assistant.route = agent`）时，快捷指令走它（呼叫转交了，快捷指令却没有——一条没人想要的岔路）；

外加两个工具：`companion.ask`（跑一次整条流程，结果是 `companion.finished` / `companion.failed` 事件）和 `companion.type`（往前台游戏聊天里打一行）。往游戏里说话从 ADR 041 起只有一个入口 `kotone_core::send_to_game::GameChat`，`companion.type` 早就只是 `game_chat.send` 的一行版（`game_chat::type_into_chat`，带着旧的结果字段）。

## 决定

### 1. 去掉的

- 旧流程本身：桌面端 `companion.rs` 里的规划和执行（`handle_assistant`、`run`、`plan`、`Outcome`、“同一时间一条”的锁、`learn_chat_region`、`quick_message_text`、请求来源 `Source`）；`kotone-ai/src/companion.rs`（提示词、受限工具、`parse_plan`）。
- `agent.inGame` 和它的开关「通话交给她」：呼叫、唤醒词、快捷指令、`assistant.submit` 一律交给她；`assistant.route = agent` 时一律原文转交外部 Agent（`user.request`）。快捷指令在转交时的来源是 `assistant-text`。
- 工具 `companion.ask`（连同 `companion.finished` / `companion.failed` 事件）和 `companion.type`；`game_chat::type_into_chat`。往游戏聊天里发话只剩 `game_chat.send`（她和外部 Agent）、语音输入、自动化的 `chat()`，都经同一个入口。

### 2. 留下的（旧流程用过、现在别人在用）

- `companion.read_chat`（她看游戏聊天：截图 + OCR + 聊天跟踪，不调模型）和语音回合开口时的截图（`speech_screen`）：读聊天那半段原样留下，结果的字段不变。
- `companion.status`（设置页「看游戏聊天」和聊天页“准备好了吗”的文字识别状态）、`companion.check` 和设置页的测试连接：测试连接挪到 `kotone_ai::connection_check`，服务商没有模型列表接口时退回的那次请求改成一句不带工具的短对话。
- `assistant.companion.autoSend`（她用 `game_chat.send` 时不写 `send` 就按它）、`assistant.companion.quickMessages`（热键表的镜像，ADR 040）、`Speaker::Companion`（她在入口里的身份）、`assistant.route` 的旧值 `reminders`（就是“交给她”，旧版本能读）。
- 游戏配置的 `companionNotes`（她的系统提示里用）和已经记住的 `chatRegion`：照旧只识别那块；以后没有谁再记新的位置（那是规划顺带回报的），「重新整窗识别」按钮还在。
- `AssistantPlan.reminders`：编排器提交提醒的那条路还在，桌面端现在总是给空的（不顺手改编排器）。

### 3. 旧配置

`agent.inGame` 从 `AgentConfig` 里去掉。`AgentConfig` / `Settings` 没有 `deny_unknown_fields`，旧配置里的 `"inGame": false` 读进来直接忽略（单元测试 `unknown_values_fall_back_instead_of_failing`）。之后保存不再写这一项；旧版本读到缺这一项按默认 `true`，也就是“交给她”。

## 谁失去什么

- **玩家**：关着「通话交给她」的人（默认是开的），呼叫以后不再是“一次调用三选一”，而是她来处理——她能看聊天（`companion.read_chat`）、发到游戏（`game_chat.send`）、设提醒（`reminder.create`），能做的是旧流程的超集。选了“转交外部 Agent”的人，快捷指令现在也转交外部 Agent，不再偷偷跑旧流程。聊天框位置不再自动学，已记住的照用。
- **外部 Agent（单向变化）**：`companion.ask`、`companion.type` 不再在工具清单里，调用就是未知工具，没有别名；`companion.finished` / `companion.failed` 不再发。替玩家发话用 `game_chat.send {text, send?, channel?}`（结果里 `typed` / `sent` / `profile` / `receipt` 这几个旧字段照旧在），要她处理一句话用 `assistant.submit`。`companion.status`、`companion.read_chat`、`companion.check` 不变。
- **内部**：`kotone_ai::companion`、`game_chat::type_into_chat`、`CompanionRuntime` 里的忙锁、`settings_guide` 的 `agent.inGame` 一条。

## 实现情况

- kotone-ai：删 `companion.rs`，加 `connection_check.rs`（测试连接两条单元测试，退回的那次请求不带工具）。
- kotone-core：`agent.rs`（去掉 `in_game`，旧配置照读）、`settings_guide.rs`、`assistant.rs` / `send_to_game.rs` / `hotkey_custom.rs` / `usage.rs` 的说明。
- kotone-tools：`catalog_host.rs` 去掉两个工具，`companion.status` / `companion.read_chat` 的说明不再提“陪玩热键流程”；kotone-agent：`catalog.rs` 的排除表、测试。
- 桌面端：`companion.rs` 只剩就绪检查、读聊天、快捷指令交给她、测试连接；`assistant.rs` 的 `RoutingHandler` 只分“她 / 外部 Agent”；`tool_config.rs`、`game_chat.rs`、`speech_screen.rs`。
- 前端：「琴音 → 权限」去掉开关；`contracts.ts`、`browserMock.ts` 去掉 `inGame`；「看游戏聊天」一段的说明改写（不再说“再设提醒、打字，或回答”）。端到端：`agent-chat.spec.ts`、`companion.spec.ts`。
- 文档与真机脚本：`docs/tool-service.md`、`integrations/skills/kotone/SKILL.md`、`PRIVACY.md`；`scripts/manual-e2e/` 的拒绝表去掉 `companion.type`，`input_gate_e2e.py` 改用 `game_chat.send`，`voice_input_real.py` 去掉 6c。
- 待真机：关过「通话交给她」的配置读进来，呼叫以后是她在答；选了转交外部 Agent 时按快捷指令，外部 Agent 收到 `user.request`（来源 `assistant-text`）；`input_gate_e2e.py` 用 `game_chat.send` 跑一遍。
