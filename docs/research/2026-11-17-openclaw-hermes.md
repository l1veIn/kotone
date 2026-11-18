# 调研：OpenClaw 与 Hermes Agent 的会话、记忆、SOUL、Skill（2026-11-17）

为 [ADR 022](../adr/022-sessions-memory-soul.md) 准备。子代理读官方文档与 GitHub 源码的结论；标“未核实”的是没找到原文。**两个项目都是 MIT**（OpenClaw：Copyright 2026 OpenClaw Foundation，[LICENSE](https://github.com/openclaw/openclaw/blob/main/LICENSE)；Hermes：Copyright 2025 Nous Research，[LICENSE](https://github.com/NousResearch/hermes-agent/blob/main/LICENSE)），文字与代码都能照搬，保留版权声明。

## OpenClaw

- **工作区文件**（[agent-workspace](https://docs.openclaw.ai/concepts/agent-workspace)、[system-prompt](https://docs.openclaw.ai/concepts/system-prompt)）：主会话每轮按 `AGENTS.md` → `SOUL.md` → `IDENTITY.md` → `USER.md` → `BOOTSTRAP.md`（只在全新工作区）→ `MEMORY.md` 注入；子 agent 只注入 `AGENTS.md`（minimal 模式）。`memory/YYYY-MM-DD.md` 不注入，按需 `memory_search` / `memory_get`。单文件 20,000 字、合计 60,000 字，`USER.md` 4,000 字。`TOOLS.md`、`HEARTBEAT.md` 已废弃。
- **身份**（[IDENTITY](https://docs.openclaw.ai/reference/templates/IDENTITY)、[agents CLI](https://github.com/openclaw/openclaw/blob/main/docs/cli/agents.md)）：`IDENTITY.md` 字段 Name、Creature、Vibe、Emoji、Avatar（工作区相对路径 / http / data URI）；`set-identity` 写进配置，**配置优先，没配的回退到文件**；本地头像 ≤ 2 MB、不能跳出工作区。`BOOTSTRAP.md` 首次运行时问用户怎么称呼它（自己不许起名）。`SOUL.md`“归你演化”，但改了要告诉用户（[SOUL](https://docs.openclaw.ai/reference/templates/SOUL)）。
- **会话**（[session-tool](https://docs.openclaw.ai/concepts/session-tool)、[subagents tool-reference](https://github.com/openclaw/openclaw/blob/main/docs/tools/subagents/tool-reference.md)）：`sessions_create` / `sessions_list` / `sessions_history` / `sessions_send` / `sessions_spawn`（`task`、`label`、`model`、`thinking`、`runTimeoutSeconds`、`context` fork|isolated、`cleanup` delete|keep……）/ `sessions_yield` / `subagents`（列出、等待、取消）/ `session_status`。子 agent 的结果作为“完成事件”交还发起它的会话（[announce](https://github.com/openclaw/openclaw/blob/main/docs/tools/subagents/announce.md)）；`/stop` 级联取消整棵子树（[operations](https://github.com/openclaw/openclaw/blob/main/docs/tools/subagents/operations.md)）；默认子 agent 最多 5 个、60 分钟归档；子 agent 模型 / thinking 用 `subagents.model` / `.thinking` 配。
- **子 agent 的工具**（[tool-policy](https://github.com/openclaw/openclaw/blob/main/docs/tools/subagents/tool-policy.md)）：硬禁 `sessions_send`、`message`、`cron` 等，配置覆盖不了；另可 `tools.subagents.tools.allow/deny` 收紧。
- **Skill**（[skills](https://docs.openclaw.ai/tools/skills)）：agentskills.io 格式；`<workspace>/skills` 优先；提示词里只放 `<available_skills>`（name、description、location），全文按需读；agent 可自建，改动保留旧版本可撤销。
- **压缩**：接近 75% 自动压缩，压缩前静默 memory flush 到当天笔记（[compaction](https://docs.openclaw.ai/concepts/compaction)）。
- **安全**（[prompt-injection](https://docs.openclaw.ai/gateway/security/prompt-injection)）：不可信内容先交给不带工具的 reader 总结；不可信来源不会被提升进 `MEMORY.md`。

## Hermes Agent

- **记忆**（[memory](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory)、[memory_tool.py](https://github.com/NousResearch/hermes-agent/blob/main/tools/memory_tool.py)）：`MEMORY.md`（≤ 2,200 字）、`USER.md`（≤ 1,375 字）；只能经 `memory` 工具写（`target` memory|user，`operations[]` add / patch / remove，整批原子）；写满直接报错；“学到的流程写进技能，不写进记忆”；**会话开始时冻结成快照注入**（缓存稳定），写入立即落盘、下个会话生效。`session_search` 用 SQLite FTS5 搜历史会话。
- **Skill**（[skills](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)、[skill_manager_tool.py](https://github.com/NousResearch/hermes-agent/blob/main/tools/skill_manager_tool.py)）：兼容 agentskills.io；`skills_list` → `skill_view(name, path?)`；agent 用 `skill_manage`（create / patch / delete / write_file / remove_file）自建；名字 64 字、描述 1024 字、正文 100,000 字。
- **人格**（[personality](https://hermes-agent.nousresearch.com/docs/user-guide/features/personality)）：`SOUL.md` 在系统提示第 1 槽，注入前扫描提示词注入，子 agent 不加载。
- **委派**（[delegation](https://hermes-agent.nousresearch.com/docs/user-guide/features/delegation)、[delegate_tool_toolsets.py](https://github.com/NousResearch/hermes-agent/blob/main/tools/delegate_tool_toolsets.py)）：`delegate_task`；子 agent 全新对话，只有最终摘要回父会话；硬禁 `delegate_task`、`clarify`、`memory`、`send_message`、`cronjob_manage`；深度 1。
- **定时任务**：`cronjob_manage`，每次运行开新会话，`deliver:"origin"` 送回创建它的地方。
- **安全**：记忆写入、SOUL、上下文文件都过威胁模式库（[threat_patterns.py](https://github.com/NousResearch/hermes-agent/blob/main/tools/threat_patterns.py)），记忆写入命中就拦截；agent 自建技能的扫描默认关。

两家都没有唤醒词；也都没有明确的“agent 禁止写哪些文件”清单（未核实）。
