# ADR 017：语音唤醒琴音与执行中的语音接话

- 状态：Accepted（2026-11-17）。用户授权按推荐方案实施；第一版已实现，见文末“实施记录”
- 细化：[ADR 014](014-game-agent-harness.md)（游戏内入口、同一段对话）、[ADR 006](006-session-lifecycle-interaction-modes.md)（交互模式里一直留着的“A3 全时免按，Phase 3”）、[ADR 007](007-vad-one-shot-mode.md)（VAD 静音判停）、[ADR 010](010-sound-feedback.md)（提示音）
- 不变：游戏语音输入（发到游戏聊天）的所有模式与热键；`assistant.interactionMode` 的取值范围
- 2026-11-27 修订（[ADR 048](048-her-voice-in-one-place.md)）：下文 TTS 一行的在线引擎现在是 DeepSeek、MiniMax、阿里云百炼、硅基流动、火山和自定义（OpenAI 兼容接口，原来叫“OpenAI 兼容”）

## 问题

现在琴音只能靠按键叫醒：按住陪玩热键说话、按快捷指令，或在聊天窗打字。用户希望：

1. 启动后像小爱同学一样**喊一声就醒**，接着说的话直接进琴音的对话；
2. 醒了先用“嗯？”“我在”接住人；
3. 执行过程中也用语音说点话，别让人干等；结束时说结果。**语音播报是这项功能的核心体验。**

## 现状（读代码得到的事实）

| 方面 | 现状 | 位置 |
| --- | --- | --- |
| 麦克风何时打开 | **只在会话开始时打开**：每次 `begin` 调 `AudioBackend::start`（cpal，WASAPI 共享模式），会话结束 drop 句柄即关闭 | `crates/kotone-core/src/orchestrator.rs` `begin_locked`；`crates/kotone-platform-windows/src/audio.rs` |
| 采集格式 | 16 kHz 单声道 f32，每 50 ms 一块，队列上限 2 秒，超了中止会话 | 同上 `CHUNK_SAMPLES`、`PCM_QUEUE_CAPACITY` |
| 会话模型 | orchestrator 是唯一状态所有者，**同一时刻只有一个会话**（游戏语音输入或助手），用途 `SessionPurpose::{Dictation, Assistant}` | `orchestrator.rs` |
| 开录即打断播报 | `before_capture` 先给 `active_captures` 加一、再 `cancel_speech()`；录音期间提醒语音最多等 30 秒，否则改弹幕 | `apps/desktop/src-tauri/src/assistant.rs` `before_capture`、`queue_speech` |
| VAD | silero（随本体分发，643 KB），one-shot 静音判停默认 700 ms，可配 200–5000 ms；**只有“说过话后的静音”才判停，开头一直不说话不会结束**（只有 10 分钟总上限） | `crates/kotone-core/src/vad.rs` `SilenceStopTracker` |
| 交给琴音 | 助手会话结束 → `RoutingHandler` → `agent::submit_from_assistant(app, text, "assistant-voice")` → `agent::send`：没在跑就在 `store.current()` 那段对话开新一轮；在跑就 `inbox.steer` 作为插话 | `assistant.rs`、`agent.rs` |
| 游戏内快速模式 | 来源不是 `chat`/`assistant-text` 的都算游戏内：先读屏、最多 6 步 / 60 秒；结束后 `speak_outcome` 播最后的回答（已经 `reply.say` 过的不重复，玩家主动停止的不播） | `agent.rs` `entry_of`、`game_limits`、`game_reply` |
| `reply.say` | 模型可调用的内置动作，≤300 字，语音和/或弹幕；**调用一次算模型的一步**（占 6 步上限） | `crates/kotone-agent/src/catalog.rs`、`port.rs` |
| TTS | 系统语音（SAPI，直接出声，不缓存）/ Kokoro（sherpa-onnx，整句合成后再播，2 线程，首次播报才加载，空闲 60 秒释放）/ 在线（OpenAI 兼容、火山）。Kokoro 与在线结果按“声音+语速+文本”在内存缓存 64 条 | `crates/kotone-tts/src/engine.rs`、`cache.rs`、`docs/assistant.md` |
| sherpa-onnx | `sherpa-onnx = 1.13.5`（官方 Rust 绑定，静态链接），STT、VAD、OCR、Kokoro 共用一份原生库 | `crates/kotone-stt/Cargo.toml`、`crates/kotone-tts/Cargo.toml` |
| **KWS 接口** | 该版本 crate **已带** `sherpa_onnx::KeywordSpotter`：`create`、`create_stream`、`create_stream_with_keywords`、`decode`、`get_result`、`reset`；配置可直接传 `keywords_buf` 字符串，不必写文件 | `~/.cargo/registry/.../sherpa-onnx-1.13.5/src/kws.rs` |
| 流式识别器 | X-ASR recognizer 懒加载后在进程内复用，唤醒后开会话不用重新加载模型 | `crates/kotone-stt/src/online_transducer.rs` |

