//! 游戏 profile：CRUD + 前台进程匹配（docs/development.md §5.1、§5.4）
//! 存储：~/.kotone/profiles/<id>.json
//!
//! 内置模板存放在 `resources/profiles/*.json`；首次运行落盘两个 profile：
//! - `lol`：League of Legends（§5.4 示例值，delay 20/20/20，Unicode 逐字）
//! - `generic`：通用，匹配任意前台窗口（processNames 为空 = 通配）

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::settings::kotone_dir;

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
    /// 用户在内置 profile 里显式删除的内置词条：版本更新合并内置热词时
    /// 尊重删除、不复活（见 ensure_builtin_in）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_builtin_hotwords: Vec<String>,
}

impl GameProfile {
    /// 内置 LOL profile。内容由 resources/profiles/lol.json 维护，
    /// Rust 代码只负责解析与返回副本。
    pub fn builtin_lol() -> Self {
        static PROFILE: OnceLock<GameProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                parse_builtin_profile(
                    "lol",
                    include_str!("../resources/profiles/lol.json"),
                )
            })
            .clone()
    }

    /// 内置通用 profile。内容由 resources/profiles/generic.json 维护。
    pub fn builtin_generic() -> Self {
        static PROFILE: OnceLock<GameProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                parse_builtin_profile(
                    "generic",
                    include_str!("../resources/profiles/generic.json"),
                )
            })
            .clone()
    }
}

fn parse_builtin_profile(expected_id: &str, json: &str) -> GameProfile {
    let profile: GameProfile = serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("内置 profile {expected_id}.json 格式错误: {e}"));
    assert_eq!(
        profile.id, expected_id,
        "内置 profile 文件名与 id 不一致"
    );
    profile
}

fn profiles_dir() -> PathBuf {
    kotone_dir().join("profiles")
}

fn profile_path_in(dir: &Path, id: &str) -> PathBuf {
    // id 只保留文件安全字符，防路径穿越
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.json"))
}

/// 确保内置 profile 已落盘（首次运行调用；不覆盖用户已有文件）。
/// 文件已存在时做「版本更新合并」：把新版扩充的内置词条追加到用户文件末尾
/// （尊重 removedBuiltinHotwords 里用户显式删除的词条，不复活）。
pub fn ensure_builtin() -> Result<(), String> {
    ensure_builtin_in(&profiles_dir())
}

pub fn ensure_builtin_in(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 profiles 目录失败: {e}"))?;
    for p in [GameProfile::builtin_lol(), GameProfile::builtin_generic()] {
        let path = profile_path_in(dir, &p.id);
        if !path.exists() {
            let json = serde_json::to_string_pretty(&p)
                .map_err(|e| format!("序列化内置 profile 失败: {e}"))?;
            std::fs::write(&path, json).map_err(|e| format!("写入内置 profile 失败: {e}"))?;
        } else {
            merge_builtin_hotwords_in(dir, &p);
        }
    }
    Ok(())
}

/// 版本更新合并：已有内置 profile 文件补上新版新增的内置词条。
/// missing = 内置词条 - 文件现有热词 - 用户显式删除的内置词条，追加到末尾
/// （用户自定义词条与其余字段原样保留）。合并失败只记日志，不阻断启动流程。
fn merge_builtin_hotwords_in(dir: &Path, builtin: &GameProfile) {
    let result = (|| -> Result<(), String> {
        let path = profile_path_in(dir, &builtin.id);
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| format!("读取内置 profile 失败: {e}"))?;
        let mut file: GameProfile =
            serde_json::from_str(&raw).map_err(|e| format!("解析内置 profile 失败: {e}"))?;
        let missing: Vec<String> = builtin
            .hotwords
            .iter()
            .filter(|w| {
                !file.hotwords.contains(w) && !file.removed_builtin_hotwords.contains(w)
            })
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        file.hotwords.extend(missing);
        save_in(dir, &file)
    })();
    if let Err(e) = result {
        eprintln!("合并内置热词失败（{}，忽略）: {e}", builtin.id);
    }
}

