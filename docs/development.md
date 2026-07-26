# Kotone 开发文档

> 状态：开发中 · 骨架与核心链路已实现（Phase 0 Spike 基本完成，Phase 1 进行中）
> 日期：2026-07-23（v3：记录首轮实现的偏差与新发现）
> 上游文档：[docs/tech-research.md](./tech-research.md)（技术预研报告）
> 本文档回答：用什么技术、为什么、怎么组织代码、按什么顺序开发。

---

## 1. 文档目的

预研报告完成的是「探路」：对比了 Tauri vs Electron、whisper.cpp vs sherpa-onnx、SendInput vs 剪贴板等候选方案，并用 LeagueAkari（MIT）验证了 LOL 局内注入路径。

本文档完成的是「拍板」：把预研中的候选方案收敛为唯一决策，给出可直接开工的架构、模块边界、数据模型、IPC 契约和开发顺序。后续开发中如需偏离本文档的决策，需在本文档中记录变更原因。

**v2 修订要点**：STT 不再绑定单一引擎。考虑到单一方案（如 whisper.cpp small）在速度与精度上可能不达标，STT 层设计为**可插拔多引擎架构**，下游链路（orchestrator → IPC → UI）**原生支持流式 partial 结果**，并内置**引擎评测工具**支撑多款方案的人工对比测试。交互模式（push-to-talk 的 hold/toggle 等）由用户可选，录音过程中悬浮窗实时回显识别内容。

**v3 修订要点**：首轮实现（骨架 → Rust 核心 → 前端 → Windows 注入）完成，记录实现偏差与两个重要环境发现（**UIPI 提权要求**、**麦克风隐私封锁**），见 §1.1 与 §11。

### 1.1 实现现状（2026-07-23）

| Commit | 内容 | 验证 |
|--------|------|------|
| `db50ac3` | Tauri 2 + Svelte 5 + Tailwind v4 骨架（双窗口 hash 路由、托盘、图标） | pnpm install / build:web / cargo check / tauri dev 全绿 |
| `5fa4c5a` | Rust 核心：orchestrator 状态机、热键 hold/toggle、mock-stream 引擎、cpal 音频、settings/profile 落盘、20 个 IPC 命令 | cargo test 31/31 |
| `0420c31` | 前端：状态驱动悬浮条（波形/partial/预览编辑/toast）、设置页、IPC 封装 | build:web + svelte-check 0 错；各状态渲染实测 |
| `cbcd246` | Windows 注入（SendInput + KEYEVENTF_UNICODE，对齐 LeagueAkari）、error 重试、后端驱动窗口显隐、关窗不退出 | cargo test 49/49；记事本中文注入 UIA 逐字校验 PASS |
| `d6d78ea` | UIPI 提权方案：权限检测（TokenElevation）、管理员重启（runas + 防循环）、设置页权限分区、InjectError.needsElevation | cargo test 55/55；对真实 LOL 进程检测实证 Some(true) |
| `05a3955` | 用户实测 bug 修复：preview 焦点恢复（目标窗口记忆 + 发送前还原）、preview 热键确认、单实例锁、设置页权限区常驻重启按钮 | cargo test 61/61 |
| `cd1f12c` | WH_KEYBOARD_LL 热键后端（游戏前台热键修复） | cargo test 73/73；LOL 真机通过 |
| `a7f4a23` / `87b58f5` | workspace 五 crate 拆分（ADR-001）+ apps/crates 归位（ADR-002） | cargo test 73/73；cli listen 无 GUI 实证 |
| `d5c6f3c` | 引擎 #1 whisper.cpp sidecar + 下载器（ADR-003） | E2E 转写成功（繁体，2.7s） |
| `0f5fb3b` | 引擎 #2 sherpa-onnx 流式 Zipformer-zh（ADR-004） | partial 27ms、final <1ms、简体全对 |
| `29fbd29` | eval 评测模块：录档/回放/标注/CER 报告（ADR-005） | 117 测试全绿；三引擎对比表实跑 |
| `84e979a` | CLI 完整化：config 点路径读写 / devices / play / listen --wav；WavFileBackend；CLI 默认带 sherpa；虚拟声卡自动化 E2E（docs/cli.md） | 135 测试全绿；VB-CABLE 回路无人值守实测 PASS |
| `fbd94f7` + `d8c0fb6` | ADR-006 会话生命周期交互模式：三决策点策略重构 + 预览只读化（编辑=伪需求）+ 热键捕获录入 | 147 测试全绿，行为零变化 |
| `2ecc3ba` + `236e813` | ADR-007 silero VAD + 模式 3「说一句就走」（A2+B3+C1） | 159 测试全绿；真实人声 wav 判停 E2E 通过 |
| `9c202ec` + `e60e14f` | core 识别历史模块（history.jsonl 三模式 + orchestrator 终态落账，sessionId 与 eval 互查）+ CLI 收尾（doctor / elevate / profile / log + history 配置键 + listen 提权预检） | 175 测试全绿；doctor / log 真机冒烟通过 |
| `e449608` | elevate 改 sudo 式语义：`kotone-cli elevate <command>` 透传执行；runas 参数转义升级为 MSVC/CommandLineToArgvW 完整规则 | 180 测试全绿；裸 elevate 用法报错实测 |
| `09f8b7e` + `06584fc` + `2c3e9e3` | 壳运行能力：core runtime「启动」全局开关（引擎 warmup/unload + 壳编排 IPC）+ 壳默认内置 sherpa 引擎与 silero VAD + 自绘标题栏（状态灯/启停/重启生效） | cargo test 全绿；GUI 真机启停链路实测 |
| `c26f8bb` + `dca8fca` + `97e714b` 等 | 前端：首启向导三步引导 + 模型管理升级（自定义目录迁移/删除/引擎页重构）+ 方向 B「中继站」全界面重设计（六页导航/toast 堆叠/两行标题栏/spotlight） | build:web + svelte-check 0/0 |
| `727759b` / `295af28` / `b20947a` / `ea50069` | 引擎扩充至六选手：SenseVoice、X-ASR（在线 transducer 骨架泛化接入）、FunASR-Nano、Qwen3-ASR；模型清单 + tar.bz2 整包下载通道 | cargo test 全绿；六引擎语料回放实跑 |
| `8a49e9f` + `7fcbacd` | P0 修复：X-ASR 崩溃——bpe_vocab 门控 + 文本格式探测 + bpe.model 现场导出；松手丢句尾——采音侧排空 + 静音尾帧 + decode 排空上限 | 甩尾语料「能听到我说话吗」复验 CER 0.000 |
| `9432dfd` | 六引擎评测手册（docs/eval-playbook.md）+ CLI download 透传清单内任意模型 id | 手册流程实跑收官 |
| 本轮 | **六引擎砍留**：默认引擎定 X-ASR（砍 whisper.cpp sidecar / 老 zipformer / Qwen3-ASR）；eval_recording 默认关 + 通用页开关；下载镜像（download.source + hf-mirror + ghProxy 回退，CLI/GUI 共用） | 212 测试全绿、build 零警告、svelte-check 0/0；doctor 下载源行 + X-ASR 回放真机复验 |

**已验证**：注入机制正确（记事本中文短句 4/4 逐字一致）；状态机全链路（cargo 集成测试）；前端各状态渲染（浏览器 demo + Tauri 内 error 态实测）。

**待验证（阻塞原因见 §10，均需 LOL 退出后的桌面窗口期）**：

- [ ] 记事本长句（>100 字）/ emoji 各 10 次成功率复跑（脚本 `scripts/notepad_inject_test.ps1` 已入库）
- [ ] 空闲内存 < 150MB 实测（dev 进程 ~40MB，含 WebView2 待 release 复测）

**已通过的真机验收（2026-07-24，用户手动实测）**：

- [x] 记事本全链路：热键 → 真麦克风录音 → mock partial → 预览 → F7 确认 → 文本入记事本（A 流程）
- [x] 提权链路：设置页横幅 → 以管理员身份重启 → UAC → 提权运行（设置页显示「管理员」）
- [x] **LOL 训练模式真机发送成功**：LL 钩子热键在游戏前台正常触发，文字直达游戏聊天框（C 流程）
- [x] 提权检测提示：未提权时发送 LOL 正确报「目标游戏正以管理员权限运行」

---

## 2. 技术选型总表（决策版）

