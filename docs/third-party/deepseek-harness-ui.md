# 来源记录：参照 DeepSeek Harness 的界面设计

设置窗口的整体重新设计（2026-11，外壳 + 「琴音」聊天页，后续各页面）参照 DeepSeek Harness 客户端界面的布局、信息层级、组件形态与交互。

- 上游：<https://github.com/deepseek-ai/deepseek-harness>，读取时的 commit `39163dd30f703a79079e2d2446545cff3c453738`，`packages/client/ui-*`（React + CSS Modules）；本机安装版 `resources/app.asar` 中的同名包作对照。
- 许可证：MIT，Copyright (c) 2026 DeepSeek，全文见 [`licenses/deepseek-harness-MIT.txt`](../../licenses/deepseek-harness-MIT.txt)。
- **没有复制源码**：Kotone 用 Svelte 与自己的 shadcn-svelte 组件重新实现，只参照尺寸、层级与交互；**不使用 DeepSeek 的品牌元素**（logo、`ui-brand-official` / `dsh-brand` 资源、“DeepSeek”字样、品牌蓝）。气质与配色用 Kotone 自己的：琴音形象、蓝黑底与青 / 粉霓虹。

## 第一阶段：设计语言、外壳与「琴音」聊天页

| Kotone 文件 | 参照的上游组件 | 采用的做法 | Kotone 的不同 |
| --- | --- | --- | --- |
| `apps/desktop/src/routes/settings/Settings.svelte` | `ui-layout/AppFrame.module.css` | Windows 下标题行与侧边栏同底色，内容区比外框亮一档并带左上圆角；窄窗口时侧边栏改成抽屉 | 断点 760px；侧边栏展开 / 图标栏的选择记在本机 |
| `apps/desktop/src/routes/settings/Titlebar.svelte` | `ui-layout` Windows 标题行、`ui-sidebar/HeaderLeadingControls` | 40px 标题行，左侧侧边栏开关，右侧窗口按钮；双击切换最大化 | 保留 Kotone 的运行状态胶囊、启动 / 停止按钮、资源占用、退出确认 |
| `apps/desktop/src/routes/settings/AppSidebar.svelte` | `ui-sidebar/SidebarRoot.module.css` | 顶部“新对话”、32px 导航行、会话列表、底部固定的设置项；56px 图标栏 | 导航是 Kotone 的设置分类；当前项的青色指示条；底部反馈群与琴音图标 |
| `apps/desktop/src/lib/agentChat/ConversationList.svelte` | 侧边栏会话列表 | 一行一个会话、悬停出现删除、正在回答的标记 | 删除前确认；回答中不能切换 / 删除 |
| `apps/desktop/src/lib/agentChat/ChatTurn.svelte`、`ChatTranscript.svelte` | `ui-chat/MessageItem`、`UserMessageBubble`、`ChatView`、`ui-conversation/skeleton/ConversationRoot` | 约 760px 的内容列；用户消息是右侧 20px 圆角气泡；回答不加气泡；过程是 24–28px 的单行“流程行” | 每轮前有琴音头像与名字；自动化结果是一条流程行（可展开执行记录） |
| `apps/desktop/src/lib/agentChat/ThinkingBlock.svelte` | `ui-chat/ReasoningRow` | 单行“思考过程”，展开后左侧细线缩进 | 流式时文字微光（`.kotone-shimmer`，参照 `ui-primitives/TextShimmer`） |
| `apps/desktop/src/lib/agentChat/ToolCallCard.svelte` | `ui-chat/GenericCommandCard`、`ui-approval/ApprovalPanel` | 图标 · 标题 · 圆点 · 状态 的单行，展开为代码块样式的参数与结果；确认卡是带警示色条的圆角卡片 | 确认卡仍挂在对应的工具行下（原需求“按钮放在工具卡片上”），没有像上游那样替换输入框 |
| `apps/desktop/src/lib/agentChat/TurnFooter.svelte` | `ui-chat/MessageIconActions`、`StatsPills` | 图标按钮 + 统计胶囊 | 只显示服务商报告的用量 |
| `apps/desktop/src/lib/agentChat/ChatComposer.svelte`、`routes/settings/pages/assistant/AssistantChat.svelte` | `ui-conversation/skeleton/InputBar`、`EmptyHero` | 大圆角输入卡（文字区在上、工具行在下）、贴底；空白对话时输入卡居中 | 发送按钮带文字（发送 / 插话）；语音按钮在工具行；空白页是琴音形象与起手提示 |
| `apps/desktop/src/routes/settings/pages/AssistantPage.svelte` | 会话头部 | 页名 / 当前对话标题，右侧分页切换 | 进行中的提醒与自动化概况 |

## 第二阶段：API 连接页与其余页面

| Kotone 文件 | 参照的上游组件 | 采用的做法 | Kotone 的不同 |
| --- | --- | --- | --- |
| `apps/desktop/src/lib/components/settings/{SettingsPage,SettingsSection,SettingsRow}.svelte` | `ui-primitives/settings-form`、各设置页的 section / settings card | 居中内容列、页标题 + 一句说明；组标题在卡片外，卡片内一行一项、细分隔线 | 也用于高级页的分区切换（分段控件） |
| `apps/desktop/src/routes/settings/pages/advanced/ConnectionsSection.svelte` | `ui-settings-models/{ModelsSection,ProviderEditor,CustomProviderCard,ApiKeyInput}` | 已有连接的卡片列表（密钥状态点、标签、行内操作）；提供商目录；编辑器里地址、模型（列表选或手填）、只写的密钥、获取密钥链接 | 按 Kotone 的分组（推荐 / 国内 / 海外 / 聚合 / 自定义）与搜索；“琴音在用”与能力自检结果放在卡片上；编辑器是对话框 |
| 其余设置页（通用、琴音设置、自动化、文字处理、游戏适配、历史记录、高级、关于、角色、首次向导、各弹窗） | 同上 | 统一的页面骨架、设置卡与表单控件 | 保留琴音立绘、贴纸等 Kotone 形象元素 |
| `apps/desktop/src/lib/components/ui/sonner/sonner.svelte` | `ui-primitives/Toast` | 右下角提示 | 每种提示配一张琴音贴纸 |