## 决策（推荐方案）

### 1. 唤醒引擎：sherpa-onnx 关键词检测（KWS）

- **用 sherpa-onnx 自带的 KWS**：项目已经链接同一份原生库，Rust 接口现成，不加新的原生依赖，不联网、不要账号。
- **模型**：`sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`（中文，拼音声母+韵母建模，约 3.3M 参数）。
  - 用到的文件：int8 encoder 4.78 MB + fp32 decoder 0.68 MB + int8 joiner 0.07 MB + tokens 1.6 KB，合计约 **5.5 MB**（整包 tar.bz2 32.6 MB，含 fp32 与两套 epoch）。
  - 实测 int8 decoder 漏掉了一条快语速的“你好琴音”，所以 decoder 用 fp32（见“验证记录”）。
  - 备选：`sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20`（中英，chunk-8 约 160 ms 延迟），英文唤醒词只能用 CMU 词典里有的词（“KOTONE”不在词典里，转换时被跳过）。
- **唤醒词**：开放词表，不用重新训练。关键词写成 token 序列，例如 `q ín y īn q ín y īn @琴音琴音`，每条可以加 `:加权` 和 `#触发阈值`。
  - 第一版只给**预置词**，token 在代码里写死（用 `sherpa-onnx-cli text2token --tokens-type ppinyin` 转出来的，已验证）：
    - `琴音琴音` → `q ín y īn q ín y īn`
    - `你好琴音` → `n ǐ h ǎo q ín y īn`
  - **不提供单独的“琴音”**：两个音节太短，实测把“秦音也来了”误判成唤醒。
  - 自定义唤醒词放到第二版：Rust 端要自己把汉字转成带声调的声母/韵母（如 `pinyin` crate），多音字要让用户确认读音。
- **灵敏度**：三档映射到 `keywords_threshold` 与加权分（低 / 中 / 高，具体数值要用真机录音调，默认值是阈值 0.25、加权 1.0）。

为什么不选别的：

| 方案 | 不选的原因 |
| --- | --- |
| openWakeWord | 代码 Apache-2.0，但官方预训练模型是 CC BY-NC-SA 4.0；自定义词要用 TTS 合成数据自己训练，现成流程以英文为主；要再接一套推理（ONNX/tflite） |
| Picovoice Porcupine | 支持中文，但必须带 AccessKey、初始化时联网校验、按套餐限制用量；自定义词文件有平台、期限、商用限制；社区帖称免费档 AccessKey 2026-06-30 后失效（未在官方页面核实） |
| 一直开 ASR 找“琴音” | 流式 ASR 常开的 CPU 和误识别都比 3M 的 KWS 高，还得自己做匹配 |
| 只用 VAD 免按（ADR 006 的 A3） | 游戏里一直有人说话，任何人声都会变成指令，不可用 |

### 2. 音频：一路共享麦克风，唤醒监听与识别分开消费

- 新增**麦克风集线器**（`kotone-platform-windows`，暂叫 `MicHub`）：唤醒打开时常开**一路** cpal 输入流，向订阅者分发 16 kHz 块，同时保留最近 **2 秒**环形缓冲（只在内存里）。
- **唤醒监听**（`WakeService`，桌面壳）订阅集线器，把音频喂给 KWS。
- **唤醒后开的那次助手会话**用一个新的 `AudioBackend` 实现：订阅同一个集线器，并先补上“唤醒词结束之后”的缓冲音频。这样“琴音琴音帮我看看队友说了什么”连着说也不丢开头。
- **热键会话不变**：游戏语音输入和陪玩热键仍走现有 `CpalBackend`，各自开流。WASAPI 共享模式允许同一设备同时有多路采集，所以两边不会抢占设备。
- **互斥规则**：orchestrator 不是 Idle（任何会话：游戏语音输入、陪玩、预览、发送中）时，唤醒检测**暂停判定**（流可以继续开着，只是不判）。这样在给队友打字的语音里说到“琴音”也不会误唤醒。
- **唤醒监听不算“录音中”**：不能给 `active_captures` 加一，否则提醒和回答会一直被挡住。只有唤醒后真正开始收指令时才走 `before_capture`。
- 设备或 `audioDeviceId` 改了、运行时停止、设置关闭时，集线器重开或关闭。

开销估算：

