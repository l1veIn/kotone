# Kotone 开发文档

> 状态：开发立项 · 技术方案已定（v2：STT 改为可插拔多引擎架构）
> 日期：2026-07-23
> 上游文档：[docs/tech-research.md](./tech-research.md)（技术预研报告）
> 本文档回答：用什么技术、为什么、怎么组织代码、按什么顺序开发。

---

## 1. 文档目的

预研报告完成的是「探路」：对比了 Tauri vs Electron、whisper.cpp vs sherpa-onnx、SendInput vs 剪贴板等候选方案，并用 LeagueAkari（MIT）验证了 LOL 局内注入路径。

本文档完成的是「拍板」：把预研中的候选方案收敛为唯一决策，给出可直接开工的架构、模块边界、数据模型、IPC 契约和开发顺序。后续开发中如需偏离本文档的决策，需在本文档中记录变更原因。

**v2 修订要点**：STT 不再绑定单一引擎。考虑到单一方案（如 whisper.cpp small）在速度与精度上可能不达标，STT 层设计为**可插拔多引擎架构**，下游链路（orchestrator → IPC → UI）**原生支持流式 partial 结果**，并内置**引擎评测工具**支撑多款方案的人工对比测试。交互模式（push-to-talk 的 hold/toggle 等）由用户可选，录音过程中悬浮窗实时回显识别内容。

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

#### 引擎抽象设计

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
    fn start_session(&self, cfg: &SessionConfig) -> Result<Box<dyn SttSession>>;
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
| `whisper-cpp-sidecar` | sidecar 子进程（whisper-cli） | ✗（finalize-only） | **Phase 1 首发** | 闭环基线、离线兜底 |
| `sherpa-onnx-zipformer-zh` | FFI（sherpa-onnx crate） | ✓ | **Phase 1 并行接入** | 中文流式主力候选 |
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

### 3.6 热键与悬浮窗

**热键：tauri-plugin-global-shortcut。**

- 默认键位：`F8`（toggle）与 `Alt+V`（hold），首次启动引导选择交互模式与键位，并检测与常见游戏键位冲突。
- 预研中提到的 `WH_KEYBOARD_LL` 低级钩子**MVP 不做**——global-shortcut 插件已覆盖需求，仅在将来需要「游戏中任意键状态感知」时再加。

**悬浮窗：Tauri 多窗口。**

- 主悬浮条：`always_on_top` + `decorations: false` + `transparent: true` + `skip_taskbar`。录音时弹出紧凑条（波形 + 流式文本），idle 时隐藏或收缩为小圆点。
- 设置窗口：独立窗口，从托盘菜单唤起。
- 点击穿透：MVP 不做（Phase 2，`set_ignore_cursor_events` 空闲时穿透）。
- **独占全屏不保证**：设置页检测全屏状态并提示用户切换无边框/窗口化，文档中明示。

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
│  │ ├─ whisper-cpp-sidecar │ │ │ SendInput/剪贴板 │           │
│  │ ├─ sherpa-onnx (FFI)   │ │ └─────────────────┘           │
│  │ └─ <feature-gated 更多>│ │ ┌─────────────────┐           │
│  └──────┬─────────────────┘ │ │ profile 游戏配置  │           │
│         │ SttEvent 通道      │ │ 前台进程匹配      │           │
│  ┌──────▼─────────────────┐ │ └─────────────────┘           │
│  │ eval 评测录档/回放       │ │                               │
│  └────────────────────────┘ │                               │
│  ┌──────────────────────────▼───────────────┐               │
│  │ settings · tray · model downloader       │               │
│  └──────────────────────────────────────────┘               │
└─────────────────────────────────────────────────────────────┘
```

**设计要点：**

1. **orchestrator 是唯一的状态所有者。** UI 不维护业务状态，只渲染 Rust 侧 emit 的状态事件。所有状态迁移（idle → listening → transcribing → preview/sending → success/error）在 Rust 侧完成，避免前后端状态不一致。
2. **STT 与 inject 完全解耦，且 STT 内部多引擎解耦。** orchestrator 只面向 `SttEngine` trait 编程；inject 只接收最终文本。「仅复制」降级模式、换引擎、加引擎、流式升级都不影响其他层。
3. **流式是一等公民。** 录音期间 PCM 持续推入 session，partial 经事件通道直达 UI；非流式引擎只是「不产生 partial 的特例」。
4. **发送可取消。** 发送时序中有多个 delay，用户按 Esc 或再次按热键应能中止（`tokio::sync::watch` 取消标志，对齐 LeagueAkari 的 AbortController）。

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
        │              │
        │     ok       │         fail
        └── Success toast ◄────┴────► Error toast（保留文本，可重试）
```

