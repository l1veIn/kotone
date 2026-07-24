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

**已验证**：注入机制正确（记事本中文短句 4/4 逐字一致）；状态机全链路（cargo 集成测试）；前端各状态渲染（浏览器 demo + Tauri 内 error 态实测）。

**待验证（阻塞原因见 §10，均需 LOL 退出后的桌面窗口期）**：

- [ ] 记事本长句（>100 字）/ emoji 各 10 次成功率复跑（脚本 `scripts/notepad_inject_test.ps1` 已入库）
- [ ] 提权链路实测：设置页横幅 → 管理员重启 → LOL 训练模式发送 10 次 ≥ 8（提权方案已实现，见 §10 R-1）
- [ ] 麦克风全链路（F8 → 真录音 → partial）——用户已开放麦克风权限（2026-07-23），待复测确认 R-4 解除
- [ ] 未提权对 LOL 发送 → Error payload 带 `needsElevation: true` + 提权文案实测
- [ ] 空闲内存 < 150MB 实测（dev 进程 ~40MB，含 WebView2 待 release 复测）

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
| STT 引擎 #1（首发） | **whisper.cpp，sidecar 子进程方式**（ggml-small，首启下载） | FFI 嵌入（后续） | 跑通闭环最快、崩溃隔离、打包简单 |
| STT 引擎 #2（首发并行） | **sherpa-onnx 流式 Zipformer-zh（FFI）** | SenseVoice ONNX | 中文流式 + 低延迟，与 #1 形成对比组 |
| STT 引擎 #3+（候选池） | FunASR ONNX / 云端 API（OpenAI、国内 ASR） | — | 人工评测后择优纳入或淘汰 |
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
    fn id(&self) -> &'static str;              // "whisper-cpp-sidecar" 等
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
| `mock-stream` | 内置 mock | ✓ | **已实现**（联调用，不进发布版或标注为调试） | 全链路联调 |
| `whisper-cpp-sidecar` | sidecar 子进程（whisper-cli） | ✗（finalize-only） | 接口已注册，实现待做 | 闭环基线、离线兜底 |
| `sherpa-onnx-zipformer-zh` | FFI（sherpa-onnx crate） | ✓ | 接口已注册，实现待做 | 中文流式主力候选 |
| `whisper-cpp-ffi` | FFI（whisper-rs） | ✗ | Phase 2 | 降延迟的 whisper 路径 |
| `sherpa-onnx-sensevoice` | FFI | ✗（快批式） | 候选池 | 中文高精度候选 |
| `funasr-paraformer` | ONNX 本地服务 | ✓ | 候选池 | 中文工业级标杆 |
| `cloud-asr` | HTTP/WebSocket | ✓ | 候选池（可选增强） | 精度上限参照系 |

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

- 各引擎自带模型声明（ID、大小、下载地址、SHA256），统一走 `model` 模块下载与校验。
- 安装包不含任何模型；首次启动引导下载当前引擎的默认模型（whisper small ~466MB / sherpa-onnx zipformer-zh 量级相近）。
- 用户可在设置中切换引擎，未就绪的引擎显示「需下载模型」。

### 3.4 交互模式：push-to-talk 为主，模式用户可选，录音实时回显

**决策：**

1. **默认 push-to-talk**，提供两种子模式，用户在设置中选择：
   - **hold**：按住说话，松手结束（游戏内语音键习惯，默认）；
   - **toggle**：按一下开始，再按一下结束（长句/双手操作场景）。
2. **交互模式与识别引擎是正交的两个配置维度。** 无论 hold 还是 toggle，对 orchestrator 都是「开始会话 → 持续 push_audio → 结束会话」；后续增加 VAD 免按键模式（hands-free）只改触发层，不动 STT 链路。
3. **录音中小悬浮窗实时回显。** 按下热键即弹出紧凑悬浮条：
   - 流式引擎：**partial 文本边说边上屏**（附波形动画），松手后替换为最终文本；
   - 非流式引擎：显示波形 + 「聆听中…」，松手后显示「转写中…」再出结果。
   UI 通过 `kotone://partial` 事件驱动，不关心引擎是否流式——没有 partial 事件就停留在波形态。

不做（MVP）：全时 VAD 免按键（误触发与资源风险，Phase 3 作为第三种交互模式评估）。

**v3 实现细节**：hold 模式用插件 `on_shortcut` 的 Pressed/Released 区分；toggle 只响应 Pressed（Idle→开始、Listening→结束、其他状态→取消）。**Esc 取消不常驻注册**（避免劫持游戏 Esc），仅在 Listening 期间临时全局注册，离开即注销。

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
│  │ ├─ whisper-cpp-sidecar │ │ └─────────────────┘           │
│  │ ├─ sherpa-onnx (FFI)   │ │ ┌─────────────────┐           │
│  │ └─ <feature-gated 更多>│ │ │ profile 游戏配置  │ ✓        │
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
| `stt::whisper_sidecar` | `stt/whisper_sidecar.rs` | whisper-cli sidecar 生命周期，wav → 文本（finalize-only），initial_prompt 热词 | 接口注册，实现待做 |
| `stt::sherpa` | `stt/sherpa.rs` | sherpa-onnx FFI 接入，流式 session，partial 回调 → SttEvent | 接口注册，实现待做 |
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
  "sttEngine": "whisper-cpp-sidecar",            // 当前引擎，设置页可切换
  "engineOptions": {                              // 引擎专有配置
    "whisper-cpp-sidecar": { "model": "ggml-small", "threads": 4 },
    "sherpa-onnx-zipformer-zh": { "model": "zipformer-zh-small", "provider": "cpu" }
  },
  "autoSend": false,               // true: 转写完直接发；false: 先预览确认
  "activeProfileId": "lol",
  "language": "zh",
  "evalRecording": true            // 评测录档开关（默认开，可在设置中关）
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
  "engineId": "sherpa-onnx-zipformer-zh",
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

