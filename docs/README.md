# Kotone 文档索引

仓库文档分为当前规范、操作手册和历史记录。当前行为以实现与通过的测试为依据；发现冲突时在同一次变更中修正文档，避免再维护第二份接口清单。

| 目的 | 权威入口 |
| --- | --- |
| 理解模块职责、依赖和关键行为 | [architecture.md](architecture.md) |
| 确认术语、代码名及中英文表达 | [glossary.md](glossary.md) |
| 开发环境、验证和修改约定 | [development.md](development.md) |
| 命令行用法 | [cli.md](cli.md)，参数以 `kotone-cli --help` 及子命令帮助为准 |
| 模型评测、标注和缓存规则 | [eval-playbook.md](eval-playbook.md) |
| 助手热键、提醒与输出限制 | [assistant.md](assistant.md) |
| 诊断包内容与操作 | [diagnostics.md](diagnostics.md) |
| 数据和网络边界 | [PRIVACY.md](../PRIVACY.md) |
| 发布门禁与分发 | [release-checklist.md](release-checklist.md) |
| 用户可见版本变化 | [CHANGELOG.md](../CHANGELOG.md) |

## 设计依据与历史

- [ADR 索引](adr/README.md) 记录“为什么这样做”。已替代或修订的决策标明状态，旧正文保留用于追溯，不作为现行 API。
- [早期开发文档](archive/development-history.md) 保存阶段计划、原始评测成绩和验收历史。
- [技术预研](tech-research.md)、[STT 调研](recon-stt-engines.md)、[后处理调研](recon-postprocess-ai.md) 是有日期的研究材料，不能用来判断当前已支持的功能。
- [Sokuji ASR/TTS 对照](research-sokuji.md) 记录本次提醒能力的参考来源和取舍。
- [本地 TTS 研究与实测](research-tts.md) 说明 Kokoro 选型、角色口吻、模型体积和许可。
- [0.1.7 发布前审计](pre-release-audit-0.1.7.md) 是对应版本的快照。
- [2026-10 整体梳理闭环](audit-2026-10.md) 记录本轮问题、实现和验证覆盖。

## 维护规则

新增领域概念先补术语表；跨 crate 边界、数据外发、持久化或取消语义改变时同步更新架构和隐私说明。重要取舍新增 ADR，已经改变的 ADR 添加修订状态和后继链接。用户操作变化同步更新 CLI/用户手册；网站中英文文案同时修改，不手填“当前版本”或复制一份模型清单。发布门禁以工作流为准，手册给出入口和验收理由。
