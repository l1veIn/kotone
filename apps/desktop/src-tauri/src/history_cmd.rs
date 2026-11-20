//! 历史记录命令：列表、清空、删除一条、读录音、打开目录。

use super::*;

/// 在系统文件管理器中打开历史记录目录（P2-⑨：历史目录可自定义后仍需可视入口）
#[tauri::command]
pub(crate) fn open_history_dir() -> Result<(), String> {
    let dir = kotone_core::history::history_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

/// 识别历史列表（新→旧；模式/容量配置走通用 update_settings patch）
#[tauri::command]
pub(crate) fn get_history() -> Result<Vec<kotone_core::history::HistoryRecord>, String> {
    kotone_core::history::list()
}

/// 清空全部识别历史（含音频文件；前端做二次确认，后端不再拦截）
#[tauri::command]
pub(crate) fn clear_history() -> Result<(), String> {
    kotone_core::history::clear()
}

/// 删除单条识别历史（按 sessionId + ts 精确匹配；带录音且不再被其他记录
/// 引用时一并删除对应 wav；记录不存在 = 静默成功，幂等）
#[tauri::command]
pub(crate) fn delete_history_record(session_id: String, ts: String) -> Result<(), String> {
    kotone_core::history::delete(&session_id, &ts)
}

/// 读取历史记录的音频文件字节（~/.kotone/history/audio/<file_name>）。
/// file_name 只允许纯文件名：含 /、\\、.. 或为空直接拒绝（防路径穿越）。
#[tauri::command]
pub(crate) fn read_history_audio(file_name: String) -> Result<Vec<u8>, String> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err(format!("非法的音频文件名：{file_name}"));
    }
    let path = kotone_core::history::history_dir()
        .join("audio")
        .join(&file_name);
    std::fs::read(&path).map_err(|e| format!("读取音频 {} 失败：{e}", path.display()))
}

// ---------- 进程资源占用 ----------
