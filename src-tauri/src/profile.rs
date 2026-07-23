//! 游戏 profile：CRUD + 前台进程匹配（docs/development.md §5.1、§5.4）
//! 存储：~/.kotone/profiles/<id>.json

/// 游戏 profile（默认值对齐 LeagueAkari 实测：delay 20/20/20）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProfile {
    pub id: String,
    pub display_name: String,
    pub process_names: Vec<String>,
    pub window_title_patterns: Vec<String>,
    pub open_chat_key: String,
    pub send_key: String,
    pub pre_open_delay_ms: u32,
    pub pre_paste_delay_ms: u32,
    pub pre_send_delay_ms: u32,
    /// false = Unicode 逐字（不污染剪贴板）；true = 剪贴板粘贴
    pub prefer_clipboard_paste: bool,
    pub hotwords: Vec<String>,
}

/// 列出全部 profile（占位实现）
pub fn list() -> Vec<GameProfile> {
    todo!("读取 ~/.kotone/profiles/*.json")
}

/// 保存 profile（占位实现）
pub fn save(_profile: &GameProfile) -> Result<(), String> {
    todo!("写入 ~/.kotone/profiles/<id>.json")
}

/// 检测当前前台游戏并匹配 profile（占位实现）
pub fn detect_foreground() -> Option<GameProfile> {
    todo!("sysinfo + windows crate 前台进程匹配")
}