| 层 | 最终选型 | 备选 / 后续 | 决策依据（详见 §3） |
|----|----------|-------------|---------------------|
| 桌面壳 | **Tauri 2.x** | Electron（不采用） | 低内存、小包体、Rust 原生层直达 OS API |
| 前端框架 | **Svelte 5 + Vite + Tailwind CSS** | React 19 | 悬浮窗 UI 极小，Svelte 产物最轻、无虚拟 DOM 开销 |
| 核心语言 | **Rust**（src-tauri）+ TypeScript（UI） | — | 输入注入 / 热键 / 音频 / STT 编排全部落在 Rust 原生层 |
| 音频采集 | **cpal** | — | Rust 跨平台音频采集事实标准 |
| VAD | **silero-vad（ONNX）** | 能量门限兜底 | 裁静音、防误触发，模型仅 ~2MB |
| **STT 框架** | **可插拔引擎架构：`SttEngine` trait + 引擎注册表** | — | 多方案并行接入、配置切换、统一评测（§3.3） |
| STT 引擎（默认，v15 评测冠军） | **sherpa-onnx X-ASR 流式中英标点**（int8，162MB，首启下载） | — | CER 0.008 / 首字 70ms / 最终 31ms（§11 v15） |
| STT 引擎（保留备选） | **SenseVoice（非流式多语言）/ FunASR-Nano（热词最强）** | 云端 API（OpenAI、国内 ASR） | 非流式质量档；已砍：whisper.cpp sidecar / 老 zipformer / Qwen3-ASR（v15） |
| 输入注入（Windows） | **raw `windows` crate 直调 SendInput + KEYEVENTF_UNICODE** | enigo（跨平台兜底）、arboard（剪贴板备选） | 与 LeagueAkari 已验证路径 1:1 对齐 |
| 全局热键 | **tauri-plugin-global-shortcut** | WH_KEYBOARD_LL（仅在需要任意键状态时） | 官方插件够用，MVP 不挂低级钩子 |
| 托盘 | **Tauri 官方 tray API** | — | 常驻后台，悬浮窗按需显示 |
| 配置存储 | **JSON（用户目录 `~/.kotone/`）** | TOML | 游戏 profile 需导入导出，JSON 最通用 |
| 包管理 | **pnpm + cargo** | — | README 既定 |
| CI | **GitHub Actions：lint + test + Windows 构建** | macOS 构建（Phase 2） | Windows-first |

---

## 3. 关键决策与理由

### 3.1 桌面壳：Tauri 2，不选 Electron

**决策：Tauri 2.x**

理由（与预研 §3 一致，此处落地为决策）：

1. **资源敏感是硬约束。** Kotone 是「与游戏同机运行的副工具」，空闲内存每省 100MB 都有意义。Tauri 空闲内存约 30–80MB，Electron 约 150–300MB；安装包也是 MB 级 vs 百 MB 级。
2. **核心能力必须落在原生层。** SendInput 注入、全局热键、音频采集在 Rust 侧可直接调 Win32 API；Electron 则需要额外的 N-API addon（LeagueAkari 就是这么做的），多一层 IPC 和构建复杂度。
3. **greenfield 无包袱。** 没有 Node 生态存量依赖，没有理由背 Chromium。
4. **路径已验证。** Whisperi（Tauri 2 + Rust 听写）等同类项目证明此栈可行。

已知代价：Windows 依赖系统 WebView2（Win10 1803+ / Win11 基本预装，安装器可引导补装）；macOS 权限链（麦克风 + 辅助功能）较繁琐，但 MVP 是 Windows-first，可接受。

**v3 补充（新发现，重要）：对以高权限（管理员）运行的游戏，Windows UIPI 会丢弃来自中权限进程的合成输入——Kotone 需与游戏同等提权运行才能注入。** 详见 §10 R-1。

### 3.2 前端：Svelte 5，不选 React

**决策：Svelte 5 + Vite + Tailwind CSS**

预研报告此处留的是「团队熟哪个用哪个」。开发文档必须拍板，选 Svelte 的理由：

1. **UI 体量极小。** Kotone 的界面 = 一个悬浮录音条 + 一个设置窗口，组件数量在几十个量级，React 的生态系统优势用不上。
2. **运行时开销最小。** Svelte 编译时产出、无虚拟 DOM，对「挂在游戏旁边」的常驻进程，前端内存与 CPU 开销越低越好，与选 Tauri 的逻辑一致。
3. **状态模型贴合。** 核心 UX 是一个状态机（idle → listening → transcribing → sending），Svelte 的响应式 store 表达这种状态驱动 UI 比 React hooks 更直接，代码量更小。
4. **动画需求。** 录音条需要流畅的波形/呼吸动效与流式文字上屏动效，Svelte 内置 transition/motion，无需额外库。

回退条件：若加入的协作者只熟 React 且 Svelte 学习成本成为瓶颈，Phase 1 结束前可迁移（UI 层与 Rust 核心通过 IPC 隔离，迁移成本可控）。

### 3.3 STT：可插拔多引擎架构（核心决策）

**决策：STT 层不绑定任何单一引擎，而是定义统一抽象 `SttEngine`，所有引擎以插件形式注册，用户在设置中切换；下游链路原生支持流式 partial；配套内置评测工具支撑人工对比测试。**

#### 为什么必须这样做

1. **速度与精度存在真实的不确定性。** whisper.cpp small 在中端 CPU 上的端到端延迟和中文游戏短句的准确率是否达标，只有实测才知道；sherpa-onnx 中文流式、FunASR、云端 API 各有trade-off，纸面对比无法替代实测。
2. **答案取决于真实使用场景。** 游戏报点是「短句 + 黑话 + 嘈杂耳麦」的特殊分布，通用 benchmark 参考价值有限——必须在项目自己的语料上做人工评测。
3. **引擎会演进。** ASR 领域迭代快（新模型、新量化方案），绑定单一引擎等于把产品体验押在一个外部项目的节奏上。抽象层让换引擎/加引擎成为配置问题而非重构问题。
4. **流式与非流式引擎的体验差异必须在架构层消化。** 流式引擎（sherpa-onnx）边说边出字，非流式引擎（whisper.cpp sidecar）松手才出结果——如果下游只按「一次性返回文本」设计，接入流式引擎时要返工 orchestrator、IPC 和 UI 三层。因此**流式支持从第一天就是架构的一等公民**。

#### 引擎抽象设计（v3：与已实现代码对齐）

```rust
/// 引擎静态能力声明，UI 据此展示可用功能与提示
struct EngineCapabilities {
    streaming: bool,        // 是否支持 partial 流式结果
    hotwords: bool,         // 是否支持热词表
    gpu: bool,              // 是否可用 GPU 加速
    offline: bool,          // 是否完全离线
    languages: Vec<String>,
}

/// 一个引擎 = 一种 STT 策略（含其模型管理）
trait SttEngine: Send + Sync {
    fn id(&self) -> &'static str;              // "sherpa-onnx-x-asr-zh-en" 等
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> EngineCapabilities;
    fn is_ready(&self) -> bool;                // 模型是否已下载/可用
    // v3 偏差：events 通道显式化为参数（文档原版隐含在返回值里）
    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>>;
}

/// 一次「按下到松手」的识别会话；流式与非流式引擎共用同一接口
trait SttSession: Send {
    /// 实时喂入 PCM（16kHz mono f32），流式引擎边收边识别
    fn push_audio(&mut self, pcm: &[f32]) -> Result<()>;
    /// 松手收尾，返回最终文本；流式引擎此时输出最终修正结果
    fn finalize(self: Box<Self>) -> Result<Transcript>;
    /// 取消（用户 Esc / 再按热键）
    fn cancel(&mut self);
}

/// partial 结果通过事件通道外发，非流式引擎只发 Final
enum SttEvent {
    Partial { text: String },
    Final   { text: String, latency_ms: u32 },
}
```

**关键点：音频推送是统一的。** 无论引擎是否流式，orchestrator 都在录音过程中持续 `push_audio`——非流式引擎内部缓存、`finalize` 时一次性转写；流式引擎边收边出 partial。这样**「录音时悬浮窗实时回显」对流式引擎零额外成本，换引擎不改上层一行代码**。

#### 引擎注册表与候选池

| 引擎 ID | 接入方式 | 流式 | 状态 | 定位 |
|---------|----------|------|------|------|
| `mock-stream` | 内置 mock | ✓ | **已实现**（联调用） | 全链路联调 |
| `sherpa-onnx-x-asr-zh-en` | 官方 sherpa-onnx crate（feature `engine-sherpa`） | ✓ | **默认引擎**（v15 评测冠军：CER 0.008 / 首字 70ms / 162MB） | 流式主力 |
| `sherpa-onnx-sensevoice` | 同上 | ✗（快批式） | **已接入**（CER 0.062 / 最终 121ms / 239MB） | 非流式质量备选 |
| `sherpa-onnx-funasr-nano` | 同上 | ✗ | **已接入**（CER 0.008 / 最终 1305ms / 948MB） | 热词最强档 |
| `cloud-asr` | HTTP/WebSocket | ✓ | 候选池（可选增强） | 精度上限参照系 |

