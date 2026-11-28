---
name: kotone-config
description: 帮玩家把 Kotone 配置好：语音识别、文字识别、她的声音、她用的 API 连接、呼叫键和别的热键、呼叫以后怎么听、语音唤醒、插件开关、游戏语音、管理员身份、各家服务商的 Key。玩家说“帮我配置 / 设置 Kotone”“下载模型”“换声音”“开唤醒”“改呼叫键”“开 / 关某个插件”“用 DeepSeek 就行”时先读这个。
---

# Kotone 配置

目标：玩家只给了一个 DeepSeek Key，也能把 Kotone 的功能配好。能用工具做的都用工具做，不让玩家自己翻设置页；改设置、下载、重启会先请玩家确认（权限规则自动处理，你照常调用）。跟玩家说位置时照 settings.describe 给的写法（「设置 → 热键」「琴音 → 唤醒」「插件 → 游戏语音」）。

## 谁来做

- 你和玩家说话时，配置用得上的只有 settings.describe / settings.get / settings.update、runtime.status、ui.open（和开着的插件给的几个）：改一两项设置自己来，先 settings.describe 查字段、可选值和生效方式，不认识的字段不要猜。
- 下面用到的别的工具（models.*、connections.*、speech.status、wake.*、hotkeys.*、elevation.*、runtime.start、app.restart、roles.update、voice_chat.install / test / default_mic）在后台：用 agent.start 交出去，task 里写清玩家选了什么、“照 kotone-config 做”。后台不能问玩家，要玩家拿主意的（在线还是本地、用哪个键）先问好再交。

## 先看现状（一次查清，再说话）

1. settings.get：语音识别（sttEngine、activeModelId）、文字识别（ocr.activeModelId）、她的声音（assistant.ttsModelId，空 = 自动）、语音唤醒（assistant.wake.enabled）、插件开关（plugins.<id>.enabled）。Key 不在里面。
2. 要细的交给后台查：connections.list（有哪些连接、hasApiKey）；models.list，category 分别是 asr / ocr / tts / search（active、下没下、ready）；speech.status（她现在用哪家声音、怎么计费、Key 从哪来）；wake.status。
3. 本地语音识别要“启动”（runtime.status 看）；在线识别打开 Kotone 就能用，没有启动这回事。

查完用一两句话告诉玩家现状和建议，然后只问一个问题，不要一口气列出所有设置。

## 推荐方案 A：只用 DeepSeek（最省事，不用下载大模型）

都用她的 DeepSeek 连接和 Key，不用另选连接：

| 功能 | 怎么设 |
| --- | --- |
| 语音识别（说话 → 文字） | `models.activate`，`category: "asr"`，`engineId: "deepseek-asr"`，`modelId: "deepseek-asr"` |
| 文字识别（看游戏画面、聊天） | `models.activate`，`category: "ocr"`，`modelId: "deepseek-ocr"` |
| 她的声音 | 默认“自动”就行：她用 DeepSeek 就是 DeepSeek 的声音 |
| 联网搜索 | 默认“自动”：有 DeepSeek 连接就用 DeepSeek（每次成功约 0.005 元） |

要告诉玩家：在线识别会把**录音**和**游戏截图**发给 DeepSeek，按量计费（很便宜，但不是零）。

## 方案 B：本地模型（不上传、要下载）

- 语音识别：models.list（asr）里挑本地的（推荐 X-ASR 流式中英），models.download，下完 models.activate，再 runtime.start 启动。每次打开 Kotone 自动启动是 ui.autoStart。
- 文字识别：ocr 里的 PP-OCRv5 本地包，按游戏语言选（国服和多数英文客户端选中英日那个）。
- 她的声音：models.activate（tts）选 `system-speech`（Windows 系统语音，不用下载）或 `kokoro-zh`（本地中文，先 speech.model.download，约 150 MB）。系统语音不能用在游戏语音里。
- 下载期间可以继续聊；下完会有结果，失败如实说原因（自动下载不行时 models.install_guide 给手动安装的办法）。

玩家没说偏好时推荐 A；在意隐私或网络不好时推荐 B。也可以混用（例如语音识别本地、文字识别 DeepSeek）。

## 她的声音、她用的模型

