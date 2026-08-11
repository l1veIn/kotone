//! history：识别历史记录（~/.kotone/history.jsonl 追加式 JSONL，docs/development.md）。
//!
//! 设计要点：
//! - 追加式 JSONL：每次会话终态（sent / cancelled / error）追加一行，零索引成本；
//!   cancelled 仅当会话有可验证的语音产出（VAD 判定过语音，或流式引擎出过
//!   识别 partial，或 finalize 产出过文本）时落账——没开口就取消（如结束独奏
//!   模式时刚开出的空段）、以及非流式引擎（不发 partial）的取消，都不写记录，
//!   与空转录「无事发生」语义一致；
//! - sessionId 与 eval 录档共用（orchestrator 生成一次 id 同时喂两边），可互查；
//! - includeAudio 开启时独立把会话 PCM 写到 history/audio/<sessionId>.wav，
//!   不依赖评测录档；记录里的 audioFile 存相对文件名；
//! - 同一历史目录的 append / trim / delete / clear / audio 写入在进程内串行，
//!   避免原子重写与追加竞争造成记录丢失；每个目录缓存非空行数，只有 capped
//!   真正超限时才读取并裁剪完整 JSONL。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct DirectoryState {
    /// 当前进程首次访问时按需统计；之后随写操作维护。
    record_count: Option<usize>,
}

fn directory_state(dir: &Path) -> Arc<Mutex<DirectoryState>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<DirectoryState>>>>> = OnceLock::new();
    let states = STATES.get_or_init(|| Mutex::new(HashMap::new()));
    let key = dir.to_path_buf();
    let mut states = states.lock().unwrap();
    states
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(DirectoryState::default())))
        .clone()
}

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
    /// 自定义历史目录（含音频）；空 = 默认 ~/.kotone/history（P2-⑨：与模型
    /// 目录同理，历史音频也可能占用大量空间）
    #[serde(default)]
    pub dir: String,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            mode: HistoryMode::Capped,
            max_records: 1000,
            include_audio: false,
            dir: String::new(),
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

/// 历史数据目录：自定义（settings.history.dir）或默认 ~/.kotone/history/
pub fn history_dir() -> PathBuf {
    let settings = crate::settings::load();
    let dir = settings.history.dir.trim();
    if dir.is_empty() {
        crate::settings::kotone_dir().join("history")
    } else {
        PathBuf::from(dir)
    }
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
    let state = directory_state(dir);
    let mut state = state.lock().unwrap();
    append_unlocked(dir, record, cfg, &mut state)
}

fn append_unlocked(
    dir: &Path,
    record: &HistoryRecord,
    cfg: &HistoryConfig,
    state: &mut DirectoryState,
) -> Result<(), String> {
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
        let count = match state.record_count {
            Some(count) => count.saturating_add(1),
            None => count_nonempty_lines(&path)?,
        };
        state.record_count = Some(count);
        if count > cfg.max_records as usize {
            state.record_count = Some(trim_unlocked(dir, cfg.max_records)?);
        }
    } else if let Some(count) = state.record_count.as_mut() {
        *count = count.saturating_add(1);
    }
    Ok(())
}

