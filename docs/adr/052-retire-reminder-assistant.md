# ADR 052：提醒助手退掉；她的连接和呼叫键只在一个地方

状态：Accepted（2026-11-28）/ 已实现（单元测试、浏览器预览的端到端测试），待真机。

取代：[ADR 012](012-reminder-assistant.md)（独立提醒助手）。修订：[ADR 027](027-0.2-upgrade-transition.md) 的“老用户升级差异与决定”（旧提醒助手快捷键、等待中的提醒、首次打开那几行）、[ADR 028](028-game-companion-interaction.md) 流程 B（升级欢迎页、启用她时写哪里、侧栏）、[ADR 040](040-hotkeys-belong-to-core.md) D10、[ADR 051](051-lol-happy-path.md) 规则第 2 条。

## 背景

玩家：

> “提醒助手本质上已经被自动化淘汰了不是吗？我觉得可以移除了？”

是的。从 [ADR 013](013-automation-domain-model.md) 起，提醒就是“延迟 + 播报”的自动化；[ADR 045](045-automation-is-programs.md) 以后是“晚一会儿开始、只说一句”的程序（`say(args.text)`），她的 `reminder.create` 工具和「自动化」里内置的“X 秒之后提醒我”都排进同一个调度器。0.1.x 留下来的「提醒助手」只是另一个入口，却带着一整套自己的东西：

- 侧栏「语音输入 → 提醒助手」一页（任务列表、文字框、语音按钮、它自己的设置）；
- 自己的热键（`legacy-reminder`，按住 / 按一下照 `assistant.interactionMode`），在「设置 → 热键」里只读、要去提醒助手页改；
- 自己的 API 连接和一次大模型调用（`kotone-ai` 的 `ReminderAssistant`，`voice_reminder` / `danmaku_reminder` 两个受限工具）；
- 自己的语音引擎（只有系统语音和 Kokoro）。

更麻烦的是：0.1.x 用户的提醒助手占着 `assistant.connectionId` 和 `assistant.hotkey`，所以这些用户启用她以后，她的连接和呼叫键只好另存在 `assistant.companionBindings`。前端、后端、热键表、引导、“准备好了吗”到处都要分“新用户写这里、升级用户写那里”（`companion_connection_id()`、`companionBindings()` / `companionBindingPatch()`、`set_mirror` 的分支……）。今天那位老用户的她停在以前配的千问上（ADR 051），就是这条缝。

## 决定

### 1. 提醒助手整个去掉

- 页面、侧栏那一项、它的文字框和语音按钮（`LegacyRemindersPage`、`ReminderTasks`、`LegacyReminderSettings`、`AssistantVoiceInput`、`assistantVoice.ts`）；
- 它的热键：热键表里的那一行、登记、按住的分派（`HK_LEGACY_REMINDER`、`Settings::legacy_reminder_hotkey`、`hasLegacyReminders`）；core 里只给它用的入口（`on_assistant_hotkey*`、设置窗口语音按钮的会话 `start/finish/confirm/cancel_assistant_voice_input`、`begin_assistant`）和 `legacy-reminder-` 会话前缀的分支；
- 它的大模型调用（`kotone-ai/src/reminders.rs`）和桌面端的命令（`list_reminders`、`cancel_reminder`、`assistant_submit_text`、`cancel_assistant_request`、`assistant_voice_*`）；
- 旧提醒用旧语音引擎说的那条路（`legacy_voice`）；`list_speech_voices` / `preview_reminder` 不再有“提醒助手”版本。

留下的：`kotone_core::reminders` 里自动化在用的东西（`DeliveryKind`、提醒视图、0.1.x `reminders.json` 的读取格式），语音和弹幕的投递，语音合成。她的 `reminder.create`、「自动化」里的“X 秒之后提醒我”照旧。

旧配置里的 `assistant.interactionMode` 不再有人读，只为旧版本能读保留（设置说明里列为内部项）。界面里写“提醒助手”的旧链接（设置说明、她以前说过的位置）跳到「自动化」。

