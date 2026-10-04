# Kotone 本地模型评测手册

评测单位是明确的 `EvalTarget = engineId + modelId + sessionConfig`，不是运行时引擎名称。默认对所有已安装的本地模型回放；在线 ASR 不进入评测，mock 仅在显式选择时运行。模型和配置不再从日常活动模型隐式回退。

早期“六引擎砍留”的成绩与取舍保存在 [开发历史 §11](archive/development-history.md#11-决策变更记录)。它们来自当时十条语料和机器环境，没有新版目标/配置指纹，不能作为当前模型版本的可复现基准或性能保证。当前模型列表由 `kotone-stt/src/model.rs` 维护。

## 准备模型与语料

```powershell
cargo run -p kotone-cli -- download x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05
cargo run -p kotone-cli -- download sense-voice-zh-en-ja-ko-yue-2024-07-17
```

也可在「设置 → 高级 → 模型」下载。评测录档默认关闭；需要采集语料时在高级模型页打开，或执行：

```powershell
cargo run -p kotone-cli -- config set evalRecording true
```

随后正常使用 GUI/CLI 识别，录档保存在 `~/.kotone/eval/<sessionId>.json` 与同名 WAV（16 kHz、16-bit、mono），只保留最近 200 条。完成采集后可关闭开关。录档含音频与原文，导出目录不等于脱敏诊断包。

选择固定语料覆盖短指令、标准长句、中英混说、游戏专名、轻读句尾、数字和快语速。不同模型使用相同 WAV 与人工标注；记录机器、软件版本和模型来源。

## 回放和标注

```powershell
cargo run -p kotone-cli -- eval list
cargo run -p kotone-cli -- eval replay <sessionId>
cargo run -p kotone-cli -- eval replay <sessionId> --model sense-voice-zh-en-ja-ko-yue-2024-07-17
cargo run -p kotone-cli -- eval replay <sessionId> --engine sherpa-offline --model funasr-nano-int8-2025-12-30
cargo run -p kotone-cli -- eval label <sessionId> --text "对面打野在下路草丛准备来抓"
cargo run -p kotone-cli -- eval report
```

`--engine` 与 `--model` 同时提供时必须匹配。旧模型族 ID（例如 `sherpa-onnx-sensevoice`）会固定解析为该家族默认模型，再记录规范引擎 ID `sherpa-offline`；绝不会因当前活动模型是 FunASR-Nano 而改跑另一个模型。仅给规范 `--engine` 时选择该引擎清单的默认模型，建议用 `--model` 消除歧义。无参数的 replay/report 按已安装模型枚举，两个共用后端的模型仍分别运行。mock 调试用 `--engine mock-stream`。

基线评测不加入当前游戏热词，使用目标模型的本地 `threads`/`language` 配置和全局语言/热词分数；实际输入快照写入结果。需要热词 A/B 时使用 [CLI 手册](cli.md) 中的固定 WAV/profile 路径，不要把无热词基线当成热词能力结论。

## 结果身份与缓存

新结果位于 `replays/<sessionId>__<targetFingerprint>.json`，包含：

- 规范 `engineId` 与完整 `target`（模型、会话配置）；
- SHA-256 `targetFingerprint`；
- 对实际解码 PCM 与人工标注计算的 `sourceFingerprint`；
- 缓存版本、识别原文、partial 时间线、延迟和 CER。

报告只复用目标/配置/语料/标注均相同的新结果。改标注后旧 CER 自动失效；替换同长度 WAV 同样失效。旧 `<sessionId>__<engineId>.json` 仍可读、可导出，但无足够身份信息，不能复用。旧缓存不必人工删除。

评测使用独立 registry，目标开始前卸载旧识别器并按当前配置预热，结束或失败时释放。不与桌面正在识别的引擎实例共享缓存。

## 指标含义

CER = 字符级编辑距离 / 参考字符数；双方去标点和空白、统一小写。参考为空且识别也为空为 0，否则为 1。数值越低越好，但样本少、只选成功录音或不同标注规范都会影响结论。

回放每次推入 100 ms 的 PCM，连续全速运行，不模拟真实说话节奏。首字延迟从喂入开始计时；最终延迟使用引擎返回的 finalize 耗时；模型预热不计入这两个数。不要用该表直接声称游戏内端到端响应速度。

同时检查内存、磁盘体积、流式回显、语言与热词需求。模型缺失时先下载；下载源和代理在设置中调整，重试无需录新语料。所有真实模型测试均需要本机已安装模型；普通单元测试用假引擎和临时目录，不联网推理。
