# Kotone STT 引擎侦察报告（2026 年中核实版）

> 任务：核实调研文档候选模型真伪与接入成本。结论先行：**最重要的发现是 sherpa-onnx v1.13.4 并非 2025 年中版本，而是 2026 年的近期版本**（v1.13.3 ≈ 2026-02-25，v1.13.4 在其后），它已原生支持 X-ASR、多语 Nemotron-3.5 流式、FunASR Nano、Qwen3-ASR、Moonshine v2、FireRedASR、Dolphin、Cohere Transcribe 等——**首选路径（sherpa-onnx 配置级接入）覆盖面比预想大得多，Python sidecar 基本不再需要**。

## ① 候选核实表

| # | 候选 | 真伪 | sherpa-onnx v1.13.4 支持 | 中文/中英混 | 热词 | 流式 | 接入成本 | 推荐度 |
|---|------|------|--------------------------|-------------|------|------|----------|--------|
| 1 | Paraformer 流式 (bilingual zh-en) | ✅ 真实 | ✅ 老支持（online paraformer） | 中（官方 demo 中英混有明显错误，无标点无时间戳） | ❌（Paraformer 非 transducer，sherpa 热词仅限 transducer） | ✅ | 配置级 | ★★（被 X-ASR 全面压制） |
| 2 | Moonshine v1/v2 | v1 真实但**纯英文**；「Base 支持中文」为假。v2 真实、含中文（阿/中/英/日/韩/西/乌/越） | ✅（v1.12.28 起支持 v2） | v1 ❌ / v2 有中文但未经验证 | ❌ | ❌ 非流式 | 配置级 | ★（中文主力不够用） |
| 3 | X-ASR | ✅ **真实**，2026-06 k2-fsa 导出的中英流式 zipformer2 transducer，自带标点 | ✅ v1.13.3+（#3662/#3656） | ✅ 强（官方 demo：「昨天是 Monday，today is 礼拜二」全对，带标点） | ✅（标准 online transducer，modified_beam_search + cjkchar+bpe） | ✅ 480ms/960ms chunk | 配置级 | ★★★★★ |
| 4 | Nemotron-3.5 ASR Streaming 0.6B | ✅ 真实，`nvidia/nemotron-3.5-asr-streaming-0.6b`（2026-06-04，OpenMDW-1.1 可商用） | ✅ v1.13.3+（#3671，prompt_index 多语） | ⚠️ 中文仅 broad-coverage 档，FLEURS CER ≈19–20%，明显弱于中文原生模型 | ❌（NeMo transducer 仅 greedy_search，issue #3572 确认无热词路径） | ✅ cache-aware 80ms–1.12s | 配置级 | ★★（英文强，中文劝退） |
| 5 | Voxtral Realtime | ✅ 真实，`mistralai/Voxtral-Mini-4B-Realtime-2602`（2026-02，Apache 2.0，13 语言含中文） | ❌ 无 ONNX/sherpa 支持 | 有中文但未见中英混专项数据 | ⚠️ context biasing 仅 Mistral 官方 API 有，开源权重/vLLM 无 | ✅ 原生流式 | 不可行（4.4B，fp16 需 16GB VRAM；CPU 流式不现实；需 vLLM/MLX sidecar） | 排除 |
| 6 | Fun-ASR-Nano | ✅ 真实，`FunAudioLLM/Fun-ASR-Nano-2512`（800M，SenseVoice encoder + Qwen3-0.6B decoder，通义/钉钉） | ✅ **v1.13.x 原生支持**（`sherpa-onnx-funasr-nano-int8-2025-12-30`），**不需要 Python** | ✅ 中英日 + 7 中文方言/26 口音，自带标点+ITN | ✅（`OfflineFunASRNanoModelConfig.hotwords` 字段，issue #3092） | ❌ 非流式（VAD 伪流式） | 适配级（新模型类型） | ★★★★ |
| 7 | 顺带：Qwen3-ASR-0.6B | ✅ 真实（阿里，52 语言 + 中文方言，Apache 2.0） | ✅（`sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`） | ✅ 强 | ✅（`OfflineQwen3ASRModelConfig.hotwords`） | ❌ 非流式 | 适配级 | ★★★★ |

## ② 推荐 shortlist（按性价比）

1. **X-ASR 480ms streaming（流式主力，直接替换 2023 zipformer bilingual）**
   - 理由：同架构（zipformer transducer）升级款，中英混 + 自带标点 + 热词 + 流式四项全满足，sherpa-onnx 配置级接入，CPU int8 RTF ≈ 0.035（极快）。
2. **Fun-ASR-Nano via sherpa-onnx（非流式质量档，升级/替换 SenseVoice）**
   - 理由：调研文档说「热词最强」属实，但**结论是错的**——不需要引入 Python 全家桶，sherpa-onnx 已原生跑它，自带标点/ITN/热词/方言，CPU RTF ≈ 0.17–0.19。
