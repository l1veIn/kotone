# Kotone 引擎评测手册（六引擎大评测）

目标：同语料、同机器，对全部引擎做「速度 + 精度」对比，砍差留优，拍板默认引擎。

## 选手一览

| 引擎 id | 模型 id | 体积 | 流式 | 热词 | 备注 |
|---|---|---|---|---|---|
| whisper-cpp-sidecar | ggml-base / ggml-small | 150MB/487MB | 否 | 否 | 每次 spawn 子进程，延迟含模型加载 |
| sherpa-onnx-zipformer-zh | zipformer-bilingual-zh-en-2023-02-20 | ~150MB | 是 | 是 | 2023 老模型，无标点 |
| sherpa-onnx-sensevoice | sense-voice-zh-en-ja-ko-yue-2024-07-17 | 239MB | 否 | 否 | 中英日韩粤 |
| sherpa-onnx-x-asr-zh-en | x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05 | 162MB | 是 | 是 | 2026，自带标点，RTF 0.035 |
| sherpa-onnx-funasr-nano | funasr-nano-int8-2025-12-30 | 948MB | 否 | 是 | 热词最强，峰值内存 ~2.5GB |
| sherpa-onnx-qwen3-asr | qwen3-asr-0.6B-int8-2026-03-25 | 938MB | 否 | 是 | 52 语言，Apache 2.0 |

## 第 0 步：下载模型

CLI（任选需要的，已下载的会秒过）：

```bash
cargo run -p kotone-cli -- download x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05
cargo run -p kotone-cli -- download funasr-nano-int8-2025-12-30
cargo run -p kotone-cli -- download qwen3-asr-0.6B-int8-2026-03-25
cargo run -p kotone-cli -- download sense-voice-zh-en-ja-ko-yue-2024-07-17
cargo run -p kotone-cli -- download zipformer
cargo run -p kotone-cli -- download small    # whisper ggml-small
```

或在 GUI「引擎与模型」页逐个点下载。磁盘共需 ~2.9GB。下完 `cargo run -p kotone-cli -- doctor` 确认引擎全部就绪。

## 第 1 步：录语料（正常使用即录档）

`eval_recording` 默认开启——GUI 里「启动」后正常说话，或 `cargo run -p kotone-cli -- listen`，每个会话自动录到 `~/.kotone/eval/`（wav + 指标 json，容量循环）。

建议录 8-12 条，覆盖这些场景（每条 2-8 秒）：

1. 短指令：「中路 miss」「打龙打龙」「撤撤撤」
2. 标准长句：「对面打野在下路草丛，准备来抓」
3. 中英混说：「这波 teamfight 等我 ult 好了再打」
4. 游戏黑话：「盲僧在下路 gank，寒冰没闪」
5. 句尾轻读 + 快速松手（考甩尾）：「能听到我说话吗」
6. 数字与装备名：「出破败王者之刃，二十分钟三百刀」
7. 情绪化/快语速：模仿真实团战报点

## 第 2 步：回放对比

```bash
cargo run -p kotone-cli -- eval list                       # 查所有录档会话 id
cargo run -p kotone-cli -- eval replay <sessionId>          # 全部就绪引擎同语料对比
cargo run -p kotone-cli -- eval replay <sessionId> --engine sherpa-onnx-x-asr-zh-en   # 单引擎
```

不指定 `--engine` 就是六引擎同题对比（未下载模型的引擎自动跳过）。看：partial 节奏（流式引擎）+ 最终文本 + 延迟 ms。

## 第 3 步：人工标注（CER 的前提）

听原音（wav 在 `~/.kotone/eval/<sessionId>.wav`），回填正确文本：

```bash
cargo run -p kotone-cli -- eval label <sessionId> "对面打野在下路草丛准备来抓"
```

CER 计算会自动去标点/空白、统一小写——照实写即可。

## 第 4 步：出报告

```bash
cargo run -p kotone-cli -- eval report
```

输出「已标注会话 × 就绪引擎」的 Markdown 表：CER（字错误率，越低越好）+ 延迟。

## 第 5 步：决策矩阵（砍谁留谁）

| 维度 | 权重 | 看什么 |
|---|---|---|
| CER | ★★★ | report 表，游戏黑话语料重点看 |
| 延迟 | ★★★ | 流式看首字（partial 出现速度），非流式看最终 ms |
| 体积/内存 | ★★ | 常驻桌面应用，948MB 的要有明显质量优势才留 |
| 热词 | ★★ | 游戏词表场景，report 里黑话条目见分晓 |
| 流式 | ★ | 悬浮窗实时回显体验 |

预期剧本：whisper.cpp 和老 zipformer 出局；X-ASR 大概率坐稳流式主力；非流式质量档在 SenseVoice / FunASR-Nano / Qwen3-ASR 里三留一（除非质量碾压，否则体积最小的赢）。

## FAQ

- **回放时报「未就绪」**：该引擎模型没下载，回第 0 步。
- **X-ASR 热词**：bpe.vocab 在首次启动/下载时自动从 bpe.model 导出，无需手工处理。
- **句尾丢字**：7fcbacd 已修（800ms 静音尾帧），如果还有个案，录进语料里标注出来。
- **录档存太多**：eval 录档有容量上限自动循环；历史记录另有 `kotone-cli log list/clear`。