/// 列出全部 profile（按 id 排序，稳定输出）
pub fn list() -> Vec<GameProfile> {
    list_in(&profiles_dir())
}

pub fn list_in(dir: &Path) -> Vec<GameProfile> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(p) = serde_json::from_str::<GameProfile>(&raw) {
                out.push(p);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 按 id 读取单个 profile
pub fn get(id: &str) -> Option<GameProfile> {
    get_in(&profiles_dir(), id)
}

pub fn get_in(dir: &Path, id: &str) -> Option<GameProfile> {
    let path = profile_path_in(dir, id);
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 保存 profile
pub fn save(profile: &GameProfile) -> Result<(), String> {
    save_in(&profiles_dir(), profile)
}

pub fn save_in(dir: &Path, profile: &GameProfile) -> Result<(), String> {
    if profile.id.trim().is_empty() {
        return Err("profile id 不能为空".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 profiles 目录失败: {e}"))?;
    let json =
        serde_json::to_string_pretty(profile).map_err(|e| format!("序列化 profile 失败: {e}"))?;
    let path = profile_path_in(dir, &profile.id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入 profile 失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘 profile 失败: {e}"))?;
    Ok(())
}

/// 删除 profile
#[allow(dead_code)] // CRUD 接口齐备，删除入口由前端子代理接入
pub fn delete(id: &str) -> Result<(), String> {
    let path = profile_path_in(&profiles_dir(), id);
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("删除 profile 失败: {e}"))?;
    }
    Ok(())
}

// ---------- 匹配纯逻辑（可单测） ----------

/// 进程名是否命中该 profile。
/// - processNames 为空（generic）→ 匹配任意进程；
/// - 否则与进程可执行文件名做大小写不敏感精确比较。
pub fn matches_process(profile: &GameProfile, process_name: &str) -> bool {
    if profile.process_names.is_empty() {
        return true;
    }
    profile
        .process_names
        .iter()
        .any(|p| p.eq_ignore_ascii_case(process_name))
}

/// 在一组 profile 中按前台进程名找匹配：
/// 优先返回「具体 profile」（processNames 非空），其次才是通配的 generic。
pub fn find_by_process(profiles: &[GameProfile], process_name: &str) -> Option<GameProfile> {
    profiles
        .iter()
        .find(|p| !p.process_names.is_empty() && matches_process(p, process_name))
        .or_else(|| {
            profiles
                .iter()
                .find(|p| p.process_names.is_empty() && matches_process(p, process_name))
        })
        .cloned()
}

// ---------- 热词导入导出（纯逻辑，可单测；文件对话框与 IO 在壳侧） ----------

/// 热词导出格式：UTF-8 文本，每行一个词条（末尾换行；空表导出空文件）。
/// 预留权重位「词条 权重」——底层 hotwords 是 Vec<String> 不支持权重，
/// 导出永不带权重列；导入按整行词条解析（见 parse_hotwords_import）。
pub fn format_hotwords_export(hotwords: &[String]) -> String {
    let mut out = String::new();
    for w in hotwords {
        out.push_str(w);
        out.push('\n');
    }
    out
}

/// 解析热词导入文件：每行一个词条；跳过空行与纯空白行，trim 两端空白，
/// 文件内重复词条只保留首次出现（保持顺序）。
pub fn parse_hotwords_import(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let w = line.trim();
        if w.is_empty() {
            continue;
        }
        if !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// 热词合并导入报告
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotwordMergeReport {
    /// 实际新增的词条数
    pub added: usize,
    /// 与现有热词重复而跳过的词条数
    pub duplicates: usize,
    /// 合并后的热词总数
    pub total: usize,
}

/// 合并导入：incoming 中与 existing 重复（精确匹配）的跳过，
/// 新增追加在末尾（保持现有顺序不变）。
pub fn merge_hotwords(existing: &[String], incoming: &[String]) -> (Vec<String>, HotwordMergeReport) {
    let mut merged = existing.to_vec();
    let mut report = HotwordMergeReport {
        total: 0,
        added: 0,
        duplicates: 0,
    };
    for w in incoming {
        if merged.iter().any(|x| x == w) {
            report.duplicates += 1;
        } else {
            merged.push(w.clone());
            report.added += 1;
        }
    }
    report.total = merged.len();
    (merged, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profiles() -> Vec<GameProfile> {
        vec![GameProfile::builtin_generic(), GameProfile::builtin_lol()]
    }

    #[test]
    fn lol_matches_its_process_case_insensitive() {
        let lol = GameProfile::builtin_lol();
        assert!(matches_process(&lol, "League of Legends.exe"));
        assert!(matches_process(&lol, "league of legends.exe"));
        assert!(matches_process(&lol, "LEAGUE OF LEGENDS.EXE"));
    }

    #[test]
    fn lol_does_not_match_other_processes() {
        let lol = GameProfile::builtin_lol();
        assert!(!matches_process(&lol, "notepad.exe"));
        assert!(!matches_process(&lol, "LeagueClient.exe"));
    }

    #[test]
    fn generic_matches_anything() {
        let generic = GameProfile::builtin_generic();
        assert!(matches_process(&generic, "notepad.exe"));
        assert!(matches_process(&generic, "anything at all"));
    }

    #[test]
    fn find_prefers_specific_over_generic() {
        let profiles = sample_profiles();
        let hit = find_by_process(&profiles, "league of legends.exe").unwrap();
        assert_eq!(hit.id, "lol", "LOL 进程应命中具体 profile 而非 generic");
    }

    #[test]
    fn find_falls_back_to_generic() {
        let profiles = sample_profiles();
        let hit = find_by_process(&profiles, "notepad.exe").unwrap();
        assert_eq!(hit.id, "generic");
    }

    #[test]
    fn find_returns_none_without_generic() {
        let profiles = vec![GameProfile::builtin_lol()];
        assert!(find_by_process(&profiles, "notepad.exe").is_none());
    }

    #[test]
    fn builtin_values_match_doc() {
        let lol = GameProfile::builtin_lol();
        assert_eq!(lol.pre_open_delay_ms, 20);
        assert_eq!(lol.pre_paste_delay_ms, 20);
        assert_eq!(lol.pre_send_delay_ms, 20);
        assert!(!lol.prefer_clipboard_paste);
        assert_eq!(lol.open_chat_key, "Enter");
        assert_eq!(lol.send_key, "Enter");
        assert!(lol.hotwords.contains(&"打野".to_string()));
        // 内置热词应去重
        let mut sorted = lol.hotwords.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), lol.hotwords.len(), "内置热词不应有重复");
    }

    #[test]
    fn crud_roundtrip_in_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();

        ensure_builtin_in(&dir).unwrap();
        let mut all = list_in(&dir);
        assert_eq!(all.len(), 2, "首次运行落盘 lol + generic");
        // 再跑一遍不重复、不覆盖
        ensure_builtin_in(&dir).unwrap();
        all = list_in(&dir);
        assert_eq!(all.len(), 2);

        let mut custom = GameProfile::builtin_lol();
        custom.id = "valorant".into();
        custom.display_name = "Valorant".into();
        custom.process_names = vec!["VALORANT-Win64-Shipping.exe".into()];
        save_in(&dir, &custom).unwrap();

        let loaded = get_in(&dir, "valorant").unwrap();
        assert_eq!(loaded.display_name, "Valorant");
        assert_eq!(list_in(&dir).len(), 3);
    }

    #[test]
    fn profile_json_uses_camel_case() {
        let lol = GameProfile::builtin_lol();
        let v = serde_json::to_value(&lol).unwrap();
        assert!(v.get("displayName").is_some());
        assert!(v.get("processNames").is_some());
        assert!(v.get("preferClipboardPaste").is_some());
        assert!(v.get("preOpenDelayMs").is_some());
        // 空删除表不序列化；非空时按 camelCase 落盘
        assert!(v.get("removedBuiltinHotwords").is_none());
        let mut lol2 = GameProfile::builtin_lol();
        lol2.removed_builtin_hotwords = vec!["提莫".into()];
        let v2 = serde_json::to_value(&lol2).unwrap();
        assert_eq!(v2["removedBuiltinHotwords"][0], "提莫");
    }

    // ---------- 内置热词版本更新合并 ----------

    /// 模拟一份「旧版本」lol 文件：内置词条的子集 + 一个用户自定义词条
    fn write_legacy_lol(dir: &PathBuf, hotwords: &[&str], removed: &[&str]) {
        let mut legacy = GameProfile::builtin_lol();
        legacy.hotwords = hotwords.iter().map(|s| s.to_string()).collect();
        legacy.removed_builtin_hotwords = removed.iter().map(|s| s.to_string()).collect();
        save_in(dir, &legacy).unwrap();
    }

    #[test]
    fn ensure_builtin_merges_new_hotwords_into_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        write_legacy_lol(&dir, &["闪现"], &[]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        // 新版内置词条全部补齐，追加在末尾（旧词条「闪现」保持在最前）
        assert_eq!(lol.hotwords.first().map(String::as_str), Some("闪现"));
        for w in &GameProfile::builtin_lol().hotwords {
            assert!(lol.hotwords.contains(w), "合并后缺少内置词条 {w}");
        }
        // 再跑一遍幂等：数量稳定不重复追加
        let n = lol.hotwords.len();
        ensure_builtin_in(&dir).unwrap();
        assert_eq!(get_in(&dir, "lol").unwrap().hotwords.len(), n);
    }

    #[test]
    fn ensure_builtin_respects_user_removed_hotwords() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        // 用户删掉了「提莫」「悠米」（记进 removedBuiltinHotwords）
        write_legacy_lol(&dir, &["闪现"], &["提莫", "悠米"]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert!(!lol.hotwords.contains(&"提莫".to_string()), "删除的词条不应复活");
        assert!(!lol.hotwords.contains(&"悠米".to_string()));
        assert!(lol.hotwords.contains(&"亚索".to_string()), "其余新词条照常合并");
    }

    #[test]
    fn ensure_builtin_keeps_user_custom_hotwords() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        write_legacy_lol(&dir, &["闪现", "我的自定义词"], &[]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert!(
            lol.hotwords.contains(&"我的自定义词".to_string()),
            "用户自定义词条应保留"
        );
        // 其他字段不被合并改动
        assert_eq!(lol.open_chat_key, "Enter");
        assert_eq!(lol.pre_send_delay_ms, 20);
    }

    // ---------- 热词导入导出 ----------

    #[test]
    fn hotwords_export_one_per_line() {
        let words = vec!["打野".to_string(), "gank".to_string(), "Blind Monk".to_string()];
        assert_eq!(format_hotwords_export(&words), "打野\ngank\nBlind Monk\n");
        assert_eq!(format_hotwords_export(&[]), "", "空表导出空文件");
    }

    #[test]
    fn hotwords_import_skips_empty_and_inner_duplicates() {
        let text = "打野\n\n  \ngank\n打野\n  推塔  \r\n";
        assert_eq!(parse_hotwords_import(text), vec!["打野", "gank", "推塔"]);
        assert!(parse_hotwords_import("").is_empty());
        assert!(parse_hotwords_import("  \n\n").is_empty());
    }

    #[test]
    fn hotwords_merge_dedupes_against_existing() {
        let existing = vec!["打野".to_string(), "gank".to_string()];
        let incoming = vec!["gank".to_string(), "推塔".to_string(), "回城".to_string()];
        let (merged, report) = merge_hotwords(&existing, &incoming);
        assert_eq!(merged, vec!["打野", "gank", "推塔", "回城"]);
        assert_eq!(
            report,
            HotwordMergeReport {
                added: 2,
                duplicates: 1,
                total: 4
            }
        );
        // 全重复：零新增，现有顺序不变
        let (m2, r2) = merge_hotwords(&existing, &["打野".to_string()]);
        assert_eq!(m2, existing);
        assert_eq!(r2.added, 0);
        assert_eq!(r2.duplicates, 1);
    }

    #[test]
    fn hotwords_export_import_roundtrip() {
        let words = GameProfile::builtin_lol().hotwords;
        let text = format_hotwords_export(&words);
        assert_eq!(parse_hotwords_import(&text), words);
    }
}
