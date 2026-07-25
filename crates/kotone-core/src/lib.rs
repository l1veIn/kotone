//! kotone-core：域模型 + ports + 状态机 + 事件 + 配置/profile/eval/log。
//!
//! 依赖纪律：只允许 serde/serde_json/tokio/dirs 等轻依赖；
//! 不得依赖 cpal / windows / tauri / 任何 STT 引擎 SDK（见 docs/adr/001）。
//!
//! 模块即端口边界：
//! - `audio`：AudioBackend 端口 + AudioHandle/AudioDevice
//! - `inject`：Injector/FocusBackend/SendOps 端口 + 发送时序编排（send_sequence）
//! - `hotkey`：HotkeySource 端口 + 键名解析/匹配状态机（纯逻辑）
//! - `stt`：SttEngine/SttSession 端口 + EngineRegistry 容器（引擎实例由 kotone-stt 注入）
//! - `orchestrator`：唯一状态所有者（状态机）
//! - `settings`/`profile`/`eval`/`log`：配置 schema 与存储（唯一写入口）、游戏 profile、评测、文件日志

pub mod audio;
pub mod eval;
pub mod hotkey;
pub mod inject;
pub mod interaction;
pub mod log;
pub mod orchestrator;
pub mod profile;
pub mod settings;
pub mod stt;
pub mod vad;