- 声音只在「琴音 → 交互 → 声音与弹幕」一处配：选服务商（DeepSeek、MiniMax、阿里云百炼、硅基流动、火山、自定义）、填 Key、选声音、试听。换服务商用 models.activate（tts）；声音、模型写 settings.update `modelConfigs.<ID>.voice / model`，可选的声音看 speech.status；speech.status 带 check 能先试一次 Key 和设置。回到“自动”：`assistant.ttsModelId` 改成空、`assistant.speechEngine` 改成 `system`。
- 回答用语音还是弹幕（`assistant.defaultDelivery`）也在那一页；悬浮窗的开关、停留时间、小窗的宽度和最高在「琴音 → 交互 → 悬浮窗」。
- 她用哪条 API 连接、哪个模型：玩家在聊天页输入框旁的“她用的模型”里选。要你改就 settings.describe “API 连接” 看该写哪个字段（0.1.x 升级用户是另一个）。

## Key

- **永远不要让玩家把 Key 发在聊天里**，也不要在回答里复述 Key。Key 都存在 Windows 的安全存储里，你看不到。
- 要 Key 就用 connections.request_key（你自己调，后台用不了；provider 用 connections.presets 里的 id），聊天里会弹出只给玩家看的输入框。玩家在游戏里用语音叫你时，告诉玩家回 Kotone 窗口里粘贴。
- 她的声音的 Key 在「声音与弹幕」那一页填；DeepSeek、MiniMax、百炼的声音会自动用同一家 API 连接的 Key。火山、讯飞的语音识别 Key 在「设置 → 模型 → 语音识别」那一项里填。

## 呼叫、热键和唤醒

- 热键只在「设置 → 热键」一页改。先 hotkeys.status：每个热键现在的键、存在哪个字段、有没有冲突（呼叫键那一行的命令是 `kotone.call`）；改之前 hotkeys.conflicts `{key, except: "kotone.call"}`，没冲突再 settings.update 那一行写的字段。呼叫键默认 Ctrl + ~（`Ctrl+Backquote`）；单独的 ~ 是英雄联盟的“只攻击英雄”键，别推荐。停下热键默认 Ctrl+Shift+F12，语音输入的说话键默认 CapsLock。
- 快捷指令（按一下替玩家跟你说一句）、运行自动化的热键也在热键表 `hotkeys.bindings` 里（看 settings.describe “热键表”）；要确认的工具绑热键要玩家在「设置 → 热键」确认一次。
- 呼叫以后怎么听（`assistant.callMode`，「琴音 → 唤醒」）：说一句就走（默认）/ 录音笔 / 通话。
- 语音唤醒：先 wake.model.download（约 5 MB），再 settings.update `assistant.wake.enabled: true`。唤醒词默认“琴音琴音”，角色可以有自己的（roles.update 的 wake）。麦克风会一直开着（只在本机检测），建议戴耳机。
- 游戏里按呼叫键、往游戏里打字没反应：elevation.status 看是不是游戏以管理员身份运行而 Kotone 不是。“启动时以管理员身份运行”（`runAsAdminOnStart`，默认开）下次启动生效；现在就要用 elevation.restart（Windows 会问一次）。
- 改完看结果里的 `effects`：要重启语音识别时问玩家要不要现在 runtime.start；要重启 Kotone 时问要不要 app.restart。不要不问就重启。

## 插件

- 左侧栏「插件」里每个插件一张卡片、一个开关：settings.update `plugins.<id>.enabled`。有哪些、默认开不开、数据开关（`sources.*`）和推送开关（`pushes.*`）看 settings.describe “插件”。关着的插件没有页面和工具；开了以后，新交给后台的事才有它的工具。
- 有风险的开关（读英雄联盟客户端、驱动 BetterGI……）照开，告诉玩家插件「设置」里的“风险和许可”有说明。
- 第三方插件（别人的 MCP 服务器）只能玩家自己在「插件」页加：粘贴配置或从 Claude Desktop、VS Code 导入，看清它会运行什么、点信任。你不替玩家加、不改 mcp.json，也不能替玩家信任。

## 游戏语音（替玩家跟队友说话）

只在玩家想要时做：先开插件（`plugins.game-voice.enabled: true`，默认关），再 voice_chat.status 看缺什么 → voice_chat.install（装虚拟麦克风 VB-CABLE，Windows 会问一次要不要以管理员身份安装）→ 游戏里麦克风选 “CABLE Output”（只认 Windows 默认麦克风的游戏用 voice_chat.default_mic）→ voice_chat.test 确认通了。

## 收尾

配完用一两句话总结改了什么、还差什么（例如“游戏语音没开，需要时跟我说”），并提醒：在聊天里点工具卡片上的设置名，能直接跳到对应的设置项看。