任意状态下按 Esc / 取消热键 → 回到 Idle（session cancel；发送中时序安全中止）。

---

## 5. 模块设计

### 5.1 Rust 侧（src-tauri/src）

| 模块 | 文件 | 职责 | 关键依赖 |
|------|------|------|----------|
| `hotkey` | `hotkey.rs` | 注册/注销全局热键，hold/toggle 两种触发模式，冲突检测 | tauri-plugin-global-shortcut |
| `audio` | `audio.rs` | 设备枚举、16kHz mono 采集、PCM 流推送、wav 编码（录档用） | cpal, hound |
| `stt` | `stt/mod.rs` | `SttEngine` / `SttSession` trait 定义、引擎注册表、当前引擎路由 | tokio |
| `stt::whisper_sidecar` | `stt/whisper_sidecar.rs` | whisper-cli sidecar 生命周期，wav → 文本（finalize-only），initial_prompt 热词 | tauri sidecar |
| `stt::sherpa` | `stt/sherpa.rs` | sherpa-onnx FFI 接入，流式 session，partial 回调 → SttEvent | sherpa-onnx（feature `engine-sherpa`） |
| `eval` | `eval.rs` | 会话录档（wav + 指标 JSONL）、语料回放、多引擎对比 | serde_json |
| `inject` | `inject/mod.rs`, `inject/windows.rs` | `send_unicode` / `key_down_up` / `is_process_foreground` | windows crate |
| `orchestrator` | `orchestrator.rs` | 状态机，串联 hotkey→audio→stt→inject，partial 转发，取消与超时 | tokio |
| `profile` | `profile.rs` | 游戏 profile CRUD、前台进程匹配 | sysinfo, windows crate |
| `settings` | `settings.rs` | 用户配置读写（`~/.kotone/config.json`） | serde_json |
| `model` | `model.rs` | 各引擎模型下载/校验/切换 | reqwest, sha2 |
| `tray` | `tray.rs` | 托盘菜单：显示悬浮条 / 设置 / 退出 | tauri tray |

### 5.2 前端（src/）

| 模块 | 职责 |
|------|------|
| `lib/stores/state.ts` | 订阅 `kotone://state` / `kotone://partial` / `kotone://level` 的 Svelte store，UI 唯一数据源 |
| `lib/components/OverlayBar.svelte` | 悬浮录音条：波形动画、**流式 partial 文本区**、状态指示、结果预览与编辑 |
| `lib/components/Waveform.svelte` | 实时音量波形（Rust 侧推送 RMS 电平） |
| `routes/settings/` | 设置页：交互模式与热键、麦克风、**STT 引擎切换与模型管理**、游戏 profile 编辑器、评测数据导出 |
| `lib/ipc.ts` | 所有 `invoke` 调用的类型化封装 |

### 5.3 IPC 契约（commands）

```typescript
// 设置与配置
get_settings()              -> Settings
update_settings(patch)      -> Settings
list_audio_devices()        -> AudioDevice[]
set_audio_device(id)        -> void

// STT 引擎
list_stt_engines()          -> EngineInfo[]   // id / displayName / capabilities / isReady / 延迟统计
set_stt_engine(id)          -> void           // 切换引擎（未就绪则提示下载模型）
get_engine_options(id)      -> Json           // 引擎专有配置项（如 whisper 线程数）

// 游戏 profile
list_profiles()             -> GameProfile[]
save_profile(profile)       -> void
detect_foreground_game()    -> GameProfile | null

// 模型
list_models()               -> ModelInfo[]    // 跨引擎统一列出：已下载/可下载
download_model(id)          -> Progress（event 流）
set_active_model(engineId, modelId) -> void

// 评测
eval_list_sessions()        -> EvalSession[]  // 录档列表
eval_replay(sessionId, engineId) -> EvalResult // 同一音频换引擎重放
eval_export()               -> path           // 导出 JSONL + wav 包

// 手动触发（调试/测试用）
simulate_send(text, profileId) -> Result<(), InjectError>
```

**事件（Rust → UI）：**

