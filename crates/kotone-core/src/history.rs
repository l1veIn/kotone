//! history：识别历史记录（~/.kotone/history.jsonl 追加式 JSONL，docs/development.md）。
//!
//! 设计要点：
//! - 追加式 JSONL：每次会话终态（sent / cancelled / error）追加一行，零索引成本；
//! - sessionId 与 eval 录档共用（orchestrator 生成一次 id 同时喂两边），可互查；
//! - includeAudio 开启时把 eval 录档的 wav 复制到 history/audio/<sessionId>.wav，
//!   记录里的 audioFile 存相对文件名（eval 录档关闭时无 wav 可复制，audioFile 为 null）；
//! - 并发取舍：单机单用户 best-effort——append 用单次 write 追加，
//!   capped 裁剪（全读 → 保留尾部 → 原子重写）极少发生，不加文件锁；
//!   最坏情况是并发写交错出一行坏 JSON，list 会跳过坏行，不致命。

use std::path::{Path, PathBuf};

/// 历史记录模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryMode {
    /// 只保留最近 maxRecords 条（默认）
    Capped,
    /// 全部保留（不裁剪）
    KeepAll,
    /// 关闭：orchestrator 侧连草稿都不建，零开销
    Off,
}

/// 历史记录配置（settings.history；默认 capped / 1000 / 不含音频）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryConfig {
    pub mode: HistoryMode,
    /// capped 模式下的容量上限
    pub max_records: u32,
    /// 是否随记录保存音频（从 eval 录档复制 wav 到 history/audio/）
    pub include_audio: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            mode: HistoryMode::Capped,
            max_records: 1000,
            include_audio: false,
        }
    }
}

/// 会话终态
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOutcome {
    Sent,
    Cancelled,
    Error,
}

/// 一条历史记录（camelCase，与 config.json / eval 录档同风格）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    /// 与 eval 录档互查的会话 id
    pub session_id: String,
    /// 终态落账时间（ISO 8601 UTC）
    pub ts: String,
    pub engine_id: String,
    pub profile_id: Option<String>,
    /// 最终文本（无产出为空串）
    pub final_text: String,
    pub audio_ms: u64,
    pub first_partial_ms: Option<u64>,
    pub finalize_latency_ms: Option<u64>,
    pub outcome: HistoryOutcome,
    pub error: Option<String>,
    /// 相对 history/audio/ 的文件名；includeAudio 关闭或无 wav 可复制时为 null
    pub audio_file: Option<String>,
}

// ---------- 路径 ----------

/// 历史数据目录：~/.kotone/history/
pub fn history_dir() -> PathBuf {
    crate::settings::kotone_dir().join("history")
}

fn jsonl_path(dir: &Path) -> PathBuf {
    dir.join("history.jsonl")
}

fn audio_dir(dir: &Path) -> PathBuf {
    dir.join("audio")
}

// ---------- 追加 ----------

/// 追加一条记录到默认目录（mode=off 时直接成功返回，零 IO）
pub fn append(record: &HistoryRecord, cfg: &HistoryConfig) -> Result<(), String> {
    append_in(&history_dir(), record, cfg)
}

/// 追加一条记录到指定目录（测试可指向临时目录）。
/// capped 且超上限时裁剪：全读 → 保留尾部 maxRecords 条 → 原子重写；
/// 被裁掉的记录若带 audioFile，一并删除对应 wav（best-effort）。
pub fn append_in(dir: &Path, record: &HistoryRecord, cfg: &HistoryConfig) -> Result<(), String> {
    if cfg.mode == HistoryMode::Off {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("创建历史目录失败: {e}"))?;
    let line = serde_json::to_string(record).map_err(|e| format!("序列化历史记录失败: {e}"))?;
    let path = jsonl_path(dir);
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("打开历史文件失败: {e}"))?;
        writeln!(f, "{line}").map_err(|e| format!("写入历史记录失败: {e}"))?;
    }
    if cfg.mode == HistoryMode::Capped {
        trim_in(dir, cfg.max_records)?;
    }
    Ok(())
}

