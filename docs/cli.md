# kotone-cli 命令参考

> 无 Tauri 的命令行前端：core 的一等消费者，也是无人值守自动化测试的入口。
> 构建：`cargo build -p kotone-cli`（默认带 sherpa 引擎，见 crates/kotone-cli/Cargo.toml）

## 命令清单

| 命令 | 说明 |
|------|------|
| `send --text <文本> [--profile lol] [--clipboard] [--delay-ms N]` | 一次性注入文本到前台窗口 |
| `listen [--engine <id>]` | 热键模式：LL 钩子 → orchestrator → JSONL 事件流（Ctrl+C 退出，码 2） |
| `listen --wav <file> [--speed N] [--engine <id>] [--profile <id>]` | wav 直灌会话模式（可固定音频 A/B 对比 profile 热词，见下） |
| `listen --no-hotkey --duration <秒> [--engine <id>]` | 无热键会话模式（配合虚拟声卡） |
| `download <模型id>` | 下载模型（清单内任意 id，如 `x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05` / `silero-vad`；镜像策略见 download.source） |
| `config show` | 打印当前完整配置（JSON，含默认值合并） |
| `config get <key>` | 读单个配置项（点路径，如 `hotkey.key`） |
| `config set <key> <value>` | 写配置项（点路径，原子写入，枚举值校验） |
| `devices` | 枚举音频输入/输出设备，标出默认与虚拟声卡 |
| `play <wav> [--device "<名称子串>"]` | 播放 16kHz wav 到输出设备（自动重采样） |
| `eval list / replay / label / report` | 引擎评测（ADR-005） |
| `doctor` | 环境自检：设备/引擎/profile/提权/VAD/history 逐项 ✓/⚠/✗ + 修复建议（有 ✗ 退出码 1） |
| `elevate <command> [args...]` | sudo 式提权：以管理员权限在新控制台执行子命令（典型 `elevate listen`；裸 elevate 报用法错误） |
| `profile list / use <id> / detect` | 游戏 profile 列表 / 激活 / 前台进程匹配检测 |
| `log list [--limit N] [--json] / clear [--yes] / delete --session-id <id> --ts <ts> [--yes]` | 识别历史查看 / 清空 / 单条删除（~/.kotone/history/） |

### config set 支持的键

`hotkey.key`、`hotkey.mode`(hold/toggle)、`hotkeyBackend`(auto/llhook/register)、
`sttEngine`（校验已注册）、`activeProfileId`、`autoSend`(true/false)、
`audioDeviceId`、`language`、`evalRecording`、`runAsAdminOnStart`、
`interactionMode`(push-to-talk/dictation/one-shot)、`vadSilenceMs`(200-5000)、
`history.mode`(capped/keep-all/off)、`history.maxRecords`(1-100000)、
`history.includeAudio`(true/false)、`download.source`(auto/official/mirror)、
`download.ghProxy`（GitHub 加速代理前缀，默认 `https://ghfast.top/`，失效可换）

### 识别历史（log 命令）

history.mode 非 off 时，每次会话终态自动追加一条 JSONL 到
`~/.kotone/history/history.jsonl`：`sent`（发送成功）/ `cancelled`（Esc 取消）/
`error`（注入或转写失败）。error 后重试成功会同 sessionId 再记一条 sent
（刻意的「失败→重试」叙事）；error 后的 Esc 是清理动作，不双记 cancelled。
取消时没有任何可验证的语音产出（流式引擎从未出过识别 partial，如结束
独奏模式时刚开出的空段；非流式引擎不发 partial，取消一律不落账）的会话
不记录——与空转录「无事发生」同理，只记真正放弃过的一句话。
sessionId 与 eval 录档一致时仍可互查；`history.includeAudio` 开启时会独立把
会话音频写到 `history/audio/<sessionId>.wav`，不要求开启 `evalRecording`。
capped 模式超上限自动裁剪最旧记录（联动删除其音频）。`log delete
--session-id <id> --ts <ts>` 按 `log list --json` 输出的 sessionId + ts 精确
删除单条记录；该记录带录音且不再被其他记录引用时（error→retry 会共享
同 sessionId 音频）一并删除对应 wav，记录不存在时幂等成功。

### doctor 与提权（elevate）