### 2. 已经安排好的提醒照常提醒

查过：提醒没有存在别的地方。从 ADR 013 起，提醒助手、她、自动化安排的提醒都在同一个 `automation-runs.json`；0.1.x 的 `reminders.json` 在第一次启动 0.2 时只读导入一次（`AutomationScheduler::import_legacy_reminders`，文件不改不删，这条导入保留）。所以不用迁移：还没到点的照常提醒，在「自动化」里看得到、能单条取消，只用语音输入的用户也一样（调度器对所有人都启动）。唯一的变化：0.1.x 导入的和提醒助手安排的提醒，到点时用现在的声音设置说（以前硬用旧的系统语音 / Kokoro）；只用语音输入的用户“自动”就是系统语音，和以前一样。

### 3. 她的连接和呼叫键只在一个地方

所有人都是 `assistant.connectionId` 和 `assistant.hotkey`（热键表 `kotone.call` 的镜像）。`companionBindings` 从代码里去掉：`Settings::call_hotkey()` / `companion_connection_id()` 只剩一个判断（只用语音输入时没有），前端直接读 `assistant.connectionId` / `assistant.hotkey`。

读取配置时（`settings::migrate_legacy_upgrade_json`，和以前一样在合并默认值之前、只动文件里原样的 JSON）：

| 读到的 | `assistant.connectionId` | `assistant.hotkey` | `ui.legacyUpgrade` | `companionBindings` |
| --- | --- | --- | --- | --- |
| `optedIn`，有 `companionBindings` | 换成 `companionBindings.connectionId` | 换成 `companionBindings.hotkey` | 写成 `notNeeded` | 去掉 |
| `optedIn`，没有 `companionBindings` | 清空 | 清空 | 写成 `notNeeded` | — |
| `pending`（含没有这个字段的 0.1.x 配置）、`dismissed` | 清空（原来是旧提醒助手的连接） | 清空（原来是提醒键） | 不变 | — |
| `pending` / `dismissed`，有 `companionBindings`（启用过她、又选回只用语音输入） | 清空 | 清空 | 不变 | 去掉（再启用她时重新设） |
| `notNeeded` / 新装 | 不动 | 不动（旧默认 `` ` `` 照旧升成 `Ctrl+` ``） | 不变 | — |

旧连接本身一条不删，还在「API 连接」里；只是不会再变成她的模型。按 [配置兼容](../development.md) 的规矩，写的都是旧版本认识的值：两个字符串字段、`legacyUpgrade` 的 `notNeeded`；旧版本保存时会丢掉新字段，这里没有新字段。设置接口传进来的 `optedIn`（旧前端、旧命令行）也读成 `notNeeded`（`#[serde(alias)]`）。

### 4. `legacyUpgrade` 只剩三个值

`optedIn` 唯一的用处是告诉代码“她的东西在 `companionBindings`”，现在没有了，合进 `notNeeded`：启用了她的 0.1.x 用户就是普通用户。`pending`（看一次升级欢迎页）和 `dismissed`（她不出声、没有呼叫键和停下热键、落地页是语音输入、不装内置程序、不做复盘）照 ADR 028 不变，只是去掉了提醒助手。Rust 里 `uses_legacy_assistant()` 改叫 `voice_input_only()`，`has_legacy_reminders()` 删掉。

在升级欢迎页或「API 连接」里第一次存 DeepSeek Key 的那一下（`switchToDeepSeek`，还在 `pending` / `dismissed`）：写 `assistant.connectionId` 和 `legacyUpgrade = notNeeded`，语音识别照旧不动（ADR 027 / 028 D6）。之后和新用户一样：再换 DeepSeek 时，自己下好并选了的本地识别不换，其余的语音识别也换成 DeepSeek（以前的 `optedIn` 用户这里一律不换）。

### 5. 升级欢迎页

