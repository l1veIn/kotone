# ADR 005：eval 评测模块 —— 录档默认开、全速回放、字符级 CER

- 状态：已采纳（评测工具 v1 落地，docs/development.md §3.3 的「评测工具」从签名变为实现）
- 上下文：Phase 1 末要用真实语料（游戏短句 + 黑话 + 耳麦噪音）人工对比
  多款 STT 引擎并拍板默认引擎。前提是把「每次识别会话」变成可复现的
  语料：录下来、能对任意引擎重放、有量化指标（延迟 + CER）。

## 决策

1. **录档默认开（evalRecording: true），本地存储，可关可导出。**
   每次 finalize 成功的会话保存 `~/.kotone/eval/<sessionId>.json +
   <sessionId>.wav`（16kHz/16bit/mono），含 partial 相对时间戳、
   firstPartialMs/finalMs、最终文本。隐私考量：
   - **全部本地**：wav 与文本不出本机，无任何上报通道；
   - **可关**：设置页开关（已实现），关闭时不建缓冲零开销；
   - **可导出可删除**：`eval_export` 打包导出（jsonl + wav 目录复制），
     删除 `~/.kotone/eval/` 即彻底清空；
   - **容量自限**：只保留最近 200 个会话（约 200 × 数秒音频 ≈ 十几 MB），
     超出连 wav 与回放缓存一起清。
   取消的会话（Esc / 再按热键）不录——语义是「用户放弃了这段话」。
   落盘失败静默记 `~/.kotone/kotone.log`，绝不影响识别主链路。
2. **回放全速灌入，不按原始节奏。** `eval::replay` 以 100ms 块连续
   push 录档 wav 到目标引擎。取舍：按原始节奏回放（sleep 模拟实时）
   能复现「实时压力下的引擎行为」，但 ① 回放 200 条语料 × N 引擎要等
   音频总时长 × N，评测流程不可用；② 我们要对比的指标是**纯计算延迟**
   （首字/最终），全速灌入恰好转嫁掉语速变量，数字更可比。实时行为差异
   （如流式引擎的解码追不上说话速度）由真机实测覆盖，不指望回放复现。
3. **CER 口径：字符级编辑距离 / 参考文本长度，双方先去标点与空白、
   统一小写。** 中文按字（CJK 表意字符），英文按字母（非词级 WER）——
   游戏报点是中英混合短句，词边界不可靠，字/字符级最稳定。手写两行
   DP（几十行），不引编辑距离 crate（core 依赖纪律）。
   已知局限：简体/繁体算不同字符（whisper 出繁体会反映为 CER > 0），
   这是有意的——输出字形与玩家预期不一致本就是质量差异。
4. **回放结果落缓存，report 优先复用。**
   `replays/<sessionId>__<engineId>.json`；`eval report` 对已标注会话
   × 全部就绪引擎出 Markdown 对比表（CER / 首字 / 最终延迟均值），
   缺失的 replay 现场跑并缓存，重复出报告零重算。
5. **依赖方向保持 core 纯净**：`eval::replay` 收 `&EngineRegistry`
   参数，引擎实例由壳 / CLI 注入（同 ADR-001 的注册表容器模式）；
   core 自带 30 行 wav 编解码（与 whisper_sidecar 的私有 write_wav
   重复是有意的）。回放**绕过 orchestrator** 直调
   engine.start_session/push_audio/finalize——回放不是一次「会话」，
   无热键/注入/窗口语义，不该进状态机。
6. **录档接线点在 orchestrator**：evalRecording 开时 begin 创建
   `SessionRecorder`（pcm 缓冲 + partial 时间线），pump 边转发边喂，
   finalize 成功后落盘。录档内存代价 = 一段会话的 f32 pcm
   （10 秒 ≈ 640KB），push-to-talk 场景可忽略。

## 被否决项

- **按原始节奏回放**（sleep 模拟实时语速）：见决策 2——评测吞吐量与
  指标可比性都更差，实时行为差异归真机实测。
- **录档默认关**：默认关则 99% 用户永远不会开，Phase 1 末无语料可评；
  默认开 + 明确告知（设置页文案）+ 本地自限容量是工程与隐私的平衡点。
- **JSONL 单文件指标日志**（docs §3.3 原提法的字面实现）：每会话一个
  json 更利于「连同名 wav 一起删/导出/标注回填」，导出时汇总生成
  sessions.jsonl 即可兼得两者。
- **引入 hound / chrono / edit-distance crate**：wav 编解码 30 行、
  UTC 日历换算 15 行、编辑距离 15 行，引 crate 不值（core 依赖纪律，
  ADR-001）。
- **词级 WER**：中英混合游戏短句词边界不可靠，字符级 CER 更稳定。