> v15 砍除：whisper-cpp-sidecar（非流式 + spawn 架构性延迟 + 繁体问题）、sherpa-onnx-zipformer-zh（2023 老模型无标点，被 X-ASR 覆盖）、sherpa-onnx-qwen3-asr（938MB 非流式无差异化优势）。历史评测数据见 §11 v10/v15 与 docs/eval-playbook.md。

引擎通过 cargo feature 控制编译（如 `engine-sherpa`），未启用的引擎不进二进制，控制包体。

#### 评测工具（人工测试的工程支撑）

内置 `kotone-eval` 模块，让「多方案人工对比」从口头测试变成可复现流程：

1. **会话录档**：每次识别会话自动保存（可在设置中开关）`wav + 引擎 ID + partial 时间线 + 最终文本 + 延迟指标` 到 `~/.kotone/eval/`。
2. **语料回放**：录下的 wav 可对**任意已安装引擎**离线重放，同一段音频跑多个引擎，产出对比表（逐条文本 + 首字延迟 + 总延迟 + 人工标注 CER）。
3. **指标日志**：JSONL 格式记录 `first_partial_ms / final_ms / 文本`，设置页提供「导出评测数据」。
4. **设置页引擎对比视图**：简单表格展示各引擎最近 N 次的延迟与样本，辅助人工决策。

评测结论（选哪款做默认引擎）是 Phase 1 末的正式决策点，记录到本文档 §11。

**v3 现状**：eval 模块签名就位（`record_session/replay/export/list_sessions`），orchestrator 内已标注录档接线点（TODO），命令暂返回「未实现」。

#### 模型管理

- 各引擎自带模型声明（ID、大小、下载地址、SHA256），统一走 `model` 模块下载与校验；下载源镜像策略（hf-mirror / ghProxy 回退）由 settings `download` 段控制（v15）。
- 安装包不含任何模型；首次启动向导推荐下载默认引擎 X-ASR 模型（162MB）。
- 用户可在设置中切换引擎，未就绪的引擎显示「需下载模型」。

### 3.4 交互模式：会话生命周期的三个决策点（v12 重构，ADR-006/007）

**v12 起，交互模式不是硬编码分支，而是三个正交决策点的策略组装**（`kotone-core/interaction.rs`）：

- **BeginTrigger**：A1 热键按住 / A2 热键点按 / A3 VAD 全时检测（Phase 3）
- **EndTrigger**：B1 松手 / B2 再按 / B3 VAD 静音判停（`vadSilenceMs` 默认 700，最短会话保护 500ms；热键强制结束恒在兜底）/ Esc 取消恒在
- **PostFinalize**：C1 直接发送 / C2 预览确认
- 流式与否、悬浮框回显**不是模式维度**（引擎能力及其投影）

**预设模式**：对讲机（A1+B1+C1）/ 录音笔（A2+B2+C2）/ **说一句就走 one-shot（A2+B3+C1，已实现）** / **独奏模式 solo（A2+B3+C1+连续，已实现）** / 全时免按（Phase 3）。

**独奏模式（solo）**：触发三元组与 one-shot 相同，靠策略上的 `continuous` 标志区分——点按热键开始**持续收音**，VAD 每判停一段 → 转写 → 直发 → **不停机**，schedule_idle 置回 Idle 后立即 begin 下一段回到 Listening 等下一句；如此循环。停止方式：Listening 态再点按热键（= 停止会话，丢弃在途段不发送）、悬浮窗/标题栏停止、全局停止。发送失败（Error 保留文本可重试）不自动续段。每个 VAD 切分段 = 完整会话周期（采音/会话/录档/history 按段独立，松手丢字甩尾修复在每段内生效）。

**单键语义表**（ADR-006 契约）：触发键按状态路由——Idle→开始、Listening→结束、Preview→确认发送、Sending→中止、Error→重试；Esc 恒为取消。

**预览只读化（用户决策）**：游戏场景错别字要么直接发要么重说，编辑是伪需求。Preview 态只读显示 +「{触发键} 发送 / Esc 重说」；confirm_send 无文本参数；overlay 永不需键盘焦点。

**热键录入**：设置页「点击录入」+ `kotone-cli config set hotkey.key --capture`——LL 钩子捕获模式，按任意组合键即录入，Esc 取消。

**VAD（ADR-007）**：silero-vad ONNX（630KB，模型清单 `silero-vad`）；推理后端复用 sherpa-onnx 内置 VAD（零新增重依赖，feature `vad-silero`，CLI 默认开 / 壳默认关）；判停阈值纯逻辑在 core（可单测可配置）。

### 3.5 游戏注入：raw windows crate 直调 SendInput，对齐 LeagueAkari

**决策：Windows 注入层不用 enigo 抽象，直接用 `windows` crate 调 Win32 `SendInput`，实现 `send_unicode` / `key_down_up` / `is_process_foreground` 三个原语，时序参数 1:1 对齐 LeagueAkari 已验证实现。**

理由：

1. **这是全项目最大的风险点，必须贴着已验证实现走。** LeagueAkari（~3.7k★，MIT）在 LOL 局内大规模验证了「Enter 开聊 → Unicode 逐字 → Enter 发送、间隔 20ms」的时序。用 enigo 等于在已验证路径和自己之间再插一层行为未知的抽象——enigo 的 Unicode 输入实现细节（是否逐字 down/up、scan code 处理）与我们需要的精确控制不完全一致。
2. **成本不高。** 所需 Win32 API 就三个：`SendInput`、`GetForegroundWindow`、`GetWindowThreadProcessId`（加 `MapVirtualKey`）。LeagueAkari 的 `input.cc` 约几百行，Rust 复刻量相当。
3. **enigo 留给跨平台兜底。** macOS（CGEvent）/ Linux（uinput）阶段可用 enigo 统一抽象，Windows 路径保持 raw 实现。

**Unicode 逐字 vs 剪贴板粘贴：默认 Unicode 逐字，剪贴板为 per-profile 备选。**

- LOL：Unicode 逐字已验证，且**不污染用户剪贴板**（玩家在局间可能复制了别的东西）。
- 剪贴板路径保留在 profile 配置中（`preferClipboardPaste`），用于某些对合成键响应差但接受 Ctrl+V 的游戏，或长文本场景。
- 注意：`KEYEVENTF_UNICODE` 按 UTF-16 code unit 发送，emoji 等代理对必须按 `u16` 单元遍历（Rust 的 `str::encode_utf16()` 天然正确）。

**合规红线（继承预研）：** 只做系统标准输入合成 + 剪贴板；不读写游戏内存、不 hook 渲染、不做驱动注入。发送前硬性校验目标游戏进程为前台进程，否则 abort 并提示。

**v3 实现现状**：已实现于 `src-tauri/src/inject/windows.rs`。与 LeagueAkari 对应关系：sendString → `send_unicode`（每 64 个 u16 单元一批 SendInput，校验返回计数）；sendKey → `key_down_up`（MapVirtualKeyW scan code，任何路径都补 up 无悬键）；IsProcessForeground → `is_process_foreground` + `foreground_process_name`（Toolhelp 快照拿进程名）。时序编排经 `SendOps` trait 与平台解耦，8 个 mock 单测覆盖时序/取消点/剪贴板恢复。剪贴板路径保存并恢复用户原文本内容（非文本格式不恢复，已知取舍）。记事本中文短句实测 4/4 PASS（UIA 逐字校验）。

### 3.6 热键与悬浮窗

**热键：WH_KEYBOARD_LL 低级键盘钩子（Windows 默认）+ tauri-plugin-global-shortcut（回退/非 Windows）。**（v6 变更）

- **变更原因（实测实证）**：RegisterHotKey 在 LOL 前台时完全不投递热键事件（`~/.kotone/kotone.log` 实证：记事本前台事件正常，游戏前台零事件，换多个键位均如此）。游戏工具（AutoHotkey、LeagueAkari）均用 LL 钩子正是为此。
- 实现：`hotkey_ll.rs` 独立钩子线程（SetWindowsHookExW + 消息循环）+ mpsc 事件通道 + 消费者线程调 orchestrator；回调内零 IO、跳过 LLKHF_INJECTED（防自我触发）。
- 吞键策略：仅「主键+修饰键严格匹配」时吞掉（防触发游戏内同键绑定），其余键立即放行；Esc 会话激活期吞掉作取消。
- 回退：`hotkeyBackend: "auto" | "llhook" | "register"`（默认 auto = Windows 优先 llhook，失败回退插件）；设置页显示当前生效后端。
- 默认键位：`F8`（toggle）/ `Alt+V`（hold），首次启动引导选择并检测冲突。