/// capped 裁剪：超出 max 时保留尾部 N 条原子重写，并清理被裁记录的音频
fn trim_in(dir: &Path, max: u32) -> Result<(), String> {
    let path = jsonl_path(dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(()), // 刚写完读不到是异常情况；不致命，下次再裁
    };
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let max = max as usize;
    if lines.len() <= max {
        return Ok(());
    }
    let (dropped, kept) = lines.split_at(lines.len() - max);
    // 被裁记录的音频一并删除（best-effort，删不掉不阻塞裁剪）
    for line in dropped {
        if let Ok(rec) = serde_json::from_str::<HistoryRecord>(line) {
            if let Some(file) = rec.audio_file {
                let _ = std::fs::remove_file(audio_dir(dir).join(file));
            }
        }
    }
    let mut out = kept.join("\n");
    out.push('\n');
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("裁剪历史失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("落盘历史裁剪失败: {e}"))?;
    Ok(())
}

// ---------- 读取 / 清空 ----------

/// 读取默认目录的全部记录（新→旧；坏行跳过）
pub fn list() -> Result<Vec<HistoryRecord>, String> {
    list_in(&history_dir())
}

/// 读取指定目录的全部记录（新→旧；坏行跳过不致命）
pub fn list_in(dir: &Path) -> Result<Vec<HistoryRecord>, String> {
    let path = jsonl_path(dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("读取历史文件失败: {e}"))?;
    let mut records: Vec<HistoryRecord> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    records.reverse();
    Ok(records)
}

/// 清空默认目录（jsonl + audio/ 下全部 wav）
pub fn clear() -> Result<(), String> {
    clear_in(&history_dir())
}

/// 清空指定目录（jsonl + audio/ 目录整体删除）
pub fn clear_in(dir: &Path) -> Result<(), String> {
    let path = jsonl_path(dir);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除历史文件失败: {e}"))?;
    }
    let adir = audio_dir(dir);
    if adir.exists() {
        std::fs::remove_dir_all(&adir).map_err(|e| format!("删除历史音频目录失败: {e}"))?;
    }
    Ok(())
}

