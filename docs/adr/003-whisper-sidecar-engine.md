# ADR 003：whisper.cpp sidecar 引擎与模型/二进制自管理

- 状态：已采纳（真实 STT 引擎 #1 落地）
- 上下文：kotone-stt 的 whisper_sidecar 原为占位（is_ready=false）。MVP 需要一个
  可用的离线中文引擎；docs v9 已确定 whisper-cli 由 kotone-stt 自管理而非
  Tauri externalBin。

## 决策

1. **sidecar 子进程**：`~/.kotone/bin/whisper-cli.exe`（+ whisper.dll + ggml*.dll），
   `std::process` 拉起。push_audio 缓冲 f32 PCM，finalize 写 16kHz/16bit/mono
   WAV 临时文件（~/.kotone/tmp/）→ `whisper-cli -m <模型> -l zh -t <threads>
   -np -nt --prompt <热词> <wav>` → 解析 stdout 即纯文本（`-np` 只出结果、
   `-nt` 去时间戳）。进程治理：30s 硬超时 kill、cancel 立即 kill、Windows
   CREATE_NO_WINDOW 防控制台闪现。
2. **非流式 session 语义**：capabilities.streaming=false，session 只发
   SttEvent::Final（latency_ms=转写耗时）；orchestrator 层无需任何改动，
   流式/非流式共用同一 trait 的契约成立。
3. **清单内置 + 下载器**：模型（ggml-tiny/base/small）与 whisper-cli 运行时
   以常量清单内置（ID/大小/URL/SHA256——模型取 HuggingFace LFS oid，发布包
   钉死 v1.9.1 取实测值）；通用下载器（download.rs）流式下载 + 进度回调 +
   SHA256 校验 + tmp→rename 原子落盘；bin 安装为 zip 解压 staging → 逐文件
   rename（同卷原子）。whisper-cli 作为伪模型条目 `whisper-cli` 出现在
   list/download，设置页零改动可见可下。
4. **热词**：`--prompt` 注入「以下是带标点的中文游戏语音，包含术语：…」，
   同时承担中文标点引导。
5. **活动模型**：`engineOptions["whisper-cpp-sidecar"].model`（config.json，
   默认 ggml-small）；`set_active_model` 校验引擎归属与「已下载」后写回。

## 被否决项

- **Tauri externalBin**：安装包多带 ~8MB 且 kotone-cli 无法复用；自管理
  下载按需获取、双前端共用。
- **whisper.cpp FFI 直接接入（whisper-rs 等）**：少一次进程边界与 WAV 落盘，
  延迟更低，但要处理 ggml 构建链/链接/ panic 跨 FFI 等复杂度——留 Phase 2
  （延迟敏感时重估）。
- **断点续传**：第一版省略（失败整包重下）；模型 466MB 在弱网下的体验问题
  留待真实反馈再补。

## 后果

- 正向：`kotone-cli download bin|small` 与设置页 IPC 共用同一下载路径；
  `kotone-cli listen --engine whisper-cpp-sidecar` 无 Tauri 全真链路可用；
  orchestrator 零改动。
- 实测（i7 本机，CPU 4 线程）：2.5s 中文音频转写 2.7s（含模型加载）。
- 已知限制：whisper small 对 SAPI 合成语音输出繁体（「對面打野在下路」），
  简繁后处理或更强 prompt 待评估；orchestrator finalize 超时 10s 对超长
  音频可能偏紧（进程级 30s 兜底，超时仅丢结果不泄漏进程）。