**悬浮窗：Tauri 多窗口。**

- 主悬浮条：`always_on_top` + `decorations: false` + `transparent: true` + `skip_taskbar`。录音时弹出紧凑条（波形 + 流式文本），idle 时隐藏或收缩为小圆点。
- 设置窗口：独立窗口，从托盘菜单唤起。
- 点击穿透：MVP 不做（Phase 2，`set_ignore_cursor_events` 空闲时穿透）。
- **独占全屏不保证**：设置页检测全屏状态并提示用户切换无边框/窗口化，文档中明示。

**v3 实现细节**：

- **窗口显隐由后端驱动**（v3 变更）：orchestrator 状态事件 → 非 Idle 时 `SW_SHOWNA` 显示 overlay（**不抢焦点**，否则注入前台校验必然失败）、Idle 时隐藏；与前端显隐调用幂等共存。
- **关窗不退出**：main/overlay 的 CloseRequested 均拦截转为 hide，仅托盘「退出」真正结束（托盘常驻语义）。
- 窗口路由：单 SPA + hash 路由（`index.html#/overlay`、`index.html#/settings`）。

### 3.7 平台策略：Windows-first

- MVP 只保证 Windows 10/11 完整体验（注入 + 热键 + 悬浮窗）。
- macOS / Linux 保持**可编译**，听写 + 剪贴板粘贴可用，游戏 profile 后补。
- 理由：玩家主战场在 Windows，且 SendInput 路径是唯一经过实战验证的注入路径。

---

## 4. 系统架构

```
┌─────────────────────────────────────────────────────────────┐
│  Svelte 5 UI (Vite + Tailwind)                              │
│  ├─ 悬浮录音条（波形 / 流式 partial 文本 / 状态 / 结果预览）    │
│  ├─ 设置窗口（热键模式 / 麦克风 / STT 引擎切换 / profile / 模型）│
│  └─ 状态展示由 Rust 侧事件驱动（state / partial / level）      │
└──────────────────────────┬──────────────────────────────────┘
                           │ Tauri IPC（commands + events）
┌──────────────────────────▼──────────────────────────────────┐
│  Rust Core (src-tauri)                                      │
│  ┌─────────────┐ ┌─────────────┐                            │
│  │ hotkey      │ │ audio       │                            │
│  │ hold/toggle │ │ cpal 采集   │                            │
│  │ 模式触发     │ │ 重采样/缓冲  │                            │
│  └──────┬──────┘ └──────┬──────┘                            │
│         │               │ pcm 流                             │
│  ┌──────▼───────────────▼──────────────────────┐            │
│  │ orchestrator（状态机 + 会话编排 + 发送时序）    │            │
│  └──────┬───────────────────┬──────────────┬───┘            │
│         │ SttEngine trait   │              │                │
│  ┌──────▼─────────────────┐ │ ┌────────────▼────┐           │
│  │ stt 引擎注册表           │ │ │ inject          │           │
│  │ ├─ mock-stream ✓      │ │ │ SendInput/剪贴板 │ ✓        │
│  │ ├─ x-asr（默认）✓     │ │ └─────────────────┘           │
│  │ ├─ sensevoice ✓       │ │ ┌─────────────────┐           │
│  │ └─ funasr-nano ✓      │ │ │ profile 游戏配置  │ ✓        │
│  └──────┬─────────────────┘ │ │ 前台进程匹配      │           │
│         │ SttEvent 通道      │ └─────────────────┘           │
│  ┌──────▼─────────────────┐ │                               │
│  │ eval 评测录档/回放（签名）│ │                               │
│  └────────────────────────┘ │                               │
│  ┌──────────────────────────▼───────────────┐               │
│  │ settings ✓ · tray ✓ · model downloader   │               │
│  └──────────────────────────────────────────┘               │
└─────────────────────────────────────────────────────────────┘
```

（✓ = v3 已实现）

**设计要点：**

1. **orchestrator 是唯一的状态所有者。** UI 不维护业务状态，只渲染 Rust 侧 emit 的状态事件。所有状态迁移（idle → listening → transcribing → preview/sending → success/error）在 Rust 侧完成，避免前后端状态不一致。
2. **STT 与 inject 完全解耦，且 STT 内部多引擎解耦。** orchestrator 只面向 `SttEngine` trait 编程；inject 只接收最终文本。「仅复制」降级模式、换引擎、加引擎、流式升级都不影响其他层。
3. **流式是一等公民。** 录音期间 PCM 持续推入 session，partial 经事件通道直达 UI；非流式引擎只是「不产生 partial 的特例」。
4. **发送可取消。** 发送时序中有多个 delay，用户按 Esc 或再次按热键应能中止（`CancelToken`，对齐 LeagueAkari 的 AbortController）。

**v3 实现细节**：并发模型为 inner 状态（std Mutex，不跨 await）+ op 串行化（tokio Mutex）+ **gen 代际计数**（begin/cancel 自增，async 空隙后校验 gen，丢弃过期结果）。PCM pump 任务持有 session（select! 三路：stop/pcm/stt 事件），end 时 oneshot 取回后 `spawn_blocking + 10s 超时` finalize。

### 4.1 核心状态机

```
                 hotkey 开始（hold 按下 / toggle 首按）
        ┌────────────────────────────────────────────┐
        ▼                                            │
      Idle ──────────────────► Listening ──hotkey 结束──► Transcribing
        ▲                     │ partial 实时上屏             │
        │                     │（流式引擎）                  │
        │              ┌─── autoSend=true ──────────────────┤
        │              ▼                                    ▼
        │           Sending ◄──用户确认/编辑── Preview（可编辑文本）
        │           ▲  │
        │           │  │ ok       │         fail
        │           │  └────► Success toast ──┐
        │           │                         ▼
        │           └── confirm_send ── Error toast（带文本，可重试/取消）
        │                （v3：重试）
        └──────────── Idle
```

任意状态下按 Esc / 取消热键 → 回到 Idle（session cancel；发送中时序安全中止）。

**v3 语义细化**：带文本的 Error（如发送失败）**不自动回 Idle**，文本保留给用户重试（`confirm_send` 在 Preview 与 Error 两种状态均可调用）或取消；无文本的 Error（如引擎未就绪）维持 toast 后自动回 Idle。Success toast 停留约 1.5s 自动回 Idle。

---

## 5. 模块设计

### 5.1 Rust 侧（src-tauri/src）

| 模块 | 文件 | 职责 | 状态（v3） |
|------|------|------|------------|
| `hotkey` | `hotkey.rs` | 注册/注销全局热键，hold/toggle 两种触发模式，冲突检测；Esc 仅 Listening 期临时注册 | ✅ |
| `audio` | `audio.rs` | `AudioBackend` trait + `CpalBackend`：设备枚举、16kHz mono 重采样、PCM 流推送、50ms RMS 事件；设备打开失败即报中文错误 | ✅ |
| `stt` | `stt/mod.rs` | `SttEngine` / `SttSession` trait、引擎注册表、当前引擎路由 | ✅ |
| `stt::mock` | `stt/mock.rs` | mock-stream：每 0.5s 音频发 partial，finalize 返回固定文本 + 实测延迟 | ✅ |
| `stt::xasr` / `stt::sensevoice` / `stt::funasr_nano` | `kotone-stt/src/` | sherpa-onnx 三引擎：X-ASR 流式（在线 transducer 骨架）+ SenseVoice/FunASR-Nano 非流式（离线骨架）；feature `engine-sherpa` 门控 | ✅（v15 保留集合） |
| `eval` | `eval.rs` | 会话录档（wav + 指标 JSONL）、语料回放、多引擎对比 | 签名就位 |
| `inject` | `inject/mod.rs`, `inject/windows.rs` | `send_unicode` / `key_down_up` / `is_process_foreground` + `send_sequence` 时序编排（SendOps trait 解耦可测） | ✅（LOL 真机待测） |
| `orchestrator` | `orchestrator.rs` | 状态机，串联 hotkey→audio→stt→inject，partial 转发，取消与超时，gen 代际防过期 | ✅ |
| `profile` | `profile.rs` | 游戏 profile CRUD、前台进程匹配（内置 lol + generic） | ✅ |
| `settings` | `settings.rs` | 用户配置读写（`~/.kotone/config.json`），默认值合并 + 原子写入 | ✅ |
| `model` | `model.rs` | 各引擎模型下载/校验/切换 | 签名就位 |
| `tray` | `tray.rs` | 托盘菜单：显示悬浮条 / 设置 / 退出；关窗拦截转 hide | ✅ |