- CPU：本机（Ryzen 7 9800X3D）单线程 KWS 的实时率约 0.011，也就是一个核心的 **1% 左右**，约等于 16 线程总量的 0.07%。cpal 采集与重采样另计（没测）。弱一些的 CPU 按 2–4 倍估算，也只占一个核心的几个百分点，这是估算，没实测。
- 内存：Python 测试进程加载模型后 RSS 增加约 51 MB（含 ONNX Runtime 会话）。Kotone 已经加载了 ONNX Runtime，增量应该更小，没实测。环形缓冲 2 秒约 128 KB。

### 3. 唤醒之后的流程

```
KWS 命中 ──► orchestrator 不空闲？──是──► 丢弃（记日志）
   │否
   ▼
看唤醒词后 300 ms：还在说话？──是──► 不应答，直接收指令（缓冲从唤醒词结束处补）
   │否
   ▼
播应答“嗯？”/“我在”（预合成、缓存的 PCM，约 0.3–0.5 s）──► 播完开始收指令（缓冲从应答播完处补，丢掉应答本身的回声）
   ▼
助手会话：VAD 静音判停（沿用 vadSilenceMs，默认 700 ms）＋ 新增「一直没开口」超时 5 s
   │超时 ──► 安静结束：悬浮窗收起 + 下沉提示音，不调 AI
   ▼
识别结果 → submit_from_assistant(text, "wake-voice") → 现有游戏内快速模式 → 语音/弹幕播结果
```

- **连着说就不应答**：参照音箱类助手的做法。连着说的人不需要被“接住”，这样还避开了应答声被麦克风收进去的问题。
- **应答必须零等待**：
  - 应答、填充语都是固定的几句。唤醒开启时（以及换声音、换语速后）**预先合成**放进现有缓存，命中后直接播 PCM。
  - 系统语音（SAPI）现在直接出声、不经过 PCM，应答要改成“合成到内存再播”。实测 SAPI 合成“嗯？”只要 18 ms，但音频开头有 120–170 ms 静音、结尾约 0.8 s 静音，要裁掉。
  - Kokoro 在首次播报时才加载、空闲 60 秒就释放：唤醒命中就**并行预加载**，用户说指令的 2–4 秒足够遮住加载时间（加载耗时没实测）。
- **“一直没开口”超时**：现有 `SilenceStopTracker` 没有这条，唤醒会话必须加，否则会一直开到 10 分钟上限。
- **悬浮窗**：复用现有 Listening/Processing 状态，载荷加 `trigger: "wake"`。显示“在听…”→“想一想…”→“在说…”，独占全屏时只靠声音。
- **打断（barge-in）**：
  - 琴音在说话时再喊唤醒词：停掉当前播报（现有 `before_capture` 已经会 `cancel_speech`），开新的收音。琴音还在执行的话，这句话按现有规则作为**插话**交给她。
  - 按热键打断恒在，行为不变。
  - 第一版**不做**“直接开口就打断”（不喊唤醒词）：外放时自己的声音会触发，这要等回声消除。

### 4. 对话与上下文

业内做法：

| 产品 | 做法 |
| --- | --- |
| Google（Continued Conversation） | 默认关；回答后约 **10 秒**内可以直接追问，说“谢谢”“停”或不说话就结束；任何人都能接着说（现行帮助页；2018 年发布时为 8 秒） |
| Alexa Follow-Up Mode | 默认关；回答后约 **5 秒**内蓝灯亮着可以追问；不确定你在对它说话时不进入；放音乐、通话时不启用（2018 年发布报道） |
| Alexa Conversation Mode（Echo Show 10） | 开启后不用唤醒词，靠摄像头看人有没有朝向屏幕，一段时间没互动自动退出 |
| Siri（iOS 17） | 回答后可以接着下一个请求，不用再喊（时长未查到） |
| 小爱同学 | 连续对话需要手动开启，唤醒一次可以连着说，可以随时打断（秒数未查到官方说明） |
| ChatGPT 语音 | 语音就发生在当前聊天里，转写写进同一段聊天记录，之后可以接着打字 |
| 豆包语音通话 | 全双工，随时打断；通话是否写入同一对话未查到 |

Kotone 的推荐：

- **唤醒指令进哪段对话**：与陪玩热键完全一样，进 `store.current()`，也就是 ADR 014 定的“游戏内与聊天窗是同一段对话”。来源记为 `wake-voice`，属于游戏内入口：快速模式、先读屏、语音或弹幕回答。
- **多久算新话题**（需要用户定）：推荐游戏内来源（热键、唤醒、快捷指令）的消息到来时，如果当前对话最后一条消息已超过 **30 分钟**，自动新建对话。聊天窗口会切过去显示，旧对话仍在列表里。这同样会改变热键的行为，所以要用户确认。另外第二版支持说“新话题 / 换个话题”。
- **与聊天窗里正在跑的一轮共存**：琴音正在为聊天窗干活时被唤醒，按现有规则作为插话进入那一轮。
  - 按代码推断，有一个现成的缺口：插话不会改变这一轮的入口。那一轮如果是聊天窗发起的，结束时不会用语音播结果，用户在游戏里就听不到回答。热键插话也有这个问题。
  - 修法：这一轮里只要有游戏内来源的插话，结束时就按游戏内规则播报。实现时先写测试确认。