3. **Qwen3-ASR-0.6B（非流式备选）**
   - 理由：Apache 2.0 许可证最干净，52 语言，热词字段已在 sherpa-onnx 配置里；与 Fun-ASR-Nano 二选一做人评即可。

## ③ 接入路径与模型文件

| 模型 | 路径 | 文件与体积 | 许可证 |
|------|------|-----------|--------|
| X-ASR streaming | sherpa-onnx OnlineTransducer（`model_type=zipformer2`），热词走 modified_beam_search + bpe.model（cjkchar+bpe） | `sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05.tar.bz2`：encoder.int8 148M + decoder 11M + joiner.int8 2.5M ≈ **162MB**（另有 960ms 版与非流式版 2026-06-03） | ⚠️ 未确认（README 仅 96B，发布方/训练数据量「百万小时」未证实，接入前需补查） |
| Fun-ASR-Nano | sherpa-onnx OfflineFunASRNano（encoder_adaptor/llm/embedding/tokenizer 四件套 + hotwords 参数） | `sherpa-onnx-funasr-nano-int8-2025-12-30.tar.bz2` ≈ **948MB**（int8）；第三方实测峰值内存 ~2.5GB | ⚠️ 需确认（FunASR 系模型多为自定义 Model License，商用条款接入前核实） |
| Qwen3-ASR-0.6B | sherpa-onnx OfflineQwen3ASR（conv_frontend/encoder/decoder/tokenizer + hotwords） | `sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2` ≈ **938MB** | Apache 2.0 ✅ |

**接入前必须核对的一件事**：sherpa-onnx **Rust crate** v1.13.4 的绑定是否已暴露 `funasr_nano` / `qwen3_asr` 配置结构体（C API 在 v1.13.4 里已有，Rust 绑定层的暴露情况需读 crate 源码确认；若未暴露，走 C API 或直接贡献绑定，工作量仍是适配级而非 sidecar 级）。

## ④ 明确排除项

- **Voxtral Realtime 4B**：体积/算力劝退（fp16 16GB VRAM；CPU 流式不现实），无 sherpa 路径，开源权重无 context biasing。
- **Nemotron-3.5 多语流式**：中文 CER ~20%（broad-coverage 档），且 sherpa 的 NeMo transducer 无热词路径。纯英文场景可留档（英文版 nemotron-speech-streaming-en-0.6b RTF 0.036 很强）。
- **Moonshine v1**：纯英文（文档「Base 支持中文」不实）。v2 虽有中文但非流式、无热词、中文精度无数据。
- **FunASR Python sidecar**：不必要——sherpa-onnx 已原生覆盖 Fun-ASR-Nano 与 Paraformer，引入 Python 运行时零收益。
- **Paraformer 流式（sherpa 版）**：不排除但降级为备选——无热词、无标点、中英混 demo 有错误，被 X-ASR 全面压制。

## ⑤ 中文流式「顺带侦察」结论

sherpa-onnx 当前中文流式梯队：**X-ASR（中英+标点+热词，首推）** > `sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30`（纯中文新模型，154M int8 encoder，RTF 0.15，无标点但支持热词；xlarge 版 726M、RTF 0.46 偏重）> Paraformer bilingual（无热词无标点）。非流式中文：Qwen3-ASR / Fun-ASR-Nano / FireRedASR v2（中英+20 方言，Apache 2.0）/ Dolphin（40 语言+22 方言）。

## ⑥ 信息来源

- sherpa-onnx CHANGELOG v1.13.3/v1.13.4: https://github.com/k2-fsa/sherpa-onnx/blob/master/CHANGELOG.md ; https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.4
- X-ASR 导出 PR: https://github.com/k2-fsa/sherpa-onnx/pull/3662 ; x-asr bug issue: https://github.com/k2-fsa/sherpa-onnx/issues/3672
- 热词机制（仅 transducer）: https://k2-fsa.github.io/sherpa/onnx/hotwords/index.html
- Online Paraformer 模型页: https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-paraformer/paraformer-models.html
- FunASR Nano in sherpa-onnx: https://k2-fsa.github.io/sherpa/onnx/funasr-nano/pretrained.html ; hotword/itn issue: https://github.com/k2-fsa/sherpa-onnx/issues/3092
- Nemotron-3.5 模型卡: https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b ; 多语支持 PR: sherpa-onnx #3671 ; 无热词 issue: https://github.com/k2-fsa/sherpa-onnx/issues/3572
- Voxtral Realtime: https://huggingface.co/mistralai/Voxtral-Mini-4B-Realtime-2602 ; arXiv 2602.11298
- Moonshine 多语 issue: https://github.com/k2-fsa/sherpa-onnx/issues/3231
- Fun-ASR-Nano 官方: https://github.com/modelscope/FunASR ; https://modelscope.cn/models/FunAudioLLM/fun-asr-nano-2512
- 第三方实测（体积/内存对照）: https://github.com/that-yolanda/voicepaste ; https://docs.openwhispr.com/guides/local-models