### 5.2 前端（src/）

| 模块 | 职责 | 状态（v3） |
|------|------|------------|
| `lib/stores/state.ts` | 订阅 `kotone://state` / `kotone://partial` / `kotone://level` 的 store，UI 唯一数据源；非 Tauri 环境容错 | ✅ |
| `lib/ipc.ts` | 全部 `invoke` 的类型化封装；浏览器环境内存 mock（dev:web 可纯前端调试） | ✅ |
| `lib/components/OverlayBar.svelte` | 悬浮录音条：波形、流式 partial 滚动区、可编辑预览、状态 toast（品牌色 token 化） | ✅ |
| `lib/components/Waveform.svelte` | rms 驱动 16 根渐变竖条，静默呼吸动画 | ✅ |
| `routes/overlay/Overlay.svelte` | overlay 视图 + 窗口显隐 + 浏览器 demo 模式 | ✅ |
| `routes/settings/Settings.svelte` | 设置页：交互模式与热键、麦克风、STT 引擎切换、autoSend、profile | ✅ |

### 5.3 IPC 契约（commands，v3 与已实现代码对齐）

```typescript
// 设置与配置
get_settings()              -> Settings
update_settings(patch)      -> Settings          // 热键变更后端自动重注册
list_audio_devices()        -> AudioDevice[]
set_audio_device(id)        -> void

// STT 引擎
list_stt_engines()          -> EngineInfo[]   // id / displayName / capabilities / isReady
set_stt_engine(id)          -> void           // 切换引擎（未就绪则提示下载模型）
get_engine_options(id)      -> Json           // 引擎专有配置项

// 游戏 profile
list_profiles()             -> GameProfile[]
save_profile(profile)       -> void
detect_foreground_game()    -> GameProfile | null   // v3：已接前台进程名匹配

// 会话控制（v3 新增：§4.1 状态机的必要入口）
confirm_send(text?)         -> void           // Preview/Error 态确认（可带编辑后文本）；Error 态即重试
cancel_session()            -> void           // 任意非 Idle 态取消回 Idle

// 模型
list_models()               -> ModelInfo[]
download_model(id)          -> Progress（event 流）
set_active_model(engineId, modelId) -> void

// 评测
eval_list_sessions()        -> EvalSession[]
eval_replay(sessionId, engineId) -> EvalResult
eval_export()               -> path

// 手动触发（调试/测试用）
simulate_send(text, profileId) -> Result<(), InjectError>   // v3：走真实发送时序
```

**事件（Rust → UI）：**

```typescript
"kotone://state"    { state: "idle"|"listening"|"transcribing"|"preview"|"sending"|"success"|"error", payload?: {...} }
                    // preview/sending/success 带 {text}；error 带 {message, text?}
"kotone://partial"  { text: string }             // 流式引擎录音期间持续推送；finalize 后也发一条最终文本
"kotone://level"    { rms: number }              // 录音音量（50ms 间隔），驱动波形
"kotone://download" { modelId, progress: 0..1 }
```

### 5.4 数据模型

```jsonc
// ~/.kotone/config.json
{
  "hotkey": { "key": "F8", "mode": "toggle" },   // toggle | hold（用户可选，默认 toggle 引导时确认）
  "audioDeviceId": "default",
  "sttEngine": "sherpa-onnx-x-asr-zh-en",        // 当前引擎（v15 默认 X-ASR），设置页可切换
  "engineOptions": {                              // 引擎专有配置
    "sherpa-onnx-x-asr-zh-en": { "provider": "cpu" }
  },
  "autoSend": false,               // true: 转写完直接发；false: 先预览确认
  "activeProfileId": "lol",
  "language": "zh",
  "evalRecording": false,          // 评测录档开关（v15 起默认关，通用页可开）
  "download": {                    // v15 模型下载源
    "source": "auto",              // auto（镜像优先+回退）| official | mirror
    "ghProxy": "https://ghfast.top/" // GitHub 加速代理前缀（公益服务不稳定，失效可换）
  }
}
```

```jsonc
// ~/.kotone/profiles/lol.json —— 与预研 §5.5 一致，两处修正
{
  "id": "lol",
  "displayName": "League of Legends",
  "processNames": ["League of Legends.exe"],
  "windowTitlePatterns": [".*League of Legends.*"],
  "openChatKey": "Enter",
  "sendKey": "Enter",
  "preOpenDelayMs": 20,          // 对齐 LeagueAkari：固定 20ms 足够
  "prePasteDelayMs": 20,
  "preSendDelayMs": 20,
  "preferClipboardPaste": false, // LOL 默认 Unicode 逐字，不污染剪贴板
  "hotwords": ["闪现", "大龙", "gank", "打野", "推塔", "回城"]
}
```

> 注：相对预研报告中的示例 profile，此处将三个 delay 默认值从 50/40/30 修正为 20/20/20（预研 §5.0 已用 LeagueAkari 实测数据修正过假设），并将 `preferClipboardPaste` 默认改为 `false`（Unicode 优先）。

```jsonc
// ~/.kotone/eval/<sessionId>.json —— 评测录档（配套同名 wav）
{
  "sessionId": "20260723-190512-3f2a",
  "engineId": "sherpa-onnx-x-asr-zh-en",
  "startedAt": "2026-07-23T19:05:12+08:00",
  "audioMs": 2400,
  "firstPartialMs": 380,          // 非流式引擎为 null
  "finalMs": 610,
  "partials": [{ "t": 380, "text": "对面打野" }, { "t": 520, "text": "对面打野在下" }],
  "finalText": "对面打野在下路",
  "humanLabel": null               // 人工评测时回填正确文本，用于 CER
}
```

---

## 6. 发送时序（核心路径，Windows）

```
hotkey 开始
  → orchestrator 记录当前前台窗口 hwnd（注入目标，v5 新增）→ 创建 STT session → Listening，悬浮条弹出（SW_SHOWNA 不抢焦点）
  → 录音 PCM 持续 push_audio
       流式引擎：partial → emit "kotone://partial" → 悬浮条实时上屏
       非流式引擎：仅波形 + 「聆听中…」
hotkey 结束
  → session.finalize() → 最终文本 T（emit transcribing → 文本上屏）
  → autoSend=false：Preview 状态，**再按热键 = 确认发送**（v5：不抢焦点的主交互），Esc 取消；鼠标编辑后点发送也可
  → **恢复焦点到记录的 hwnd**（SetForegroundWindow，失败用 AttachThreadInput 重试）→ 30ms 延迟（v5 新增）
  → inject::is_process_foreground(profile.processNames)
       false → Error toast「游戏不在前台：目标进程 X 未处于前台（当前前台：Y）」→ 文本保留可重试
  → key_down_up(openChatKey)          // VK_RETURN, scan code 经 MapVirtualKeyW
  → sleep(20ms)
  → preferClipboardPaste ?
       arboard 保存旧值 → 写入 → Ctrl+V → 100ms 后恢复旧值
     : send_unicode(T)                // encode_utf16 逐单元 KEYEVENTF_UNICODE down+up，64 单元/批
  → sleep(20ms)
  → key_down_up(sendKey)
  → Success toast「收到，已发送！✨」→ 1.5s 后 Idle
  → eval 录档落盘（wav + 指标，可在设置中关闭）   // v3：接线点已标注，实现待做
```

取消点：录音中 cancel session；finalize 10s 超时；发送时序每次 sleep 前后检查 CancelToken，取消时已按下的键补 up。

**v3 重要前提（UIPI）**：若目标游戏以高权限（管理员）运行，Kotone 进程权限低于它时，**合成输入会被系统整体丢弃、前台切换也会失败**。Kotone 需以不低于游戏的权限运行（提权方案见 §10 R-1）。前台校验的错误消息已包含当前前台进程名，便于诊断。

---

## 7. 项目结构

> v9：`apps/` + `crates/` 产品 monorepo（ADR-002）。根目录 = 纯 Rust workspace + 转发脚本。