- **免唤醒追问**：第二版加，**默认关**，开启后默认 6 秒。只在琴音用语音播完最后回答后打开；用 VAD 判断有没有人开口；悬浮窗显示“还在听”。任何热键、游戏语音输入都会立即关闭这个窗口。游戏里队友语音和自己跟朋友说话很多，这个窗口的误触发风险比音箱大，所以不默认开。

### 5. 执行中的语音接话

业内做法：

- Alexa 的 Progressive Response：处理中先播中间语句，每次请求最多 5 条，正式回答发出后不能再发。
- LiveKit Agents：工具执行时播**预合成**的等待语，接口很快返回就提前取消；或者在“思考中”状态放循环的思考音。社区也指出，在工具调用前让模型先说一句，会造成停顿。

Kotone 的推荐：**由宿主按规则播，不让模型生成**。模型生成的话，每句都要多一次模型调用，还占快速模式 6 步里的一步。

| 时机 | 说什么 | 规则 |
| --- | --- | --- |
| 唤醒 | “嗯？”/“我在” | 见 §3，连着说就不说 |
| 交给琴音后 **1.5 秒**仍没有任何语音输出 | 轮换的固定短句：“好，我看看”“收到，稍等”“嗯，马上” | 预合成缓存；1.5 秒内最后回答已经好了就不说 |
| 执行中，遇到慢工具开始执行 | 按工具分组的固定模板：读屏 → “我看一下屏幕”，运行自动化 → “开始执行了”，打开游戏 → “正在打开游戏” | 距上一次出声至少 **8 秒**；每轮最多 2 句；模型自己用 `reply.say` 说过就算一次出声 |
| 结束 | 最后的回答或出错原因 | 现有 `speak_outcome` 不变 |

- 所有接话走同一把输出锁（`output_lock`），优先级为：最后回答 > `reply.say` > 填充语。
  - 最后回答好了时，排队中的填充语直接丢弃。
  - 填充语正在播就让它播完（≤1 秒），再播回答，不打断。
- 新开关 `agent.progressSpeech`（默认开），对热键入口同样生效。
- **TTS 引擎建议**：
  - 默认沿用用户选的引擎；固定短句一律预合成缓存，跟引擎快慢无关。
  - 最后回答每次文本都不一样，没法缓存。Kokoro 是整句合成完才播，在线引擎多一个网络来回；这两者的首句延迟都没实测。
  - 推荐：装了 Kokoro 时用 Kokoro（本地、不上传、音质好于 SAPI），唤醒时预加载。
  - 第二版考虑把较长的回答按句子切开，合成一句、播一句。

### 6. 误唤醒的来源与缓解

| 来源 | 第一版 | 以后 |
| --- | --- | --- |
| 琴音自己的 TTS 被麦克风收进去 | 播报文本里含唤醒词读音（qin yin）时，播放期间加 300 ms 暂停判定；应答和填充语都不含唤醒词。其余时间照常监听，这样戴耳机的人可以喊停 | Windows 回声消除，见下 |
| 游戏声、队友语音（外放） | 唤醒词用 4 个音节；灵敏度三档；设置页提示“建议戴耳机”；日志记录每次唤醒，方便统计误唤醒 | 唤醒命中后用已加载的流式 ASR 复核唤醒词前后 1.5 秒音频，读音对不上就不应答。识别器已缓存，开一个新流成本很低；要不要默认开，先看真机误唤醒数据 |
| 玩家自己在语音里跟队友提到“琴音琴音” | 视为正常唤醒；说“没事”或按 Esc / 热键取消 | — |
| 游戏语音输入会话中说到唤醒词 | 会话期间暂停判定（§2） | — |

- **Windows 回声消除**，第三版评估：
  - `IAcousticEchoCancellationControl` 可以指定回声参考的输出设备，要求 Windows 11 22H2（build 22621）及以上，而且采集端点得有支持它的 AEC 效果。拿不到接口就说明不支持，需要用 Communications 类别的原生 WASAPI 打开，cpal 不支持。
  - 老的 Voice Capture DSP（`CLSID_CWMAudioAEC`）是 DMO，带回声消除、降噪、增益和 VAD。
  - 两者在用户机器上的可用性与效果都没验证。

