//! kotone-eval：引擎评测工具 —— 会话录档、语料回放、多引擎对比
//! （docs/development.md §3.3「评测工具」、§5.4 eval json；取舍见 docs/adr/005）
//!
//! 存储布局（~/.kotone/eval/）：
//! - `<sessionId>.json` + `<sessionId>.wav`：录档（wav 固定 16kHz/16bit/mono）
//! - `replays/<sessionId>__<engineId>.json`：回放结果缓存（report 优先复用）
//! 容量：只保留最近 200 个会话，超出连同 wav/replay 缓存一起清理。
//!
//! 依赖纪律：core 不认识 kotone-stt，回放所需引擎实例经 `EngineRegistry`
//! 参数由调用方（壳 / CLI）注入；wav 编解码自带（与 whisper_sidecar 的
//! 私有 write_wav 重复是有意的——core 不能依赖 stt）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::stt::{EngineRegistry, SessionConfig, SttEvent};

/// 录档容量上限：只保留最近 N 个会话
pub const MAX_SESSIONS: usize = 200;
/// 采样率契约：16kHz mono f32（与 audio / stt 层一致）
const SAMPLE_RATE: u32 = 16000;
/// 回放时的喂入块大小（100ms）；第一版全量灌，不按原始节奏（ADR-005）
const REPLAY_CHUNK_SAMPLES: usize = 16000 / 10;

/// 一条 partial 时间线记录
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialRecord {
    /// 相对会话开始（开始喂音频）的毫秒偏移
    pub t: u64,
    pub text: String,
}

/// 评测会话录档（字段与 docs/development.md §5.4 eval json 对应）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalSession {
    pub session_id: String,
    pub engine_id: String,
    pub started_at: String,
    pub audio_ms: u64,
    /// 非流式引擎为 None
    pub first_partial_ms: Option<u64>,
    pub final_ms: u64,
    pub partials: Vec<PartialRecord>,
    pub final_text: String,
    /// 人工评测时回填正确文本，用于 CER
    pub human_label: Option<String>,
}

/// 一次回放的结果（docs/development.md §5.3 eval_replay -> EvalResult）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    pub session_id: String,
    /// 回放所用引擎（可与录档引擎不同——这正是多引擎对比的意义）
    pub engine_id: String,
    pub replayed_at: String,
    /// 全速灌入下首条 partial 的纯计算延迟；非流式引擎为 None
    pub first_partial_ms: Option<u64>,
    pub final_ms: u64,
    pub partials: Vec<PartialRecord>,
    pub final_text: String,
    /// 录档带 humanLabel 时算出（字符级，去标点/空白、统一小写）
    pub cer: Option<f64>,
}

// ---------- 路径 ----------

/// 评测数据目录：~/.kotone/eval/
pub fn eval_dir() -> PathBuf {
    crate::settings::kotone_dir().join("eval")
}

fn session_json_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.json"))
}

fn session_wav_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.wav"))
}

fn replays_dir(dir: &Path) -> PathBuf {
    dir.join("replays")
}

fn replay_path(dir: &Path, session_id: &str, engine_id: &str) -> PathBuf {
    replays_dir(dir).join(format!("{session_id}__{engine_id}.json"))
}

/// sessionId 只允许时间戳式字符，防路径穿越（label/replay 接受外部输入）
fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("非法 sessionId：{session_id:?}"));
    }
    Ok(())
}

// ---------- 时间（core 不引时间 crate；UTC 日历用 Howard Hinnant 算法） ----------

/// epoch 天数 → 公历（年, 月, 日）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn utc_now_parts() -> (i64, u32, u32, u32, u32, u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let days = (now.as_secs() / 86400) as i64;
    let secs = (now.as_secs() % 86400) as u32;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, secs / 3600, secs % 3600 / 60, secs % 60, now.subsec_millis())
}