```
kotone/
├─ Cargo.toml / Cargo.lock / rust-toolchain.toml   # workspace；依赖版本统一 workspace.dependencies
├─ package.json / pnpm-workspace.yaml              # 根级转发脚本；单 lock，packages: ["apps/*"]
├─ apps/
│  └─ desktop/                  # kotone-desktop：标准 Tauri 应用（canonical 形态）
│     ├─ package.json / vite.config.ts / index.html / src/ (Svelte)
│     └─ src-tauri/             # kotone-tauri 薄壳：IPC/窗口/托盘/单实例/热键回退
├─ crates/
│  ├─ kotone-core/              # 域模型 + ports + Orchestrator + settings/profile/eval/log
│  ├─ kotone-stt/               # STT 引擎适配器 + 模型管理（~/.kotone/models/）
│  │                            # features: engine-sherpa（默认关）/ vad-silero
│  ├─ kotone-platform-windows/  # cpal / SendInput / WH_KEYBOARD_LL / elevation
│  └─ kotone-cli/               # clap：send / listen / eval —— core 的无 GUI 消费者
├─ scripts/ docs/ assets/
└─ docs/adr/{001,002}-*.md
```

依赖方向：`kotone-stt → kotone-core ← kotone-platform-windows`；`kotone-cli / kotone-tauri → 三者`。引擎经 `kotone_stt::register_builtin()` 注入 core 的空注册表容器，避免循环依赖。

### 7.1 架构原则（v8 起生效，详见 ADR-001）

1. **crate 拆分判据**：独立消费者 / 重依赖编译隔离 / 变更节奏——三者满足其一才拆
2. **ports 在 core**：`SttEngine`、`AudioBackend`、`Injector`、`FocusBackend`、`HotkeySource`、`EventSink`
3. **配置**：schema/存储/唯一写入口在 core；消费者构造注入配置值；engineOptions 不透明 JSON
4. **事件**：core 产出结构化事件，壳映射为 `kotone://*` IPC，CLI 打印 JSONL

---

## 8. 开发计划与里程碑

### Phase 0：技术 Spike（第 1 周）—— v3 状态

| # | Spike | 通过标准 | 状态 |
|---|-------|----------|------|
| 1 | Tauri 2 骨架 + 全局热键 + 透明置顶悬浮窗 | 游戏前台时热键触发、窗可见 | ✅ 基本完成（热键触发 + overlay 弹出实测；游戏前台场景受 R-1/R-4 限制待复测） |
| 2 | `SttEngine` trait 骨架 + whisper.cpp sidecar 转写 3s 中文 | 延迟与准确率基线数据 | 🔶 trait/注册表/mock 引擎完成；whisper sidecar 实现待做（被 R-4 阻塞联调） |
| 3 | **Rust SendInput 复刻 LeagueAkari 时序，LOL 训练模式实测** | 前台检测 + Enter×2 + Unicode 字符串，10 次 ≥ 8 成功 | 🔶 机制验证通过（记事本 4/4 UIA 逐字一致）；LOL 真机待测（被 R-1 阻塞，需提权） |
| 4 | sherpa-onnx 流式 session 同句对比，partial 事件打通 | partial 延迟 < 500ms 体感流畅 | 🔶 partial 事件链路已通（mock 引擎实测）；sherpa FFI 实现待做 |

Spike 3 降级预案（转写 + 复制到剪贴板）仍然有效，但当前证据方向乐观：注入机制本身正确，阻塞点是权限而非时序。

### Phase 1：MVP（第 2–4 周）—— v3 状态

1. ✅ Tauri 2 骨架 + 托盘 + 全局热键（hold / toggle 双模式可选）
2. 🔶 **STT 引擎抽象落地**：trait + 注册表 + mock 引擎 ✅；引擎 #1 whisper.cpp sidecar 待做
3. 🔶 **引擎 #2 sherpa-onnx 流式接入**：partial → 悬浮条链路 ✅；sherpa FFI 待做
4. ⬜ **评测工具 v1**：签名就位，录档接线待做
5. ✅ 通用注入：任意前台窗口（记事本回归脚本 + UIA 校验）
6. 🔶 LOL profile：实现完成，真机实测待提权方案
7. ✅ 设置页：交互模式与热键 / 麦克风 / STT 引擎切换 / autoSend / profile 选择
8. 🔶 品牌悬浮 UI ✅；模型下载器待做

**Phase 1 末决策点：默认 STT 引擎。** 用评测工具积累的真实语料（游戏短句 + 黑话 + 耳麦噪音）做人工对比，结论记录到 §11。

**MVP 验收标准（在预研 §10 基础上修订）：**

- [x] 记事本路径中文短句上屏（机制验证 4/4；10 次 × 三类用例复跑待窗口期）
- [ ] LOL 无边框训练模式发送 10 次 ≥ 8 成功（需提权后实测）
- [ ] 空闲内存 < 150MB（不含模型）（release 复测）
- [x] 至少两款引擎完成接入并可设置页切换（mock + 注册表机制 ✅；两款真实引擎待做）
- [x] 流式引擎录音时 partial 可见（mock 实测 ✅）
- [ ] 选定默认引擎的「松键到上屏」P50 < 2s（流式引擎以 final 为准，partial 首字 P50 < 500ms）

### Phase 2：体验增强（第 5–8 周）

- 依据 Phase 1 评测结论：优胜引擎打磨（热词、VAD 参数、GPU 加速）；whisper 路径迁 FFI
- 候选池引擎按评测需要补充接入（SenseVoice / FunASR / 云端参照系）
- 点击穿透悬浮窗、发送历史、用户自定义热词库
- Valorant / Apex / 原神 profile 实测扩表
- Tauri updater 自动更新

### Phase 3：平台扩展

- macOS 完整适配（CGEvent 注入 + 权限引导）
- Linux X11 支持（Wayland best-effort）
- VAD 免按键（hands-free）作为第三种交互模式评估
- 按游戏自动切换词表

### 8.1 壳（Tauri）缺口清单（封 Tauri 开工单，v14）

core / CLI 侧能力已齐备（历史、提权、交互模式、VAD、评测），壳是薄封装。
封 Tauri 前按此清单逐项收口：

1. **构建变体**：壳默认不含 sherpa / silero VAD（feature 默认关，原生库 ~50MB），
   one-shot 模式在 GUI 不可用、引擎只有 whisper sidecar。需要定「带 feature 的壳
   构建 / 发布策略」：发布版是否默认带 sherpa+VAD、dev 与 release 的 feature 矩阵、
   模型按需下载与二进制体积的取舍。
2. **历史面板**：`get_history` / `clear_history` 两个 IPC + 设置页历史 UI。
   core `history` 模块（list/clear/includeAudio）与 CLI `log` 参考实现已就位，
   纯接线无设计工作。
3. **交互模式选择器 + vadSilenceMs 滑块**：`interactionMode`
   （push-to-talk / dictation / one-shot）与 `vadSilenceMs`（200–5000）设置项
   已在 config schema 与 CLI 落地，设置页缺对应 UI（one-shot 依赖第 ① 条的
   VAD 构建变体，UI 需按引擎/VAD 就绪态禁用并提示）。
4. **首次启动引导**：模型下载向导（首启检测引擎未就绪 → 引导选择并下载模型 /
   VAD）。`list_models` / `download_model` IPC 已有，缺向导页与首启判定逻辑。
5. **打包发布**：NSIS 安装包、manifest 保持 asInvoker（运行时检测 + 一键提权
   方案不变）、GUI 自启动提权入口对齐 CLI 的 auto-elevate 防循环链路
   （`runAsAdminOnStart` + `restart_for_auto_elevate`，壳侧已具备，打包后回归验证）。

---

## 9. 测试策略

| 层 | 方式 | 工具 | v3 状态 |
|----|------|------|---------|
| profile 匹配、配置合并等纯逻辑 | Rust 单元测试 | cargo test | ✅ |
| STT 引擎契约 | 各引擎跑同一 fixture wav，断言 `SttSession` 行为（push/finalize/cancel） | cargo test + fixtures | mock ✅，真实引擎待做 |
| **STT 引擎横向评测** | eval 回放：同一语料库 × 全部引擎 → 延迟/CER 对比表 | `kotone-eval` + 人工标注 | 待做 |
| 注入时序 | mock SendOps 单测（不真睡不真发键）+ 记事本集成测试（UIA 逐字校验） | cargo test + `scripts/notepad_inject_test.ps1` | ✅ |
| 状态机 | orchestrator 单测（mock 引擎/audio/inject/emitter，含 gen 过期丢弃） | cargo test（tokio::test） | ✅ 49 全过 |
| LOL 局内 | 人工验收清单（无边框训练模式，**需提权运行**） | 手动 | 待做 |
| 前端 | 组件测试 + IPC mock（vitest 未配置，暂以 svelte-check + demo 模式覆盖） | svelte-check ✅ | 部分 |

