# Kotone 技术预研报告

> 状态：预研完成 · 待立项决策  
> 日期：2026-07-23  
> 范围：桌面框架 · 语音识别 · 游戏输入注入 · 全局快捷键 · MVP 架构

---

## 1. 产品技术约束（从需求反推）

| 约束 | 来源 | 技术含义 |
|------|------|----------|
| 游戏中不中断操作 | 核心痛点 | 全局热键 + 后台录音；发送链路尽可能短 |
| 一键进游戏聊天框 | 差异化 | OS 级键鼠模拟 + 每游戏配置（开聊/发送键） |
| 跨平台桌面 | 访谈 q6 | 框架需 Win/macOS/Linux；**Windows 是主战场** |
| 游戏已占大量资源 | 场景事实 | 空闲内存尽量低；STT 推理尽量不抢 GPU 或可回落 CPU |
| 中文游戏黑话 | 人设/场景 | 热词/词典；中文流式 ASR 优于通用 Whisper |
| 适度扩展 | 访谈 q7 | 热键 + 多游戏配置；不做生态平台 |
| 悬浮窗深色电竞 UI | README | always-on-top 透明 overlay，少抢焦点 |

**最难、最易翻车的点不是 STT，而是「把文字可靠送进前台游戏聊天框」**（全屏独占、反作弊、焦点、时序）。

---

## 2. 竞品与同类项目速览

