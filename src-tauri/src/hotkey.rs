//! 全局热键：注册/注销，hold / toggle 两种触发模式，冲突检测
//! 依赖 tauri-plugin-global-shortcut（docs/development.md §3.6、§5.1）

/// 热键触发模式（用户在设置中选择，默认 toggle 引导时确认）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    /// 按住说话，松手结束
    Hold,
    /// 按一下开始，再按一下结束
    Toggle,
}

/// 注册全局热键（占位实现）
pub fn register(_key: &str, _mode: HotkeyMode) -> Result<(), String> {
    todo!("接入 tauri-plugin-global-shortcut，按模式映射按下/松开事件到 orchestrator")
}

/// 注销当前热键（占位实现）
pub fn unregister() -> Result<(), String> {
    todo!("注销已注册的全局热键")
}

/// 检测与常见游戏键位的冲突（占位实现）
pub fn detect_conflicts(_key: &str) -> Vec<String> {
    todo!("首次启动引导时调用")
}
