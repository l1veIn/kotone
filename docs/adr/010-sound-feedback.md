# ADR-010：热键音效提示（录制音 / 发送音）

状态：已接受（2026-08，待发布版本落地生效）

## 背景

不开悬浮窗的用户（尤其全屏游戏场景）无法确认「热键是否真的按下、
消息是否真的发出去」。需求：开始监听时播一个提示音、发送成功时播
另一个提示音（微信语音风格的「起落」感觉：录制音上扬、发送音下沉），
并支持关闭与音量调节，未来扩展到用户自定义音效。

## 决策

### 1. 触发点复用现有核心事件，零新增核心事件

- **录制音**：`capture_started`（真正开录成功时发出，含引擎/麦克风
  就绪检查，失败不响）。
- **发送音**：`injection_succeeded`（注入游戏成功时发出；失败走
  Error 提示不发声，避免「响一下等于成功」的误判）。
- 预览确认（Preview → ConfirmSend）、重发、VAD 一句话自动发送都
  落在 `injection_succeeded`，无需区分交互模式——触发语义与模式解耦。
- **solo 独奏模式**：每句发送都会响发送音（与逐句发送节奏一致）；
  自动续录的 `begin` 不再重复录制音。实现：`capture_started` 事件
  载荷携带 `resumed: true`（`begin_resumed` 路径），壳按标记过滤。

### 2. 播放放壳层（kotone-tauri），核心保持平台无关

`TauriEmitter::emit` 在 `kotone://process` 分支记录事件后，按配置
（`soundFeedback`）触发 `kotone_platform_windows::playback::play_sfx`：

- 复用现有 cpal 输出管线（16kHz mono → 设备率重采样 → 默认输出设备），
  新增 `play_pcm_blocking(device, pcm)`，`play_wav`（CLI，支持
  `device_match` 虚拟声卡回环）与新提示音共享同一实现。
- 非阻塞：播放放独立线程，播完自动销毁输出流，绝不阻塞语音链路；
  失败只写 stderr 静默忽略（无输出设备场景不打扰）。
- 放在壳层而非核心：核心不引入音频输出依赖，前端窗口隐藏时也不
  受 WebView 节流影响（全屏游戏可靠性）。

### 3. 音效 v1 为内置合成音，录制音 / 发送音各自 3 选 1

`synth_sfx`：纯正弦扫频 / 定频音段 + 短起音 + 尾段淡出，16kHz mono，
零资源文件，每事件 3 个内置音效：

- 录制音（开始监听）：`rise` 上扬扫频（150ms，620→1020Hz，默认）、
  `ding-up` 双音上行、`chirp-up` 三连上滑；
- 发送音（发送成功）：`fall` 下沉扫频（200ms，980→560Hz，默认）、
  `knock` 低频双击、`ding-down` 双音下行。

未知 `soundId` 回退该事件默认音；后续「自定义音效」功能以各事件
card 的扩展项（自定义 wav 路径）替换播放源即可，接口不变。

### 4. 配置与 UI

- `config.json` 新增 `soundFeedback` 段：`record` / `send` 两个独立
  卡片，各含 `enabled`（默认 true）、`volume`（0-100，默认 60）、
  `soundId`（默认 rise / fall）；`#[serde(default)]` 兼容老配置。
- 高级页新增「音效」tab：录制音 / 发送音两张卡片（独立开关 + 音量
  滑块 + 3 选 1 音效，点选即保存并试听）+ 触发时机说明；为自定义
  音效预留同 tab 扩展位。
- `preview_sfx` Tauri 命令：设置页试听（`kind` + `soundId`，固定
  0.8 增益）。