### 7. 隐私与设置

- **全本地**：
  - 唤醒检测在本机运行，不联网，不存音频；环形缓冲只在内存里，最多 2 秒。
  - 唤醒后的指令和现在的陪玩热键一样，按用户选的 ASR、AI、TTS 走，在线服务的数据边界不变。
  - 文档和 PRIVACY.md 要写明：开启后 Windows 任务栏会**一直显示“麦克风使用中”**。
- **默认关**。在「琴音陪玩 → 设置」里开启，开启时说清楚“常开麦克风、本地检测、建议戴耳机”。托盘菜单加一个开关。
- **前提**：语音运行时已启动、`agent.inGame` 开着、有可用的 AI 连接（与陪玩热键相同）。
- **新字段**（遵守 `docs/development.md` 的配置兼容规则：旧字段只写旧版认识的值，新能力用新字段）：

```jsonc
"assistant": {
  // …原有字段不动，assistant.interactionMode 不加新值…
  "wake": {
    "enabled": false,
    "phrase": "qinyin-qinyin",      // 预置：qinyin-qinyin | nihao-qinyin（第二版加 custom）
    "sensitivity": "normal",        // low | normal | high
    "ack": "voice",                 // voice（“嗯？”/“我在”）| tone（ADR 010 提示音）| none
    "noSpeechTimeoutMs": 5000,
    "followUpSeconds": 0            // 第二版：0 = 关
  }
},
"agent": {
  "progressSpeech": true            // 执行中的语音接话（热键入口同样适用）
}
```

- 旧版本读到 `assistant.wake`、`agent.progressSpeech` 时会忽略。代价是用旧版保存一次后，这两个字段会丢。按现有惯例在 `kotone-core` 加回归测试。

## 分阶段实施

**第一版（小而可用）**

1. `kotone-stt`：`wake` 模块（feature `wake-kws`，依赖已有的 `sherpa-onnx`），封装 `KeywordSpotter`，预置两个词的 token。模型随本体分发，同 silero 的做法（或改为下载，见待定 2）。
2. `kotone-platform-windows`：`MicHub`（一路流、订阅、2 秒环形缓冲）＋ 带缓冲补齐的 `AudioBackend`。
3. `kotone-core`：
   - 助手会话支持“由唤醒开始”的策略：开始 = 唤醒，结束 = VAD 静音，提交 = 直接；
   - 新增“一直没开口超时”；
   - 新增 `assistant.wake`、`agent.progressSpeech` 配置字段与兼容测试。
4. 桌面壳 `wake.rs`：
   - 状态机：关 / 监听 / 暂停（原因）；
   - 暂停规则：有会话、热键录入中、播报文本含唤醒词；
   - 应答判定、预合成应答与填充语；
   - 命中时预加载 Kokoro；
   - 来源记为 `wake-voice`；
   - 托盘开关、悬浮窗状态、诊断日志（命中、超时、丢弃次数）。
5. 修“插话不播报”的缺口（§4）。
6. 1.5 秒填充语（§5 第二行）。
7. 设置页：开关、唤醒词、灵敏度、应答方式；文档更新 `docs/assistant.md`、`PRIVACY.md`、`THIRD_PARTY_NOTICES.md`、CHANGELOG。
8. 测试：
   - 假的检测器驱动的状态机单元测试；
   - 录音回放回归：参照 `crates/kotone-stt/tests/fixtures`，加正例、反例和“连着说”样本；
   - 真机：戴耳机和外放各在一款游戏里挂机 1 小时以上，统计误唤醒和漏唤醒，用任务管理器看 CPU。

**第二版**：免唤醒追问窗口；慢工具进度播报模板；30 分钟新话题规则与“新话题”口令；ASR 复核唤醒词；自定义唤醒词（汉字转拼音，多音字确认）；长回答分句合成、边合成边播。

**第三版**：Windows 回声消除（`IAcousticEchoCancellationControl` / Voice Capture DSP），之后才考虑“直接开口打断”；按游戏配置的灵敏度。

**已提前做（2026-11-17，用户提出）**：琴音主动追问——内置工具 `reply.ask` 用语音问一句、不用唤醒词听回答几秒，回答交回这次提问；语音入口的权限确认用同样的方式问“这一步会……，可以吗？”，听懂“好 / 不要”就照办，没听清退回窗口按钮。语音入口也能 `tools.find`，上限放宽到 10 步 / 120 秒。与上面“免唤醒追问”（回答后开一个窗口等下一句）不同，这里只在琴音提问时才听。

## 风险

