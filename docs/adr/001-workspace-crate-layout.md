# ADR 001：Cargo workspace 五 crate 布局

- 状态：已采纳（2025-01，随 workspace 拆分重构落地）
- 上下文：Kotone 原为 `src-tauri` 单 crate（`kotone_lib` + `kotone` bin），所有域逻辑、
  STT 引擎、Windows 平台适配、Tauri 壳混在一个编译单元里。

## 决策

根目录改为 Cargo 虚拟 workspace（`members = ["src-tauri", "crates/*"]`），拆为五个 crate：

```
kotone-core               域模型 + ports（trait）+ 状态机（Orchestrator）
                          + settings / profile / eval / log / hotkey 端口
kotone-stt                STT 引擎适配器（mock / whisper-sidecar / sherpa）+ 模型管理
                          + register_builtin（把内置引擎注入 core 的注册表容器）
kotone-platform-windows   cpal 音频采集 / SendInput 注入 / WH_KEYBOARD_LL 钩子 / 提权
                          （非 Windows 留兜底实现，保持可编译）
kotone-cli                clap 命令行：send / listen / eval-stub
kotone-tauri（src-tauri） Tauri 壳：IPC 命令、窗口/托盘/单实例、PluginHotkeySource
```

依赖方向（单向，禁止反向）：

```
kotone-stt ──────┐
                 ├─→ kotone-core
kotone-platform ─┘
kotone-cli  ─→ core + stt + platform
kotone-tauri ─→ core + stt + platform（+ tauri 框架）
```

## 判据

1. **独立消费者**：CLI / 自动化脚本需要不拉起 Tauri 也能调用发送与监听链路
   （`kotone-cli send/listen`），单 crate 做不到。
2. **重依赖编译隔离**：cpal、windows-rs、未来各 STT 引擎 SDK 都是重依赖；
   core 只允许轻量通用依赖（serde/tokio/dirs），保证域逻辑秒级编译、可在非
   Windows 平台跑测试。
3. **变更节奏**：域模型/状态机变动频繁且需要稳定测试；平台适配与引擎适配
   变动独立，拆开后互不重编译。

## 关键设计

### 循环依赖：orchestrator ↔ 引擎注册表

Orchestrator 依赖 `EngineRegistry`（容器在 core），但引擎实例在 kotone-stt，
core 若正常依赖 stt 即成环。解法：

- core 的 `EngineRegistry` 是**空容器**（`new()` + `register()`），不认识任何具体引擎；
- kotone-stt 提供 `register_builtin(&mut registry)`，由消费者（tauri 壳 / CLI）在
  启动时注入——依赖方向保持 `kotone-stt → kotone-core` 单向；
- core 的 orchestrator 测试需要 mock-stream 引擎：core 以 **dev-dependency** 依赖
  kotone-stt（Cargo 允许 dev 环）。

  **注意**：dev 环下单元测试（src 内 `#[cfg(test)]`）会把 core 源码重编译为独立
  test crate，与 kotone-stt 链接的 core rlib 是两个编译单元，`EngineRegistry`
  类型互不兼容（E0308）。因此 orchestrator 全链路测试必须放
  `crates/kotone-core/tests/orchestrator.rs`（集成测试与 kotone-stt 链接同一
  rlib，类型一致）。

### HotkeySource 端口：SharedState 拆解

LL 钩子实现原本直接依赖 Tauri `AppHandle` 与壳的 `SharedState`（发事件、读取消键
状态）。拆分后：

- core 定义 `hotkey::HotkeySource` 端口（register/unregister/set_cancel_active），
  事件经构造时注入的 `HookSink = Box<dyn Fn(HookEvent) + Send + Sync>` 外发；
- `LlHookSource`（WH_KEYBOARD_LL）移入 platform crate，不再认识 Tauri；
- 壳保留 `PluginHotkeySource`（依赖 AppHandle 的 global-shortcut 插件）与
  `HotkeyManager`（选择/回退/status），由壳负责把 sink 事件桥接为 Tauri 事件并
  驱动 orchestrator。

### 配置原则

- settings/profile 由 **core 持有**（serde 模型 + 读写 `~/.kotone`）；
- 引擎/平台实例由消费者**值注入**（`Arc<dyn ...>` 经构造器传入 Orchestrator）；
- `engineOptions` 保持不透明 JSON，core 不理解各引擎私有配置。

## 被否决项

- **每引擎一个 crate**：当前引擎均为占位实现，粒度太细、workspace 噪音大；
  待真实引擎接入且 SDK 够重时再按 feature/crate 拆。
- **游戏输出 provider crate**（按游戏协议输出适配层）：需求未落地，YAGNI。
- **src-tauri 改名并移动到 crates/**：tauri.conf.json、前端引用、打包配置都以
  `src-tauri` 为约定根，移动收益小风险大。package 改名 `kotone-tauri` 但
  `[[bin]] name = "kotone"` 保持 exe 产物名不变。

## 后果

- 正向：CLI 可独立使用；core 编译快、可跨平台跑测试；引擎/平台变更隔离；
  `cargo test --workspace` 73 个测试全绿。
- 代价：dev 环迫使 orchestrator 测试迁为集成测试（见上）；tauri CLI 在
  workspace 下正常探测（`cargo run` 从根执行、bin 名 kotone 不变）；vite 需
  ignore 根级 `target/`（chokidar watch 编译产物 DLL 会 EBUSY 崩溃）；
  壳新增对 `windows` crate 的直接依赖（窗口 SW_SHOWNA UI 代码，合理留壳）。
- exe 名、IPC 签名、事件名、窗口/托盘/单实例/提权行为全部不变。
