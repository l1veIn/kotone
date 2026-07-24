# ADR 004：sherpa-onnx 流式引擎（官方 Rust 绑定，feature 默认关）

- 状态：已采纳（真实 STT 引擎 #2 落地）
- 上下文：whisper.cpp（ADR-003）是非流式兜底；partial 流式上屏链路已在
  mock 引擎验证，需要一个真实流式中文引擎。sherpa.rs 原为占位。

## 决策

1. **绑定选型：官方 `sherpa-onnx` crate（1.13.4）**，非社区 sherpa-rs（已弃用，
   上游收编）也非手写 C FFI。理由：官方维护、API 完整覆盖 OnlineRecognizer
   （`create_stream_with_hotwords` per-stream 热词、`is_ready`/`decode`/
   `get_result` 流式三件套）、`Send + Sync` 满足引擎 trait 约束、默认静态链接
   免 DLL 分发。build script 自动从 GitHub Releases 拉预编译静态库
   （win-x64-static-MT），Windows MSVC 链接实测无 CRT 冲突。
2. **模型：streaming Zipformer 中英双语 2023-02-20**（int8 encoder ~182MB +
   fp32 decoder + int8 joiner + tokens，共 ~200MB），HuggingFace 逐文件下载
   （复用 ADR-003 下载器，新增多文件模型清单：逐文件 SHA256（LFS oid），
   tokens.txt 为 git 内小文件无 oid 仅按大小校验；多文件下载天然支持
   「已存在且大小匹配则跳过」的续传）。
3. **feature 默认关**：`engine-sherpa` 引入 ~50MB 原生库下载 + 静态链接体积，
   默认构建保持零原生依赖、秒级编译；feature 关时注册占位引擎
   （恒 is_ready=false），开启时编译真实实现。两种组合均纳入验证
   （`cargo test --workspace` 与 `cargo test -p kotone-stt --features
   engine-sherpa` + `-p kotone-tauri -p kotone-cli` 二进制链接）。
4. **热词**：`create_stream_with_hotwords`（每行一短语）。关键约束：
   contextual biasing **只支持 modified_beam_search 解码器**（greedy_search
   遇热词 stream 会 SHERPA_ONNX_EXIT），故恒用 modified_beam_search
   （max_active_paths=4，hotwords_score=1.5）。
5. **解码语义**：enable_endpoint=false（push-to-talk 自管理语句边界）；
   recognizer 懒加载共享（Arc + Mutex，模型加载一次性成本），stream 每会话
   新建；partial 变化检测——文本非空且有变化才发 SttEvent::Partial；
   latency_ms = finalize（松手→最终文本）耗时。

## 被否决项

- **社区 sherpa-rs**：已被上游官方 rust-api 取代并标记弃用。
- **手写 C API FFI**：省一层依赖但全部安全封装（RAII/Send/配置转换）要自己
  维护，官方 crate 已做且质量高。
- **shared（DLL）模式**：静态链接避免 DLL 搜索路径问题（test exe 在 deps/
  子目录、安装包分发），代价是二进制体积 +10~20MB——MVP 接受。
- **feature 默认开**：每次默认构建都拉 ~50MB 原生库且链接变慢，拖垮
  `cargo test --workspace` 日常循环。

## 后果

- 正向：真实流式 partial 上屏（E2E 实测首条 partial 27ms）；final 文本简体
  且全对（同 fixture 下 whisper small 输出繁体）；两引擎可并排对比；
  热词真实生效。
- E2E 数据（2.5s SAPI 中文语音，i7 CPU 4 线程）：
  sherpa partial「对面打→对面打野→对面打野在下路」，final 同文，
  finalize latency <1ms（音频已边收边 decode 完）；
  对比 whisper：无 partial，转写 2.7s（含进程+模型加载），输出繁体。
- 代价：feature 构建需一次性下载原生库；push_audio 内联 decode 是阻塞 CPU
  调用（实测 chunk 级毫秒级，orchestrator 音频泵无明显影响；更长音频可再
  评估挪阻塞线程）。
- 已知限制：recognizer 绑定创建时模型，切换模型需重启进程（单模型 MVP
  无影响）；`enable_endpoint=false` 意味着长静音不产生段切分（push-to-talk
  语义下符合预期）。