| 产品 | 定位 | 技术信号 | 对 Kotone 的启示 |
|------|------|----------|------------------|
| [VoiceInk](https://github.com/Beingpax/VoiceInk) | macOS 本地听写 | Swift 原生 + whisper.cpp | 本地、按键说话、无订阅；**无游戏一键发送** |
| Superwhisper / Wispr Flow | 通用听写 | 本地 / 云端 ASR + 粘贴进当前输入框 | 交互范式成熟；**不做游戏适配** |
| [Whisperi](https://github.com/xarthurx/whisperi) | Windows 听写 | **Tauri 2 + Rust**，转写后 paste | 技术栈高度可参考 |
| Whisper Desktop 等 | 桌面 Whisper 壳 | Tauri + 拉起 whisper.cpp 进程 | 进程隔离 STT 可行 |
| [Voxtype](https://github.com/peteonrails/voxtype) | Linux 推说 STT | 单二进制 Rust + Whisper | 轻量、离线优先 |
| AutoHotkey 脚本 | 游戏宏/聊天 | SendInput 进 LOL 聊天 | 证明「键入路径」可行，缺产品化 |

**市场空白**：通用听写工具很多，**「游戏场景：语音 → 文 → 自动开聊框 → 发送 → 回到操作」** 的产品化方案稀缺。这正是 Kotone 的壁垒。

---

## 3. 桌面框架：Tauri 2 vs Electron

### 3.1 对比（与游戏副工具场景相关）

| 维度 | Tauri 2.x | Electron | 游戏副工具倾向 |
|------|-----------|----------|----------------|
| 安装包 | 数 MB 级 | 常 80–200+ MB | Tauri |
| 空闲内存 | 约 30–80 MB 量级 | 约 150–300+ MB | Tauri（与游戏并存） |
| 冷启动 | 更快 | 偏慢 | Tauri |
| 系统能力 | Rust 侧直接调 OS API | Node + 原生 addon | 两者均可，Tauri 更贴注入/热键 |
| 生态成熟度 | 2.0 后快速成熟 | 极成熟 | Electron 略优 |
| 前端 | 任意 Web（React/Svelte/Vue） | 同左 | 平手 |
| Windows 系统 WebView | WebView2 | 自带 Chromium | Electron 一致性略好 |

公开基准（不同文章数字略有出入，方向一致）：Tauri 包体可小一个数量级，空闲内存通常明显更低。Windows 上二者都依赖 Chromium 系渲染时，内存差会缩小，但**仍建议默认选 Tauri**——副工具与 3A/LOL 同机运行时，每一百 MB 都敏感。

### 3.2 结论

**推荐：Tauri 2 + 前端（Svelte 或 React）+ Rust 核心逻辑。**

理由：

1. 低占用、小包体契合「游戏开着还能挂着」  
2. 输入模拟、热键、音频采集更适合在 **Rust 原生层** 完成  
3. 同类听写桌面应用已有 Tauri 先例，路径可验证  
4. 访谈选项「跨平台 / Rust·Tauri」与此一致  

**不选 Electron 的条件**：除非团队强依赖 Node 原生生态且不愿维护 Rust 侧；当前 greenfield 无此包袱。

**平台策略建议**：

- **MVP：Windows-first**（玩家主战场、SendInput 路径最清晰）  
- macOS / Linux 同期保持编译与通用听写，游戏适配表后补  

---

## 4. 语音识别（STT）预研

### 4.1 交互形态：先「推说」再「真流式」

| 形态 | 体验 | 实现难度 | MVP 建议 |
|------|------|----------|----------|
| **Push-to-talk**（按住说话 / 按一下开始再按结束） | 边界清晰、误触发少、与游戏热键习惯一致 | 低 | **默认采用** |
| 边说边出字（真流式 partial） | 悬浮窗实时字，观感强 | 中高 | Phase 2 |
| 全时 VAD 免按键 | 最「不打断」 | 高（误触发、噪音、资源） | 后期可选 |

业界本地听写（VoiceInk / Superwhisper / Whisperi）主流是 **按键说话 → 松手转写 → 注入**，体感延迟可压到约 0.5–2s（视模型与硬件）。README 中的「实时」可作为目标体验，**工程上 Phase 1 用 push-to-talk + 快模型即可达标感**。

### 4.2 引擎候选

| 方案 | 优势 | 劣势 | 适用 |
|------|------|------|------|
| **whisper.cpp** | 跨平台、Metal/CUDA/Vulkan、C API 易嵌、社区大 | 非原生流式；中文游戏黑话一般；chunk 需调参 | 多语言兜底、离线默认 |
| **faster-whisper / CTranslate2** | 吞吐高 | Python 运行时重，桌面分发痛苦 | 服务端，不适合嵌客户端 |
| **sherpa-onnx** | 真流式 Zipformer、中英、ONNX 轻、可 CPU | 模型选择多，集成文档曲线 | **中文流式 / 低延迟** |
| **FunASR Paraformer（streaming）** | 中文工业级、热词、标点、流式成熟 | 运行时偏重（Python/服务），Rust 嵌需 ONNX 路径 | 中文质量标杆；可本地服务或后续 |
| 云端 API（OpenAI / 国内 ASR） | 准、省端侧资源 | 延迟、隐私、费用、游戏时网络 | 可选「增强模式」 |
| Web Speech API | 零模型 | 依赖系统/网络，桌面可控性差 | 不推荐作核心 |

中文会议场景公开对比中，FunASR 系（SenseVoice / Paraformer）CER 常明显优于 Whisper-large；**游戏中文短句 + 黑话**更适合「中文优化模型 + 热词表」。

### 4.3 推荐 STT 策略

```
Phase 1 (MVP)
  Push-to-talk
  → Silero VAD（可选，裁静音）
  → whisper.cpp (small / turbo 量级)  或  sherpa-onnx 中文流式小模型
  → 简单标点/脏话过滤（可先不做 LLM 润色）
  → 游戏术语热词表（LOL：闪现、大龙、gank…）

Phase 2
  真流式 partial 上屏（sherpa-onnx / FunASR streaming）
  可选云端 fallback
  轻量 LLM 清理口癖（「那个」「就是」）

Phase 3
  按游戏切换词表；用户自定义词库
```

**嵌入方式**：

1. **进程内**：Rust 通过 FFI 调 whisper.cpp / sherpa-onnx（延迟最低、打包可控）  
2. **子进程**：拉起 `whisper-cli` / 本地 server（实现快、崩溃隔离）——适合先做通再优化  

推荐：**MVP 用子进程或 sidecar 跑通；稳定后迁 FFI。**

### 4.4 音频采集注意点

- 默认系统默认麦克风；允许选择设备（外接麦 / 耳机麦）  
- 采样：16 kHz mono（ASR 惯例）  
- 游戏噪音：耳麦近讲 + VAD 阈值；后续可加简单降噪  
- 权限：macOS 麦克风 + 辅助功能；Windows 麦克风隐私开关  

---

## 5. 游戏聊天注入（核心难点）

### 5.0 实战参考：LeagueAkari（MIT，已验证 LOL 局内发送）

仓库：[LeagueAkari/LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)（~3.7k★，MIT）  
原生输入：[`native/win32-x64/src/input`](https://github.com/LeagueAkari/LeagueAkari/tree/main/native/win32-x64/src/input)  
编排层：`src/main/shards/in-game-send/send-executor.ts`

这是**局内一键发聊天的成熟实现**，可直接作为 Kotone 的算法与 API 参考（MIT 允许借鉴；移植时保持独立实现并注明致谢即可）。

#### 分层结构

| 层 | 文件 | 职责 |
|----|------|------|
| Win32 原生 | `input.cc` / `input.h` | `SendInput` 发 Unicode 字符串、按键 down/up；可选 `WH_KEYBOARD_LL` 钩子做全局键状态 |
| Node 绑定 | N-API addon + `lib/input/index.ts` | 暴露 `sendString` / `sendKey` / `install` 热键钩 |
| 业务编排 | `InGameSendExecutor` | 前台校验 → Enter 开聊 → 打字 → Enter 发送；可 abort |
| 进程工具 | `tools.cc` | `getPidsByName("League of Legends.exe")` + `GetForegroundWindow` 判断前台 |

#### 局内发送时序（他们实测可用）

```
// 常量：VK_RETURN = 13，内部间隔 20ms
pressEnter:  sendKey(13, down) → sleep(20) → sendKey(13, up)
sleep(20)
sendString(line)          // KEYEVENTF_UNICODE 逐字 down+up
sleep(20)
pressEnter                // 发送
// 多行时再 sleep(sendInterval)
```

**前置条件**：`League of Legends.exe` 对应 PID 必须是前台进程；否则直接 abort。

#### 双通道策略（LOL 特有，Kotone 可借鉴）

| 阶段 | 路径 | 说明 |
|------|------|------|
| 选人 / 房间（lobby） | **LCU HTTP API** `chatSend` | 不碰键盘，最稳 |
| **对局中 in-game** | **SendInput 模拟** | 游戏内聊天无 LCU 等价写入 |

Kotone 主场景是对局中报点 → **in-game 键盘路径是主路径**；LCU 可作为「选人/大厅」增强，非 MVP 必须。

#### 原生 `SendString` 核心（Unicode 而非剪贴板）

```cpp
// 每个字符：KEYEVENTF_UNICODE 的 KEYDOWN + KEYUP
inputs[j].ki.wVk = 0;
inputs[j].ki.wScan = ch;  // UTF-16 code unit
inputs[j].ki.dwFlags = KEYEVENTF_UNICODE;
// ... KEYUP 带 KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
SendInput(expected, inputs.data(), sizeof(INPUT));
```

按键：

```cpp
// 带 scan code 的 VK down/up（比只填 wVk 更接近真实键盘）
input.ki.wVk = key;
input.ki.wScan = MapVirtualKey(key, MAPVK_VK_TO_VSC);
```

前台检测：

```cpp
bool IsProcessForeground(DWORD processID) {
  HWND hwnd = GetForegroundWindow();
  DWORD foregroundProcID;
  GetWindowThreadProcessId(hwnd, &foregroundProcID);
  return foregroundProcID == processID;
}
```

#### 对预研结论的修正 / 强化

| 原预研假设 | LeagueAkari 实战 | Kotone 调整 |
|------------|------------------|-------------|
| 剪贴板 Ctrl+V 优先 | **局内用 KEYEVENTF_UNICODE 逐字**，已大规模验证 | **LOL 默认走 Unicode SendInput**；剪贴板作备选/其他游戏实验 |
| 开聊延迟 30–80ms | **固定 20ms** 足够 | 默认 20ms，profile 仍可调 |
| 注入是最大未验证风险 | **LOL 路径已验证** | Spike 3 从「能不能」变为「Rust 复刻是否一致」 |
| 需驱动/内存 | **纯 SendInput** | 维持：不做驱动、不读内存 |

#### 移植到 Tauri/Rust 的对应关系

| LeagueAkari | Kotone 建议 |
|-------------|-------------|
| Electron + N-API `input.cc` | Tauri 内 **Rust 直接调 `windows` crate / `SendInput`**（无需 Node addon） |
| `sendString` / `sendKey` | `inject::send_unicode` / `inject::key_down_up` |
| `isProcessForeground` + 进程名 | `sysinfo`/`windows` 枚举 + `GetForegroundWindow` |
| `AbortController` 取消发送 | `tokio::sync::watch` / `AtomicBool` 取消标志 |
| 全局钩子 `WH_KEYBOARD_LL` | 热键优先用 Tauri global-shortcut；仅在需要「任意键状态」时再挂 LL hook |

**注意**：`KEYEVENTF_UNICODE` 按 UTF-16 code unit 发送；代理对（emoji 等）需按 `u16` 单元处理，与他们的 `std::u16string` 一致。

### 5.1 目标流水线（Kotone，对齐实战）

```
用户热键松开
  → STT 得到文本 T
  → [可选] 确认悬浮窗 / 自动发送
  → 校验目标游戏进程在前台（否则失败提示）
  → sendKey(openChat, down) → delay → up     // LOL: Enter=13
  → delay（默认 20ms）
  → sendUnicodeString(T)                     // 或备选 clipboard+Ctrl+V
  → delay
  → sendKey(send, down) → delay → up
  → 反馈 Toast：「收到，已发送！✨」
```

### 5.2 Windows 注入手段

| 方法 | 原理 | 游戏兼容 | 反作弊风险感 | 说明 |
|------|------|----------|--------------|------|
| **SendInput + KEYEVENTF_UNICODE** | 系统级合成 Unicode 键 | **LOL 已验证**（LeagueAkari） | 相对常规 | **LOL/MVP 首选** |
| **SendInput VK + scan** | 开聊/发送键 down-up | 同上 | 相对常规 | Enter 等功能键 |
| 剪贴板 + Ctrl+V | 先写 CF_UNICODETEXT 再粘贴 | 待各游戏实测 | 低 | **备选**；长文本可能更快 |
| PostMessage / SendMessage | 投递到指定 HWND | 多数游戏/DirectInput **无效** | 中 | 不适合当主路径 |
| 驱动级注入 | 虚拟键鼠驱动 | 高 | 高 | **不做** |

要点：

1. **游戏必须是前台焦点窗口**；LeagueAkari 硬性检查 `League of Legends.exe` 前台。  
2. **独占全屏** 仍可能出问题；引导无边框/窗口化。  
3. LOL：**Unicode 逐字** 已够用；不必先上剪贴板。  
4. 时序：开聊 → delay → 文本 → delay → 发送；**默认 20ms**，profile 可覆盖。 

### 5.3 各平台差异

| 平台 | 注入 | 全局热键 | 备注 |
|------|------|----------|------|
| Windows | SendInput + 剪贴板 | RegisterHotKey / 库封装 | MVP 主平台 |
| macOS | CGEvent / Accessibility | 需辅助功能权限 | 游戏市场较小，后做 |
| Linux | uinput / ydotool / enigo | 视 X11/Wayland | Wayland 限制多 |

### 5.4 反作弊与合规边界

- 仅做 **系统标准输入合成 + 剪贴板**，不读写游戏内存、不 hook 渲染、不绕过反作弊。  
- 功能等价于「玩家自己打字发聊天」，但自动化仍可能被个别厂商政策灰色对待——需在 README 写清 **用户自担**、推荐合法沟通用途。  
- 避免与已知会封禁「宏/键鼠脚本」的场景捆绑营销；优先 **MOBA/合作沟通** 话术。  

### 5.5 多游戏适配模型

```json
{
  "id": "lol",
  "displayName": "League of Legends",
  "processNames": ["League of Legends.exe", "LeagueClientUx.exe"],
  "windowTitlePatterns": [".*League of Legends.*"],
  "openChatKey": "Enter",
  "sendKey": "Enter",
  "channelPrefix": "",
  "preOpenDelayMs": 50,
  "prePasteDelayMs": 40,
  "preSendDelayMs": 30,
  "preferClipboardPaste": true
}
```

- **自动检测**：前台进程名 / 窗口标题匹配配置  
- **通用模式**：用户手动指定「当前游戏配置」  
- MVP 先做：**通用配置 + LOL 预设**；Valorant / Apex / 原神 用同模型扩表  

---

## 6. 全局热键与悬浮窗

### 6.1 热键

- Tauri 官方/社区 **global-shortcut** 插件  
- 默认建议：`CapsLock` 双击 / `F8` / `Alt+V`（安装时引导避开游戏常用键）  
- 支持 **按住说话 / 切换说话** 两种模式  
- 冲突检测：与游戏键位冲突时提示改键  

### 6.2 悬浮窗（Overlay）

| 能力 | 实现要点 |
|------|----------|
| 始终置顶 | `always_on_top` |
| 透明/无边框 | transparent + decorations false |
| 少抢操作 | 尽量 `skip_taskbar`；录音时才显示紧凑条 |
| 点击穿透（可选） | 空闲穿透，仅控件可点——后置 |
| 深色霓虹 UI | 品牌色 `#00E5FF` / `#1A1A2E` / `#FF2D78` |

与 **独占全屏** 不兼容时：提示切换无边框；部分 Windows 全屏优化可叠 UI，不可依赖。

---

## 7. 建议系统架构

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (Svelte/React + Tailwind)                         │
│  悬浮录音条 · 设置页 · 游戏配置 · 历史 · 品牌 UI              │
└──────────────────────────┬──────────────────────────────────┘
                           │ Tauri IPC
┌──────────────────────────▼──────────────────────────────────┐
│  Rust Core (src-tauri)                                      │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌──────────┐ │
│  │ hotkeys    │ │ audio I/O  │ │ stt bridge │ │ inject   │ │
│  │            │ │ (cpal)     │ │ whisper /  │ │ enigo +  │ │
│  │            │ │            │ │ sherpa     │ │ clipboard│ │
│  └────────────┘ └────────────┘ └────────────┘ └──────────┘ │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ game profile store (JSON) · settings · tray            │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                           │
              whisper.cpp / sherpa-onnx (lib or sidecar)
```

### 模块边界

| 模块 | 职责 | 可测性 |
|------|------|--------|
| `audio` | 采集、设备列表、缓冲 | 单元 + 设备 mock |
| `stt` | 模型加载、转写、取消 | 固定 wav 回归 |
| `inject` | 开聊/粘贴/发送时序 | 记事本集成测 → 再真游戏 |
| `game_profile` | 进程匹配、配置 CRUD | 纯逻辑单测 |
| `ui` | 状态机：idle/listening/transcribing/sending | E2E 可选 |

### 状态机（核心 UX）

```
Idle --[hotkey down]--> Listening --[hotkey up]--> Transcribing
                                                      │
                         ┌──[auto send]──► Sending ───┤
                         │                            │
                         └──[confirm]──► Preview ─────┘
Sending --[ok]--> Success toast --> Idle
       --[fail]--> Error toast --> Idle (保留文本可重试)
```

---

## 8. 推荐技术栈（立项默认）

| 层 | 选型 | 备注 |
|----|------|------|
| 壳 | **Tauri 2** | Windows-first，跨平台可编译 |
| UI | **Svelte 5** 或 **React 19** + Tailwind | 悬浮窗轻；团队熟哪个用哪个 |
| 语言 | **Rust**（核心）+ TypeScript（UI） | |
| 音频 | `cpal` | 跨平台采集 |
| STT MVP | **whisper.cpp** sidecar 或 FFI | small/turbo；中文可并行试 sherpa-onnx |
| STT 中文增强 | **sherpa-onnx** Zipformer-zh / FunASR ONNX | Phase 2 |
| 输入模拟 | **enigo** + `arboard`（剪贴板） | Windows SendInput 路径 |
| 热键 | `tauri-plugin-global-shortcut` | |
| 托盘 | `tauri-plugin-tray` / 官方 tray | 常驻后台 |
| 配置 | JSON/TOML + 用户目录 | 游戏 profile 可导入导出 |
| 构建 | pnpm + cargo | README 已写 pnpm |
| CI | GitHub Actions：lint + Windows 构建 | macOS 可选 |

**明确不做（MVP）**：驱动注入、内存读游戏、订阅云强制依赖、Electron 双栈。

---

## 9. 风险清单与缓解

| 风险 | 等级 | 缓解 |
|------|------|------|
| 独占全屏无法注入/叠 UI | 高 | 文档强制推荐无边框；设置页检测全屏并提示 |
| 部分游戏不认合成输入 | 中（LOL 已降） | LOL 有 LeagueAkari 验证路径；其他游戏白名单实测；失败则「仅复制」降级 |
| 反作弊误报 | 中 | 只用 SendInput（与 Akari 同级）；开源透明；免责声明 |
| STT 延迟/占 GPU | 中 | 默认小模型；CPU 回落；推说非常驻推理 |
| 中文黑话识别差 | 中 | 热词表；后续中文流式模型 |
| 热键与游戏冲突 | 中 | 安装引导 + 冲突检测 |
| macOS 权限劝退 | 低（MVP 次要） | 清晰权限说明页 |
| 包体含模型过大 | 中 | 首次启动下载模型；可选 tiny/small/medium |

---

## 10. MVP 范围与里程碑建议

### MVP（约 2–4 周可验证，视人力）

1. Tauri 2 骨架 + 托盘 + 全局热键  
2. 按住说话录音 → whisper.cpp 转写 → 结果显示在悬浮窗  
3. **通用注入**：任意前台窗口（先记事本验证）剪贴板粘贴  
4. **LOL profile**：Enter → 粘贴 → Enter（无边框窗口实测）  
5. 基础设置：热键、麦克风、自动发送开关、开聊延迟  
6. 品牌悬浮 UI（沿用 RepoChan 色板与素材）

### 成功标准（预研验收）

- [ ] 记事本路径：热键 → 中文短句出现在光标处，成功率 > 95%  
- [ ] LOL 无边框：训练模式发一条队内聊天成功（人工测 10 次 ≥ 8）  
- [ ] 与客户端同开时，空闲内存目标 < 150 MB（不含已加载模型）  
- [ ] 端到端（松键 → 字上屏）P50 < 2s（small 模型 + 中端 CPU/核显）  

### 明确延后

- 真流式逐字上屏  
- 云端 ASR / LLM 润色  
- 完整游戏库（先 1 通用 + 1–2 预设）  
- Linux Wayland 完整支持  
- 商店上架与自动更新（可随后用 Tauri updater）  

---

## 11. 建议立即做的技术 Spike（立项第一周）

按优先级，每个 spike 目标 **1 天内出 go/no-go**：

| # | Spike | 通过标准 |
|---|--------|----------|
| 1 | Tauri 2 空壳 + global shortcut + always-on-top 透明窗 | 游戏前台时热键仍触发、窗可见 |
| 2 | enigo：记事本 Enter 不需要；直接粘贴中文 | 中文与 emoji 正确 |
| 3 | **按 LeagueAkari 时序** 用 Rust `SendInput` 复刻：前台检测 + Enter×2 + Unicode 字符串；LOL 训练模式实测 | 与 Akari 行为一致则注入闭环成立 |
| 4 | whisper.cpp small 转 3s 中文 | 延迟与准确率可接受 |
| 5 | （可选）sherpa-onnx 同句对比 | 决定默认引擎 |

**Spike 3 参考实现**：直接对照 [LeagueAkari `input.cc`](https://github.com/LeagueAkari/LeagueAkari/blob/main/native/win32-x64/src/input/input.cc) + [`send-executor.ts`](https://github.com/LeagueAkari/LeagueAkari/blob/main/src/main/shards/in-game-send/send-executor.ts)（MIT）。不必用 enigo 抽象层也可先 raw `windows` crate。

**若 Spike 3 失败**：先核对前台进程名、是否无边框、Enter 是否被改键；仍失败则降级「转写 + 复制」，差异化减弱。

---

## 12. 决策摘要（给立项会）

| 议题 | 建议决策 |
|------|----------|
| 桌面壳 | **Tauri 2**，非 Electron |
| 首发平台 | **Windows first** |
| STT | **本地优先**，MVP = push-to-talk + whisper.cpp；中文增强走 sherpa-onnx/FunASR 路线 |
| 注入 | **SendInput + KEYEVENTF_UNICODE**（对齐 LeagueAkari）；剪贴板为备选；每游戏 profile |
| 前端 | Svelte 或 React + Tailwind（轻 UI） |
| 全屏策略 | **只保证无边框/窗口化**；独占全屏 best-effort |
| 最大风险 | 已从「LOL 能否注入」降为「Rust 复刻 + 多游戏扩展」；**第一周仍应用真 LOL 验收 Spike 3** |
| 参考实现 | [LeagueAkari native input](https://github.com/LeagueAkari/LeagueAkari/tree/main/native/win32-x64/src/input)（MIT） |

---

## 13. 参考链接（预研检索）

- Tauri vs Electron 体量/内存对比：[tech-insider 2026](https://tech-insider.org/tauri-vs-electron-2026/)、[gethopp 实测](https://www.gethopp.app/blog/tauri-vs-electron)  
- whisper.cpp 流式与延迟：[stream 讨论](https://github.com/ggml-org/whisper.cpp)、[本地 STT 对比 2026](https://www.promptquorum.com/power-local-llm/local-whisper-stt-comparison-2026)  
- 中文 ASR： [FunASR](https://github.com/modelscope/FunASR)、[sherpa-onnx 预训练](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/index.html)  
- 输入模拟：[enigo](https://github.com/enigo-rs/enigo)、Windows [SendInput / 合成输入讨论](https://news.ycombinator.com/item?id=41568418)  
- 同类： [VoiceInk](https://github.com/Beingpax/VoiceInk)、[Whisperi](https://github.com/xarthurx/whisperi)  

---

*本报告服务于 Kotone 工程立项；品牌资产（人设/视觉/官网 starter）已由 RepoChan 流水线就绪，与本文技术选型正交。*