CI：GitHub Actions（Windows runner 为主）— fmt / clippy / cargo test（各 feature 组合）/ pnpm build / tauri build。**待做。**

已知测试环境事项：Windows tokio 定时器粒度 ~15.6ms，涉及时序的断言需留余量。

---

## 10. 风险登记（开发期跟踪）

| # | 风险 | 等级 | 状态 | 缓解 |
|---|------|------|------|------|
| R-1 | ~~游戏高权限运行时 UIPI 丢弃合成输入~~ | **已解决** | **实测通过（2026-07-24）** | 「asInvoker + 运行时检测 + 一键管理员重启」全链路实测：横幅提示 → runas 重启 → 提权后 LOL 训练模式发送成功 |
| R-2 | 独占全屏无法注入/叠 UI | 高 | 已知，接受 | 只保证无边框；设置页检测提示（Dota 无边框实测 overlay 正常） |
| R-3 | ~~Rust 复刻注入时序与 LeagueAkari 行为不一致~~ | **已解决** | **LOL 真机验证通过（2026-07-24）** | 记事本 UIA 校验 + LOL 训练模式实测均成功 |
| R-4 | ~~本机麦克风隐私封锁（0x80070005）~~ | **已解决** | **实测通过（2026-07-24）** | 用户开放权限后真麦克风全链路（A 流程）正常 |
| R-13 | ~~RegisterHotKey 游戏前台不投递热键~~ | **已解决** | **LL 钩子实测通过（2026-07-24）** | v6 改用 WH_KEYBOARD_LL（见 §3.6），LOL 前台热键正常触发，ACE 反作弊未拦截 |
| R-5 | 单一 STT 引擎速度/精度不达标 | 中 | 架构已缓解 | 可插拔多引擎 + 评测工具；默认引擎由 Phase 1 末人工评测决定 |
| R-6 | 多引擎抬高包体与维护面 | 中 | 已知 | cargo feature 按需编译；候选池引擎评测不通过即淘汰 |
| R-7 | 反作弊误报 | 中 | 监控 | 仅 SendInput；开源透明；免责声明；**注意 R-1 提权会提高敏感度，需在文档中说明提权原因仅为 UIPI** |
| R-8 | STT 推理与游戏抢资源 | 中 | 待验证 | 推说非常驻推理；CPU 回落；引擎能力页标注 GPU/CPU 占用 |
| R-9 | 中文黑话识别差 | 中 | MVP 缓解 | 热词表（引擎能力声明驱动 UI）；评测语料覆盖黑话 |
| R-10 | 模型下载体积劝退 | 低 | 已知 | 安装包不含模型；提供小模型选项 |
| R-11 | macOS 权限链劝退 | 低 | Phase 3 再议 | 权限引导页 |
| R-12 | 剪贴板恢复仅覆盖文本格式 | 低 | 已知取舍 | 用户剪贴板为图片等时不恢复；Unicode 逐字路径不受影响（LOL 默认） |

---

## 11. 决策变更记录

