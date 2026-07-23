//! orchestrator：唯一状态所有者。串联 hotkey → audio → stt → inject，
//! partial 转发、取消与超时（docs/development.md §4、§4.1、§5.1）
//!
//! 状态迁移全部在 Rust 侧完成，UI 只渲染 emit 的状态事件。

/// 核心状态机（§4.1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrchestratorState {
    Idle,
    Listening,
    Transcribing,
    Preview,
    Sending,
    Success,
    Error,
}

/// 开始一次「按下到松手」的会话（占位实现）
pub fn begin_session() -> Result<(), String> {
    todo!("创建 STT session → Listening → 弹出悬浮条 → 持续 push_audio")
}

/// 结束会话：finalize → 预览/发送时序（占位实现）
pub fn end_session() -> Result<(), String> {
    todo!("finalize → autoSend 分流 → inject 时序 → Success/Error → eval 录档")
}

/// 任意状态下取消：回到 Idle（session cancel；发送中时序安全中止）
pub fn cancel() {
    todo!("tokio watch 取消标志，对齐 LeagueAkari AbortController")
}
