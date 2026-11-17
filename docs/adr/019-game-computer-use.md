# ADR 019：琴音在游戏里“看屏幕、动鼠标”（computer use）

- 状态：Accepted（设计，2026-11-17）。用户确认方向（“方案对了，就是这样”），并要求**尽量照搬开源社区打磨过的项目，不自己造轮子**。**已实现（2026-11-17，分支 `claude/kotone-agent`）**，与设计的出入见文末“实施记录”。
- 细化：[ADR 014](014-game-agent-harness.md)（游戏 Agent 三层：基础能力 → 自动化 → Agent）、[ADR 017](017-voice-wake.md)（语音入口、追问）、[ADR 018](018-companion-bubble.md)（对话气泡）
- 不变：自动化（Luau 沙盒）的接口与权限；对外工具服务里现有的 `screen.capture` / `image.crop` / `ocr.recognize` 等通用工具

## 问题

玩家会说“琴音，帮我找找背包里有没有 xxx”“帮我完成这几道问答”。每次的画面都不一样，事先写不了脚本。需要琴音**一边看一边做**：看一眼画面，决定点哪里，点完再看结果。

现状（2026-11-17 盘点）：

| 层 | 现在有什么 | 缺什么 |
|---|---|---|
| 底层 | `kotone_platform_windows::inject`：扫描码按键、绝对坐标鼠标（越界校正）；`GdiForegroundCapturer` 只截游戏窗口客户区；本地 PP-OCRv5 / 在线 OCR；自动化宿主 `DesktopHost`（绑定窗口、窗口比例坐标、前台检查、结束松开按住的键、纯色画面判独占全屏） | 模拟输入没打标记（`dwExtraInfo = 0`），分不清“琴音按的”和“玩家按的” |
| 看 | 通用工具：`screen.capture` → `image.crop` → `ocr.recognize`，按屏幕像素，要串几次调用 | 没有“一次看清游戏窗口 + 元素清单”的工具 |
| 动 | 琴音**没有直接的键鼠工具**，只能写一段 Luau 程序交给 `automation.run`（AI 新写的程序第一次运行要用户批准） | 没有一步一步操作的工具 |

DeepSeek 现在能看图；琴音的消息已经支持图片附件（`kotone_ai::agent::Attachment::Image`）。

## 参考与照抄对象（每个关注点一个主参考）

调研逐个读了源码（工作笔记与下载的源码在会话 scratchpad `ref/`，不进仓库）。【源码】= 在源码里看到，【文档】= 官方文档。

| 关注点 | 主参考（许可） | 照抄什么 |
|---|---|---|
| **动作清单** | Anthropic `computer_toolset_20260801` 与 anthropic-quickstarts `computer-use-demo/tools/computer.py`（MIT） | 动作名与参数名原样照抄（见决定 3）；打字 50 字一组、组间 12 ms【源码】；双击间隔 10 ms【源码】 |
| **批量执行** | Yu-tao-Li/dsh-computer-use-win `batch`（MIT） | 1–20 步；先校验整份计划再动手；出错、取消、投递不确定就停；新窗口 / 弹窗 / 目标关闭在下一步前停，已完成的保留并给 `stopReason`；批内持有输入锁不与别的输入交错；结尾观察一次；结果只说 `sent`，不说“成功”【文档】 |
| **截图编号** | 微软 UFO²（MIT）+ OmniParser（代码 CC-BY-4.0，需署名） | 编号 `1..N` 的标签贴在框上；给模型的清单只含 `{id, text, box}`；标签底色按亮度选黑 / 白字（亮度 > 160 用黑字），标签位置依次试左上、框外左、框外右、右上，取第一个不重叠的【源码】 |
| **截图缩放** | Anthropic 文档 | 缩放比 `min(1, L/长边, sqrt(P/(w·h)))`，点击坐标按 `x / scale` 换回【文档】；`zoom` 从原分辨率裁剪，但坐标仍用整张截图的坐标系【文档】 |
| **游戏循环与提示词** | 智源 Cradle（MIT）、清华 GameVerse（MIT，真实 Windows 游戏）、字节 UI-TARS-1.5 游戏提示词（Apache-2.0） | 循环：看 → 反思上一步有没有用 → 计划 → 做；提示词条款几乎原样照抄（决定 6）；Cradle 的“技能是代码”= Kotone 的自动化 |
| **游戏输入细节** | Cradle `gui_utils.py`、GameVerse | 视角转动用**不带 ABSOLUTE 的相对移动**（`MOVE_BY{dx,dy}`），绝对移动在原始输入的游戏里转不动视角【源码】；按住走路 = `KEY_DOWN w → WAIT 3 → KEY_UP w`【源码】；按键默认按住一小段（游戏常漏掉瞬间的按下 / 松开）【源码】；每步结束必须松开所有按住的键（UI-TARS 提示词）【源码】；UI-TARS 的鼠标移动后等 100 ms 再点【源码】 |
| **安全与交互** | dsh-computer-use-win + deepseek-harness `packages/experimental/cua`（MIT） | 模拟输入在 `dwExtraInfo` 打标记，自己的按键不触发停止；停止 = 取消整串、本轮不再自动重试；接管提示出现于第一次真实输入、结束后淡出；点击点被别的进程窗口盖住就拒绝（`POINT_COVERED`）；按应用（这里是按游戏）授权，允许 / 拒绝可持久保存，撤销会取消进行中的操作；窗口尺寸变了旧坐标作废（`WINDOW_CHANGED`），只移动窗口不作废【文档】 |

