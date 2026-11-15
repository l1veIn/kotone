# Agent 底座第一批实现与验证记录（2026-11-15）

范围：[演进计划 §5.5](../product-evolution-game-tools.md) 的第一批“底座”——验证“Agent 从打开 exe 开始驱动 Kotone、并收到用户经 Kotone 说给它的话”是否可行。分支 `claude/game-tools-m2`（M2 之后）。现行用法与语义见 [工具服务手册 §10](../tool-service.md#10-应用生命周期事件与用户请求agent-底座)；本文只记录做了什么、怎样验证、哪些仍未验证。

按 2026-11-15 的决定，工具全部开放、不设授权分级（演进计划 §5.5.1 已改写）。

## 1. 交付内容

| 部分 | 实现 |
| --- | --- |
| 数据目录 | `kotone-core::settings::kotone_dir()` 读取 `KOTONE_HOME`；桌面端、CLI、工具服务描述文件目录随之切换 |
| 助手去向 | `AssistantConfig.route`（`reminders` 默认 / `agent`）；CLI `config set assistant.route`；前端契约与浏览器 mock 同步 |
| 事件总线 | `kotone-tools::events`：有界保留（1000）、单调 `seq` + 全局 `id`、游标 `ev1.<实例随机数>.<序号>`、`Invalid/Stale/Expired` 三类游标错误、`wait_after` 长轮询唤醒、关闭即唤醒；`GET /v1/events` 与 `event.read`（不占执行名额，等待者上限 8） |
| 用户请求 | `kotone-tools::agent_requests`：pending → claimed → replied / expired，TTL 10 分钟、最多 64 条、领取排他（同一领取者幂等）、维护任务按时过期并发布 `agent.request.expired` |
| 宿主端口 | `kotone-tools::host::AppHost`（`app_info`、`runtime_status/start/stop`、`request_quit`、`deliver`、`assistant_submit`、`can_deliver`）；`NoAppHost` 让纯测试服务明确报 `HOST_ACTION_FAILED`；`FakeAppHost` 供测试 |
| 新工具（10 个） | `app.status`、`app.quit`、`runtime.status/start/stop`、`event.read`、`agent.requests`、`agent.request.claim`、`agent.reply`、`assistant.submit`；宿主动作与调用期限、服务关闭竞争（到期丢弃 future），状态变更在提交闸门后执行 |
| 回复语义 | `agent.reply` 默认受理即返回（`status=accepted`、`deliveryId`），后台交付，结果发 `agent.reply.delivered/failed`；`wait=true` 同步等待；已受理未完成的回复上限 8（`BUSY`）；服务关闭丢弃后台交付 |
| 桌面端 | `--background` 启动参数（不显示窗口；单实例回调遇到后台参数保持静默）；`RoutingHandler` 包住提醒助手，`route=agent` 时把原文发布为 `user.request`（按会话前缀区分 `assistant-voice` / `assistant-text` / `tool`）；`DesktopHost` 实现 `AppHost`，`deliver_text` 复用 `DesktopDelivery`（输出互斥、等麦克风空闲、弹幕、失败降级）；运行时相位变化发布 `runtime.phase` |
| 进程启动 | `kotone-platform-windows::detached::spawn_detached`：`CreateProcessW(bInheritHandles=FALSE, DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP)`，`DetachedChild::try_wait` |
| CLI | `app status/launch/quit`（`--exe` / `KOTONE_EXE` 显式路径不存在即报错，不悄悄换文件）；`tools events [--follow]`（JSON Lines）；`mcp`（stdio MCP：6 个静态工具 + 运行时映射的 `kotone_*` 工具，`kotone_launch` 后发 `tools/list_changed`） |
| DSH 插件 | `integrations/dsh-kotone`（2026-11-15 迁出为独立仓库 `dsh-game-communicate`，消费者前缀与状态目录随之改为 `dsh-game-communicate:<pid>`、`<DSH_HOME>/game-communicate/`）：bundle 补丁挂 `@deepseek-ai/dsh-mcp-client`（`kotone-cli mcp`）与反向通道插件；插件只依赖 `sessionController`（headless 等无会话 profile 不激活，不会领走请求）；经 `kotone-cli tools events --follow` 收 `user.request`、`agent.request.claim` 领取、投进复用的「Kotone」会话（ID 记在 `<DSH_HOME>/kotone/state.json`，被删则重建）；投递失败用 `agent.reply` 告知用户；退避重连；落盘日志 `<DSH_HOME>/kotone/bridge.log` |
| Skill | `integrations/skills/kotone/SKILL.md`：生命周期、调用、用户请求的领取与回复、看屏幕、事件、隔离 |
| 文档 | 工具服务手册（§1、§4 端点/上限/错误码、§5 工具表、新 §10、§11 CLI、§12 隐私、§13 实现位置）、CLI、术语、架构、隐私政策、README、演进计划 §5.5 |

没有新增外部依赖（`Cargo.lock` 不变）。

## 2. 验证

**自动化**（`TMP/TEMP` 指向 `target/tmp`）：

- `cargo test --workspace`：全部通过；其中 `kotone-tools` 86 项（新增 `tests_app` 8 项：长轮询唤醒、领取排他、游标 stale/expired 不跳过、请求过期、迟到交付被取消且不记回复、受理模式的后台交付与 `deliveryId` 事件、名额满 `BUSY`、服务关闭取消后台交付、无宿主时应用级工具明确失败），连续 3 次稳定。
- `kotone-cli` 新测试 `agent_cli`：MCP 服务器握手、未运行时的静态工具、运行时动态工具映射（inputSchema 均为 object）、事件读取、领取、回复、通用 `kotone_call`、未知方法 `-32601`。
- `kotone-platform-windows`：引号规则、脱离进程退出码、缺失可执行文件报错。
- `cargo clippy --workspace --all-targets -D warnings`、改动文件 `rustfmt --check`、`pnpm check`（svelte-check 0 错误）均通过。
- DSH 插件 `npm test`：4 项（提示内容、领取去重与顺序、投递失败告知用户、会话复用与删除后重建）。

**真机冒烟**（调试构建，嵌入前端；隔离 `KOTONE_HOME` 与 `WEBVIEW2_USER_DATA_FOLDER`）：

| 场景 | 结果 |
| --- | --- |
| `kotone-cli app launch`（输出被调用方捕获） | 0.6 s 返回，就绪 533–557 ms；`launch=background`，无窗口 |
| MCP 全链路（Python 驱动 `kotone-cli mcp`） | `kotone_launch` → `assistant.submit` → `user.request` 2 ms 内到达 → 领取、第二领取者 `AGENT_REQUEST_CLAIMED` → `agent.reply`（弹幕）2 ms 受理 → 18.7 s 后 `agent.reply.delivered`（弹幕滚完）→ 状态 `replied` → `kotone_quit` 411 ms，无残留进程 |
| DSH 端到端（`integrations/dsh-kotone/test/e2e/run.mjs`，web profile、脚本化模型） | 8/8 通过：用户请求被 `dsh-kotone:<pid>` 领取、投进「Kotone」会话、模型经 MCP 依次调用 `mcp__kotone__kotone_app_status` 与 `mcp__kotone__kotone_agent_reply`、Kotone 交付弹幕、请求 `replied`；两端退出无残留 |

## 3. 真机发现与处理

1. **句柄继承导致调用方挂起**：最初用 `std::process::Command` 启动 `kotone.exe`，调用方（Agent）捕获输出的管道被长驻进程继承，调用方永远等不到 EOF。改为 `spawn_detached`（不继承任何句柄）。
2. **弹幕回复阻塞 Agent**：同步 `agent.reply` 要等弹幕滚完（约 18.7 s），语音稍长就撞上 30 s 调用上限并在到期时被取消。改为默认受理即返回 + 回执事件，保留 `wait=true`。
3. **本机默认 WebView2 数据目录不可用**：`%LOCALAPPDATA%\com.l1vein.kotone\EBWebView` 上创建 WebView2 返回 `0x800700AA`（资源正在使用），没有任何进程持有该目录的文件、也没有相关 WebView2 进程；换成全新目录立即正常。结果是窗口（设置页、弹幕）全部创建失败，但工具服务照常工作——表现为弹幕回复超时。原因未查明，**未改动该目录**（其中有已安装 Kotone 的网页数据）。所有真机冒烟改用隔离的 `WEBVIEW2_USER_DATA_FOLDER`。已安装的 Kotone 在本机是否同样受影响**未验证**。
4. **DSH 首次 MCP 调用延迟约 30 s**：模型发出工具调用后，DSH 约 30 s 才把首个 `tools/call` 写到 `kotone-cli mcp` 的标准输入（stdio 抓包确认），之后的调用 3 ms 内往返；Kotone 侧每次都在毫秒级响应。原因在 DSH 内部，未查明；影响是每次 DSH 启动后的第一条用户请求回复慢 30 s。

## 4. 未验证 / 未做

- **助手热键语音路径**：真机只走了 `assistant.submit` 与文字入口共用的路径；按住助手热键说话 → `user.request` 未在真机上演示（路由逻辑与文字入口相同，有单元覆盖）。
- **语音播报回复**：真机只验证了弹幕；`speech`（系统声音 / Kokoro）及“等麦克风空闲”未在本批真机验证。
- **真实模型**：DSH 端到端使用脚本化模型；真实 DeepSeek 模型能否按提示可靠地领取上下文并调用 `kotone_agent_reply` 未验证（未调用付费 API）。
- **发布构建与 DSH desktop profile**：只用调试构建与 web profile。
- 演进计划第一批剩余：`app.restart`、`job.*`、`artifact.put` 与音频/文件产物、SSE、事件到 MCP 通知的映射、`user.request` 附截图产物；pi 插件。
- `assistant.route` 与 `toolService.enabled` 一样只在桌面端启动时读取；设置类工具（`settings.update` 等）在下一批。

## 5. 过程问题

格式化时把文件清单从 `git status` / `git ls-files --others` 推导，误把其他会话的未跟踪目录 `.tmp-analysis/` 下 6 个 `.rs` 文件一并交给 `rustfmt` 改写。已全部按字节恢复：4 个 `web_atoms` 生成文件来自 `target/` 中同一构建哈希的原件（格式化后逐字节相同以确认，连同修改时间一起恢复），`cubeb-probe/src/main.rs` 与 `log_wrap.rs` 来自 git 对象库中的同路径 blob（同样以格式化比对确认，`git hash-object` 校验）；这两个文件的修改时间无法恢复。之后只对显式列出的自有文件运行 `rustfmt`。