/// ISO 8601 UTC（简化：不带本地时区偏移）
pub fn utc_now_iso() -> String {
    let (y, m, d, hh, mm, ss, _) = utc_now_parts();
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn utc_compact() -> String {
    let (y, m, d, hh, mm, ss, _) = utc_now_parts();
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// 时间戳式会话 ID：YYYYMMDD-HHMMSS-<毫秒尾>
pub fn new_session_id() -> String {
    let (_, _, _, _, _, _, ms) = utc_now_parts();
    format!("{}-{ms:03}", utc_compact())
}

// ---------- 录档 ----------

/// orchestrator 的录档句柄：begin 时创建（evalRecording 开启），录音期间由
/// pump 喂 pcm / partial，finalize 成功后 finish 落盘；取消时随会话句柄丢弃。
#[derive(Clone)]
pub struct SessionRecorder {
    dir: PathBuf,
    session_id: String,
    engine_id: String,
    started_at: String,
    started: Instant,
    pcm: Arc<Mutex<Vec<f32>>>,
    partials: Arc<Mutex<Vec<PartialRecord>>>,
}

impl SessionRecorder {
    /// 录到默认目录（~/.kotone/eval/）
    pub fn new(engine_id: &str) -> Self {
        Self::new_in(eval_dir(), engine_id)
    }

    /// 录到指定目录（测试可指向临时目录）
    pub fn new_in(dir: PathBuf, engine_id: &str) -> Self {
        Self {
            dir,
            session_id: new_session_id(),
            engine_id: engine_id.to_string(),
            started_at: utc_now_iso(),
            started: Instant::now(),
            pcm: Arc::new(Mutex::new(Vec::new())),
            partials: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// pump 录音线程喂入一块 pcm（追加到录制缓冲）
    pub fn push_pcm(&self, chunk: &[f32]) {
        self.pcm.lock().unwrap().extend_from_slice(chunk);
    }

    /// pump 记录一条 partial（带相对毫秒偏移）
    pub fn push_partial(&self, text: &str) {
        let t = self.started.elapsed().as_millis() as u64;
        self.partials.lock().unwrap().push(PartialRecord {
            t,
            text: text.to_string(),
        });
    }

    /// finalize 成功后完成录档：构造 EvalSession 并落盘（wav + json + 容量清理）
    pub fn finish(self, final_text: &str, final_ms: u64) -> Result<EvalSession, String> {
        let pcm = std::mem::take(&mut *self.pcm.lock().unwrap());
        let partials = std::mem::take(&mut *self.partials.lock().unwrap());
        let session = EvalSession {
            session_id: self.session_id.clone(),
            engine_id: self.engine_id.clone(),
            started_at: self.started_at.clone(),
            audio_ms: pcm.len() as u64 * 1000 / SAMPLE_RATE as u64,
            first_partial_ms: partials.first().map(|p| p.t),
            final_ms,
            partials,
            final_text: final_text.to_string(),
            human_label: None,
        };
        record_session_at(&self.dir, &session, &pcm)?;
        Ok(session)
    }
}

/// 录档一次识别会话（wav + 指标 JSON）到默认目录
pub fn record_session(session: &EvalSession, pcm: &[f32]) -> Result<(), String> {
    record_session_at(&eval_dir(), session, pcm)
}

/// 录档到指定目录：json 原子写入 + wav + 容量清理
pub fn record_session_at(dir: &Path, session: &EvalSession, pcm: &[f32]) -> Result<(), String> {
    validate_session_id(&session.session_id)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("创建评测目录失败: {e}"))?;
    write_session_json(dir, session)?;
    write_wav(&session_wav_path(dir, &session.session_id), pcm)
        .map_err(|e| format!("写入 wav 失败: {e}"))?;
    prune_at(dir)
}

fn write_session_json(dir: &Path, session: &EvalSession) -> Result<(), String> {
    let path = session_json_path(dir, &session.session_id);
    let json =
        serde_json::to_string_pretty(session).map_err(|e| format!("序列化录档失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入录档失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("落盘录档失败: {e}"))?;
    Ok(())
}

/// 容量清理：会话数超 MAX_SESSIONS 时，最旧的连 wav / 回放缓存一起删
fn prune_at(dir: &Path) -> Result<(), String> {
    let mut ids = session_ids_at(dir)?;
    if ids.len() <= MAX_SESSIONS {
        return Ok(());
    }
    for id in ids.drain(..ids.len() - MAX_SESSIONS) {
        let _ = std::fs::remove_file(session_json_path(dir, &id));
        let _ = std::fs::remove_file(session_wav_path(dir, &id));
        let prefix = format!("{id}__");
        if let Ok(rd) = std::fs::read_dir(replays_dir(dir)) {
            for entry in rd.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

/// 全部会话 id（按字典序升序 = 时间升序，id 为时间戳式）
fn session_ids_at(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(e) => return Err(format!("读取评测目录失败: {e}")),
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

// ---------- 列表 / 读取 / 标注 ----------

/// 列出录档会话（新的在前）
pub fn list_sessions() -> Result<Vec<EvalSession>, String> {
    list_sessions_at(&eval_dir())
}

pub fn list_sessions_at(dir: &Path) -> Result<Vec<EvalSession>, String> {
    let mut out = Vec::new();
    for id in session_ids_at(dir)? {
        // 损坏的单个文件不拖垮整个列表（静默跳过并记日志）
        match load_session_at(dir, &id) {
            Ok(s) => out.push(s),
            Err(e) => crate::log::log(&format!("eval 录档 {id} 读取失败（跳过）: {e}")),
        }
    }
    out.reverse(); // id 时间戳式，逆序 = 新的在前
    Ok(out)
}

fn load_session_at(dir: &Path, session_id: &str) -> Result<EvalSession, String> {
    let raw = std::fs::read_to_string(session_json_path(dir, session_id))
        .map_err(|e| format!("读取录档 {session_id} 失败: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("解析录档 {session_id} 失败: {e}"))
}

/// 回填人工标注（正确文本），供 CER 计算
pub fn label(session_id: &str, text: &str) -> Result<EvalSession, String> {
    label_at(&eval_dir(), session_id, text)
}

pub fn label_at(dir: &Path, session_id: &str, text: &str) -> Result<EvalSession, String> {
    validate_session_id(session_id)?;
    let mut session = load_session_at(dir, session_id)?;
    session.human_label = Some(text.to_string());
    write_session_json(dir, &session)?;
    Ok(session)
}

// ---------- 回放 ----------

/// 语料回放：录档 wav 对任意已安装引擎离线重放（默认目录）
pub fn replay(
    session_id: &str,
    engine_id: &str,
    registry: &EngineRegistry,
) -> Result<EvalResult, String> {
    replay_at(&eval_dir(), session_id, engine_id, registry)
}

/// 回放到指定目录中的会话。第一版全速灌入（100ms 块连续推，不按原始节奏），
/// 因此 first_partial_ms 反映纯计算延迟，便于引擎横向对比（ADR-005）。
pub fn replay_at(
    dir: &Path,
    session_id: &str,
    engine_id: &str,
    registry: &EngineRegistry,
) -> Result<EvalResult, String> {
    validate_session_id(session_id)?;
    let session = load_session_at(dir, session_id)?;
    let pcm = read_wav(&session_wav_path(dir, session_id))?;
    let engine = registry
        .get(engine_id)
        .ok_or_else(|| format!("未注册的 STT 引擎: {engine_id}"))?;
    if !engine.is_ready() {
        return Err(format!(
            "引擎「{}」未就绪（模型未下载或未启用）",
            engine.display_name()
        ));
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<SttEvent>();
    let mut stt = engine.start_session(&SessionConfig::default(), tx)?;
    let started = Instant::now();
    let mut partials: Vec<PartialRecord> = Vec::new();
    let drain = |rx: &mut mpsc::UnboundedReceiver<SttEvent>,
                     partials: &mut Vec<PartialRecord>| {
        while let Ok(ev) = rx.try_recv() {
            if let SttEvent::Partial { text } = ev {
                partials.push(PartialRecord {
                    t: started.elapsed().as_millis() as u64,
                    text,
                });
            }
        }
    };
    for chunk in pcm.chunks(REPLAY_CHUNK_SAMPLES) {
        stt.push_audio(chunk)?;
        drain(&mut rx, &mut partials);
    }
    let t = stt.finalize()?;
    drain(&mut rx, &mut partials); // finalize 前可能补发最后一条 partial

    let result = EvalResult {
        session_id: session.session_id.clone(),
        engine_id: engine_id.to_string(),
        replayed_at: utc_now_iso(),
        first_partial_ms: partials.first().map(|p| p.t),
        final_ms: t.latency_ms as u64,
        partials,
        final_text: t.text.clone(),
        cer: session.human_label.as_deref().map(|l| cer(l, &t.text)),
    };
    save_replay_at(dir, &result)?;
    Ok(result)
}

fn save_replay_at(dir: &Path, result: &EvalResult) -> Result<(), String> {
    let rp = replays_dir(dir);
    std::fs::create_dir_all(&rp).map_err(|e| format!("创建回放目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(result).map_err(|e| format!("序列化回放失败: {e}"))?;
    std::fs::write(replay_path(dir, &result.session_id, &result.engine_id), json)
        .map_err(|e| format!("写入回放结果失败: {e}"))?;
    Ok(())
}

/// 读取已存回放结果（report 优先复用，避免重复跑引擎）
pub fn stored_replay(session_id: &str, engine_id: &str) -> Option<EvalResult> {
    stored_replay_at(&eval_dir(), session_id, engine_id)
}

pub fn stored_replay_at(dir: &Path, session_id: &str, engine_id: &str) -> Option<EvalResult> {
    let raw = std::fs::read_to_string(replay_path(dir, session_id, engine_id)).ok()?;
    serde_json::from_str(&raw).ok()
}

// ---------- 报告 ----------

/// 多引擎对比报告（Markdown）：已人工标注的会话 × 全部就绪引擎，
/// 各引擎 CER 与延迟均值。优先复用已存回放，缺失的现场跑（并落缓存）。
pub fn report(registry: &EngineRegistry) -> Result<String, String> {
    report_at(&eval_dir(), registry)
}

pub fn report_at(dir: &Path, registry: &EngineRegistry) -> Result<String, String> {
    let sessions: Vec<EvalSession> = list_sessions_at(dir)?
        .into_iter()
        .filter(|s| s.human_label.is_some())
        .collect();
    if sessions.is_empty() {
        return Err("暂无人工标注的会话（先用 eval label <id> --text <正确文本> 标注）".into());
    }
    let engines: Vec<_> = registry
        .list_info()
        .into_iter()
        .filter(|i| i.is_ready)
        .collect();
    if engines.is_empty() {
        return Err("没有就绪的引擎（需至少一款已安装模型的引擎）".into());
    }

    let mut out = format!("# Kotone 引擎评测报告（{} 条标注语料）\n\n", sessions.len());
    out.push_str("| 引擎 | 样本数 | 平均 CER | 平均首字延迟 ms | 平均最终延迟 ms |\n");
    out.push_str("|------|-------:|---------:|----------------:|----------------:|\n");
    for info in &engines {
        let mut results = Vec::new();
        for s in &sessions {
            let r = match stored_replay_at(dir, &s.session_id, &info.id) {
                Some(r) => Some(r),
                None => match replay_at(dir, &s.session_id, &info.id, registry) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        crate::log::log(&format!(
                            "eval 报告：{} × {} 回放失败（跳过）: {e}",
                            s.session_id, info.id
                        ));
                        None
                    }
                },
            };
            if let Some(r) = r {
                results.push(r);
            }
        }
        if results.is_empty() {
            out.push_str(&format!("| {} | 0 | — | — | — |\n", info.id));
            continue;
        }
        let n = results.len();
        let cers: Vec<f64> = results.iter().filter_map(|r| r.cer).collect();
        let mean_cer = if cers.is_empty() {
            "—".to_string()
        } else {
            format!("{:.3}", cers.iter().sum::<f64>() / cers.len() as f64)
        };
        let firsts: Vec<u64> = results.iter().filter_map(|r| r.first_partial_ms).collect();
        let mean_first = if firsts.is_empty() {
            "—（非流式）".to_string()
        } else {
            format!("{}", firsts.iter().sum::<u64>() / firsts.len() as u64)
        };
        let mean_final = results.iter().map(|r| r.final_ms).sum::<u64>() / n as u64;
        out.push_str(&format!(
            "| {} | {n} | {mean_cer} | {mean_first} | {mean_final} |\n",
            info.id
        ));
    }
    out.push_str(
        "\n> CER = 字符级编辑距离 / 参考文本长度（双方去标点与空白、统一小写）；\
         首字延迟为全速回放的纯计算延迟，仅流式引擎有。\n",
    );
    Ok(out)
}

// ---------- 导出 ----------

/// 导出评测数据：复制全部录档（json + wav + 回放缓存）到时间戳目录，
/// 并附 sessions.jsonl 索引（每行一条 EvalSession）。返回导出目录路径。
pub fn export() -> Result<String, String> {
    export_at(&eval_dir())
}

pub fn export_at(dir: &Path) -> Result<String, String> {
    let parent = dir.parent().unwrap_or(dir);
    let dest = parent.join(format!("eval-export-{}", utc_compact()));
    std::fs::create_dir_all(&dest).map_err(|e| format!("创建导出目录失败: {e}"))?;

    let sessions = list_sessions_at(dir)?;
    let mut jsonl = String::new();
    for s in &sessions {
        let line =
            serde_json::to_string(s).map_err(|e| format!("序列化索引导出失败: {e}"))?;
        jsonl.push_str(&line);
        jsonl.push('\n');
        for p in [
            session_json_path(dir, &s.session_id),
            session_wav_path(dir, &s.session_id),
        ] {
            if p.exists() {
                std::fs::copy(&p, dest.join(p.file_name().unwrap()))
                    .map_err(|e| format!("复制 {} 失败: {e}", p.display()))?;
            }
        }
    }
    std::fs::write(dest.join("sessions.jsonl"), jsonl)
        .map_err(|e| format!("写入索引失败: {e}"))?;

    // 回放缓存整个子目录复制
    let rp = replays_dir(dir);
    if rp.exists() {
        let dest_rp = dest.join("replays");
        std::fs::create_dir_all(&dest_rp).map_err(|e| format!("创建导出回放目录失败: {e}"))?;
        for entry in std::fs::read_dir(&rp)
            .map_err(|e| format!("读取回放目录失败: {e}"))?
            .flatten()
        {
            std::fs::copy(entry.path(), dest_rp.join(entry.file_name()))
                .map_err(|e| format!("复制回放 {} 失败: {e}", entry.file_name().to_string_lossy()))?;
        }
    }
    Ok(dest.to_string_lossy().into_owned())
}

// ---------- CER（字符级，去标点/空白、统一小写；手写 DP 不引 crate） ----------

/// 字符错误率：编辑距离 / 参考文本字符数。参考为空时：假设也为空记 0，否则记 1。
pub fn cer(reference: &str, hypothesis: &str) -> f64 {
    let r = normalize_cer(reference);
    let h = normalize_cer(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    levenshtein(&r, &h) as f64 / r.len() as f64
}

/// 归一化：只保留字母数字（CJK 表意字符 is_alphanumeric 为 true），统一小写
fn normalize_cer(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 两行 DP 编辑距离
fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// ---------- wav 编解码（16kHz/16bit/mono；与 whisper_sidecar 重复是有意的） ----------

/// f32 PCM（-1..1）→ 16bit PCM wav 文件
pub fn write_wav(path: &Path, pcm: &[f32]) -> std::io::Result<()> {
    use std::io::Write as _;
    let data_len = (pcm.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);

    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&SAMPLE_RATE.to_le_bytes())?;
    f.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?; // byte rate
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits per sample
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for &s in pcm {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()
}

/// 读取 16bit PCM mono wav → f32 PCM（-1..1）
pub fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let data =
        std::fs::read(path).map_err(|e| format!("读取 wav 失败 {}: {e}", path.display()))?;
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("不是有效的 RIFF/WAVE 文件：{}", path.display()));
    }
    let mut pos = 12usize;
    let mut fmt_checked = false;
    let mut pcm: Option<Vec<f32>> = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if body + size > data.len() {
            break;
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err("wav fmt chunk 过短".into());
                }
                let format = u16::from_le_bytes(data[body..body + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap());
                let bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().unwrap());
                if format != 1 || channels != 1 || bits != 16 {
                    return Err(format!(
                        "仅支持 16bit PCM mono wav（format={format} channels={channels} bits={bits}）"
                    ));
                }
                fmt_checked = true;
            }
            b"data" => {
                pcm = Some(
                    data[body..body + size]
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                        .collect(),
                );
            }
            _ => {}
        }
        pos = body + size + (size & 1); // chunk 按 2 字节对齐
    }
    if !fmt_checked {
        return Err("wav 缺少 fmt chunk".into());
    }
    pcm.ok_or_else(|| "wav 缺少 data chunk".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::{EngineCapabilities, SttEngine, SttSession, Transcript};

    // ---------- 手写假引擎（回放路径测试；core 单测不依赖 kotone-stt，见 ADR-001） ----------

    /// 假流式引擎：每收到一块音频发一条 partial，finalize 返回固定文本 + 固定延迟
    struct FakeStreamEngine {
        ready: bool,
    }

    impl SttEngine for FakeStreamEngine {
        fn id(&self) -> &'static str {
            "fake-stream"
        }
        fn display_name(&self) -> &str {
            "假流式引擎"
        }
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: true,
                hotwords: false,
                gpu: false,
                offline: true,
                languages: vec!["zh".into()],
            }
        }
        fn is_ready(&self) -> bool {
            self.ready
        }
        fn start_session(
            &self,
            _cfg: &SessionConfig,
            events: mpsc::UnboundedSender<SttEvent>,
        ) -> Result<Box<dyn SttSession>, String> {
            Ok(Box::new(FakeSession {
                events,
                pushes: 0,
            }))
        }
    }

    struct FakeSession {
        events: mpsc::UnboundedSender<SttEvent>,
        pushes: usize,
    }

    impl SttSession for FakeSession {
        fn push_audio(&mut self, _pcm: &[f32]) -> Result<(), String> {
            self.pushes += 1;
            if self.pushes == 1 {
                let _ = self.events.send(SttEvent::Partial {
                    text: "对面打野".into(),
                });
            }
            Ok(())
        }
        fn finalize(self: Box<Self>) -> Result<Transcript, String> {
            Ok(Transcript {
                text: "对面打野在下路".into(),
                latency_ms: 123,
            })
        }
        fn cancel(&mut self) {}
    }

    fn fake_registry(ready: bool) -> EngineRegistry {
        let mut reg = EngineRegistry::new();
        reg.register(Box::new(FakeStreamEngine { ready }));
        reg
    }

    fn sample_session(id: &str) -> EvalSession {
        EvalSession {
            session_id: id.into(),
            engine_id: "fake-stream".into(),
            started_at: "2026-07-25T12:00:00Z".into(),
            audio_ms: 100,
            first_partial_ms: Some(10),
            final_ms: 123,
            partials: vec![PartialRecord {
                t: 10,
                text: "对面打野".into(),
            }],
            final_text: "对面打野在下路".into(),
            human_label: None,
        }
    }

    // ---------- wav 编解码 ----------

    #[test]
    fn wav_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let pcm: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) * 2.0 - 1.0).collect();
        write_wav(&path, &pcm).unwrap();
        let back = read_wav(&path).unwrap();
        assert_eq!(back.len(), pcm.len());
        for (a, b) in pcm.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn read_wav_rejects_non_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.wav");
        std::fs::write(&path, b"not a wav").unwrap();
        assert!(read_wav(&path).is_err());
    }

    // ---------- 落盘契约 ----------

    #[test]
    fn record_writes_json_and_wav_with_contract_fields() {
        let dir = tempfile::tempdir().unwrap();
        let session = sample_session("20260725-120000-001");
        let pcm = vec![0.1f32; 1600];
        record_session_at(dir.path(), &session, &pcm).unwrap();

        let json_path = dir.path().join("20260725-120000-001.json");
        let wav_path = dir.path().join("20260725-120000-001.wav");
        assert!(json_path.exists());
        assert!(wav_path.exists());

        // §5.4 字段契约：camelCase 键名逐一对齐
        let raw = std::fs::read_to_string(&json_path).unwrap();
        for key in [
            "\"sessionId\"",
            "\"engineId\"",
            "\"startedAt\"",
            "\"audioMs\"",
            "\"firstPartialMs\"",
            "\"finalMs\"",
            "\"partials\"",
            "\"finalText\"",
            "\"humanLabel\"",
        ] {
            assert!(raw.contains(key), "缺字段 {key}: {raw}");
        }
        let loaded = load_session_at(dir.path(), &session.session_id).unwrap();
        assert_eq!(loaded.final_text, "对面打野在下路");
        assert_eq!(loaded.first_partial_ms, Some(10));
        assert_eq!(loaded.human_label, None);

        // wav 可解码回等量采样
        let back = read_wav(&wav_path).unwrap();
        assert_eq!(back.len(), pcm.len());
    }

    #[test]
    fn list_sessions_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["20260725-120000-001", "20260725-120000-003", "20260725-120000-002"] {
            record_session_at(dir.path(), &sample_session(id), &[]).unwrap();
        }
        let sessions = list_sessions_at(dir.path()).unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(
            ids,
            ["20260725-120000-003", "20260725-120000-002", "20260725-120000-001"]
        );
    }

    // ---------- 容量清理 ----------

    #[test]
    fn prune_keeps_newest_200() {
        let dir = tempfile::tempdir().unwrap();
        // 预置一个回放缓存，验证随会话一起被清理
        let rp = replays_dir(dir.path());
        std::fs::create_dir_all(&rp).unwrap();
        std::fs::write(rp.join("20260725-000000-000__fake-stream.json"), "{}").unwrap();

        for i in 0..205 {
            let id = format!("20260725-000000-{i:03}");
            record_session_at(dir.path(), &sample_session(&id), &[0.0f32; 16]).unwrap();
        }
        let ids = session_ids_at(dir.path()).unwrap();
        assert_eq!(ids.len(), MAX_SESSIONS);
        // 最旧 5 个（000..004）应连同 wav 一起被删
        for i in 0..5 {
            let id = format!("20260725-000000-{i:03}");
            assert!(!session_json_path(dir.path(), &id).exists(), "{id} 应被清理");
            assert!(!session_wav_path(dir.path(), &id).exists(), "{id}.wav 应被清理");
        }
        assert!(
            !rp.join("20260725-000000-000__fake-stream.json").exists(),
            "被清理会话的回放缓存应一并删除"
        );
        // 最新的仍在
        assert!(session_json_path(dir.path(), "20260725-000000-204").exists());
    }

    // ---------- CER ----------

    #[test]
    fn cer_identical_is_zero() {
        assert_eq!(cer("对面打野在下路", "对面打野在下路"), 0.0);
    }

    #[test]
    fn cer_ignores_punctuation_whitespace_case() {
        assert_eq!(cer("对面打野，在下路！", "对面打野在下路"), 0.0);
        assert_eq!(cer("Gank 上路", "gank上路"), 0.0);
        assert_eq!(cer("a b c", "abc"), 0.0);
    }

    #[test]
    fn cer_char_level_chinese() {
        // 「对面打野」vs「对面中单」：替换 2 字 / 4 字 = 0.5
        assert_eq!(cer("对面打野", "对面中单"), 0.5);
        // 缺 1 字 / 4 字 = 0.25
        assert_eq!(cer("对面打野", "对面打"), 0.25);
    }

    #[test]
    fn cer_empty_reference() {
        assert_eq!(cer("", ""), 0.0);
        assert_eq!(cer("", "有内容"), 1.0);
    }

    // ---------- 回放 ----------

    #[test]
    fn replay_feeds_wav_and_collects_partials() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = sample_session("20260725-130000-001");
        session.human_label = Some("对面打野在下路".into());
        // 0.3s 假音频（3 个 100ms 块）
        let pcm = vec![0.05f32; 4800];
        record_session_at(dir.path(), &session, &pcm).unwrap();

        let reg = fake_registry(true);
        let r = replay_at(dir.path(), &session.session_id, "fake-stream", &reg).unwrap();
        assert_eq!(r.session_id, session.session_id);
        assert_eq!(r.engine_id, "fake-stream");
        assert_eq!(r.final_text, "对面打野在下路");
        assert_eq!(r.final_ms, 123);
        assert_eq!(r.partials.len(), 1);
        assert_eq!(r.partials[0].text, "对面打野");
        assert_eq!(r.first_partial_ms, r.partials.first().map(|p| p.t));
        assert_eq!(r.cer, Some(0.0), "标注与回放文本一致时 CER 应为 0");
        // 回放结果落缓存
        let cached = stored_replay_at(dir.path(), &session.session_id, "fake-stream");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().final_text, "对面打野在下路");
    }

    #[test]
    fn replay_unknown_engine_errors() {
        let dir = tempfile::tempdir().unwrap();
        let session = sample_session("20260725-130000-002");
        record_session_at(dir.path(), &session, &[0.0; 160]).unwrap();
        let reg = fake_registry(true);
        assert!(replay_at(dir.path(), &session.session_id, "no-such", &reg).is_err());
    }

    #[test]
    fn replay_unready_engine_errors() {
        let dir = tempfile::tempdir().unwrap();
        let session = sample_session("20260725-130000-003");
        record_session_at(dir.path(), &session, &[0.0; 160]).unwrap();
        let reg = fake_registry(false);
        let e = replay_at(dir.path(), &session.session_id, "fake-stream", &reg).unwrap_err();
        assert!(e.contains("未就绪"), "{e}");
    }

    #[test]
    fn replay_rejects_traversal_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let reg = fake_registry(true);
        assert!(replay_at(dir.path(), "../evil", "fake-stream", &reg).is_err());
    }

    // ---------- 标注 ----------

    #[test]
    fn label_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let session = sample_session("20260725-140000-001");
        record_session_at(dir.path(), &session, &[]).unwrap();
        let updated = label_at(dir.path(), &session.session_id, "对面打野在下路").unwrap();
        assert_eq!(updated.human_label.as_deref(), Some("对面打野在下路"));
        let loaded = load_session_at(dir.path(), &session.session_id).unwrap();
        assert_eq!(loaded.human_label.as_deref(), Some("对面打野在下路"));
    }

    // ---------- 报告 ----------

    #[test]
    fn report_aggregates_labeled_sessions() {
        let dir = tempfile::tempdir().unwrap();
        for (i, label) in ["对面打野在下路", "中路小心 gank"].iter().enumerate() {
            let mut s = sample_session(&format!("20260725-150000-{i:03}"));
            s.human_label = Some(label.to_string());
            record_session_at(dir.path(), &s, &[0.0f32; 1600]).unwrap();
        }
        let reg = fake_registry(true);
        let md = report_at(dir.path(), &reg).unwrap();
        assert!(md.contains("fake-stream"), "{md}");
        assert!(md.contains("| 引擎 |"), "{md}");
        assert!(md.contains("2 条标注语料"), "{md}");
        // 回放结果已缓存：第二条语料文本与假引擎输出不同 → CER > 0
        let r1 = stored_replay_at(dir.path(), "20260725-150000-000", "fake-stream").unwrap();
        assert_eq!(r1.cer, Some(0.0));
        let r2 = stored_replay_at(dir.path(), "20260725-150000-001", "fake-stream").unwrap();
        assert!(r2.cer.unwrap() > 0.0);
    }

    #[test]
    fn report_without_labels_errors() {
        let dir = tempfile::tempdir().unwrap();
        record_session_at(dir.path(), &sample_session("20260725-150100-000"), &[]).unwrap();
        let reg = fake_registry(true);
        assert!(report_at(dir.path(), &reg).is_err());
    }

    // ---------- 导出 ----------

    #[test]
    fn export_copies_sessions_wavs_and_index() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            let id = format!("20260725-160000-{i:03}");
            record_session_at(dir.path(), &sample_session(&id), &[0.0f32; 160]).unwrap();
        }
        let dest = PathBuf::from(export_at(dir.path()).unwrap());
        assert!(dest.exists());
        assert_eq!(
            std::fs::read_dir(&dest)
                .unwrap()
                .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "json"))
                .count(),
            3
        );
        let jsonl = std::fs::read_to_string(dest.join("sessions.jsonl")).unwrap();
        assert_eq!(jsonl.lines().count(), 3);
        for i in 0..3 {
            assert!(dest.join(format!("20260725-160000-{i:03}.wav")).exists());
        }
    }

    // ---------- 录档句柄 ----------

    #[test]
    fn recorder_finish_builds_session_and_writes_files() {
        let dir = tempfile::tempdir().unwrap();
        let rec = SessionRecorder::new_in(dir.path().to_path_buf(), "fake-stream");
        rec.push_pcm(&[0.1f32; 16000]); // 1s
        rec.push_partial("对面打野");
        rec.push_pcm(&[0.1f32; 8000]); // 0.5s
        let session = rec.finish("对面打野在下路", 321).unwrap();

        assert_eq!(session.engine_id, "fake-stream");
        assert_eq!(session.audio_ms, 1500);
        assert_eq!(session.final_ms, 321);
        assert_eq!(session.partials.len(), 1);
        assert_eq!(session.first_partial_ms, session.partials.first().map(|p| p.t));
        assert_eq!(session.human_label, None);
        assert!(session_json_path(dir.path(), &session.session_id).exists());
        assert!(session_wav_path(dir.path(), &session.session_id).exists());
    }

    // ---------- UTC 日历换算 ----------

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-07-25 = epoch 20659 天（20454 = 2026-01-01，+205 天）
        assert_eq!(civil_from_days(20659), (2026, 7, 25));
        // 闰日：2000-02-29 = epoch 11016 天
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
    }
}