```typescript
"kotone://state"    { state: "idle"|"listening"|"transcribing"|"preview"|"sending"|"success"|"error", payload?: {...} }
"kotone://partial"  { text: string }             // 流式引擎录音期间持续推送；核心契约，非流式引擎不发
"kotone://level"    { rms: number }              // 录音音量，驱动波形
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
  → orchestrator 创建 STT session（当前引擎）→ Listening，悬浮条弹出
  → 录音 PCM 持续 push_audio
       流式引擎：partial → emit "kotone://partial" → 悬浮条实时上屏
       非流式引擎：仅波形 + 「聆听中…」
hotkey 结束
  → session.finalize() → 最终文本 T（emit transcribing → 文本上屏）
  → autoSend=false：Preview 状态，用户确认/编辑后继续
  → inject::is_process_foreground(profile.processNames)
       false → Error toast「游戏不在前台」→ 保留文本可重试
  → key_down_up(openChatKey)          // VK_RETURN, scan code 经 MapVirtualKey
  → sleep(20ms)
  → preferClipboardPaste ?
       arboard 写入 + Ctrl+V
     : send_unicode(T)                // encode_utf16 逐单元 KEYEVENTF_UNICODE down+up
  → sleep(20ms)
  → key_down_up(sendKey)
  → Success toast「收到，已发送！✨」→ Idle
  → eval 录档落盘（wav + 指标，可在设置中关闭）
```

取消点：录音中 cancel session；finalize 设置 10s 超时；发送时序每次 sleep 前后检查取消标志。

---

## 7. 项目结构

```
kotone/
├─ package.json / pnpm-lock.yaml
├─ vite.config.ts / svelte.config.js / tailwind.config.ts
├─ src/                        # Svelte 前端
│  ├─ routes/ (overlay, settings)
│  └─ lib/ (stores, components, ipc.ts)
├─ src-tauri/
│  ├─ Cargo.toml               # features: engine-whisper-sidecar / engine-sherpa / ...
│  ├─ tauri.conf.json
│  ├─ src/
│  │  ├─ main.rs / orchestrator.rs / hotkey.rs / audio.rs
│  │  ├─ stt/ (mod.rs, whisper_sidecar.rs, sherpa.rs, ...)
│  │  ├─ inject/ (mod.rs, windows.rs)
│  │  ├─ eval.rs / profile.rs / settings.rs / model.rs / tray.rs
│  ├─ binaries/                # whisper-cli sidecar（构建期下载）
│  └─ icons/
├─ assets/                     # RepoChan 品牌资产（已有）
├─ docs/
│  ├─ tech-research.md         # 预研报告（已有）
│  └─ development.md           # 本文档
└─ .github/workflows/ci.yml
```

---

## 8. 开发计划与里程碑

### Phase 0：技术 Spike（第 1 周）

按预研 §11 执行，每个 spike 一天内出 go/no-go：

| # | Spike | 通过标准 | 对应模块 |
|---|-------|----------|----------|
| 1 | Tauri 2 骨架 + 全局热键 + 透明置顶悬浮窗 | 游戏前台时热键触发、窗可见 | hotkey, 悬浮窗 |
| 2 | `SttEngine` trait 骨架 + whisper.cpp sidecar 转写 3s 中文 | 延迟与准确率基线数据（记录，不作 go/no-go 硬门槛） | stt, audio |
| 3 | **Rust SendInput 复刻 LeagueAkari 时序，LOL 训练模式实测** | 前台检测 + Enter×2 + Unicode 字符串，10 次 ≥ 8 成功 | inject |
| 4 | sherpa-onnx 流式 session 同句对比，partial 事件打通 | partial 延迟 < 500ms 体感流畅；与 whisper 基线对比数据落档 | stt::sherpa |

Spike 3 失败时的降级：转写 + 复制到剪贴板（功能保留，差异化减弱），并排查前台进程名 / 无边框 / 改键。

### Phase 1：MVP（第 2–4 周）

1. Tauri 2 骨架 + 托盘 + 全局热键（**hold / toggle 双模式可选**）
2. **STT 引擎抽象落地**：`SttEngine` trait + 注册表 + 引擎 #1 whisper.cpp sidecar
3. **引擎 #2 sherpa-onnx 流式接入**，partial → 悬浮条实时上屏全链路打通
4. **评测工具 v1**：会话录档 + 语料回放 + 延迟/文本对比导出
5. 通用注入：任意前台窗口（记事本回归测试）
6. LOL profile：Enter → Unicode → Enter（无边框实测）
7. 设置页：交互模式与热键 / 麦克风 / **STT 引擎切换** / autoSend / profile 选择
8. 模型下载器 + 品牌悬浮 UI（沿用 RepoChan 色板 `#00E5FF` / `#1A1A2E` / `#FF2D78` / `#7B2FFF`）

