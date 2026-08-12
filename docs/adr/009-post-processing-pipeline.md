# ADR 009：文本后处理 —— 双文本边界、注册发现与有序 Pipeline

- 状态：已采纳（Phase 1：领域骨架与 mock 编排）
- 主题：STT final 与 Preview/Injector 之间的可扩展文本处理阶段

## 上下文

识别结果过去从 `SttSession::finalize` 直接进入预览或发送。AI 润色、翻译、
规范化等能力若分别嵌入 orchestrator，会让状态、超时、取消、失败降级与模块
配置不断产生硬编码分支；若放进 STT 或 Injector，又会破坏现有端口边界。

## 决策

### 1. 双文本模型

- `RecognizedText`：STT 的原始 final；eval 始终使用它。
- `ReadyText`：完成全部后处理的可交付文本；只有它能进入 Preview/Injector。
- 处理失败保留 `RecognizedText`，重试会重新运行同一份 pipeline 快照。
- 注入失败保留 `ReadyText`，重试不会重复调用后处理服务。

### 2. 独立 Processing 状态

主链路为：

```text
Listening → Transcribing → Processing → Preview/Sending → Success/Error
```

pipeline 关闭或没有启用步骤时零步骤透传，不产生 Processing 状态，旧配置的
行为与事件序列保持不变。Esc/cancel 通过独立 token 取消 Processing future，
orchestrator 的 gen 代际继续负责丢弃过期异步结果。

### 3. 线性有序 Pipeline

Phase 1 使用 step 数组表达顺序；每一步只消费上一步的 `TextDocument` 并产生
下一份。暂不引入 DAG、Rust 动态库 ABI 或任意分支合并。

每个 step 包含：

- 稳定实例 ID；
- 注册表中的 `processorId`；
- enabled；
- 处理器私有 JSON config；
- 独立 timeout；
- `required` / `best-effort` 失败策略。

`required` 失败会阻止发送；`best-effort` 保留上一步文本继续。空输出视为错误，
不能借空字符串绕过 Injector 的非空约束。

### 4. 注册与发现

`kotone-core::postprocess` 只定义 `TextProcessor`、`ProcessorFactory`、
`ProcessorRegistry`、配置和 runner。具体模块放在 `kotone-postprocess`，由桌面壳
和 CLI 在 composition root 注册，依赖方向与 STT EngineRegistry 一致。

新增模块不修改 orchestrator：实现 factory/processor，并在实现 crate 的
`register_builtin` 注册即可。重复 ID 明确报错，不允许按注册顺序静默覆盖。
桌面端通过 `list_post_processors` 返回注册表描述，设置页不维护硬编码清单。

### 5. 配置与运行快照

`Settings.postProcessing` 持久化 pipeline。每轮处理开始前由 registry 把配置
编译成处理器实例快照；运行期间修改设置不改变本轮步骤或顺序。

### 6. 诊断与隐私

process 事件只记录 pipeline ID、step 数、耗时和字符数，不记录中间文本。
未来在线 API 凭据不得进入 pipeline JSON、日志、profile 包或诊断包。

## Phase 1 验证模块

- `mock.append-exclamation`：句尾追加 `！`。
- `mock.wrap-brackets`：使用 `【】` 包裹当前文本。

顺序 `append → wrap` 的结果必须是 `【原文！】`；这同时验证第二步消费第一步
输出、注册发现和声明顺序，而非 orchestrator 中的模块特判。

## 后续

- 为处理器 descriptor 增加配置 schema、网络/隐私能力声明；
- 增加连接与凭据层，AI 润色/翻译引用 connection ID；
- 历史同时展示识别原文与最终可交付文本；
- 加入真实在线 API 与本地 OpenAI-compatible adapter。
