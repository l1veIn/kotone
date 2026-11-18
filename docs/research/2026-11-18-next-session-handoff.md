# 交接：下一个会话要做的事（2026-11-18）

分支 `claude/kotone-agent`（已推送，没有合并、没有 tag）。0.2.0 的功能和玩家第一轮试用反馈都已做完，见 `CHANGELOG.md` 的 Unreleased、ADR 019–023 的实施记录、`docs/research/2026-11-18-goose-evaluation.md`。

## 规矩（照旧，外加一条新授权）

- 只在 `claude/kotone-agent` 上工作；每个提交单独能编译；只暂存自己的文件（不要碰 `.archify/ .codegraph/ .rambledesk/ .reasonix/ .tmp-analysis/ bash.exe.stackdump`、`apps/desktop/src-tauri/Cargo.toml` 的纯换行改动、stash `codex-preserve-before-single-folder-consolidation`）。可以推送分支。**玩家明确说“合并并发布”之前不合并、不打 tag、不发版。**
- 测试用隔离实例：设 `KOTONE_HOME`（在 `target/` 下）和 `WEBVIEW2_USER_DATA_FOLDER`；隔离实例不参与单实例，配置里要关掉全部热键（参考 `scripts/manual-e2e/happy_path_e2e.py` 写的配置）。不碰玩家的 `~/.kotone`、正式安装（`D:\琴音`）和玩家自己在跑的 Kotone。
- 发模拟键鼠之前先确认前台不是游戏（`scripts/manual-e2e/computer_use_e2e.py` 有守卫）。
- 每一步用简单的中文汇报：做了什么、怎么验证的、没验证的；不编造。
- **新授权：可以用真实的 DeepSeek 服务测试，不用担心花钱。** 取 Key 的方式：玩家的 DeepSeek 连接 id 是 `deepseek-mwjiotc0`，Key 在 Windows 凭据库里（keyring 服务名 `kotone`，用户名 `connection:deepseek-mwjiotc0`，见 `crates/kotone-ai/src/secrets.rs`）。只在内存里读出来，经 `connections.upsert` 的 `apiKey` 交给隔离实例（隔离实例有自己的凭据空间）；**不打印、不写进任何文件、日志、提交或报告**；测完在隔离实例里 `connections.delete`。拿不到就请玩家用 `! <命令>` 设一个环境变量。

## 要做的事（按顺序）

### 1. 缓存命中统计与费用统计（照搬 DeepSeek Harness）

玩家：dsh（DeepSeek Harness）的 DeepSeek 缓存命中率很高，照搬它的做法；再加费用统计。

- 参考：DeepSeek Harness 源码 https://github.com/deepseek-ai/deepseek-harness （MIT，我们的界面已经参考过它，声明在 `THIRD_PARTY_NOTICES.md`，许可证在 `licenses/deepseek-harness-MIT.txt`）；本机装的 `C:\Users\A\AppData\Local\Programs\DeepSeek Harness\resources\app.asar` 可以解开看实际实现。先读清楚它怎么组织请求才让前缀缓存命中（系统提示和工具列表是否固定不变、动态内容放在哪、历史是否只追加、压缩时怎么保住前缀），以及它怎么统计和显示命中率、费用。DeepSeek 的缓存是自动的前缀缓存，返回的 usage 里有 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`（以官方文档为准）。
- 先审我们自己的请求会在哪里打断前缀：`kotone-agent/src/prompt.rs` 的系统提示（有没有时间、读屏结果这类每轮变化的内容放在前面）；`tools.find` 启用工具组时工具列表变化；游戏内入口每轮注入的读屏数据；工具结果里的图片；子会话。改成“固定的放前面、变化的放最后”，历史只追加。
- 统计：每次请求记下输入 / 命中 / 未命中 / 输出 token 和费用（`kotone-ai` 的 usage 已有 `cached_input_tokens`，要确认 DeepSeek 的字段映射对不对）；按轮、按对话、按天汇总，存一个本地账本（例如 `<KOTONE_HOME>/usage.jsonl`）。语音合成（按字）、语音识别（按时长）、OCR 的调用也记进去。价格表按模型写在代码里，标明来源和日期，实施时去官方价格页核对。
- 界面：聊天每轮下面的 token 用量旁边加命中率和费用；「琴音 → 设置」（或单独一页）看今天 / 本月的用量、命中率、费用。给琴音和外部 Agent 一个只读工具（如 `usage.summary`）。
- 验收：用真实 DeepSeek 连续聊多轮、在游戏内入口连续呼叫几次，命中率接近 dsh 的水平（实测记下数字和 dsh 对比）；费用和 DeepSeek 后台账单对得上（误差说明原因）。