不照抄的：OmniParser 的图标检测模型（旧版 AGPL，且游戏图标靠大模型看图即可）；DSH 的“每步操作后把前台还给用户”（游戏里要一直留在前台）；Esc 作为停止键（Esc 是游戏按键，见决定 7）。

## 决定

### 1. 底层统一，不另起一套

截图、文字识别、键鼠输入、窗口绑定、停止热键、游戏配置的“允许自动化输入”只有一份，自动化与 computer use 共用。实现上把 `apps/desktop/src-tauri/src/automation.rs` 里的 `DesktopHost` 的窗口绑定、截图、坐标换算、前台等待、松开按住的键抽成共享模块（例如 `game_session.rs`），自动化宿主和 computer use 都用它。

### 2. 两种用法，只新增两个工具

- **一步一步做**：`game.look`、`game.act`（下面两节）。
- **写成程序做**：现有 `automation.run` / 已保存的自动化，相当于 DSH 官方的 `cua_run_code`。批量、重复的操作（“把 20 件装备逐个分解”）写一小段程序一次跑完。
- 现有通用工具（`screen.capture`、`ocr.recognize`……）保留给外部 Agent 与写自动化时调试；`game.look` 内部复用它们的实现。
- 做成一件事后，琴音可以问“要不要把刚才的步骤存成自动化？”（Cradle 的技能积累思路），以后不再调模型。

### 3. `game.act`：动作清单照抄 Anthropic，批量规则照抄 dsh-computer-use-win

```text
game.act {
  observationId,                 // 上一次 game.look 的编号；不是最新的就拒绝，要求先看一眼
  actions: [ {action, ...}, ... ]  // 1–20 步
}
```

动作（名字、参数照抄 Anthropic；坐标 `coordinate` 是**最近一次截图里的像素**，也可以用 `element` 填编号）：

| action | 参数 | 说明 |
|---|---|---|
| `left_click` / `right_click` / `middle_click` / `double_click` / `triple_click` | `coordinate` 或 `element`；`text`（按住的修饰键，如 `"shift"`） | 先移过去、等 100 ms 再点 |
| `left_click_drag` | `start_coordinate`、`coordinate` | 拖拽 |
| `mouse_move` | `coordinate` 或 `element` | 悬停（看物品说明） |
| `left_mouse_down` / `left_mouse_up` | 无 | 先 `mouse_move` |
| `scroll` | `scroll_direction`（up/down/left/right）、`scroll_amount`（滚轮格数）、`coordinate?` | |
| `type` | `text` | 50 字一组、组间 12 ms |
| `key` | `text`（如 `"i"`、`"ctrl+s"`）、`repeat`（1–100） | 默认按住约 50–100 ms 再松开 |
| `hold_key` | `text`、`duration`（秒，上限 10） | 走路、蓄力 |
| `wait` | `duration`（秒，上限 10） | |
| **游戏补充**：`key_down` / `key_up` | `text` | GameVerse；本次 `act` 结束时自动松开 |
| **游戏补充**：`mouse_move_by` | `dx`、`dy`（像素） | 相对移动，转视角用（不带 ABSOLUTE） |

批量规则（dsh-computer-use-win）：先校验整份计划；按顺序执行，第一处失败就停，后面的每一步返回 `Not executed: an earlier action failed.`；每步前检查游戏仍在前台（不在就暂停等待，与自动化一致）、玩家有没有接手（决定 7）、窗口尺寸有没有变；全部做完（或停下）后**自动再看一眼**，把新的 `game.look` 结果带回去；结果只说动作已发出（`sent`），由模型看新画面判断有没有生效。

