# 评估：用 Goose 作为琴音的 Agent 底座？（2026-11-18）

被评估的方案（玩家提出）：

> 用 Goose 作为 agent 底座，先以 goose-server sidecar 的形式接入，等 crate 化成熟后再考虑直接嵌入。Rust 核心继续作为唯一的 MCP server。会话层级映射到 Goose 的用户会话和子 agent 会话上。对局复盘先用“对局结束时由 Rust 核心主动触发一次摘要”来实现，不必一开始就改 Goose 的压缩逻辑。

## 我们现在有什么

- Agent 相关代码约 9300 行（含测试）：`kotone-ai/src/agent`（循环、存储、模型接口）约 3300 行，`kotone-agent`（工具目录、权限、提示词、工作区）约 3500 行，桌面端会话运行时与相关模块约 2600 行。
- rig 只在 `kotone-ai/src/agent/model.rs` 里当“调模型的接口层”（OpenAI 兼容格式、流式）；循环、会话、子会话、权限、提示词都是自己写的。
- **还没有上下文压缩**：对话超过条数上限时直接丢最早的几轮（`store.rs` 的 `trim_messages`）。
- 已经做好、并且是陪玩特有的：游戏内快速模式（预先读屏、常用工具、6 步 / 60 秒）、插话（正在回答时再说一句）、语音追问与语音确认、按工具的允许 / 询问 / 禁止、每个游戏“让琴音操作”的同意、Esc 停止、屏幕文字按不可信数据处理、后台子会话（同传整局在听，听到的话进子会话，子会话只能给父会话发消息，结束时由代码停掉它名下的监听）、工作区（SOUL / 记忆 / 游戏记忆 / 技能）。

## Goose 的现状（查自仓库与官方文档，2026-11-18）

- Apache-2.0，现由 Linux 基金会 AAIF 托管；最新 v1.60.1（11-18），近 5 周发了约 10 个版本。
- **已经没有 `goose-server` / `goosed`**：服务端现在是 `goose serve`，说 ACP（Agent Client Protocol，JSON-RPC），走 HTTP / WebSocket（`/acp`），或者 `goose acp` 走标准输入输出。流程：`initialize` → `session/new`（可带 `mcpServers`）→ `session/prompt`，更新以 `session/update` 推回；权限用 `session/request_permission` 问客户端。
- **crate 化**：工作区里的 `goose`、`goose-agent` 等用路径依赖，crates.io 上是否发布没能确认；`goose-sdk` 是 `0.1.0-alpha.14`。官方没有说服务端接口稳定；近期版本删掉了后台子 agent、orchestrator、旧的 agent 循环。
- **子 agent**：`delegate` 工具，前台执行，默认 5 分钟超时、最多 5 个、不能再派子 agent；**1.55 起去掉了后台子 agent**；在“手动批准 / 智能批准”模式下**子 agent 不可用**。
- **上下文压缩**：做得成熟——到上下文的 80%（或 22.5 万 token）自动摘要，摘要提示词是可编辑的 `compaction.md`；另有截断、清空等策略，工具输出过长也会被摘要。
- 权限模式只有三档（全自动 / 手动批准 / 智能批准，且只针对“写”类工具），没有按工具的规则。
- 模型：DeepSeek 没有单独的条目，走 OpenAI 兼容地址；Key 从环境变量或系统钥匙串读，**没有找到“每次请求由宿主传 Key”的方式**。
- Windows：桌面版有 Windows 安装包；单独的 `goose serve` Windows 二进制在发布资产里没看到。CLI 压缩包约 55 MB。
- 工具结果里的图片：发布说明里有相关修复，文档没写，未确认。

## 逐条评估

| 方案里的说法 | 评估 |
| --- | --- |
| 以 goose-server sidecar 接入 | 前提已经变了：现在是 `goose serve` + ACP。sidecar 本身可行（Goose 桌面版就是这么做的），但要自己写 ACP 客户端、管进程生死、把 Key 同步进 Goose 的配置 / 钥匙串（我们的 Key 在自己的凭据库里），安装包多 50 MB 左右。 |
| 等 crate 化成熟再嵌入 | 近期看不到：没发布到 crates.io、SDK 是 alpha、接口每周在变。 |
| Rust 核心作为唯一 MCP server | 可行，而且我们已经是了（`kotone.exe` 兼任 MCP 服务器，工具服务 80 多个工具）。`session/new` 可以按会话带 MCP 服务器。 |
| 会话层级映射到 Goose 的用户会话和子 agent | **映射不上最关键的一块**：同传要整局在后台听、不断收到“听到的话”、再转告父会话；Goose 已经去掉后台子 agent，子 agent 默认 5 分钟超时，并且在需要询问权限的模式下不能用。只能把同传做成另一个 Goose 用户会话、由我们自己来调度——那就等于把我们的会话层重写一遍放在 Goose 外面。 |
| 对局复盘由 Rust 核心触发摘要 | 好主意，**和用不用 Goose 无关**：我们自己的运行时也能这么做（游戏进程退出 → 用这局的对话生成复盘 → 写进游戏记忆）。 |

还有几处会丢掉或要重做的：插话（ACP 只有新一轮 prompt / 取消）、游戏内快速模式的步数和工具子集、语音确认（要把 `request_permission` 接到语音上）、按工具的权限规则（要挪到我们的 MCP 服务端自己拦）、每段对话开头注入 SOUL / 记忆 / 游戏记忆（Goose 的系统提示是全局的提示文件）。

## 结论

1. **现在不换底座。** 自己写的部分已经写完、测过，并且大多是陪玩特有、Goose 也没有的；换过去的工作量（ACP 客户端、权限与语音的桥接、Key 同步、sidecar 打包、把子会话重新做在外面）比继续自己写剩下的部分大，还要追一个每周在变的 alpha 接口。
2. **真正还缺的是上下文压缩——照搬 Goose 的做法，不自己发明**：到上下文的 80% 时把较早的轮次用同一个模型摘要成一段（提示词参考 Goose 的 `compaction.md`，Apache-2.0，保留声明）；过长的工具输出先截断 / 摘要；最近几轮原样保留。预计几百行，加测试。
3. **对局复盘照方案里的想法做**：Rust 核心在游戏进程退出时触发一次摘要，写进这个游戏的记忆（ADR 022 的游戏记忆）。
4. **给以后留门**：把“琴音的大脑”收在一个接口后面（发消息、收事件、回答询问、停止），事件格式已经是 ACP 风格。以后 Goose 的 SDK 稳定、`goose serve` 有 Windows 版时，可以先做成“可选的外部 Agent（ACP）”，内置运行时继续是默认。

## 来源

- 仓库与发布：https://github.com/aaif-goose/goose 、https://github.com/aaif-goose/goose/releases
- ACP 接入与服务端：https://goose-docs.ai/docs/gdk/acp/ 、https://goose-docs.ai/docs/guides/remote-goose-server
- 子 agent：https://goose-docs.ai/docs/guides/context-engineering/subagents
- 上下文管理：https://goose-docs.ai/docs/guides/sessions/smart-context-management
- 权限：https://goose-docs.ai/docs/guides/managing-tools/goose-permissions
- 配置与环境变量：https://goose-docs.ai/docs/guides/config-files 、https://goose-docs.ai/docs/guides/environment-variables
- 模型：https://goose-docs.ai/docs/getting-started/providers