| 日期 | 变更 | 原因 |
|------|------|------|
| 2026-07-23 | 初版：桌面壳 Tauri 2、前端 Svelte 5、注入 raw windows crate、profile 默认值修正 | 基于 tech-research.md 立项 |
| 2026-07-23 | **STT 从「whisper.cpp sidecar 单引擎」改为「可插拔多引擎架构」**：`SttEngine` trait + 注册表；whisper.cpp sidecar 与 sherpa-onnx 流式双引擎首发；候选池 feature-gated | 单一方案速度/精度可能不达标，需多方案人工评测择优；评测结论出来前不锁定默认引擎 |
| 2026-07-23 | **流式支持从 Phase 2 提前为架构一等公民**：录音期统一 `push_audio`，partial 事件为核心 IPC 契约 | 流式与非流式引擎必须随时可换，下游不支持流式则换引擎要返工三层 |
| 2026-07-23 | **新增 eval 评测模块**（会话录档 / 语料回放 / 指标导出） | 「人工测试选引擎」需要可复现的工程支撑，而非口头体感 |
| 2026-07-23 | **交互模式用户可选**（hold / toggle），后续可扩 VAD hands-free；录音时悬浮窗实时回显（流式 partial / 非流式波形） | 不同游戏与操作习惯对触发方式需求不同；录音反馈是核心体验 |
| 2026-07-23 | **v3：`start_session` 增加 `events: UnboundedSender<SttEvent>` 参数**（partial 通道显式化） | 实现对齐：事件通道作为参数比隐含返回值更直接，语义不变 |
| 2026-07-23 | **v3：新增 `confirm_send` / `cancel_session` 两个 IPC 命令；`confirm_send` 接受 Preview 与 Error 两种状态** | §4.1 状态机的 Preview 确认/编辑与「Error 保留文本可重试」需要命令入口；前端首轮联调发现契约缺口 |
| 2026-07-23 | **v3：带文本的 Error 不自动回 Idle**（无文本 Error 维持自动回 Idle） | 「保留文本可重试」的语义闭环 |
| 2026-07-23 | **v3：overlay 窗口显隐改由后端驱动**（非 Idle 经 SW_SHOWNA 显示不抢焦点，Idle 隐藏；前端调用幂等共存） | 「按下热键即弹出悬浮条」是核心体验，不应依赖前端自行调窗口 API；且显示必须不抢焦点，否则注入前台校验必败 |
| 2026-07-23 | **v3：main/overlay 窗口 CloseRequested 拦截转 hide，仅托盘退出** | 托盘常驻语义；关设置窗导致整个应用退出是 Tauri 默认行为，不符合常驻工具定位 |
| 2026-07-23 | **v3：风险登记新增 R-1（UIPI 提权，高）与 R-4（麦克风隐私封锁，中）；R-3 降级** | 首轮实测新发现；注入机制风险下降，权限问题成为 LOL 真机验收的前置阻塞 |
| 2026-07-23 | **v4：提权方案定为「asInvoker + 运行时检测 + 一键管理员重启 + runAsAdminOnStart 选项」，不用 requireAdministrator 常驻清单**（R-1 原缓解方案之一） | 常驻提权每次启动弹 UAC，对麦克风工具过度打扰；检测驱动按需提权体验更好，且检测链路对真实 LOL 进程已实证 |
| 2026-07-23 | **v4：`InjectError` 增加 `needsElevation: bool` 字段；`detect_foreground_game` 返回附带 `targetElevated`** | 前端可据此显示提权引导，而非解析错误文案 |
| 2026-07-24 | **v5：preview 确认主路径改为「再按热键发送 / Esc 取消」；新增目标窗口记忆 + 发送前焦点恢复**（begin 记 hwnd → Sending 前 SetForegroundWindow + AttachThreadInput 兜底） | 用户实测：SW_SHOWNA 不抢焦点导致 Enter 穿透到目标窗口、点按钮又把焦点抢到 overlay 注入错目标。热键确认全程不碰鼠标，是游戏场景的自然交互 |
| 2026-07-24 | **v5：引入 tauri-plugin-single-instance**；热键注册失败状态暴露到设置页并可重试 | 用户实测：dev 重启多实例导致热键 already registered / WebView2 类注册错误 |
| 2026-07-24 | **v5：「以管理员身份重启」按钮未提权时常驻显示**；权限状态轮询；修复 activeGameElevated 断链（profile 读盘失败静默返回 null，改为内置 profile 回退） | 用户实测：找不到重启入口；勾选 runAsAdminOnStart 无反馈 |
| 2026-07-24 | **v6：热键后端从 RegisterHotKey 改为 WH_KEYBOARD_LL 低级钩子（Windows 默认），插件保留为回退** | 日志实证：RegisterHotKey 在 LOL 前台不投递任何热键事件（提权后也无效）；预研 §6.1 预留的 LL hook 方案启用 |
| 2026-07-24 | **v6：新增文件日志 `~/.kotone/kotone.log`**（启动/注册/触发/状态迁移） | GUI/提权进程无控制台，eprintln 无处可去；本次热键问题的定位即依赖该日志 |
| 2026-07-24 | **v7：MVP 注入链路全部真机验收通过**（用户手动实测）：A 记事本全链路、提权重启、LL 钩子游戏前台触发、**LOL 训练模式发送成功**。R-1/R-3/R-4/R-13 关闭 | 预研定义的最大风险（游戏注入）正式解除；剩余 MVP 缺口集中在真实 STT 引擎 |
| 2026-07-24 | **v8：重构为 cargo workspace 五 crate**（core / stt / platform-windows / cli / tauri），开发叙事从业务里程碑切换为架构决策（ADR 起始于 docs/adr/001） | 拆分判据：独立消费者、重依赖编译隔离、变更节奏。core 成为无 Tauri 可跑的独立包（kotone-cli listen 实证）；被否决项：每引擎一 crate、游戏 provider crate（数据驱动差异） |
| 2026-07-25 | **v9：归位为 `apps/` + `crates/` 产品 monorepo**（ADR-002）；whisper-cli 二进制管理从「Tauri sidecar」改为「kotone-stt 自管理（~/.kotone/bin/）」 | 根目录双重身份（JS+Rust）是脚手架与拆分的两次战术妥协叠加；Tauri sidecar 机制对 CLI 不可用，违背「core 无 Tauri 可跑」原则 |
| 2026-07-25 | **v10：双引擎接入 + eval 模块落地**（ADR-003/004/005）。sherpa 绑定选官方 crate（社区 sherpa-rs 已被上游收编弃用）；`engine-sherpa` feature 默认关（原生库 ~50MB）；sherpa 热词恒用 modified_beam_search（greedy 遇热词崩进程）；whisper 繁体问题挂起 | eval 实跑数据 sherpa 全面占优（首字 30ms/CER 0 vs whisper 2651ms/CER 0.143）；默认引擎正式决策仍待真人人声语料评测（Phase 1 末决策点） |
| 2026-07-25 | **v11：CLI 为一等消费者完整化**（docs/cli.md）；`WavFileBackend` 归 platform crate（AudioBackend 的虚拟采集实现，core 不放测试工装）；kotone-cli 默认启用 engine-sherpa（kotone-tauri 保持默认关）；无人值守测试双路径（wav 直灌 / VB-CABLE 回路，`scripts/e2e-virtual-audio.sh`） | 用户决策：界面是薄封装最后做，测试自动化优先；虚拟声卡路径的 partial 时间线含系统音频栈延迟，引擎对比以 eval replay 数据为准 |
| 2026-07-25 | **v12：交互模式业务建模（ADR-006/007）**——三个决策点策略组装替代硬编码分支；**预览只读化**（用户裁定：编辑是伪需求，要么发要么重说）；热键捕获录入；**VAD 与模式 3「说一句就走」落地** | 组合爆炸前的主动建模：流式/回显是引擎能力投影而非模式维度，组合数塌缩为 2×3×2 策略组装；VAD 推理复用 sherpa-onnx 内置实现避免引入第二套 ONNX Runtime |
| 2026-07-25 | **v13：core 识别历史模块 + CLI 收尾**——`history`（capped/keep-all/off，JSONL 追加，sessionId 与 eval 录档互查）进 settings 并由 orchestrator 在 sent/cancelled/error 终态落账；CLI 补 doctor / elevate / profile / log 四个一等子命令与 listen 提权预检 | 会话可观测性从「日志排查」升级为「结构化历史可查」；CLI 作为一等消费者补齐运维入口，doctor 把六类环境问题（设备/引擎/profile/提权/VAD/配置）变成一条命令的自检 |
| 2026-07-25 | **v14：elevate 语义修正为 sudo 式**——`kotone-cli elevate <command> [args...]` 透传执行子命令（替换语义，参数完全由调用方给定）；GUI 的「重启自身」语义（`restart_as_admin`）保留不变；runas 参数拼接升级为 MSVC/CommandLineToArgvW 完整转义规则 | 用户裁定：CLI 是无状态进程，「重启自身」副本打 help 即退出毫无意义；sudo 式透传让提权副本直接干正事（`elevate listen`）。原 quote_arg 只包空白不转引号，`--text "a \"quote\""` 类参数会拼坏，一并修正 |
| 2026-07-26 | **补记（评测期 P0 修复）**：`8a49e9f` X-ASR 崩溃三连防——bpe.vocab 文本格式探测（二进制/畸形一律不下传，防 C++ 解析直接 exit 进程）+ 从 bpe.model 现场导出兜底（纯 protobuf wire 解析，不引 sentencepiece 依赖）；`7fcbacd` 松手丢句尾——采音侧排空 + 800ms 静音尾帧 + decode 排空上限 | 六引擎评测真机暴露：热词词表格式问题是崩溃级 P0；甩尾是松手即停的采音-解码竞争，评测语料「能听到我说话吗」复验 CER 0.000 |
| 2026-07-26 | **v15：六引擎评测收官，默认引擎定 X-ASR**（sherpa-onnx-x-asr-zh-en）。10 条标注语料（游戏黑话/中英混说/甩尾场景）实测：X-ASR CER 0.008 + 首字 70ms + 最终 31ms，162MB——精度与 FunASR-Nano 并列第一（0.008）但最终延迟 31ms vs 1305ms、体积 1/6，且是唯一流式冠军；SenseVoice CER 0.062 居非流式质量备选 | eval report 复验（docs/eval-playbook.md 成绩表）；流式回显是核心体验，同精度下延迟与体积碾压，默认引擎无悬念 |
| 2026-07-26 | **v15：砍三引擎**——whisper.cpp sidecar（v10 起评测持续落后：非流式 + 每次 spawn 加载模型的架构性延迟 + 繁体问题）、老 zipformer（2023 模型无标点，能力被 X-ASR 完全覆盖）、Qwen3-ASR（938MB 非流式，精度与 FunASR-Nano 同档无差异化优势）。保留 X-ASR（默认）/ SenseVoice / FunASR-Nano（热词最强档）/ mock；ggml 清单、whisper-cli bin 下载通道（zip 依赖一并移除）、CLI 短名、相关 e2e 同步清除；在线 transducer 骨架保留给 X-ASR | 砍差留优收窄维护面：引擎面 6→3+mock，清单/测试/文档同步减半；默认引擎 id 全局切 `sherpa-onnx-x-asr-zh-en`（settings 默认、doctor、CLI、首启向导推荐） |
| 2026-07-26 | **v15：eval_recording 默认关**（通用页底部加「评测录档」开关；已录语料不动，CLI/doctor 行为不变） | 评测收官后录档回到「按需开」：常态使用不再默认落 wav+json，减少磁盘占用与隐私面 |
| 2026-07-26 | **v15：模型下载镜像**——settings 新增 `download.source`（auto 默认 / official / mirror）+ `download.ghProxy`（默认 `https://ghfast.top/`）；URL 重写纯函数：HF host 换 `hf-mirror.com`、GitHub 直链拼 ghProxy 前缀；auto 模式镜像失败回退官方一次（SHA 校验失败同样触发）；model.rs 统一下载入口走 `download_resolved`，CLI/GUI 共用；doctor 显示下载源 | HF/GitHub 国内直连常超时，镜像显著提速；公益代理稳定性无保障故前缀做成配置项——失效换前缀即可，无需发版 |
| 2026-07-24 | **v5：preview 交互不抢焦点三连修**——begin 记录前台 hwnd 为注入目标（`FocusBackend` 抽象），Sending 前先 `SetForegroundWindow` 恢复焦点（AttachThreadInput 兜底）；toggle 热键在 Preview 态路由为 `confirm_send`；Esc 临时注册从仅 Listening 扩展到全部非 Idle 态；前端 overlay 显隐调用移除（后端 SW_SHOWNA 全权驱动） | 用户实测：preview 按 Enter 键去了记事本（焦点未跟随悬浮条）、点「发送」按钮激活 overlay 导致文字注入给 overlay 自己 |
| 2026-07-24 | **v5：引入 tauri-plugin-single-instance；热键注册失败状态经 `get_hotkey_status` 暴露到设置页** | 用户实测：`pnpm tauri dev` 重启时旧实例未退出 → 热键 already registered / WebView2 类注册冲突 |
| 2026-07-24 | **v5：设置页权限分区「以管理员身份重启」未提权时常驻显示；权限状态 3s 轮询（页面隐藏暂停）；`runAsAdminOnStart` 勾选后提示「下次启动生效」+ 立即重启链接；`get_elevation_status` 链路修复**（profile 文件缺失回退内置 profile，纯逻辑 `resolve_active_game_pid` 可单测） | 用户实测：重启入口只在横幅条件触发时出现；勾选自动提权无反馈；LOL 运行时 activeGameElevated 仍可能返回 null |

---

*本文档与预研报告的关系：预研回答「走哪条路」，本文档回答「怎么走」，并随实现进展持续记录偏差与新发现（见 §1.1 / §11）。品牌资产由 RepoChan 流水线提供，与工程选型正交。*
