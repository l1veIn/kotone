# 本地中文 TTS 选型与验证

更新：2026-10-04。当前默认 Windows 系统语音，使用已安装的 SAPI 声音，无需下载任何 Kotone TTS 模型。Kokoro v1.1-zh INT8 是可选本地引擎，通过已有的 sherpa-onnx 1.13.5 Rust 绑定在 CPU 上运行，不启动 Python 服务；模型由用户在应用中手动下载。已有明确的引擎选择会保留。

## 选型证据

| 模型 | 中文声音与体积 | 决策 |
| --- | --- | --- |
| Kokoro v1.1-zh INT8 | 82M 参数；官方转换版含 55 个中文女声、45 个中文男声和 3 个英语声音；24 kHz。官方压缩包约 147 MB，解压约 215 MB。 | 已实现。可比较多个预设中文女声；模型许可证为 Apache-2.0。 |
| MeloTTS Chinese INT8 | 转换版一个中文女声，支持中英混读；INT8 ONNX 约 53.5 MB，官方整包约 167 MB，另含词典等资源。 | 更小的候选，但本轮没有运行或评价其音质。上游模型卡标注 MIT。 |
| Matcha icefall zh-baker | 一个中文女声；声学模型约 72 MB，另需约 51 MB 的 Vocos 声码器。 | 未选择：官方说明其训练数据仅限非商业用途，不能仅按推理框架许可证判断模型用途。 |

