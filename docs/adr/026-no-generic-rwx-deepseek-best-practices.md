# ADR 026：琴音不要通用读写执行；按 DeepSeek 官方最佳实践对齐，联网搜索走 DeepSeek 搜索接口

- 状态：Accepted（2026-11-18，实施在分支 `claude/kotone-agent`）
- 照搬：DeepSeek 官方指南「构建 Chatbot」及其子页（会话标题 `title`、用户元信息 `latest_reminder`、模型内容安全 `safety_check`、思考模式、上下文硬盘缓存、错误码与保活）与「网页搜索与抓取」接口（`/v1/tools/search`、`/v1/tools/fetch`），https://api-docs.deepseek.com/zh-cn/guides/build_a_chatbot
- 修订：[ADR 015](015-agent-scope-and-stack.md)（模型通信不再全部经 rig：DeepSeek 连接用自己的 Chat Completions 客户端）
- 被修订：第 1 条由 [ADR 030](030-subagent-shell-and-files.md) 修订（2026-11-20）——主会话仍然没有通用读写执行；子代理有，读过外面的东西就先问一句

## 问题

1. 通用 Agent（OpenClaw、Hermes、Goose、DeepSeek Harness、Claude Code）都有 read / write / bash。琴音要不要？
2. 搜索、读网页要不要？怎么做？
3. DeepSeek 是 Kotone 的主力服务商（一个 Key 就有对话、语音识别、语音合成、文字识别），它的官方指南里有一批 rig 的通用 OpenAI 格式表达不了的能力（`latest_reminder` 角色、`title`、`safety_check`、最后一块才报错、内容审核拒绝）。

## 决定

### 1. 琴音没有通用的 read / write / bash（陪玩特有，自己的判断）

从玩家要的事出发：读 = 看屏幕和聊天（`game.look`、读聊天）；写 = 记忆，以及在游戏自己的设置界面里改设置（`game.act`）；执行 = Luau 沙盒自动化、`game.launch`。玩家的“文件系统”就是屏幕。通用工具多出来的只有装 mod（不该做）和读配置 / 日志（很少要，真要时做成只读、限定目录的专用工具）。

不给的理由：

- 琴音同时有三样东西：读不可信内容（陌生人的游戏聊天、屏幕文字、网页）、机器上的私密数据（游戏账号、浏览器 Cookie、各家 Key）、往外发的渠道（`game_chat.send`、游戏语音）。再加 bash，聊天里一句注入就是在玩家电脑上执行任意命令。
- bash 绕过所有已有的防护：游戏不在前台不打字、每个游戏能不能让她动手、Esc 立刻停、每个工具单独的允许 / 询问 / 禁止。模型在 bash 上训练得多，有了它就会用 PowerShell 的 SendKeys 而不是 `game_chat.send`。
- 玩家判断不了 `Remove-Item …` 该不该允许，“先问你”会变成形式；碰游戏目录还可能被反作弊封号。
- 游戏里要一两句话、几秒内回答；“试命令、看输出、再试”的循环太慢。

需要通用能力的人用外部 Agent（Claude Code 等经 MCP 调 Kotone），由它们自己的权限体系管 bash。

### 2. 联网搜索：`web.search` / `web.fetch` 两个普通工具，接 DeepSeek 的搜索与抓取接口

- DeepSeek 指南写明：对话里的内置联网工具（`deepseek_web_search`）是给 Chatbot 用的；**Agent 或自己的程序要搜索、抓取，用「网页搜索与抓取」接口**。每 1000 次成功请求 5 元，失败、无结果不计费。
- `web.search`：返回标题、网址、站点、日期和一段写给模型看的摘要（接口的 `summary`，约 600–800 字），默认 5 条。`web.fetch`：读一页，返回 Markdown 正文（截到上限）。
- 网页内容一律当数据（和屏幕文字同一条规则）；`web.fetch` 只读这段对话里出现过的网址（搜索结果或玩家给的），模型不能自己拼网址把数据带出去。抓取在 DeepSeek 的服务器上做，实测读 `127.0.0.1` 返回 403，访问不到玩家的本机和局域网。
- **和语音识别、文字识别、语音合成并列**：统一模型清单多一类“联网搜索”（`category=search`，`search.activeModelId`），现在有“DeepSeek 联网搜索”和“不联网”两项；空 = 自动（有 DeepSeek 连接就用）。连接照 DeepSeek OCR 的写法绑定（`modelConfigs`，留空跟随琴音的 DeepSeek 连接）。工具名不带服务商，以后加别家只是多一个选项。
- 费用记进用量账本（“搜索”一类）。