### 2. 上下文压缩（照搬 Goose）

见 Goose 评估。到上下文窗口的 80% 时把较早的轮次用同一个模型摘要成一段（提示词参考 Goose 的 `compaction.md`，Apache-2.0，保留声明，写进 `THIRD_PARTY_NOTICES.md`）；过长的工具输出先截断或摘要；最近几轮原样保留。**要和第 1 条一起设计**：压缩只在需要时发生一次（那次缓存不命中），之后前缀重新稳定。替换掉 `kotone-ai/src/agent/store.rs` 里“超过条数直接丢最早几轮”的做法。用真实 DeepSeek 跑一段长对话验证：压缩后还记得早先说过的关键事实。

### 3. 角色（可切换）和自定义唤醒词

按我的推荐（玩家已同意），**放进 0.2.0**（玩家此前的原则：以后再升级太麻烦）：

- **每个角色各一份**：名字、头像和表情贴纸、性格（SOUL）、声音（TTS 音色）、唤醒词。
- **所有角色共用**：关于玩家的记忆（USER.md）、琴音自己的笔记（MEMORY.md）、每个游戏的记忆、技能、设置。换角色不会忘掉玩家。
- **内置角色**：琴音是默认，用现有的全套立绘和表情；另外内置几个角色，先用占位头像（不要冒用别人的形象），以后玩家补画。用户也能新建自己的角色（名字、头像、性格、声音、唤醒词）。
- 数据：工作区下每个角色一个目录（例如 `<workspace>/roles/<id>/`：角色说明、SOUL.md、头像），现在的 `IDENTITY.md` / `agent.identity` 迁移成“琴音”这个角色；当前角色记在设置里。气泡、聊天、「她是谁」、系统提示都用当前角色。「琴音 → 她是谁」改成角色列表 + 编辑。
- **自定义唤醒词**：现在的唤醒模型（sherpa-onnx KWS，WenetSpeech zipformer）认拼音音节。把唤醒词（默认取角色名重复两遍，如“小雨小雨”）转成拼音、检查每个音节都在模型的 `tokens.txt` 里，不在就提示换一个。中文转拼音需要一个字典库（选 MIT 等宽松许可证的，写进第三方声明）。
- **准确度要实测**：照 `target/agent-smoke/wake_e2e.py` 的办法，用几种合成声音（SAPI、DeepSeek 语音、Kokoro）把唤醒词通过 VB-CABLE 放给隔离实例，统计喊中率；再放一段游戏声音 / 普通对话，统计误唤醒。达不到预置唤醒词的水平就在界面上标“实验性”。
- 测试：Playwright（切换角色、新建角色、唤醒词校验提示）；真机（换角色后气泡、名字、声音、唤醒词都跟着变）。

### 4. 对局复盘

Rust 核心检测到游戏进程退出时，主动触发一次摘要：把这一局里和琴音的对话、提醒、自动化执行整理成复盘，写进这个游戏的记忆（ADR 022 的游戏记忆，注意上限 4000 字，超了先合并旧内容），并在聊天里留一条。可以关。用真实 DeepSeek 验证生成质量。

### 5. 用真实 DeepSeek 补验证

- **玩家报的“试听语音没声音”**（DeepSeek 语音）：之前用假 DeepSeek 复现不出来，已加日志（`kotone.log` 里 `speech preview:` 开头的行）。先在隔离实例里用真实 DeepSeek 语音复现，找到原因修掉。
- `scripts/manual-e2e/happy_path_e2e.py` 改成可选用真实 DeepSeek（聊天、语音识别、语音合成、OCR），验证琴音真的会读《Kotone 配置》技能、照着做。
- 同传子会话用真实模型跑一次（`scripts/manual-e2e/sessions_e2e.py`）。
- 琴音替玩家跟队友说话（游戏语音）在真机上跑一次：先应一声，再从虚拟麦克风说出来，玩家耳机里也听得到。

### 6. 零碎

- `apps/desktop/index.html` 的页面安全策略没有放行 `http://ipc.localhost`，Tauri 一直在退回慢的 IPC 方式。放行后启动会崩：某个窗口的早期 IPC 在 `OverlayManager` 还没 `manage()` 时就到了（`state() called before manage()`）。查清启动顺序再放行。
- 设置窗口旧的“按住说话”代码已经没人用了：`apps/desktop/src/lib/assistantVoice.ts` 和后端 `start_assistant_voice_input` 等命令，确认没人用后删掉。

## 收尾

全部检查（`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`pnpm check`、`npx playwright test`），更新 CHANGELOG / ADR / 文档 / 第三方声明，提交、推送分支，更新记忆，用简单的中文汇报。仍然不合并、不打 tag，等玩家说“合并并发布”。
