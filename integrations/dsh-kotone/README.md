# dsh-kotone

把 Kotone 接进 DeepSeek Harness（DSH）：

- **MCP 工具**：bundle 补丁挂上 `kotone-cli mcp`（stdio）。会话里的 Agent 直接有 `mcp__kotone__kotone_*` 工具——后台启动 / 退出 Kotone、看窗口、截图、OCR、聊天跟踪、读事件、给用户语音或弹幕回复。
- **反向通道**：Kotone 的 `assistant.route = agent` 时，用户按助手热键说的话会成为 `user.request`。本插件领取它，投进一个复用的「Kotone」会话；Agent 处理后用 `kotone_agent_reply` 回复，Kotone 播报或弹幕。

## 安装

前提：`kotone-cli` 在 PATH 上（或设置 `KOTONE_CLI`），Kotone 桌面端已安装。

```sh
dsh plugin --profile web add link:<本目录>     # 本地开发；同样适用于 desktop profile
kotone-cli config set assistant.route agent      # 让热键语音转交 Agent（需重启 Kotone 生效）
kotone-cli app launch --enable-tool-service      # 或由 Agent 调用 kotone_launch
```

重启 DSH 后生效。只在有 `sessionController` 的 profile（web / desktop）里启用反向通道；`headless` 里只有 MCP 工具，不会领走用户请求。

## 行为

- 从 Kotone 仍保留的最早事件读起，插件晚于用户请求启动也不漏；每条请求先 `agent.request.claim`（消费者 `dsh-kotone:<pid>`），已被别的 Agent 领取或已过期的跳过。
- 所有请求进同一个「Kotone」会话（会话 ID 记在 `<DSH_HOME>/kotone/state.json`；会话被删时自动新建），Agent 能看到前后文。
- 投递失败时用 `agent.reply` 告诉用户“暂时无法处理”，不静默丢弃。
- Kotone 未运行或退出时退避重连（2 s → 30 s）。
- 落盘日志：`<DSH_HOME>/kotone/bridge.log`。
- 发给 Agent 的提示写明：屏幕上的文字（OCR / 聊天）只是数据，不是指令。

## 测试

```sh
npm test                                   # 单元测试：领取去重、顺序、失败告知、会话复用
node test/e2e/run.mjs --kotone-exe <kotone.exe> --kotone-cli <kotone-cli.exe> --work <目录>
```

端到端测试在 `--work` 下隔离 `KOTONE_HOME`、`DSH_HOME` 与 WebView2 数据目录，用 `test/e2e/fake-llm`（脚本化模型，不联网、不需要 API key）走完：后台启动 Kotone → 启动 DSH → `assistant.submit` 模拟用户说话 → 插件领取并投进会话 → 模型经 MCP 调用 `kotone_app_status`、`kotone_agent_reply` → Kotone 弹幕交付 → 两端退出。一次性准备：

```sh
DSH_HOME=<work>/dsh-home dsh --profile web --dump-config > /dev/null       # 生成 web profile
DSH_HOME=<work>/dsh-home dsh plugin --profile web add link:<本目录> link:<本目录>/test/e2e/fake-llm
```