### 4. `game.look`：截图 + 编号元素清单

```text
game.look { zoom?: [x0, y0, x1, y1] }   // zoom：在整张截图坐标系里的区域，从原分辨率裁剪放大
→ { observationId, image, scale, elements: [{id, text, box:[x0,y0,x1,y1]}], window: {width, height} }
```

- 只截绑定的游戏窗口客户区；纯色画面（独占全屏）直接报错，请玩家改成无边框窗口。
- 缩放照 Anthropic：长边默认 1280、像素上限可配置（DeepSeek 的图片上限没查到，先保守）；坐标一律是返回图片里的像素。
- 文字识别的每一行编号，画到图上（UFO² / OmniParser 的标签样式），同时以 `{id, text, box}` 清单给模型。只看文字就够的事（问答题）主要靠清单；图标、物品靠看图。
- 编号只对最近一次 `look` 有效；窗口尺寸变了，旧坐标与编号都作废。
- 上下文里只保留最近几张截图（Anthropic：一次请求超过 20 张图有更严的限制），更早的换成文字占位。

### 5. 坐标

模型面对的始终是“这张截图里的像素”；内部换成窗口比例（0–1，和自动化一致），再按当前客户区换成屏幕物理像素。窗口移动不影响；尺寸变了要求重新看。点击点被别的进程窗口盖住时拒绝（`POINT_COVERED`）。

### 6. 循环与提示词

- 每一步：看 → 反思上一步有没有用 → 计划 → 做（一个 `game.act` 批量）。
- 提示词条款照抄（保留来源说明）：把新发现的游戏规则记下来后面用（UI-TARS 游戏提示词）；同一个动作没效果就换方法，不要重复（UI-TARS / GameVerse）；每步结束松开所有按住的键（UI-TARS）；卡住或需要玩家决定时问玩家——Kotone 里就是 `reply.ask`（UI-TARS 的 `call_user`）；先假设、再验证的思考示例（UI-TARS 的示例 Thought）。
- 预算：computer use 任务单独放宽，暂定 30 步、5 分钟（现在语音入口是 10 步、2 分钟）。

### 7. 安全与交互

- **按游戏授权**（DSH CUA 的应用授权）：第一次在某个游戏里用 `game.act` 时问一次：“允许琴音在『英雄联盟』里操作鼠标键盘吗？”选项：这次 / 一直 / 不允许；“一直”和“不允许”持久保存（新配置字段），设置页可撤销，撤销会取消进行中的操作。游戏配置里“允许自动化输入”关着时一律不操作。
- **竞技对局默认不动**：对局进程（如 `League of Legends.exe`、`VALORANT-Win64-Shipping.exe`）默认不允许 `game.act`，客户端、商店等对局外界面可以；玩家可在游戏配置里自己放开（原因：反作弊可能检测模拟点击）。
- **接手 = 暂停**：给 Kotone 的模拟输入在 `dwExtraInfo` 打标记（`inject.rs` 现在是 0）；低级键鼠钩子已能读到 `LLKHF_INJECTED` / `LLMHF_INJECTED`，再记下“最近一次真人输入的时间”。`game.act` 执行中发现真人输入就停下，告诉琴音“玩家接手了”。
- **停止**：Esc 是游戏按键，不当停止键；用现有的停止自动化热键（默认 Ctrl+Shift+F12），以及语音说“停下”。停止取消整串，本轮不再自动重试。
- **看得见**：右下角气泡显示“正在操作：打开背包…”（ADR 018 的气泡加一个状态）；屏幕边缘发光等更强的提示以后再说。
- **输入锁**：自动化程序与 `game.act` 不能同时往同一个游戏按键。
- **隐私**：只截游戏窗口，不截整个桌面。

### 8. 范围

第一版**只做游戏窗口**。普通桌面软件、网页交给社区现成的方案（UI Automation / 浏览器调试协议），以后再说。

## 实施步骤（下一个会话）

