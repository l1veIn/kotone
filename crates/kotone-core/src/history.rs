//! history：识别历史记录（~/.kotone/history.jsonl 追加式 JSONL，docs/development.md）。
//!
//! 设计要点：
//! - 追加式 JSONL：每次会话终态（sent / cancelled / error）追加一行，零索引成本；
//! - sessionId 与 eval 录档共用（orchestrator 生成一次 id 同时喂两边），可互查；
//! - includeAudio 开启时独立把会话 PCM 写到 history/audio/<sessionId>.wav，
//!   不依赖评测录档；记录里的 audioFile 存相对文件名；
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
    /// 是否随记录独立保存音频到 history/audio/（不依赖评测录档）
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
    /// 相对 history/audio/ 的文件名；includeAudio 关闭或音频写入失败时为 null
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
    let kept_audio: std::collections::HashSet<String> = kept
        .iter()
        .filter_map(|line| serde_json::from_str::<HistoryRecord>(line).ok())
        .filter_map(|rec| rec.audio_file)
        .collect();
    // 被裁记录的音频一并删除（best-effort，删不掉不阻塞裁剪）
    for line in dropped {
        if let Ok(rec) = serde_json::from_str::<HistoryRecord>(line) {
            if let Some(file) = rec.audio_file {
                // error → retry 会产生两条共享同一会话音频的记录；
                // 只裁掉旧记录时不能误删仍被保留记录引用的 WAV。
                if !kept_audio.contains(&file) {
                    let _ = std::fs::remove_file(audio_dir(dir).join(file));
                }
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
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取历史文件失败: {e}"))?;
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

/// includeAudio 落音频：把会话 PCM 直接写到 history/audio/<sessionId>.wav。
/// 成功返回相对文件名（填入 HistoryRecord.audioFile）；失败返回 None，
/// 记录本身仍照常写入。该路径与 evalRecording 完全独立。
pub fn write_audio_in(dir: &Path, session_id: &str, pcm: &[f32]) -> Option<String> {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return None;
    }
    let adir = audio_dir(dir);
    std::fs::create_dir_all(&adir).ok()?;
    let name = format!("{session_id}.wav");
    crate::eval::write_wav(&adir.join(&name), pcm).ok()?;
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
        append_in(
            dir.path(),
            &rec("s1", HistoryOutcome::Sent),
            &cfg(HistoryMode::Capped, 10),
        )
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
        append_in(
            dir.path(),
            &rec("s1", HistoryOutcome::Sent),
            &cfg(HistoryMode::Off, 10),
        )
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
        append_in(
            dir.path(),
            &rec("s1", HistoryOutcome::Sent),
            &cfg(HistoryMode::Capped, 2),
        )
        .unwrap();
        append_in(
            dir.path(),
            &rec("s2", HistoryOutcome::Error),
            &cfg(HistoryMode::Capped, 2),
        )
        .unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].session_id, "s2");
        assert_eq!(records[1].session_id, "s1");
        assert!(!adir.join("s0.wav").exists(), "被裁记录的音频应一并删除");
    }

    #[test]
    fn capped_trim_keeps_audio_still_referenced_by_retry_record() {
        let dir = tempfile::tempdir().unwrap();
        let adir = audio_dir(dir.path());
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(adir.join("shared.wav"), b"RIFF-fake").unwrap();

        let mut failed = rec("same-session", HistoryOutcome::Error);
        failed.audio_file = Some("shared.wav".to_string());
        append_in(dir.path(), &failed, &cfg(HistoryMode::Capped, 1)).unwrap();

        let mut retried = rec("same-session", HistoryOutcome::Sent);
        retried.audio_file = Some("shared.wav".to_string());
        append_in(dir.path(), &retried, &cfg(HistoryMode::Capped, 1)).unwrap();

        assert_eq!(list_in(dir.path()).unwrap().len(), 1);
        assert!(
            adir.join("shared.wav").exists(),
            "仍被重试成功记录引用的音频不能删除"
        );
    }

    #[test]
    fn list_skips_corrupt_lines() {
        let dir = tempfile::tempdir().unwrap();
        append_in(
            dir.path(),
            &rec("s1", HistoryOutcome::Sent),
            &cfg(HistoryMode::Capped, 10),
        )
        .unwrap();
        // 手工追加坏行（并发写交错的最坏情况）
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(jsonl_path(dir.path()))
            .unwrap();
        writeln!(f, "not json {{{{").unwrap();
        append_in(
            dir.path(),
            &rec("s2", HistoryOutcome::Sent),
            &cfg(HistoryMode::Capped, 10),
        )
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
    fn write_audio_writes_history_wav_without_eval_dir() {
        let dir = tempfile::tempdir().unwrap();
        let pcm = vec![0.25f32; crate::eval::SAMPLE_RATE as usize];
        let name = write_audio_in(dir.path(), "s1", &pcm);
        assert_eq!(name.as_deref(), Some("s1.wav"));
        let written = crate::eval::read_wav(&audio_dir(dir.path()).join("s1.wav")).unwrap();
        assert_eq!(written.len(), pcm.len());
        assert!(write_audio_in(dir.path(), "../escape", &pcm).is_none());
    }
}