/// capped 裁剪：超出 max 时保留尾部 N 条原子重写，并清理被裁记录的音频
fn trim_unlocked(dir: &Path, max: u32) -> Result<usize, String> {
    let path = jsonl_path(dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(0), // 刚写完读不到是异常情况；不致命，下次再裁
    };
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let max = max as usize;
    if lines.len() <= max {
        return Ok(lines.len());
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
    Ok(kept.len())
}

fn count_nonempty_lines(path: &Path) -> Result<usize, String> {
    use std::io::BufRead as _;
    let file = std::fs::File::open(path).map_err(|e| format!("读取历史记录数失败: {e}"))?;
    let reader = std::io::BufReader::new(file);
    reader.lines().try_fold(0usize, |count, line| {
        let line = line.map_err(|e| format!("读取历史记录数失败: {e}"))?;
        Ok(count + usize::from(!line.trim().is_empty()))
    })
}

// ---------- 读取 / 清空 ----------

/// 读取默认目录的全部记录（新→旧；坏行跳过）
pub fn list() -> Result<Vec<HistoryRecord>, String> {
    list_in(&history_dir())
}

/// 读取指定目录的全部记录（新→旧；坏行跳过不致命）
pub fn list_in(dir: &Path) -> Result<Vec<HistoryRecord>, String> {
    let state = directory_state(dir);
    let _state = state.lock().unwrap();
    list_unlocked(dir)
}

fn list_unlocked(dir: &Path) -> Result<Vec<HistoryRecord>, String> {
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

/// 删除一条记录（按 sessionId + ts 精确匹配；未找到 = 静默成功，幂等）。
/// 该记录带录音时，若对应 wav 不再被任何保留记录引用则一并删除——
/// error → retry 会共享同 sessionId 的音频，不能误删仍被引用的 WAV。
pub fn delete(session_id: &str, ts: &str) -> Result<(), String> {
    delete_in(&history_dir(), session_id, ts)
}

/// 删除指定目录中的一条记录：全读 → 过滤目标行 → 原子重写（同 trim_in 模式），
/// 再按引用保护清理音频。记录不存在时直接返回成功（幂等，无 IO 副作用）。
pub fn delete_in(dir: &Path, session_id: &str, ts: &str) -> Result<(), String> {
    let state = directory_state(dir);
    let mut state = state.lock().unwrap();
    let removed = delete_unlocked(dir, session_id, ts)?;
    if removed > 0 {
        if let Some(count) = state.record_count.as_mut() {
            *count = count.saturating_sub(removed);
        }
    }
    Ok(())
}

fn delete_unlocked(dir: &Path, session_id: &str, ts: &str) -> Result<usize, String> {
    let path = jsonl_path(dir);
    if !path.exists() {
        return Ok(0);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取历史文件失败: {e}"))?;
    let mut kept: Vec<String> = Vec::new();
    let mut removed_audio: Option<String> = None;
    let mut removed_count = 0usize;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(rec) = serde_json::from_str::<HistoryRecord>(line) {
            if rec.session_id == session_id && rec.ts == ts {
                removed_count += 1;
                removed_audio = rec.audio_file;
                continue; // 丢弃目标行
            }
        }
        kept.push(line.to_string());
    }
    if removed_count == 0 {
        return Ok(0);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("删除历史记录失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("落盘历史删除失败: {e}"))?;
    // 音频清理：仅当该 wav 不再被保留记录引用（error→retry 共享音频保护）
    if let Some(file) = removed_audio {
        let still_referenced = kept.iter().any(|line| {
            serde_json::from_str::<HistoryRecord>(line)
                .ok()
                .is_some_and(|r| r.audio_file.as_deref() == Some(file.as_str()))
        });
        if !still_referenced {
            let _ = std::fs::remove_file(audio_dir(dir).join(&file));
        }
    }
    Ok(removed_count)
}

/// 清空指定目录（jsonl + audio/ 目录整体删除）
pub fn clear_in(dir: &Path) -> Result<(), String> {
    let state = directory_state(dir);
    let mut state = state.lock().unwrap();
    // 任一步骤失败都让下次 append 从磁盘重新统计，避免部分清理后的缓存过期。
    state.record_count = None;
    let path = jsonl_path(dir);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除历史文件失败: {e}"))?;
    }
    let adir = audio_dir(dir);
    if adir.exists() {
        std::fs::remove_dir_all(&adir).map_err(|e| format!("删除历史音频目录失败: {e}"))?;
    }
    state.record_count = Some(0);
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
    let state = directory_state(dir);
    let _state = state.lock().unwrap();
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
            dir: String::new(),
        }
    }

    #[test]
    fn default_config_matches_doc() {
        let c = HistoryConfig::default();
        assert_eq!(c.mode, HistoryMode::Capped);
        assert_eq!(c.max_records, 1000);
        assert!(!c.include_audio);
        assert!(c.dir.is_empty(), "默认历史目录为空 = ~/.kotone/history");
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

    // ---------- 单条删除（delete_in） ----------

    fn rec_with_audio(session_id: &str, ts: &str, audio: Option<&str>) -> HistoryRecord {
        let mut r = rec(session_id, HistoryOutcome::Sent);
        r.ts = ts.to_string();
        r.audio_file = audio.map(String::from);
        r
    }

    fn write_audio_file(dir: &Path, name: &str) {
        let adir = audio_dir(dir);
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(adir.join(name), b"RIFF-fake").unwrap();
    }

    #[test]
    fn delete_removes_record_and_its_audio_only() {
        let dir = tempfile::tempdir().unwrap();
        write_audio_file(dir.path(), "a.wav");
        write_audio_file(dir.path(), "b.wav");
        append_in(
            dir.path(),
            &rec_with_audio("a", "t1", Some("a.wav")),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();
        append_in(
            dir.path(),
            &rec_with_audio("b", "t2", Some("b.wav")),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();
        append_in(
            dir.path(),
            &rec_with_audio("c", "t3", None),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();

        delete_in(dir.path(), "a", "t1").unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 2, "records: {records:?}");
        assert!(!records.iter().any(|r| r.session_id == "a"));
        assert!(
            !audio_dir(dir.path()).join("a.wav").exists(),
            "被删记录的音频应一并删除"
        );
        assert!(
            audio_dir(dir.path()).join("b.wav").exists(),
            "其他记录的音频不受影响"
        );
    }

    #[test]
    fn delete_keeps_audio_still_referenced_by_retry_record() {
        let dir = tempfile::tempdir().unwrap();
        write_audio_file(dir.path(), "shared.wav");
        // error → retry 两条同 sessionId、不同 ts，共享同一音频
        let mut failed = rec_with_audio("s1", "t1", Some("shared.wav"));
        failed.outcome = HistoryOutcome::Error;
        failed.error = Some("注入失败".into());
        append_in(dir.path(), &failed, &cfg(HistoryMode::KeepAll, 10)).unwrap();
        append_in(
            dir.path(),
            &rec_with_audio("s1", "t2", Some("shared.wav")),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();

        // 删 error 那条：记录消失，但 wav 仍被 retry 记录引用，不能删
        delete_in(dir.path(), "s1", "t1").unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].ts, "t2");
        assert!(
            audio_dir(dir.path()).join("shared.wav").exists(),
            "仍被保留记录引用的音频不能删除"
        );

        // 删最后一条（retry）：wav 不再被引用，一并删除
        delete_in(dir.path(), "s1", "t2").unwrap();
        assert!(list_in(dir.path()).unwrap().is_empty());
        assert!(!audio_dir(dir.path()).join("shared.wav").exists());
    }

    #[test]
    fn delete_missing_record_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        append_in(
            dir.path(),
            &rec_with_audio("a", "t1", None),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();
        // 不存在的记录：静默成功，文件不变
        delete_in(dir.path(), "a", "tX").unwrap();
        delete_in(dir.path(), "nope", "t1").unwrap();
        let records = list_in(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "a");
        // 空目录/不存在文件：幂等成功
        let empty = tempfile::tempdir().unwrap();
        delete_in(empty.path(), "a", "t1").unwrap();
    }

    #[test]
    fn delete_last_record_leaves_valid_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        write_audio_file(dir.path(), "a.wav");
        append_in(
            dir.path(),
            &rec_with_audio("a", "t1", Some("a.wav")),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();
        delete_in(dir.path(), "a", "t1").unwrap();
        assert!(list_in(dir.path()).unwrap().is_empty());
        assert!(
            jsonl_path(dir.path()).exists(),
            "jsonl 保留为空文件（append 可继续）"
        );
        assert!(!audio_dir(dir.path()).join("a.wav").exists());
        // 删除后仍可继续追加
        append_in(
            dir.path(),
            &rec_with_audio("b", "t2", None),
            &cfg(HistoryMode::KeepAll, 10),
        )
        .unwrap();
        assert_eq!(list_in(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn concurrent_appends_do_not_lose_or_corrupt_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().to_path_buf());
        let barrier = Arc::new(std::sync::Barrier::new(33));
        let mut workers = Vec::new();
        for index in 0..32 {
            let path = path.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                append_in(
                    &path,
                    &rec(&format!("concurrent-{index}"), HistoryOutcome::Sent),
                    &cfg(HistoryMode::Capped, 100),
                )
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let records = list_in(&path).unwrap();
        assert_eq!(records.len(), 32);
        let ids: std::collections::HashSet<_> = records
            .iter()
            .map(|record| record.session_id.as_str())
            .collect();
        assert_eq!(ids.len(), 32);
    }

    #[test]
    fn concurrent_delete_and_append_preserve_all_unrelated_records() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..20 {
            append_in(
                dir.path(),
                &rec(&format!("old-{index}"), HistoryOutcome::Sent),
                &cfg(HistoryMode::KeepAll, 100),
            )
            .unwrap();
        }

        let path = Arc::new(dir.path().to_path_buf());
        let barrier = Arc::new(std::sync::Barrier::new(21));
        let mut workers = Vec::new();
        for index in 0..10 {
            let path = path.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                delete_in(&path, &format!("old-{index}"), "2026-05-20T12:00:00Z")
            }));
        }
        for index in 0..10 {
            let path = path.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                append_in(
                    &path,
                    &rec(&format!("new-{index}"), HistoryOutcome::Sent),
                    &cfg(HistoryMode::KeepAll, 100),
                )
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let records = list_in(&path).unwrap();
        assert_eq!(records.len(), 20);
        for index in 0..10 {
            assert!(records
                .iter()
                .any(|record| record.session_id == format!("new-{index}")));
            assert!(!records
                .iter()
                .any(|record| record.session_id == format!("old-{index}")));
        }
    }
}