- **游戏噪声下的漏唤醒**：混同一个声音的人声干扰实测（最坏情形，像外放的队友语音），信噪比 10 dB 只检出 3/5，5 dB 2/5，0 dB 0/5。戴耳机时麦克风里主要是玩家本人的声音，情况会好得多，但没有真机数据。上游公开数据也说明，没微调的模型对唤醒词召回一般（“小云小云”70%，“你好问问”79%，误报 0），微调后能明显提高，但需要录音数据。
- **模型许可**：模型仓库标的是 Apache-2.0，但训练数据 WenetSpeech 的许可只允许非商用（论文与访问条款）。项目以前因为 CC-BY-NC 排除过 NLLB，这一项要用户定。zh-en 2025 模型没公开训练数据。
- **外放回声**：没有回声消除时，外放用户的“直接开口打断”和免唤醒追问都不可靠。所以第一版只做“喊唤醒词打断”。
- **常开麦克风的观感**：任务栏麦克风图标常亮，有的用户会介意。默认关，并写清楚。
- **与独占麦克风的软件冲突**：某些语音软件独占麦克风（WASAPI exclusive）时，共享流可能拿不到数据，没验证。
- **Kokoro 首句延迟**：整句合成再播，加载和合成耗时都没实测。最后回答的延迟可能是体验短板。
- **会话互斥带来的边界**：游戏语音输入正在预览、发送时喊唤醒词会被丢弃，用户可能觉得“没反应”。要在日志与悬浮窗里给出提示。

## 决定（2026-11-17）

用户把方案交给实施方决定（“方案你觉得差不多了就可以做了”）。按推荐执行，两处改得更保守：

| # | 问题 | 决定 |
| --- | --- | --- |
| 1 | 唤醒词 | 默认“琴音琴音”，可选“你好琴音”；不提供两个音节的“琴音” |
| 2 | 模型与分发 | **改为下载，不随安装包分发**：训练数据 WenetSpeech 只许非商用，项目以前也因类似原因排除过 NLLB。用户开启唤醒时从 k2-fsa 官方 GitHub release 下载整包（32.6 MB），只解出 4 个文件（约 5.5 MB），逐文件 SHA-256 校验；国内按现有下载设置走 GitHub 代理。ModelScope 上同名仓库的文件与官方包不是同一份导出（哈希不同），不能当镜像 |
| 3 | 默认开关 | 默认关 |
| 4 | 唤醒应答 | 说“嗯？/我在”，连着往下说时不应答；应答期间玩家开口就停掉应答、从开口处开始收音（实施时加的，见实施记录） |
| 5 | 唤醒后一直不说话 | 5 秒后安静结束（可在 2–15 秒之间设置） |
| 6 | 免唤醒追问 | 第二版 |
| 7 | 多久算新话题 | **这一版不做**：它会改变现有热键的行为，等用户确认 |
| 8 | Kotone 窗口在前台时唤醒 | 一律按游戏内处理 |
| 9 | 执行中接话频率 | 第一版只有 1.5 秒那句（`agent.progressSpeech`，默认开） |
| 10 | Kokoro 常驻 | 不常驻（沿用现有的按需加载） |
| 11 | 外放用户 | 只提示“建议戴耳机” |

## 考虑过的方案

- **唤醒后用现有 `CpalBackend` 新开一路流**：最省事，但会丢掉“唤醒词后紧接着说”的开头几百毫秒（开流耗时没测），所以改为集线器加缓冲补齐。
- **让模型用 `reply.say` 自己说“我看看”**：每句多一次模型调用，并占快速模式的步数，还会造成停顿。由宿主按规则播固定短句更快更省。
- **唤醒判定放进 orchestrator**：orchestrator 是“一个会话”的状态机，常开监听会让每个状态都要考虑“还在听唤醒”。放在壳层独立服务，只在空闲时向 orchestrator 发起会话，边界更清楚。
- **给 `assistant.interactionMode` 加一个 “wake” 值**：违反配置兼容规则，旧版本会把整份配置当作损坏。所以用新字段。

## 验证记录（2026-11-17）

**做了的：**

