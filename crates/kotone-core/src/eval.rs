//! kotone-eval：引擎评测工具 —— 会话录档、语料回放、多引擎对比
//! （docs/development.md §3.3「评测工具」、§5.1）
//!
//! 存储：~/.kotone/eval/<sessionId>.json + 同名 wav

/// 一条 partial 时间线记录
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialRecord {
    /// 相对会话开始的毫秒偏移
    pub t: u64,
    pub text: String,
}

/// 评测会话录档（字段与 §5.4 eval json 对应）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalSession {
    pub session_id: String,
    pub engine_id: String,
    pub started_at: String,
    pub audio_ms: u64,
    /// 非流式引擎为 None
    pub first_partial_ms: Option<u64>,
    pub final_ms: u64,
    pub partials: Vec<PartialRecord>,
    pub final_text: String,
    /// 人工评测时回填正确文本，用于 CER
    pub human_label: Option<String>,
}

/// 录档一次识别会话（wav + 指标 JSONL）
/// TODO(eval 子代理)：写入 ~/.kotone/eval/，可在设置中关闭
#[allow(dead_code)] // orchestrator 的录档接线由 eval 子代理完成
pub fn record_session(_session: &EvalSession) -> Result<(), String> {
    Err("eval 录档未实现".into())
}

/// 语料回放：同一 wav 对任意已安装引擎离线重放
/// TODO(eval 子代理)：多引擎对比（逐条文本 + 首字延迟 + 总延迟 + 人工标注 CER）
pub fn replay(_session_id: &str, _engine_id: &str) -> Result<EvalSession, String> {
    Err("eval 回放未实现".into())
}

/// 导出评测数据（JSONL + wav 包）
/// TODO(eval 子代理)：打包导出，返回路径
pub fn export() -> Result<String, String> {
    Err("eval 导出未实现".into())
}

/// 列出录档会话
/// TODO(eval 子代理)：读取 ~/.kotone/eval/*.json
pub fn list_sessions() -> Result<Vec<EvalSession>, String> {
    Err("eval 列表未实现".into())
}