/// includeAudio 落音频：把 eval 录档 wav 复制到 history/audio/<sessionId>.wav，
/// 成功返回相对文件名（填入 HistoryRecord.audioFile）；
/// eval 录档关闭（无 wav）或复制失败返回 None（记录照写，不致命）。
pub fn copy_audio_in(dir: &Path, eval_dir: &Path, session_id: &str) -> Option<String> {
    let src = crate::eval::session_wav_path(eval_dir, session_id);
    if !src.exists() {
        return None;
    }
    let adir = audio_dir(dir);
    std::fs::create_dir_all(&adir).ok()?;
    let name = format!("{session_id}.wav");
    std::fs::copy(&src, adir.join(&name)).ok()?;
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(session_id: &str, outcome: HistoryOutcome) -> HistoryRecord {
        HistoryRecord {
            session_id: session_id.to_string(),
            ts: "2026-05-20T12:00:00Z".to_string(),
            engine_id: "mock-stream".to_string(),
            profile_id: Some("lol".to_string()),
            final_text: format!("文本-{session_id}"),
            audio_ms: 1500,
            first_partial_ms: Some(300),
            finalize_latency_ms: Some(120),
            outcome,
            error: (outcome == HistoryOutcome::Error).then(|| "注入失败".to_string()),
            audio_file: None,
        }
    }

    fn cfg(mode: HistoryMode, max: u32) -> HistoryConfig {
        HistoryConfig {
            mode,
            max_records: max,
            include_audio: false,
        }
    }

    #[test]
    fn default_config_matches_doc() {
        let c = HistoryConfig::default();
        assert_eq!(c.mode, HistoryMode::Capped);
        assert_eq!(c.max_records, 1000);
        assert!(!c.include_audio);
        // serde 形态：kebab-case 枚举 + camelCase 字段
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j["mode"], "capped");
        assert!(j.get("maxRecords").is_some());
        assert!(j.get("includeAudio").is_some());
        assert_eq!(
            serde_json::from_value::<HistoryMode>(serde_json::json!("keep-all")).unwrap(),
            HistoryMode::KeepAll
        );
        assert_eq!(
            serde_json::from_value::<HistoryMode>(serde_json::json!("off")).unwrap(),
            HistoryMode::Off
        );
    }

    #[test]
    fn append_and_list_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        append_in(dir.path(), &rec("s1", HistoryOutcome::Sent), &cfg(HistoryMode::Capped, 10))
            .unwrap();
        append_in(
            dir.path(),
            &rec("s2", HistoryOutcome::Cancelled),
            &cfg(HistoryMode::Capped, 10),
        )
        .unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 2);
        // 新→旧
        assert_eq!(records[0].session_id, "s2");
        assert_eq!(records[1].session_id, "s1");
        assert_eq!(records[1].outcome, HistoryOutcome::Sent);
        assert_eq!(records[1].profile_id.as_deref(), Some("lol"));
        assert!(records[1].error.is_none());
        assert_eq!(records[0].outcome, HistoryOutcome::Cancelled);
    }

    #[test]
    fn off_mode_skips_io() {
        let dir = tempfile::tempdir().unwrap();
        append_in(dir.path(), &rec("s1", HistoryOutcome::Sent), &cfg(HistoryMode::Off, 10))
            .unwrap();
        assert!(!jsonl_path(dir.path()).exists(), "off 模式不应产生任何 IO");
        assert!(list_in(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn keep_all_never_trims() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            append_in(
                dir.path(),
                &rec(&format!("s{i}"), HistoryOutcome::Sent),
                &cfg(HistoryMode::KeepAll, 2),
            )
            .unwrap();
        }
        assert_eq!(list_in(dir.path()).unwrap().len(), 5);
    }

    #[test]
    fn capped_boundary_exact_max_kept() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            append_in(
                dir.path(),
                &rec(&format!("s{i}"), HistoryOutcome::Sent),
                &cfg(HistoryMode::Capped, 3),
            )
            .unwrap();
        }
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 3, "恰好到上限不应裁剪");
    }

    #[test]
    fn capped_over_max_trims_oldest_and_deletes_audio() {
        let dir = tempfile::tempdir().unwrap();
        // 给最老一条挂音频文件，验证裁剪联动删除
        let adir = audio_dir(dir.path());
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(adir.join("s0.wav"), b"RIFF-fake").unwrap();
        let mut r0 = rec("s0", HistoryOutcome::Sent);
        r0.audio_file = Some("s0.wav".to_string());
        append_in(dir.path(), &r0, &cfg(HistoryMode::Capped, 2)).unwrap();
        append_in(dir.path(), &rec("s1", HistoryOutcome::Sent), &cfg(HistoryMode::Capped, 2))
            .unwrap();
        append_in(dir.path(), &rec("s2", HistoryOutcome::Error), &cfg(HistoryMode::Capped, 2))
            .unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].session_id, "s2");
        assert_eq!(records[1].session_id, "s1");
        assert!(!adir.join("s0.wav").exists(), "被裁记录的音频应一并删除");
    }

    #[test]
    fn list_skips_corrupt_lines() {
        let dir = tempfile::tempdir().unwrap();
        append_in(dir.path(), &rec("s1", HistoryOutcome::Sent), &cfg(HistoryMode::Capped, 10))
            .unwrap();
        // 手工追加坏行（并发写交错的最坏情况）
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(jsonl_path(dir.path()))
            .unwrap();
        writeln!(f, "not json {{{{").unwrap();
        append_in(dir.path(), &rec("s2", HistoryOutcome::Sent), &cfg(HistoryMode::Capped, 10))
            .unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 2, "坏行应跳过: {records:?}");
        assert_eq!(records[0].session_id, "s2");
    }

    #[test]
    fn clear_removes_jsonl_and_audio() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = rec("s1", HistoryOutcome::Sent);
        r.audio_file = Some("s1.wav".to_string());
        append_in(dir.path(), &r, &cfg(HistoryMode::KeepAll, 10)).unwrap();
        let adir = audio_dir(dir.path());
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(adir.join("s1.wav"), b"x").unwrap();
        clear_in(dir.path()).unwrap();
        assert!(!jsonl_path(dir.path()).exists());
        assert!(!adir.exists());
        // 空目录清空不报错
        clear_in(dir.path()).unwrap();
    }

    #[test]
    fn copy_audio_copies_eval_wav() {
        let dir = tempfile::tempdir().unwrap();
        let eval = tempfile::tempdir().unwrap();
        std::fs::write(eval.path().join("s1.wav"), b"RIFF-fake").unwrap();
        let name = copy_audio_in(dir.path(), eval.path(), "s1");
        assert_eq!(name.as_deref(), Some("s1.wav"));
        assert_eq!(
            std::fs::read(audio_dir(dir.path()).join("s1.wav")).unwrap(),
            b"RIFF-fake"
        );
        // eval 无 wav → None（记录照写，audioFile 为 null）
        assert!(copy_audio_in(dir.path(), eval.path(), "ghost").is_none());
    }
}
