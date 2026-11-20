//! 游戏配置命令：列表、保存、导入导出、热词、图标。

use super::*;

#[tauri::command]
pub(crate) fn list_profiles() -> Vec<GameProfile> {
    profile::list()
}

/// 保存 profile。内置 profile（lol / dota2 / generic）会同步推导 removedBuiltinHotwords：
/// 内置热词全量列表 − 传入热词列表 = 用户主动删除的内置热词（供后续合并逻辑排除）。
/// 非内置 id 不触碰该字段。
#[tauri::command]
pub(crate) fn save_profile(mut profile: GameProfile) -> Result<(), String> {
    let builtin = match profile.id.as_str() {
        "lol" => Some(GameProfile::builtin_lol()),
        "dota2" => Some(GameProfile::builtin_dota2()),
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