**Phase 1 末决策点：默认 STT 引擎。** 用评测工具积累的真实语料（游戏短句 + 黑话 + 耳麦噪音）做人工对比，结论记录到 §11。

**MVP 验收标准（在预研 §10 基础上修订）：**

- [ ] 记事本路径中文短句上屏成功率 > 95%
- [ ] LOL 无边框训练模式发送 10 次 ≥ 8 成功
- [ ] 空闲内存 < 150MB（不含模型）
- [ ] 至少两款引擎完成接入并可设置页切换；流式引擎录音时 partial 可见
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

| 层 | 方式 | 工具 |
|----|------|------|
| profile 匹配、配置合并等纯逻辑 | Rust 单元测试 | cargo test |
| STT 引擎契约 | 各引擎跑同一 fixture wav，断言 `SttSession` 行为（push/finalize/cancel） | cargo test + fixtures |
| **STT 引擎横向评测** | eval 回放：同一语料库 × 全部引擎 → 延迟/CER 对比表 | `kotone-eval` + 人工标注 |
| 注入时序 | 记事本集成测试（CI 上 Windows runner） | 自动化脚本 |
| 状态机 | orchestrator 单测（mock audio/stt/inject，含 mock 流式引擎发 partial） | cargo test |
| LOL 局内 | 人工验收清单（无边框训练模式） | 手动 |
| 前端 | 组件测试 + IPC mock | vitest |

CI：GitHub Actions（Windows runner 为主）— fmt / clippy / cargo test（各 feature 组合）/ pnpm build / tauri build。

---

## 10. 风险登记（开发期跟踪）

| 风险 | 等级 | 状态 | 缓解 |
|------|------|------|------|
| 独占全屏无法注入/叠 UI | 高 | 已知，接受 | 只保证无边框；设置页检测提示 |
| Rust 复刻注入时序与 LeagueAkari 行为不一致 | 中 | Spike 3 验证 | 1:1 对照 input.cc；失败降级「转写+复制」 |
| **单一 STT 引擎速度/精度不达标** | 中 | **架构已缓解** | 可插拔多引擎 + 评测工具；默认引擎由 Phase 1 末人工评测决定 |
| 多引擎抬高包体与维护面 | 中 | 已知 | cargo feature 按需编译；候选池引擎评测不通过即淘汰，不进发布版 |
| 反作弊误报 | 中 | 监控 | 仅 SendInput；开源透明；免责声明 |
| STT 推理与游戏抢资源 | 中 | Spike 2/4 验证 | 推说非常驻推理；CPU 回落；引擎能力页标注 GPU/CPU 占用 |
| 中文黑话识别差 | 中 | MVP 缓解 | 热词表（引擎能力声明驱动 UI）；评测语料覆盖黑话 |
| 模型下载体积劝退 | 低 | 已知 | 安装包不含模型；提供小模型选项 |
| macOS 权限链劝退 | 低 | Phase 3 再议 | 权限引导页 |

---

## 11. 决策变更记录

| 日期 | 变更 | 原因 |
|------|------|------|
| 2026-07-23 | 初版：桌面壳 Tauri 2、前端 Svelte 5、注入 raw windows crate、profile 默认值修正 | 基于 tech-research.md 立项 |
| 2026-07-23 | **STT 从「whisper.cpp sidecar 单引擎」改为「可插拔多引擎架构」**：`SttEngine` trait + 注册表；whisper.cpp sidecar 与 sherpa-onnx 流式双引擎首发；候选池 feature-gated | 单一方案速度/精度可能不达标，需多方案人工评测择优；评测结论出来前不锁定默认引擎 |
| 2026-07-23 | **流式支持从 Phase 2 提前为架构一等公民**：录音期统一 `push_audio`，partial 事件为核心 IPC 契约 | 流式与非流式引擎必须随时可换，下游不支持流式则换引擎要返工三层 |
| 2026-07-23 | **新增 eval 评测模块**（会话录档 / 语料回放 / 指标导出） | 「人工测试选引擎」需要可复现的工程支撑，而非口头体感 |
| 2026-07-23 | **交互模式用户可选**（hold / toggle），后续可扩 VAD hands-free；录音时悬浮窗实时回显（流式 partial / 非流式波形） | 不同游戏与操作习惯对触发方式需求不同；录音反馈是核心体验 |

---

*本文档与预研报告的关系：预研回答「走哪条路」，本文档回答「怎么走」。品牌资产由 RepoChan 流水线提供，与工程选型正交。*
