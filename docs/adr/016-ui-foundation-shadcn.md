# ADR 016：前端引入 shadcn-svelte，分三步重构；聊天界面复用 RambleDesk

- 状态：Accepted / 已采纳（2026-11-16，用户确认“按三步走”；整体重构由另一个 Agent 按[任务文档](../research/2026-11-16-ui-refactor-task.md)完成）
- 关联：[ADR 014](014-game-agent-harness.md)（聊天界面放在「琴音」页）、[ADR 015](015-agent-scope-and-stack.md) §3（聊天事件按 ACP 设计）

## 问题

Kotone 的设置窗口前端全部手写：Svelte 5 + Tailwind 4，没有组件库和图标库，48 个组件约 1.1 万行，样式散在各组件和一份 `panel.css` 里。新增的内置 Agent 需要一个完整的聊天界面：流式输出、工具调用卡片与状态、权限确认按钮、思考过程折叠、长对话滚动与翻页、输入法安全的输入框。这在 Agent 产品里是最重的一块前端，继续手写成本高、质量难保证。

用户之前的项目 RambleDesk（`C:\Users\A\Desktop\rambledesk`，基于 ACP 的 Agent 工作台）已经用 shadcn-svelte 做过一整套 ACP 聊天界面（部分从 Codeg 改写，Apache-2.0）。它的底层与 Kotone 相同（Svelte 5 + Tailwind 4）。

## 决策

1. **引入 shadcn-svelte**（bits-ui、tailwind-variants、tailwind-merge、lucide 图标；按需加 svelte-sonner、paneforge）。组件以源码形式复制进项目，不是运行时大依赖。
2. **分三步，不一次重写**：
   1. **底子**：初始化 shadcn-svelte，把 Kotone 的配色与圆角等做成设计变量，引入基础组件与图标；现有页面不动，全部测试照常通过。
   2. **「琴音」聊天页**：在新底子上做 ADR 014 的聊天界面，时间线、工具卡片、思考块、安全 Markdown、滚动锚点等复用 RambleDesk 的实现；补上 Kotone 自己的部分（权限按钮、语音输入、起手提示、用量）。
   3. **其他页面逐页迁移**：每迁一页同步改它的测试，每个提交都能单独构建。
3. **悬浮窗与弹幕窗不套组件库**：它们是透明、穿透的轻量窗口，保持现状。
4. **外观可切换**（用户 2026-11-16 确认）：参照 RambleDesk 的外观系统（`apps/desktop/src/lib/appearance/`：多套配色、明暗主题、窗口缩放、界面 / 代码字体），支持切换外观主题、字体大小等；Kotone 现在的深色 + 青 / 粉作为默认主题之一。偏好只影响设置窗口，悬浮窗与弹幕窗保持现状。
5. **复用来源要记账**：从 RambleDesk 复制的代码中，源自 Codeg 的部分保留 Apache-2.0 声明，并在 Kotone 里建来源记录（参照 RambleDesk 的 `docs/CODEG_PORTS.md`）与 `THIRD_PARTY_NOTICES.md` 条目。

## 已定与待定

| 事项 | 结论 |
| --- | --- |
| 外观 | **已定**：参照 RambleDesk 支持切换外观主题、字体大小等（见决策 4） |
| 聊天组件怎么复用 | **暂按复制 + 记录来源执行**（两边数据来源不同：RambleDesk 是 ACP 活动快照，Kotone 是内置 Agent 事件，先各自适配，稳定后再考虑抽成共用包）；用户可以改 |
| 执行方式 | 用户同意：由子代理做前端，与 Agent 后端（第 5 步）在同一分支上按文件分工并行；子代理先做第一、二步，汇报后再决定第三步 |

## 结果

聊天界面用经过验证的实现起步，不在 Kotone 里从零写一套；其余页面在不打断现有功能和测试的前提下逐步统一到同一套组件。代价是新增前端依赖与一次性的迁移工作量，以及跟 RambleDesk 两份聊天代码需要分别维护（直到决定是否抽包）。
