//! 游戏配置命令：列表、保存、导入导出、热词、图标、控制台通道开关。

use super::*;
use serde_json::Value;

#[tauri::command]
pub(crate) fn list_profiles() -> Vec<GameProfile> {
    profile::list()
}

/// 保存 profile。内置 profile（lol / dota2 / cs2 / generic）会同步推导 removedBuiltinHotwords：
/// 内置热词全量列表 − 传入热词列表 = 用户主动删除的内置热词（供后续合并逻辑排除）。
/// 非内置 id 不触碰该字段。
#[tauri::command]
pub(crate) fn save_profile(mut profile: GameProfile) -> Result<(), String> {
    let builtin = match profile.id.as_str() {
        "lol" => Some(GameProfile::builtin_lol()),
        "dota2" => Some(GameProfile::builtin_dota2()),
        "cs2" => Some(GameProfile::builtin_cs2()),
        "generic" => Some(GameProfile::builtin_generic()),
        _ => None,
    };
    if let Some(builtin) = builtin {
        profile.removed_builtin_hotwords = builtin
            .hotwords
            .iter()
            .filter(|w| !profile.hotwords.contains(w))
            .cloned()
            .collect();
    }
    profile::save(&profile)
}

/// 导出 profile 热词到 UTF-8 文本（每行一词条，无权重），返回条数。
#[tauri::command]
pub(crate) fn export_hotwords(profile_id: String, path: String) -> Result<u32, String> {
    let p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let text = format_hotwords_export(&p.hotwords);
    std::fs::write(&path, text).map_err(|e| format!("写入 {path} 失败：{e}"))?;
    Ok(p.hotwords.len() as u32)
}

/// 导出内置屏蔽词表到 UTF-8 CSV（两列：屏蔽词,替换词），返回规则条数。
#[tauri::command]
pub(crate) fn export_builtin_blocklist(path: String) -> Result<usize, String> {
    std::fs::write(&path, kotone_postprocess::blocklist::builtin_csv())
        .map_err(|error| format!("写入 {path} 失败：{error}"))?;
    Ok(kotone_postprocess::blocklist::builtin_rule_count())
}

/// 从 UTF-8 文本导入热词（合并去重，追加到现有列表末尾），返回合并报告。
#[tauri::command]
pub(crate) fn import_hotwords(
    profile_id: String,
    path: String,
) -> Result<HotwordMergeReport, String> {
    profile::import_hotwords_from_file(&profile_id, std::path::Path::new(&path))
}

/// 导出整包 profile 为 .kprofile ZIP（profile.json + icon.*）。
#[tauri::command]
pub(crate) fn export_profile(profile_id: String, path: String) -> Result<(), String> {
    profile::export_profile(&profile_id, std::path::Path::new(&path))
}

/// 导入 .kprofile ZIP 包：生成新随机 id 落盘，返回导入后的 profile。
#[tauri::command]
pub(crate) fn import_profile(path: String) -> Result<GameProfile, String> {
    profile::import_profile(std::path::Path::new(&path))
}

/// 删除 profile：内置 = 恢复出厂；导入的 = 永久删除（含 icon）。
#[tauri::command]
pub(crate) fn delete_profile(profile_id: String) -> Result<ProfileDeleteOutcome, String> {
    profile::delete_profile(&profile_id)
}

/// 读取 profile 图标字节（无图标 → None）。前端按扩展名推断 mime 建 Blob URL。
#[tauri::command]
pub(crate) fn get_profile_icon(profile_id: String) -> Result<Option<Vec<u8>>, String> {
    Ok(profile::profile_icon_bytes(&profile_id))
}

// ---------- 提权（UIPI 方案，§10 R-1） ----------

// ---------- Source 2 控制台通道（ADR 032 决定 3） ----------

/// 游戏配置页“不开聊天框直接发”开关的状态：找不找得到游戏目录、`autoexec.cfg` 里有没有
/// Kotone 那一行、游戏开没开。
#[tauri::command]
pub(crate) async fn console_chat_status(profile_id: String) -> Result<Value, String> {
    let p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    tauri::async_runtime::spawn_blocking(move || console_status(&p))
        .await
        .map_err(|e| e.to_string())?
}

/// 打开 / 关闭控制台通道：只在游戏没运行时改；改 `autoexec.cfg` 之前备份到
/// `<KOTONE_HOME>/agent/backups/`。返回改完的状态。
#[tauri::command]
pub(crate) async fn console_chat_set(profile_id: String, enabled: bool) -> Result<Value, String> {
    let p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    tauri::async_runtime::spawn_blocking(move || console_set(&p, enabled))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn console_status(p: &GameProfile) -> Result<Value, String> {
    serde_json::to_value(kotone_platform_windows::console_chat::status(p))
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn console_set(p: &GameProfile, enabled: bool) -> Result<Value, String> {
    let backups = kotone_core::settings::kotone_dir()
        .join("agent")
        .join("backups");
    let status = kotone_platform_windows::console_chat::set_enabled(p, enabled, &backups)?;
    serde_json::to_value(status).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn console_status(_p: &GameProfile) -> Result<Value, String> {
    Err("控制台通道只支持 Windows".into())
}

#[cfg(not(windows))]
fn console_set(_p: &GameProfile, _enabled: bool) -> Result<Value, String> {
    Err("控制台通道只支持 Windows".into())
}