`doctor` 启动自检六项：音频输入设备（标注虚拟声卡）、STT 引擎就绪、
激活 profile 存在性、提权链路（目标进程已提权而自身未提权 → ✗ 并提示
`kotone-cli elevate listen`）、VAD 模型、eval/history 配置摘要。
`listen`（热键模式）启动时也会做同样的提权预检，命中即 stderr 警告（不阻断）。
`elevate <command> [args...]` 是 sudo 式语义：ShellExecuteExW runas 拉起提权副本，
在新控制台窗口执行给定子命令（参数按 MSVC/CommandLineToArgvW 规则转义透传，
带空格的参数如 `--profile "lol oce"` 原样还原），UAC 确认后新进程接管；
裸 `elevate` 无参数报用法错误；已是管理员时直接提示无需提权。

### listen 退出码

| 码 | 含义 |
|----|------|
| 0 | 会话成功（到达 Preview 或 Success） |
| 1 | 错误（引擎未就绪 / wav 读失败 / finalize 失败） |
| 2 | 中断（Ctrl+C）或用法错误（如 --no-hotkey 缺 --duration） |

`--wav` 模式**强制预览收尾**（autoSend 视为 false），绝不触发真实注入——无人值守安全。
`--speed` 为喂入倍率：1.0 实时（默认），0 全速。

## 自动化测试路径一：wav 直灌（零音频设备依赖）

`WavFileBackend` 把 16kHz wav 当采集设备喂给 orchestrator，不经过系统音频栈：

```bash
cargo run -p kotone-cli -- listen --wav crates/kotone-stt/tests/fixtures/zh-game-3s.wav \
    --engine sherpa-onnx-x-asr-zh-en > out.jsonl
# 断言 final 文本
grep -F '"text":"对面打野在下路"' out.jsonl && echo PASS
```

### 热词 A/B 验证

对同一个包含稀有词的 WAV 分别使用无热词 `generic` 和 LOL profile 回放，排除
两次说话内容、语速和麦克风噪声不同造成的干扰：

```powershell
cargo run -p kotone-cli -- listen --engine sherpa-onnx-x-asr-zh-en `
  --profile generic --wav "C:\path\lol-hotword.wav" --speed 0 > without-hotwords.jsonl
cargo run -p kotone-cli -- listen --engine sherpa-onnx-x-asr-zh-en `
  --profile lol --wav "C:\path\lol-hotword.wav" --speed 0 > with-hotwords.jsonl
```

建议语料包含容易写成同音常用词的专名，例如“悠米跟打野去大龙”或
“璐璐辅助去插真眼”。第二次运行的控制台或 `~/.kotone/kotone.log` 应出现
“已向本次会话提交 N 条热词（modeling_unit=bpe）”，且不得出现 `Cannot find ID` /
`Encode hotwords failed`；JSONL 最终文本中目标专名的命中率应高于无热词对照。
热词是解码偏置而非强制替换，单句两边都正确不能证明无效，应使用多条或多次回放统计。

## 自动化测试路径二：虚拟声卡回路（全系统音频栈）

play 播到 `CABLE Input`（输出设备），listen 从 `CABLE Output`（输入设备）采集，
覆盖 cpal 采集 + 重采样 + 播放重采样全链路：

```bash
# 1. 找到虚拟声卡（devices 输出第 2 列是设备 id）
cargo run -p kotone-cli -- devices

# 2. 采集指向 CABLE Output，关 autoSend
cargo run -p kotone-cli -- config set audioDeviceId "CABLE Output (VB-Audio Virtual Cable)"
cargo run -p kotone-cli -- config set autoSend false

# 3. 后台 listen，前台 play
cargo run -p kotone-cli -- listen --no-hotkey --duration 6 \
    --engine sherpa-onnx-x-asr-zh-en > out.jsonl &
sleep 1.5
cargo run -p kotone-cli -- play crates/kotone-stt/tests/fixtures/zh-game-3s.wav \
    --device "CABLE Input"
wait
grep -F '"text":"对面打野在下路"' out.jsonl && echo PASS

# 4. 恢复配置
cargo run -p kotone-cli -- config set audioDeviceId default
```

一键双路径验证：`scripts/e2e-virtual-audio.sh`（无虚拟声卡时自动跳过路径 2 并提示）。
