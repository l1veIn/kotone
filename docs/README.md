# Kotone 文档索引

仓库文档分为当前规范、操作手册和历史记录。当前行为以实现与通过的测试为依据；发现冲突时在同一次变更中修正文档，避免再维护第二份接口清单。

| 目的 | 权威入口 |
| --- | --- |
| 理解模块职责、依赖和关键行为 | [architecture.md](architecture.md) |
| Kotone 是什么、本体和插件怎么分、插件重构怎么做（计划，ADR 038） | [plugin-refactor.md](plugin-refactor.md) |
| 确认术语、代码名及中英文表达 | [glossary.md](glossary.md) |
| 开发环境、验证和修改约定 | [development.md](development.md) |
| 命令行用法 | [cli.md](cli.md)，参数以 `kotone-cli --help` 及子命令帮助为准 |
| 模型评测、标注和缓存规则 | [eval-playbook.md](eval-playbook.md) |
| 陪玩（助手热键、快捷指令、提醒 / 打字 / 回答）与输出限制 | [assistant.md](assistant.md) |
| 本机 Agent 工具服务（开启、协议、工具 schema、捕获限制、OCR 与聊天跟踪、应用启停、事件与用户请求、配置与首次引导、Kotone 陪玩工具、MCP / DSH 插件） | [tool-service.md](tool-service.md) |
| 诊断包内容与操作 | [diagnostics.md](diagnostics.md) |
| 数据和网络边界 | [PRIVACY.md](../PRIVACY.md) |
| 发布门禁与分发 | [release-checklist.md](release-checklist.md) |
| 用户可见版本变化 | [CHANGELOG.md](../CHANGELOG.md) |

## 设计依据与历史

- [游戏聊天 OCR 会话研究报告（2026-11-15）](research/2026-11-15-lol-chat-ocr-report.md) 汇总诊断、产品分工与韩服聊天实验；含39图配对对照及可提交的小型证据，不是整体准确率承诺。
- [面向 Agent 的游戏工具演进计划](product-evolution-game-tools.md) 给出基础服务、目标暴露列表和分阶段验收范围；M1、M2 与 §5.5 的底座第一批已按 [工具服务手册](tool-service.md) 实现，其余仍是计划，不是当前 API。§10“Kotone 陪玩”与 §11 ASR / OCR / TTS 统一模型管理已在分支 `claude/companion` 实现（先做成 Kotone 自己的功能，再以 `companion.*`、带 `category` 的 `models.*` 开放给 Agent；Edge TTS 不接入）。§12“自动化”（大模型写程序、沙盒执行，代替按键精灵式手写脚本）是 2026-11-16 定的开发方向。
- [M1 实现与验证记录](research/game-tools-m1-implementation.md) 记录本轮命令、结果、捕获限制与未完成的人工检查。
- [M2 实现与验证记录](research/game-tools-m2-implementation.md) 记录预处理、OCR、聊天跟踪的实现、测试、真实本地韩文 OCR 冒烟与未验证条件。
- [Agent 底座第一批实现与验证记录](research/agent-base-batch1-implementation.md) 记录应用启停、事件总线、用户请求反向通道、MCP 与 DSH 插件的实现、真机端到端结果、发现的问题与未验证条件。
- [Agent 底座第二批实现与验证记录](research/agent-base-batch2-implementation.md) 记录与设置页对齐的 35 个配置类工具（设置、模型、连接、profile、热键、提权、自启、历史、诊断）、委托宿主的设计、凭据隔离与真机冒烟结果。
- [交接：Kotone 陪玩与统一模型管理（2026-11-15）](research/2026-11-15-companion-handoff.md) 记录"先有完整功能再开放给 Agent"的方向修正、已定的陪玩 / 快捷消息设计、ASR/OCR/TTS 统一模型管理方案与开工顺序；设计已正式写入演进计划 §10、§11。
- [陪玩与统一模型管理实现记录（2026-11-15）](research/2026-11-15-companion-implementation.md) 按步骤记录统一模型清单、OCR 多语种包（哈希复核、逐包接入确认与实测语种）、TTS、陪玩与工具对齐的实现、测试与未验证条件。
- [游戏陪玩 Agent 现状调研（2026-11-16）](research/2026-11-16-game-agent-landscape.md) 是 ADR 014（Kotone 定位为游戏场景的 Agent 运行框架）的依据。
- [交接：内置 Agent（2026-11-16）](research/2026-11-16-agent-handoff.md) 是下一轮开发的起点。
- [自动化实现记录（2026-11-16 起）](research/2026-11-16-companion-scripts.md) 记录 Luau 沙盒实验、护栏与各阶段验证。
- [ADR 索引](adr/README.md) 记录“为什么这样做”。已替代或修订的决策标明状态，旧正文保留用于追溯，不作为现行 API。
- [早期开发文档](archive/development-history.md) 保存阶段计划、原始评测成绩和验收历史。
- [技术预研](tech-research.md)、[STT 调研](recon-stt-engines.md)、[后处理调研](recon-postprocess-ai.md) 是有日期的研究材料，不能用来判断当前已支持的功能。
- [Sokuji ASR/TTS 对照](research-sokuji.md) 记录本次提醒能力的参考来源和取舍。
- [本地 TTS 研究与实测](research-tts.md) 说明 Kokoro 选型、角色口吻、模型体积和许可。
- [0.1.7 发布前审计](pre-release-audit-0.1.7.md) 是对应版本的快照。
- [2026-10 整体梳理闭环](audit-2026-10.md) 记录本轮问题、实现和验证覆盖。

## 维护规则

新增领域概念先补术语表；跨 crate 边界、数据外发、持久化或取消语义改变时同步更新架构和隐私说明。重要取舍新增 ADR，已经改变的 ADR 添加修订状态和后继链接。用户操作变化同步更新 CLI/用户手册；网站中英文文案同时修改，不手填“当前版本”或复制一份模型清单。发布门禁以工作流为准，手册给出入口和验收理由。