- 读代码确认了上面“现状”表各项，以及 sherpa-onnx 1.13.5 Rust crate 的 KWS 接口（`src/kws.rs`）。没有改仓库代码。
- 下载两个 KWS 模型到 scratchpad。用 `sherpa-onnx-cli text2token` 把“琴音琴音”“你好琴音”转成 token。
- 用 Python sherpa-onnx 1.13.4 做冒烟测试：
  - 测试音频是 Windows SAPI 慧慧合成的 6 条正例、5 条反例，前后各补 1 秒静音，按 100 ms 一块流式送入，单线程。

    | 模型 | 唤醒词 | 正例 | 反例误报 | 实时率 |
    | --- | --- | --- | --- | --- |
    | wenetspeech | 琴音琴音 + 你好琴音 | 5/5（第 6 条只说了“琴音”，按预期不触发） | 0/5 | 0.011 |
    | zh-en chunk-8 | 同上 | 5/5 | 0/5 | 0.015 |
    | 两个模型 | 单独“琴音” | 6/6 | **1/5**（“秦音也来了”） | — |

  - 命中时刻都在音频开始后 1.7–2.5 秒，其中前 1 秒是补的静音。
  - wenetspeech 换成 int8 decoder 后，漏了 1 条 +4 语速的“你好琴音”。
  - 正例混同声人声干扰：10 dB 3/5，5 dB 2/5，0 dB 0/5。阈值 0.15 时 5 dB 为 3/5。
  - Python 进程加载模型后 RSS 增加约 51 MB。
- SAPI 合成耗时：短句 4–18 ms（合成到内存）。音频开头静音 120–170 ms，结尾静音约 0.8 s。

**没做的：**

- 在 Rust / Kotone 进程里跑 KWS；
- 真人、真麦克风、真游戏环境；
- 长时间误唤醒率；
- 弱 CPU 的占用；
- Kokoro 与在线 TTS 的延迟；
- WASAPI 回声消除接口在本机是否可用；
- 多路共享采集与独占麦克风软件的兼容；
- “插话不播报”缺口是按代码推断的，没有运行验证。

调研脚本与音频不进仓库。

## 出处

- sherpa-onnx KWS 预训练模型（文件大小、text2token 用法、zh-en 模型延迟）：https://k2-fsa.github.io/sherpa/onnx/kws/pretrained_models/index.html
- sherpa-onnx KWS 关键词文件格式（`:` 加权、`#` 阈值、开放词表）：https://k2-fsa.github.io/sherpa/onnx/kws/index.html
- icefall WenetSpeech KWS 结果（召回、误报、微调前后）：https://github.com/k2-fsa/icefall/blob/master/egs/wenetspeech/KWS/RESULTS.md
- KWS 模型下载：https://github.com/k2-fsa/sherpa-onnx/releases/tag/kws-models ；模型包内 README（ModelScope 元数据 `license: Apache License 2.0`）：https://www.modelscope.cn/pkufool/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.git
- zh-en 2025 模型第三方模型卡（训练数据未公开）：https://huggingface.co/openEuler/sherpa-kws
- WenetSpeech 论文（数据非商用许可）：https://arxiv.org/abs/2110.03370 ；访问条款：https://huggingface.co/datasets/wenet-e2e/wenetspeech
- sherpa-onnx Rust KWS 示例：https://github.com/k2-fsa/sherpa-onnx/blob/master/rust-api-examples/examples/keyword_spotter.rs
- openWakeWord 模型许可：https://huggingface.co/davidscripka/openwakeword
- Picovoice 价格与 FAQ：https://picovoice.ai/pricing 、https://picovoice.ai/docs/faq/general ；Porcupine：https://github.com/Picovoice/porcupine ；免费档 AccessKey 失效的社区帖（未核实）：https://community.home-assistant.io/t/fyi-picovoice-confirmed-free-tier-accesskeys-will-stop-working-after-june-30-2026/1012744
- Google Continued Conversation（约 10 秒）：https://support.google.com/googlenest/answer/7685981 ；2018 年发布：https://blog.google/products/assistant/chatting-your-google-assistant-just-got-easier/
- Alexa Follow-Up Mode（5 秒，2018 报道）：https://www.macrumors.com/2018/03/09/amazon-alexa-follow-up-mode/ 、https://techcrunch.com/2018/03/10/now-you-can-have-a-conversation-with-alexa-without-screaming-hey-alexa-for-every-request/
- Alexa Conversation Mode：https://www.aboutamazon.com/news/devices/conversation-mode-helps-interactions-with-alexa-feel-more-natural 、https://techcrunch.com/2021/11/18/amazon-officially-launches-its-more-natural-conversation-mode-option-for-echo-show-10
- Siri iOS 17 连续请求：https://www.tomsguide.com/how-to/how-to-make-back-to-back-siri-requests-iphone
- 小爱同学连续对话（IT 之家）：https://www.ithome.com/0/460/198.htm
- ChatGPT 语音在聊天内进行：https://help.openai.com/en/articles/8400625-voice-mode-faq
- 豆包实时语音通话（可打断）：https://www.ifanr.com/1612635
- Alexa Progressive Response：https://developer.amazon.com/en-US/docs/alexa/custom-skills/send-the-user-a-progressive-response.html
- LiveKit 背景音与等待语：https://docs.livekit.io/agents/build/background-audio/ 、https://docs.livekit.io/agents/multimodality/audio/customization.md ；社区讨论：https://community.livekit.io/t/we-need-to-talk-about-pre-tool-call-speech/1152
- `IAcousticEchoCancellationControl`（Windows 11 22621+）：https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iacousticechocancellationcontrol-setechocancellationrenderendpoint
- Voice Capture DSP：https://learn.microsoft.com/en-us/windows/win32/medfound/voicecapturedmo
- WASAPI 共享 / 独占模式：https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams

