//! 运行时「启动」开关：Stopped / Starting / Running / Stopping + restartNeeded 推导。
//!
//! 壳（kotone-tauri）持有 `RuntimeManager`（Mutex<RuntimePhase> + 启动快照），
//! 本模块只做纯迁移/推导逻辑，不碰 IO，可单测：
//!
//! ```text
//! Stopped ──begin_start──▶ Starting ──finish_start──▶ Running
//!   ▲                        │ fail_start               │ begin_stop
//!   └────────────────────────┘                            ▼
//!   └──────────────────finish_stop─────────────────── Stopping
//! ```
//!
//! 语义（用户拍板）：「启动」= 引擎 warmup（模型入内存）+ 注册热键 + 显示悬浮窗；
//! Running 期间切换引擎/活动模型 → restartNeeded = true（不自动重启），
//! 再次 start 等价于 stop + start。

/// 运行时相位
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimePhase {
    /// 未启动：热键不注册，按热键无反应
    Stopped,
    /// 启动中（warmup / 注册热键 / 显示悬浮窗的过渡态）
    Starting,
    /// 运行中：模型在内存、热键已注册、悬浮窗可见
    Running,
    /// 停止中（取消会话 / 注销热键 / 卸载模型的过渡态）
    Stopping,
}

impl RuntimePhase {
    /// IPC/事件 payload 用字符串（与 serde lowercase 一致）
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
        }
    }
}

/// 能否开始启动（仅 Stopped；Running+restartNeeded 的「重启」由壳做 stop+start）
pub fn can_start(p: RuntimePhase) -> bool {
    p == RuntimePhase::Stopped
}

/// 能否开始停止（仅 Running；Stopped 的 stop 是幂等 no-op，由壳直接放行）
pub fn can_stop(p: RuntimePhase) -> bool {
    p == RuntimePhase::Running
}

/// 过渡态（Starting/Stopping）：此时 start/stop 请求应拒绝或禁用
pub fn is_transitioning(p: RuntimePhase) -> bool {
    matches!(p, RuntimePhase::Starting | RuntimePhase::Stopping)
}

// ---------- 迁移（非法迁移返回 Err，壳兜底记录日志并保持原相位） ----------

pub fn begin_start(p: RuntimePhase) -> Result<RuntimePhase, String> {
    if can_start(p) {
        Ok(RuntimePhase::Starting)
    } else {
        Err(format!("当前状态（{}）不能启动", p.as_str()))
    }
}

pub fn finish_start(p: RuntimePhase) -> Result<RuntimePhase, String> {
    if p == RuntimePhase::Starting {
        Ok(RuntimePhase::Running)
    } else {
        Err(format!("当前状态（{}）不是启动中", p.as_str()))
    }
}

/// 启动失败回滚：Starting → Stopped
pub fn fail_start(p: RuntimePhase) -> Result<RuntimePhase, String> {
    if p == RuntimePhase::Starting {
        Ok(RuntimePhase::Stopped)
    } else {
        Err(format!("当前状态（{}）不是启动中", p.as_str()))
    }
}

pub fn begin_stop(p: RuntimePhase) -> Result<RuntimePhase, String> {
    if can_stop(p) {
        Ok(RuntimePhase::Stopping)
    } else {
        Err(format!("当前状态（{}）不能停止", p.as_str()))
    }
}

pub fn finish_stop(p: RuntimePhase) -> Result<RuntimePhase, String> {
    if p == RuntimePhase::Stopping {
        Ok(RuntimePhase::Stopped)
    } else {
        Err(format!("当前状态（{}）不是停止中", p.as_str()))
    }
}

/// restartNeeded 推导：Running 期间，启动快照（引擎/活动模型）与当前配置不一致
/// → 变更需重启生效。仅 Running 相位有义（Stopped 重启概念无意义，恒 false）。
pub fn restart_needed(
    phase: RuntimePhase,
    started: Option<(&str, &str)>,
    current: (&str, &str),
) -> bool {
    if phase != RuntimePhase::Running {
        return false;
    }
    match started {
        Some((se, sm)) => se != current.0 || sm != current.1,
        // Running 但无快照属内部异常；保守起见不提示重启
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_full_cycle() {
        let p = RuntimePhase::Stopped;
        let p = begin_start(p).unwrap();
        assert_eq!(p, RuntimePhase::Starting);
        let p = finish_start(p).unwrap();
        assert_eq!(p, RuntimePhase::Running);
        let p = begin_stop(p).unwrap();
        assert_eq!(p, RuntimePhase::Stopping);
        let p = finish_stop(p).unwrap();
        assert_eq!(p, RuntimePhase::Stopped);
    }

    #[test]
    fn start_failure_rolls_back_to_stopped() {
        let p = begin_start(RuntimePhase::Stopped).unwrap();
        let p = fail_start(p).unwrap();
        assert_eq!(p, RuntimePhase::Stopped);
    }

    #[test]
    fn illegal_transitions_error() {
        assert!(begin_start(RuntimePhase::Running).is_err());
        assert!(begin_start(RuntimePhase::Starting).is_err());
        assert!(begin_stop(RuntimePhase::Stopped).is_err());
        assert!(finish_start(RuntimePhase::Stopped).is_err());
        assert!(finish_stop(RuntimePhase::Running).is_err());
        assert!(fail_start(RuntimePhase::Running).is_err());
    }

    #[test]
    fn guards_match_phases() {
        assert!(can_start(RuntimePhase::Stopped));
        assert!(!can_start(RuntimePhase::Running));
        assert!(can_stop(RuntimePhase::Running));
        assert!(!can_stop(RuntimePhase::Stopped));
        assert!(is_transitioning(RuntimePhase::Starting));
        assert!(is_transitioning(RuntimePhase::Stopping));
        assert!(!is_transitioning(RuntimePhase::Running));
    }

    #[test]
    fn restart_needed_matrix() {
        let started = Some(("sherpa-onnx-x-asr-zh-en", "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"));
        // 同配置 → 不需要
        assert!(!restart_needed(
            RuntimePhase::Running,
            started,
            ("sherpa-onnx-x-asr-zh-en", "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05")
        ));
        // 换模型 → 需要
        assert!(restart_needed(
            RuntimePhase::Running,
            started,
            ("sherpa-onnx-x-asr-zh-en", "other-model")
        ));
        // 换引擎 → 需要
        assert!(restart_needed(
            RuntimePhase::Running,
            started,
            ("sherpa-onnx-sensevoice-zh", "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05")
        ));
        // 非 Running 相位恒 false
        assert!(!restart_needed(RuntimePhase::Stopped, started, ("a", "b")));
        assert!(!restart_needed(RuntimePhase::Starting, started, ("a", "b")));
        // 无快照不提示
        assert!(!restart_needed(RuntimePhase::Running, None, ("a", "b")));
    }

    #[test]
    fn phase_serde_lowercase() {
        let j = serde_json::to_string(&RuntimePhase::Starting).unwrap();
        assert_eq!(j, "\"starting\"");
        let p: RuntimePhase = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(p, RuntimePhase::Running);
        assert_eq!(RuntimePhase::Stopping.as_str(), "stopping");
    }
}