摘要里不再写“提醒助手照旧”。“升级不会改你的任何设置”改成说实话：“语音输入的设置一项没改。「提醒助手」不再单独有了：它的热键不再用，还没到点的提醒照常提醒，在「自动化」里看得到、能取消。以后要提醒，跟{name}说一声。”设呼叫键那一步的小字改成“你原来的语音输入热键不受影响。”

## 玩家失去什么

- **0.1.x 用户里留着语音输入、又在用提醒助手、没启用她的**：「提醒助手」页、它的热键（对讲机 / 录音笔 / 说一句就走）、文字框和它自己那条 API 连接都没有了。要新的提醒，启用她（填 DeepSeek Key）以后跟她说“五分钟后提醒我打小龙”，或者在「自动化」里新建一个“说一句”。已经安排好的提醒不受影响。
- **装回 0.1.x**：0.2 存过一次配置以后，0.1.x 读到的提醒键和提醒连接是空的（还没启用她的），或者是她的（启用过的），要在 0.1.x 里重新设。ADR 027 的备份照旧在。
- **启用过她的 0.1.x 用户以后再换 DeepSeek**：语音识别按新用户的规矩（见第 4 条）。

## 不做

- 不删 `reminders.json` 的导入（没升级过的 0.1.x 用户第一次打开还要它）。
- 不删 `assistant.interactionMode`、`assistant.defaultDelivery` 这些字段（旧版本要读；`defaultDelivery` 是她的回答方式）。
- 不改她的 `reminder.create` 和内置的“X 秒之后提醒我”。

## 实现情况

- core：`settings.rs`（`LegacyUpgrade` 三个值、`migrate_legacy_upgrade_json` 和上表的单元测试 6 条、`migrate_call_hotkey_json` 不再分升级用户）、`assistant.rs`（去掉 `CompanionBindings`、会话前缀、`AssistantVoiceSession`）、`hotkey_bindings.rs` / `hotkey_table.rs`（呼叫键只有一个镜像、没有提醒助手那一行）、`orchestrator/`（去掉提醒助手热键和设置窗口语音按钮的入口，她的会话只由呼叫开始）、`settings_guide.rs`、`voice_vendor.rs`、`automation.rs`（导入的提醒照旧带 `legacy-reminder-` 前缀，只是标来源）。`tests/orchestrator.rs`：只测提醒助手热键 / 语音按钮的删掉，测助手会话本身的（不进游戏、取消后不能提交、语音输入键不结束她的这句）改用呼叫键的录音笔。
- kotone-ai：删 `reminders.rs`。桌面端：`assistant.rs`（`RoutingHandler` 只用语音输入时不处理，回一句“先添加 DeepSeek Key”）、删 `assistant_input.rs`、`hotkey.rs`、`settings_cmd.rs`、`events.rs`、`bubble.rs`、`automation/program.rs`。
- 前端：删页面和组件；`contracts.ts` 去掉 `companionBindings` / `companionBindingPatch` / `hasLegacyReminders`；`deepseekSetup.ts`、`Onboarding.svelte`、`ReadyChecklist.svelte`、`ModelMenu.svelte`、`ConnectionsSection.svelte`、`OverlayBar.svelte`、`SpeechSettings.svelte`、侧栏（只剩「游戏陪玩」一组，旧配置里记着的 `legacy-reminders` 排序时跳过）；`browserMock.ts` 的 `mockLegacyUpgrade` 按读取迁移以后的样子造。
- 端到端：`lol-happy-path.spec.ts`（带千问的 0.1.x 用户：读取以后千问不是她的模型、还在「API 连接」里，最后她在 DeepSeek 上）、`onboarding.spec.ts`、`copilot-flow.spec.ts`、`hotkeys-page.spec.ts`。
- 待真机：一份真的 0.1.x 配置（带提醒键、提醒连接、还没到点的 `reminders.json`）升级上来，看欢迎页、「自动化」里的提醒到点照常说；一份启用过她的 rc 配置读成 `notNeeded`、呼叫键照旧能用。