没用内置联网工具：实测它只在思考模式下才能和我们自己的工具一起用（见下），文档写的是“暂不支持”，随时可能变；搜到什么我们看不见（没法列来源、控制长度、记账）；high 强度时模型有一次干脆不搜、说自己“没有联网能力”。

### 3. DeepSeek 连接用自己的 Chat Completions 客户端（`kotone-ai/src/agent/deepseek.rs`）

rig 的 OpenAI 兼容格式没有 `latest_reminder` 角色，读不到响应里的 `title`，也不认最后一块里的 `error`；它的传输层接口是 crate 内部的，包不了一层。DeepSeek 是主力服务商，这里自己写一个只认 DeepSeek 的客户端，其余服务商照旧走 rig。照官方指南：

- **`latest_reminder`**：每条玩家消息前加一条 `{"role": "latest_reminder", "content": "2026-11-18,星期三,,App,中文"}`（日期取那条消息的时间，历史里的照样带回，前缀不变）。几点几分、当前游戏、游戏语音状态仍在我们每轮的 `<kotone-context>` 里（官方字段只有日期）。
- **`title`**：一段对话还没有标题时请求带 `"title": {"enabled": true}`，拿到就不再带；没拿到时照旧用第一句话。标题不计费。
- **`safety_check`**：设置里的开关（`agent.safetyCheck`，默认开）。审核拒绝（`finish_reason: "content_filter"`）时丢掉已经显示的半句，换成一句说明；输入被拦（400 `Content Exists Risk`）同样说明。
- **思考**：思考模式下 `reasoning_content` 每一轮都原样带回（包括没调工具的轮次，否则 400）；不再传 `temperature`（思考模式下无效）。思考强度 `reasoning_effort` 游戏里、聊天窗口里各一个设置（`agent.gameEffort` / `agent.chatEffort`），默认都跟服务商（实测见下）。
- **错误**：最后一块带 `error`、没有 `[DONE]`（HTTP 已经是 200）按失败处理；`: keep-alive` 心跳行跳过（不算有新内容）；429 / 500 / 503 / 504 在还没出字时等一下重发（最多 2 次）；401 说密钥无效，402 说余额不足。

## 实测（真实 DeepSeek，2026-11-18；文档和实际不一致的以实测为准）

| 项目 | 结果 |
|---|---|
| `title` + 我们的工具 | 能用；这一步以文字回答结束时返回标题，以工具调用结束时不返回 |
| `latest_reminder` + 我们的工具 | 能用；不给日期时模型把英雄联盟当前版本答成 26.9（4 月），给了日期并搜索后答对 26.22 |
| `safety_check` + 我们的工具 | 能用；满是脏话的游戏聊天当工具结果没被拦，审核 0 token |
| `deepseek_web_search` + 我们的工具 | 文档说不行；实际报错“client tools can only be used together with `deepseek_web_search` in thinking mode”，思考模式下能用 |
| 搜索接口 | 0.4 秒；中文游戏结果（九游、360、NGA、MetaBot）新到前几天 |
| 抓取接口 | 3–7 秒；`127.0.0.1` → 403 |
| 琴音完整一轮（`scripts/manual-e2e/search_real.py`，3 个海克斯大乱斗 / 版本问题 × 3 种思考强度 + 1 个游戏内问题） | 每个问题都先搜了（多数搜 2 次），答出当前版本 26.22、26.23 次日更新、海斗强势英雄与寒冰的强化推荐，都说了依据；每轮 5–6 秒；模型费用约 ¥0.007–0.011 / 轮，加搜索 ¥0.01；high / low / none 速度没有差别，所以默认不改；DeepSeek `title` 起的标题如“英雄联盟版本查询”；内容审核没有拦任何一轮 |

## 没有照搬的

- 严格模式（strict）：要走 beta 地址、所有参数必填，几十个工具的参数格式都得改，收益小。
- 文件解析、FIM、JSON Output、`user_id` 并发隔离：陪玩用不上。
- Responses / Anthropic 兼容接口：Chat Completions 已经够用，缓存字段也最全。
