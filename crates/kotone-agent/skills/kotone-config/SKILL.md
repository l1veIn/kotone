---
name: kotone-config
description: 帮玩家把 Kotone 配置好：语音识别、文字识别、语音合成、唤醒词、呼叫热键、游戏语音、其他服务商的Key。玩家说“帮我配置 / 设置 Kotone”“下载模型”“换语音”“开唤醒”“用 DeepSeek 就行”时先读这个。
---

# Kotone 配置

目标：玩家只要给了一个 DeepSeek Key，就能把 Kotone 的全部功能配好。每一步都用工具做，不让玩家自己去翻设置页；改设置、下载、换模型会先请玩家确认（权限规则自动处理，你只管调用）。

## 先看现状（一次查清，再说话）

1. `connections.list`：有哪些连接、有没有保存Key（`hasApiKey`）。有 DeepSeek 连接且有Key = 可以走“只用 DeepSeek”。
2. `models.list`，`category` 分别是 `asr`（语音识别）、`ocr`（文字识别）、`tts`（语音合成）：看现在用的是哪个（`active`）、哪些已下载。
3. `wake.status`：唤醒词开没开、唤醒模型下没下。
4. `runtime.status`：Kotone 有没有在运行（没运行时语音入口都不能用，`runtime.start` 启动）。
5. 需要时 `settings.describe` 查字段路径、可选值、生效方式。不认识的字段不要猜。

查完用一两句话告诉玩家现状和你的建议，然后问一个问题，不要一口气列出所有设置。

## 推荐方案 A：只用 DeepSeek（最省事，不用下载大模型）

都复用琴音的 DeepSeek 连接和Key，不用再选连接：

| 功能 | 怎么设 |
| --- | --- |
| 语音识别（玩家说话 → 文字） | `models.activate`，`category: "asr"`，`engineId: "deepseek-asr"`，`modelId: "deepseek-asr"` |
| 文字识别（读游戏画面、聊天） | `models.activate`，`category: "ocr"`，`modelId: "deepseek-ocr"` |
| 语音合成（琴音说话） | 默认“自动”就会用 DeepSeek 语音（`assistant.ttsModelId` 为空），不用改；想换声音请玩家在「琴音 → 设置 → 回答」里试听后选 |

要告诉玩家：在线识别会把**录音**和**游戏截图**发给 DeepSeek，按量计费（很便宜，但不是零）。

## 方案 B：本地模型（不上传、要下载）

- 语音识别：`models.list`（`asr`）里挑推荐的本地模型（例如 X-ASR 流式中英，约 160 MB），`models.download` 下载，完成后 `models.activate`。
- 文字识别：`ocr` 里的 PP-OCRv5 本地包（按玩家的游戏语言选，中英日包最常用）。
- 语音合成：系统语音不用下载；Kokoro 本地中文约 300 MB（`speech.model.download`）。
- 下载期间可以继续聊；下载完会有结果，失败如实说原因。

玩家没说偏好时推荐 A；玩家在意隐私或网络不好时推荐 B。也可以混用（例如语音识别本地、文字识别 DeepSeek）。

## 呼叫和唤醒

- 呼叫琴音的热键默认是 `~` 键（`assistant.hotkey` = `Backquote`），按一下就在右下角气泡里听。玩家想换就 `settings.update`。
- 唤醒词（“琴音琴音”/“你好琴音”）：先 `wake.model.download`（约 5 MB），再 `settings.update` 把 `assistant.wake.enabled` 设为 `true`。会一直开着麦克风，建议戴耳机。
- 改完看结果里的 `effects`：需要重启语音识别时问玩家要不要现在 `runtime.start`；需要重启 Kotone 时问要不要 `app.restart`。不要不问就重启。

## 其他服务商的Key（不用 DeepSeek 时）

- **永远不要让玩家把Key发在聊天里**，也不要在回答里复述Key。
- 用 `connections.request_key`：传服务商（`provider`，可选值看 `connections.presets`），Kotone 会弹出一个只给玩家看的输入框，Key直接存进系统凭据库，你只会收到“已保存”。玩家在游戏里用语音叫你时，告诉他到 Kotone 窗口里粘贴。
- 存好后再按上面的方式 `models.activate` 对应的在线模型，或 `settings.update` 选连接。

## 游戏语音（替玩家跟队友说话）

只在玩家想要时做：`voice_chat.status` 看缺什么，按 `voice_chat.install`（装虚拟麦克风，会弹一次管理员授权）→ 开启 `voiceChat.enabled` → 游戏里麦克风选 “CABLE Output”（只认 Windows 默认麦克风的游戏用 `voice_chat.default_mic`）→ `voice_chat.test` 确认通了。

## 收尾

配完用一两句话总结改了什么、还差什么（例如“游戏语音没开，需要时跟我说”），并提醒：在聊天里点工具卡片上的设置名，能直接跳到对应的设置项看。
