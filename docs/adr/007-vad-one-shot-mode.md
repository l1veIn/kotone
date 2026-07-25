# ADR-007：VAD 静音判停与模式 3「说一句就走」（one-shot）

状态：已接受（2025-07，随 one-shot 落地生效）

## 背景

ADR-006 把会话生命周期拆成 BeginTrigger × EndTrigger × PostFinalize 三决策点，
其中 B3（VAD 静音判停）当期只留枚举与路由接口。本 ADR 落地 B3，并启用
模式 3：**one-shot = A2（点按开始）+ B3（VAD 静音判停）+ C1（转写完直接发）**。
`interactionMode` 新增可选值 `one-shot`。

## 决策

### 1. VAD 选型：silero-vad ONNX + sherpa-onnx 推理后端

- 模型：silero_vad.onnx（~630KB），来源钉死为 sherpa-onnx 官方 release
  （`k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx`），
  入模型清单（id `silero-vad`，SHA256 校验，`kotone-cli download silero-vad`）。
- 推理后端：**复用 sherpa-onnx crate 的 `VoiceActivityDetector`**（内部即
  ONNX Runtime + silero），不引入独立 `ort` crate。理由：零新增 crate 依赖、
  与 engine-sherpa 共享同一份原生库（feature 统一编译只链接一次）、
  Rust 绑定现成且已钉版本。`ort` 独立引入会再拉一套 ONNX Runtime 二进制
  （~百 MB 级下载 + 链接体积翻倍），编译影响明显更大。
- feature 策略：kotone-stt 新增 `vad-silero = ["dep:sherpa-onnx"]`，默认关；
  kotone-cli 默认开（开发/自动化工具不在乎原生体积），kotone-tauri 默认关
  （壳保持默认零重依赖；要启用 one-shot 用 `--features vad-silero` 构建）。
  未编译/未接入时 one-shot 的 begin 报清晰错误，不开半截会话。

### 2. 分层：帧级判定在实现，判停逻辑在 core（纯逻辑可单测）

- core `vad.rs`：
  - `Vad` port：`push_frame(&[f32]) -> Result<bool, String>`（30ms 帧 @16kHz）
    + `reset()`；实现每会话一个实例（工厂注入，避免跨会话状态泄漏）；
  - `FrameSplitter`：采集端 50ms chunk → 30ms 定长帧（残留跨 chunk 补齐）；
  - `SilenceStopTracker`：判停状态机——**见过语音后**连续静音 ≥ `vadSilenceMs`
    （默认 700ms，CLI 可配 200–5000）判停；**会话最短保护 500ms**（固定常量）
    防误触；开始即安静永不判停（未说过话不算「说完」）。
- kotone-stt `vad.rs`：`SileroVad` 把 sherpa 内部平滑参数调到最小
  （min_speech/min_silence = 50ms ≈ 帧级原始判定），判停阈值全部归 core；
  完成的 segment 立即出队丢弃（音频归 STT 管，防 60s 环形缓冲被塞满）。

### 3. 判停路径：与热键结束同路

orchestrator 的 PCM 泵在 Listening 态把帧并行喂 VAD 与 STT session；
判停 → emit `kotone://vad-stop` → 经 Weak 自引用触发 `end()`
（Orchestrator 新增 `into_arc()` 接线自引用）→ finalize → C1 直发，
**与热键结束完全同路径**。热键强制结束恒在兜底（on_hotkey_toggle 的
Listening 分支对 B3 生效）：VAD 失效/推理失败时可手动结束；
VAD 推理失败只禁用本次会话判停（记日志），不断会话。

### 4. 无人值守安全：wav 直灌用 NullInjector

one-shot 的 C1 直发在 `listen --wav`（无人值守自动化）里绝不能碰真实窗口：
wav 会话模式改用 `NullInjector`（注入结果 JSONL 打印供断言）。
非 one-shot 的 wav 会话仍强制预览收尾（auto_send=false），行为不变。

### 5. whisper 静音裁剪：本期不做

whisper.cpp 是非流式整段转写，静音尾对结果影响小（自身带 30s 窗内
静音容忍）；模式 3 主要配 sherpa 流式引擎使用。若后续实测静音尾显著
拖慢 whisper finalize 或引入幻觉文本，再在 whisper sidecar 侧加
「按 VAD 判停点裁剪尾部静音」优化（接口已就位：判停时刻 = end() 起点，
eval 录档含完整音频可回放验证）。

## 被否决项

- **独立 `ort` crate 跑 silero**：依赖体积翻倍，与 sherpa-onnx 的 ONNX
  Runtime 并存有符号冲突风险；sherpa 绑定已覆盖需求。
- **判停阈值放 sherpa 内部**（min_silence_duration = 700ms）：不可单测、
  不可运行时配置，且最短会话保护仍需 core 参与——不如全部收归 core。
- **VAD 判停经 emitter 事件由壳/CLI 回调 end()**：把核心语义依赖集成方
  接线，漏接即坏；改 Weak 自引用收在 core 内。

## 验证

- 纯逻辑单测：FrameSplitter 切帧 / SilenceStopTracker 阈值、最短保护、
  静音中断重计、未见语音不判停；
- 集成测试（mock 引擎 + 脚本 VAD）：one-shot 全链路（A2→B3→C1→Success）、
  VAD 失效热键兜底、未接入 VAD 时 begin 报错；
- E2E（真机）：`config set interactionMode one-shot` + `listen --wav`
  （真实语音 + 1.5s 静音尾）→ 自动判停、final 输出、退出码 0。