主要来源：[Kokoro 模型卡](https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh)、[官方声音映射与采样率](https://k2-fsa.github.io/sherpa/onnx/tts/all/Chinese-English/kokoro-multi-lang-v1_1.html)、[MeloTTS 模型卡](https://huggingface.co/myshell-ai/MeloTTS-Chinese)、[Melo 转换文件](https://huggingface.co/csukuangfj/vits-melo-tts-zh_en/tree/main)、[Matcha 模型与数据说明](https://huggingface.co/csukuangfj/matcha-icefall-zh-baker)、[Matcha 官方部署说明](https://k2-fsa.github.io/sherpa/onnx/tts/pretrained_models/matcha.html)。体积使用十进制 MB，不等同运行内存。

## 模型版本与完整性

- [官方下载包](https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2)
- 文件名：`kokoro-int8-multi-lang-v1_1.tar.bz2`
- 大小：147,031,220 字节。
- SHA-256：`a1e94694776049035c4f2c6529f003aaece993c76aae9a78995831c3c4dcafc6`。
- 本地下载后的散列与 [GitHub release API](https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/tts-models) 中该资产的 `digest` 一致。
- 完整解压文件合计 215,321,602 字节，其中 `model.int8.onnx` 114,299,010 字节，`voices.bin` 53,790,720 字节。

运行目录为 `<modelsDir>/speech/kokoro-int8-multi-lang-v1_1`。必需资源的唯一代码清单是 `crates/kotone-tts/src/engine.rs` 的 `REQUIRED_FILES` / `REQUIRED_DIRS`：包括模型、声音向量、tokens、中英词典、中文 FST、jieba 字典、eSpeak 数据和许可证。下载器与后端共同使用 `model_ready`，不各自维护另一份清单。当前 Windows 原生词典加载路径要求 ASCII，代码在进入原生库前返回清晰错误，避免原生层无法打开路径时终止进程。

## 琴音的表达依据

人设真源是 `.repochan/persona/current.json`，界面中的 `apps/desktop/src/lib/data/persona.ts` 是展示副本。提醒助手采用琴音温暖、轻快、熟悉游戏的表达风格，服务对象是屏幕前的用户本人；品牌人设不改变提醒的接收者与请求范围。关键内容应准确简短。人设没有声纹录音或可直接生成的声学定义。

选择 Kokoro 后使用模型自带中文女声，不克隆任何真人。界面提供 `kokoro:zf_001`、`kokoro:zf_003`、`kokoro:zf_017`，分别映射为官方 sid 3、5、11；001 是该引擎的默认声音。该引擎默认语速为模型标准速度的 1.06 倍，用户仍可调整。没有变调处理，也没有声称模型支持可控情绪。

`crates/kotone-ai/src/reminders.rs` 的提示词要求直接说简短正文，例如“打野有闪了”“该打小龙了”。不套用“提醒你”“提醒一下”“队友”“主人”等开场白，也不自报姓名。保留英雄、技能、对象、数量、否定与动作事实，计时参数不因润色改变；不替用户向第三人喊话，不添加用户未请求的战术指导、emoji 或舞台动作。用户要求原话时保留原话。温暖通过自然措辞体现，不机械附加人设口头禅。

## 运行与取消边界

`kotone-tts` 实现 core 的 `SpeechOutput`。`LocalSpeechOutput::new` 只创建串行工作线程；首次播报才加载模型，CPU 使用 2 个推理线程，闲置 60 秒后释放模型。声音列表不依赖模型已安装。音频由 cpal 输出，支持常见浮点和整数设备格式，并按设备采样率重采样。

每次请求持有入场时的 generation。取消先使旧 generation 失效：尚未打开音频设备时可立即确认；已有输出流时等待其释放，最多 2 秒。模型加载和单句 ONNX 推理不能被强行中断，但取消后产生的结果无法获得播放资格；旧请求自己的超时也不能取消新一代请求。句间生成回调检查取消。显式 `close()` 最多等待 10 秒，成功才保证推理引擎和输出流均已释放，供模型目录迁移使用；失败时调用方保留屏障并稍后重试。`Drop` 不阻塞界面。

## 实测

以下保留选型时实际使用的样本和测量结果。样本文字属于历史性能测试，包含的战术语气不是当前助手的生成规则；当前试玩/试听样本已改为面向用户本人的简短提醒，未沿用旧样本重新声称测量结果。

在 Windows、AMD Ryzen 7 9800X3D（8 核 / 16 线程）、CPU provider、2 个推理线程下，以同一文本分别冷启动生成 WAV：

> 收到！小龙快刷新啦，记得提前占好位置。别急，稳住这波！

| 预设声音 | 模型加载 | 合成耗时 | 音频时长 | RTF（合成 / 音频） |
| --- | ---: | ---: | ---: | ---: |
| zf_001 | 1,222 ms | 4,631 ms | 5.65 s | 0.819 |
| zf_003 | 1,240 ms | 5,179 ms | 6.29 s | 0.823 |
| zf_017 | 1,192 ms | 4,950 ms | 6.01 s | 0.824 |

验证程序为 `crates/kotone-tts/examples/synthesize.rs`，Rust debug 构建使用 sherpa 的原生预编译库。输出均为非空、有限采样值的 24 kHz 单声道 WAV。开发试听文件位于未提交的 `.tmp-analysis/tts-voices/kotone-zf_001.wav`、`kotone-zf_003.wav`、`kotone-zf_017.wav`。可复现命令：

```powershell
cargo run -p kotone-tts --example synthesize -- <模型目录> <输出.wav> kokoro:zf_001
cargo test -p kotone-tts -p kotone-ai
```

另外显式运行了需要真实输出设备的 ignored 测试：使用临时模型、音量 15%，等待默认输出设备回调收到非零 PCM，继续输出 100 ms 后取消。实测 `cancel` 在 37.142 ms 返回，输出流已释放；随后 `close` 在 63.391 ms 返回，推理线程已退出。播报调用返回“播报已取消”。测试不打开麦克风，不修改应用设置。

```powershell
$env:KOTONE_KOKORO_TEST_MODEL = '<临时模型目录>'
cargo test -p kotone-tts --locked real_device_playback_cancel_and_close -- --ignored --nocapture
```

已验证实际推理、文件生成、真实设备输出回调，以及取消、播放租约、旧请求超时、懒加载、关闭状态等回归。没有使用录音回环测量扬声器声学输出，也未以自动化方式判断声音是否符合个人审美。当前生成完整句子后播放，首次提醒还包含模型加载时间；这些数据不代表低端 CPU 或游戏高负载下的延迟保证。官方词典加载会输出一次 `Unknown token: ❓` 警告，本次合成均成功。

## 许可证边界

模型卡和下载包内 `LICENSE` 标注 Apache-2.0；上游说明中文声音来自获准提供的数据集。该结论仅描述模型权重，不能推广成整个运行依赖链只有 Apache-2.0。

模型包还包含 `espeak-ng-data`。sherpa-onnx 1.13.5 的 [构建定义](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.5/cmake/espeak-ng-for-piper.cmake) 引入 eSpeak NG fork `ed530aa113046142eb5115cf2fc9157854d0ffe1`，该版本的 [COPYING](https://github.com/csukuangfj/espeak-ng/blob/ed530aa113046142eb5115cf2fc9157854d0ffe1/COPYING) 是 GPL-3.0。发布二进制时须核对所用原生运行库及数据的对应第三方许可、通知与源码提供要求；模型按需下载不自动排除运行库的许可要求。相关来源记录于仓库 `THIRD_PARTY_NOTICES.md`，本次没有擅自修改 Kotone 自身的 MIT 许可证。