1. 抽共享的 `game_session`（窗口绑定、截图、坐标、前台等待、松开按住的键）；自动化改用它，自动化测试保持通过。
2. `inject.rs` 给模拟输入打 `dwExtraInfo` 标记；钩子记录最近一次真人输入时间；补 `mouse_move_by`（相对移动）与带按住时长的按键。
3. `game.look`：缩放、文字识别编号、画标签（照 OmniParser 的颜色与位置规则）、`observationId`、`zoom`；单元测试覆盖缩放换算、编号清单、过期编号。
4. `game.act`：动作校验、批量规则、`Not executed`、接手暂停、尺寸变化、`POINT_COVERED`、结尾自动再看；用假宿主做单元测试。
5. 工具定义（工具服务目录 + 琴音的常用工具）、按游戏授权（新配置字段、设置页撤销）、竞技对局默认不动、气泡“正在操作”状态、提示词条款、步数预算、旧截图从上下文里换成占位。
6. 真机测试：隔离的 KOTONE_HOME + 模拟 AI（脚本化的 look/act 调用）+ 一个测试窗口程序（例如自己写的小窗口或记事本），验证点击位置、相对移动、接手暂停、停止热键、独占全屏报错。不要在用户的真实游戏对局里测。
7. 文档：工具服务手册、琴音说明、CHANGELOG、第三方声明（Anthropic quickstarts、Cradle、GameVerse、UFO²、dsh-computer-use-win 的 MIT 声明；UI-TARS 的 Apache-2.0；OmniParser 的 CC-BY-4.0 署名）。

## 待用户确认

1. 竞技对局默认不动——是否同意，以及默认名单（LoL、无畏契约之外还要哪些）。
2. 停止键用 Ctrl+Shift+F12（现有）+ 语音“停下”，不用 Esc——是否同意。
3. computer use 的步数 / 时长预算：30 步、5 分钟是否合适。
4. 屏幕边缘发光的接管提示要不要第一版就做（dsh-computer-use-win 有）。

## 实施记录（2026-11-17）

**待确认的 4 个问题**：用户这次没有填写答案，按本 ADR 的推荐实现，可以随时改：

1. 竞技对局默认不动：名单是 `League of Legends.exe`、`VALORANT-Win64-Shipping.exe`、`cs2.exe`、`TslGame.exe`（PUBG）、`r5apex.exe`（Apex）、`Overwatch.exe`（`kotone_core::profile::COMPETITIVE_PROCESSES`）。CS2、PUBG、Apex、守望先锋的大厅和对局是同一个进程，所以这几个游戏整个都默认不动。
2. 停止：停止自动化热键（默认 Ctrl+Shift+F12）、托盘“停止自动化”、语音“停下”（先唤醒；整句只有停止的意思才算，例如“停下”“别动了”“住手”）。
3. 预算：用上 `game.look` / `game.act` 的那一轮放宽到 30 步、5 分钟（两个入口都是）。
4. 屏幕边缘发光：第二版再做；第一版只有气泡“正在操作：点击（2/5）”。

**与设计的出入**：

- **按游戏授权的询问**：没有新做“这次 / 一直 / 不允许”三个按钮，而是复用琴音现有的确认（聊天窗口“允许 / 本段对话都允许 / 拒绝”，语音“好 / 不要”），原因是“一直 / 不允许”要持久保存在游戏配置里、并能在设置页撤销——这部分做成了游戏配置的新字段 `computerUse`（「游戏适配 → 让琴音操作」：先问我 / 一直允许 / 不允许）。设成“一直允许”后琴音不再问；用户把 `game.act` 的权限规则设成禁止、或只读模式时仍不执行。撤销（改成不允许）在下一次 `game.act` 生效，不会打断正在进行的一次（一次最多约 30 秒）。
- **游戏不在前台**：设计是“暂停等待”；实现是每步前最多等 5 秒，还不回来就以 `not_foreground` 停下（工具调用有 30 秒期限，不能无限等）。
- **时长**：一次 `game.act` 里 `hold_key` 与 `wait` 合计不超过 20 秒，保证在工具服务的 30 秒期限里做完并再看一眼。
- **玩家接手的判定**：不带 Kotone 标记的键盘、鼠标点击、滚轮都算；别的程序注入的单纯光标移动不算（光标校正会产生这类事件）；物理的鼠标移动算。
- **截图**：JPEG（质量 80）；没有文字识别模型时照样返回截图，只是没有编号。上下文里保留最近 3 张工具截图。
- **外部 Agent**：`game.look` / `game.act` 也在工具服务里，外部 Agent 不经过琴音的询问（与其他工具一致），但游戏配置的拒绝、竞技对局名单、输入锁同样生效。

**验证**：纯逻辑（动作校验、缩放、编号、标签、批量规则、按住的键松开、停止、遮挡不点）、钩子里的 Esc 判定、授权判定、工具结果带图、步数放宽、语音“停下”的识别、气泡状态都有单元测试；设置页的新选项有 Playwright 测试。**真机测试还没做完**：测试开始时发现用户正在英雄联盟对局里（前台是游戏），立即中止；测试脚本 `scripts/manual-e2e/computer_use_e2e.py` 已加上“前台不是测试窗口就退出”的保护，等用户不在游戏里时再跑。

