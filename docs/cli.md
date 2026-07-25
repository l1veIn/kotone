# kotone-cli 命令参考

> 无 Tauri 的命令行前端：core 的一等消费者，也是无人值守自动化测试的入口。
> 构建：`cargo build -p kotone-cli`（默认带 sherpa 引擎，见 crates/kotone-cli/Cargo.toml）

## 命令清单

| 命令 | 说明 |
|------|------|
| `send --text <文本> [--profile lol] [--clipboard] [--delay-ms N]` | 一次性注入文本到前台窗口 |
| `listen [--engine <id>]` | 热键模式：LL 钩子 → orchestrator → JSONL 事件流（Ctrl+C 退出，码 2） |
| `listen --wav <file> [--speed N] [--engine <id>]` | wav 直灌会话模式（见下） |
| `listen --no-hotkey --duration <秒> [--engine <id>]` | 无热键会话模式（配合虚拟声卡） |
| `download <bin\|tiny\|base\|small\|zipformer>` | 下载 whisper-cli 运行时 / 模型 |
| `config show` | 打印当前完整配置（JSON，含默认值合并） |
| `config get <key>` | 读单个配置项（点路径，如 `hotkey.key`） |
| `config set <key> <value>` | 写配置项（点路径，原子写入，枚举值校验） |
| `devices` | 枚举音频输入/输出设备，标出默认与虚拟声卡 |
| `play <wav> [--device "<名称子串>"]` | 播放 16kHz wav 到输出设备（自动重采样） |
| `eval list / replay / label / report` | 引擎评测（ADR-005） |

### config set 支持的键

`hotkey.key`、`hotkey.mode`(hold/toggle)、`hotkeyBackend`(auto/llhook/register)、
`sttEngine`（校验已注册）、`activeProfileId`、`autoSend`(true/false)、
`audioDeviceId`、`language`、`evalRecording`、`runAsAdminOnStart`

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
    --engine sherpa-onnx-zipformer-zh > out.jsonl
# 断言 final 文本
grep -F '"text":"对面打野在下路"' out.jsonl && echo PASS
```

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
    --engine sherpa-onnx-zipformer-zh > out.jsonl &
sleep 1.5
cargo run -p kotone-cli -- play crates/kotone-stt/tests/fixtures/zh-game-3s.wav \
    --device "CABLE Input"
wait
grep -F '"text":"对面打野在下路"' out.jsonl && echo PASS

# 4. 恢复配置
cargo run -p kotone-cli -- config set audioDeviceId default
```

一键双路径验证：`scripts/e2e-virtual-audio.sh`（无虚拟声卡时自动跳过路径 2 并提示）。