## 实施记录

- 2026-11-17：玩家反馈唤醒后要等两秒多才应答（日志：开始应答到开始收音 2.1–2.4 秒，在线语音现场合成）。改为开始监听时预合成应答句（`SpeechOutput::prefetch`），合成结果裁掉首尾静音（各留 40 ms）。实测（模拟在线合成延迟 1.5 秒、首尾各 0.5 秒静音）：0.54 秒。（2026-11-17）

**实现**

- `kotone-stt::wake`：唤醒模型清单（官方整包、逐文件 SHA-256）、下载、`WakeDetector`（sherpa-onnx `KeywordSpotter`，预置两个唤醒词的 token，三档灵敏度）。
- `kotone-core`：`assistant.wake` 与 `agent.progressSpeech` 新字段（不认识的值按默认处理；设置说明表已收录）；`SilenceStopTracker::with_no_speech_timeout` 与 `StopDecision::NoSpeech`；`Orchestrator::begin_assistant_wake`：助手会话，说完即提交（与助手“一句话”预设相同，不看助手热键的交互模式），一直没开口就安静取消；会话 ID 前缀 `assistant-wake-`，宿主记为来源 `wake-voice`（游戏内入口）。上一次会话停在“成功 / 出错”的提示上时先清掉，否则“出错”会一直挡住唤醒。
- 桌面壳 `wake.rs`：唤醒线程用现有 `CpalBackend` 常开一路麦克风；任何会话进行中不判唤醒。命中后看 300 ms 判断是否连着在说：是 → 直接开会话，把这 300 ms 补进去；否 → 用当前语音说“嗯？/我在”（`assistant::speak_now`，先停掉正在播的），应答期间继续听，玩家开口（连续 3 块明显有声）就停掉应答，在缓冲里往回找话头（最近一段 ≥200 ms 的安静之后），从话头前 300 ms 开始补进会话。唤醒会话的音频经 `WakeAudioBackend` 从唤醒线程转发，不另开麦克风。
- `agent.rs`：游戏内一轮开始 1.5 秒后仍没出声就说一句固定短话（“好，我看看”等，不经过模型）；聊天窗口发起的一轮里玩家在游戏里插了话（热键、唤醒）时，结束后也用语音播出回答（`Inbox::take_delivered`）。
- 工具：`wake.status`、`wake.model.download`（进度事件 `wake.model.progress`，至多每秒一次）。设置页「琴音陪玩 → 设置 → 语音唤醒」。

**验证**

- 单元 / 集成测试：唤醒词与灵敏度映射、模型文件就绪判断、判断“连着在说”、找话头；录音会话层：唤醒会话说完即提交（助手热键是对讲机也一样）、没开口安静取消、停在“成功 / 出错”时可再次唤醒；插话交付记录；设置说明表覆盖新字段、新字段的每个可选值都能被设置接受。
- Rust 里跑真实模型（调研下载的同一模型，Windows 系统语音合成的样本）：3 条“琴音琴音……”全部检出；只说“琴音”、读音相近的“秦音”、游戏声音、长段语音、钢琴共 6 条反例都没有误报。
- 真实桌面程序（隔离的 `KOTONE_HOME`、本地模拟 AI、不花钱），经 VB-Audio 虚拟声卡把合成语音送进“麦克风”：唤醒模型真实下载 13 秒；开启后进入监听；四种说法识别全部正确——喊完停顿再说（琴音先应答）、整句连着说、应答没说完就开口（应答被打断）、上一次会话刚结束又唤醒；不带唤醒词的话不触发；识别结果以 `wake-voice` 进入琴音的对话；模拟 AI 故意慢 3 秒时，1.6 秒先播了“好，我看看”。
- 调试中发现并修掉的问题：应答期间丢掉玩家已经开口的话；判断窗口的音频没补进会话；话头之前留白太短，流式识别把第一个字认错（“我”→“或”）；“出错”状态挡住后续唤醒。

**没验证**

- 真人、真麦克风、真游戏环境；外放时应答声被收进麦克风的情形；长时间挂机的误唤醒率；弱 CPU 上的占用；Kokoro 与在线语音做应答的延迟；“插话后播报”只有单元测试，没有端到端跑。