```
kotone/
├─ index.html / package.json / pnpm-lock.yaml / pnpm-workspace.yaml
├─ vite.config.ts / svelte.config.js / tsconfig.json / tailwind (v4, css 方式)
├─ src/                        # Svelte 前端
│  ├─ main.ts / App.svelte (hash 路由) / app.css
│  ├─ routes/ (overlay, settings)
│  └─ lib/ (stores, components, ipc.ts)
├─ scripts/
│  └─ notepad_inject_test.ps1  # 注入记事本回归脚本（UIA 逐字校验）
├─ src-tauri/
│  ├─ Cargo.toml               # features: engine-whisper-sidecar / engine-sherpa / ...
│  ├─ tauri.conf.json          # 双窗口：overlay(480x120 透明置顶) + main(800x600)
│  ├─ capabilities/default.json
│  ├─ examples/inject_cli.rs   # 注入命令行测试入口
│  ├─ src/
│  │  ├─ main.rs / lib.rs / orchestrator.rs / hotkey.rs / audio.rs
│  │  ├─ stt/ (mod.rs, mock.rs, whisper_sidecar.rs, sherpa.rs)
│  │  ├─ inject/ (mod.rs, windows.rs)
│  │  ├─ eval.rs / profile.rs / settings.rs / model.rs / tray.rs
│  ├─ binaries/                # whisper-cli sidecar（构建期下载，待做）
│  └─ icons/                   # 由 assets/kotone-foundation.png 生成
├─ assets/                     # RepoChan 品牌资产（已有）
├─ docs/
│  ├─ tech-research.md         # 预研报告（已有）
│  └─ development.md           # 本文档
└─ .github/workflows/ci.yml    # 待做
```

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
| R-1 | **游戏高权限运行时 UIPI 丢弃合成输入（实证）** | **高** | **方案已实现（d6d78ea），实测待游戏退出** | 方案定为「asInvoker + 运行时检测 + 一键管理员重启」（非常驻 requireAdministrator）：`elevation.rs` TokenElevation 检测自身与目标进程权限（OpenProcess 被拒视为目标更高权限）；`restart_as_admin` 经 ShellExecuteExW runas 重启（UAC 取消不退出、防循环标记 `--kotone-elevated-spawn`）；设置页权限分区 + 品红横幅；`runAsAdminOnStart` 可选开机自动提权；InjectError 带 `needsElevation` 字段 + 提权文案。对真实 LOL 进程检测已实证 Some(true)。剩余：横幅/重启/真机发送实测 |
| R-2 | 独占全屏无法注入/叠 UI | 高 | 已知，接受 | 只保证无边框；设置页检测提示（Dota 无边框实测 overlay 正常） |
| R-3 | Rust 复刻注入时序与 LeagueAkari 行为不一致 | 低（原中） | **机制已验证** | 记事本 UIA 逐字校验 PASS；LOL 时序待 R-1 实测复核 |
| R-4 | ~~本机麦克风隐私封锁（0x80070005）~~ | 低（原中） | **用户已解除（2026-07-23），待复测确认** | 用户已在 Windows 设置中允许所有应用使用麦克风；audio 层有清晰中文报错兜底。待 LOL 退出后重跑 F8 全链路确认解除 |
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
| 2026-07-24 | **v5：preview 交互不抢焦点三连修**——begin 记录前台 hwnd 为注入目标（`FocusBackend` 抽象），Sending 前先 `SetForegroundWindow` 恢复焦点（AttachThreadInput 兜底）；toggle 热键在 Preview 态路由为 `confirm_send`；Esc 临时注册从仅 Listening 扩展到全部非 Idle 态；前端 overlay 显隐调用移除（后端 SW_SHOWNA 全权驱动） | 用户实测：preview 按 Enter 键去了记事本（焦点未跟随悬浮条）、点「发送」按钮激活 overlay 导致文字注入给 overlay 自己 |
| 2026-07-24 | **v5：引入 tauri-plugin-single-instance；热键注册失败状态经 `get_hotkey_status` 暴露到设置页** | 用户实测：`pnpm tauri dev` 重启时旧实例未退出 → 热键 already registered / WebView2 类注册冲突 |
| 2026-07-24 | **v5：设置页权限分区「以管理员身份重启」未提权时常驻显示；权限状态 3s 轮询（页面隐藏暂停）；`runAsAdminOnStart` 勾选后提示「下次启动生效」+ 立即重启链接；`get_elevation_status` 链路修复**（profile 文件缺失回退内置 profile，纯逻辑 `resolve_active_game_pid` 可单测） | 用户实测：重启入口只在横幅条件触发时出现；勾选自动提权无反馈；LOL 运行时 activeGameElevated 仍可能返回 null |

---

*本文档与预研报告的关系：预研回答「走哪条路」，本文档回答「怎么走」，并随实现进展持续记录偏差与新发现（见 §1.1 / §11）。品牌资产由 RepoChan 流水线提供，与工程选型正交。*
