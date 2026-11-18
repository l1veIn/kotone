//! 用量与费用账本（照 DeepSeek Harness 的用量统计；出处见 THIRD_PARTY_NOTICES.md）。
//!
//! 每次花钱的调用（琴音的模型请求、上下文压缩、对局复盘、在线语音合成 / 识别、在线文字识别）
//! 追加一行到 `<KOTONE_HOME>/usage.jsonl`。只追加、不改写；读的时候按天 / 月 / 对话汇总。
//! 记的只有数字（token、字数、秒数、费用）与模型名、对话 id，不记任何内容。
//!
//! 费用：服务商报了就用服务商的；否则按 [`price_for`] 的价格表估算（DeepSeek 用人民币）。
//! 价格表里没有的模型只记 token，不估费用（界面显示“—”，不编数字）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};

/// 账本文件名（在 KOTONE_HOME 下）。
pub const FILE_NAME: &str = "usage.jsonl";
/// 超过这个大小就把旧的一半挪到 `usage.1.jsonl`（只留一份旧档），避免无限增长。
const ROTATE_BYTES: u64 = 8 * 1024 * 1024;

/// 一次调用是做什么的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageKind {
    /// 琴音（含子会话）的一次模型请求。
    Chat,
    /// 上下文压缩（把较早的对话摘要成一段）。
    Compaction,
    /// 对局复盘。
    Recap,
    /// 在线语音合成（按字数）。
    Tts,
    /// 在线语音识别（按时长）。
    Asr,
    /// 在线文字识别（按次 / token）。
    Ocr,
    /// 联网搜索、读网页（按次，ADR 026）。
    Search,
    /// 其余单次调用（旧的单次陪玩、文字处理的 AI 润色等）。
    Other,
}

impl UsageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "对话",
            Self::Compaction => "压缩",
            Self::Recap => "复盘",
            Self::Tts => "语音合成",
            Self::Asr => "语音识别",
            Self::Ocr => "文字识别",
            Self::Search => "联网搜索",
            Self::Other => "其他",
        }
    }
}

/// 账本里的一行。字段只增不改；没有的就不写。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub at_ms: u64,
    pub kind: Option<UsageKind>,
    /// 连接的服务商 id（`deepseek`、`openai`……）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// 琴音的对话 id（子会话是它自己的 id）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    /// 输入 token（含命中缓存的部分）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 输入里命中服务商前缀缓存的部分（DeepSeek `prompt_cache_hit_tokens`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// 语音合成的字数（按服务商的计法：DeepSeek 汉字算 2 个字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub characters: Option<u64>,
    /// 语音识别的音频时长（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ms: Option<u64>,
    /// 文字识别的图片张数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<u64>,
    /// 费用；None = 不知道（价格表里没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// `CNY` / `USD`。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
}

impl UsageRecord {
    pub fn cache_miss_tokens(&self) -> Option<u64> {
        self.input_tokens
            .map(|input| input.saturating_sub(self.cache_hit_tokens.unwrap_or(0)))
    }
}

/// 每百万 token 的价格。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub currency: &'static str,
    /// 输入命中缓存，每百万 token。
    pub cache_hit_per_m: f64,
    /// 输入未命中缓存，每百万 token。
    pub cache_miss_per_m: f64,
    /// 输出（含推理），每百万 token。
    pub output_per_m: f64,
}

impl Price {
    fn half(self) -> Self {
        Self {
            cache_hit_per_m: self.cache_hit_per_m / 2.0,
            cache_miss_per_m: self.cache_miss_per_m / 2.0,
            output_per_m: self.output_per_m / 2.0,
            ..self
        }
    }
}

/// 价格表的来源与核对日期（界面和文档里显示）。价格会变：改价格时一并改日期。
pub const PRICE_SOURCE: &str =
    "DeepSeek 官方价格页 api-docs.deepseek.com/zh-cn/quick_start/pricing（2026-11-18 核对）";

/// DeepSeek deepseek-flash（V4.1-Flash）高峰价，人民币 / 百万 token。空闲时段半价。
pub const DEEPSEEK_FLASH_PEAK: Price = Price {
    currency: "CNY",
    cache_hit_per_m: 0.04,
    cache_miss_per_m: 2.0,
    output_per_m: 8.0,
};

/// DeepSeek deepseek-v4-pro 高峰价，人民币 / 百万 token。空闲时段半价。
pub const DEEPSEEK_PRO_PEAK: Price = Price {
    currency: "CNY",
    cache_hit_per_m: 0.30,
    cache_miss_per_m: 9.0,
    output_per_m: 27.0,
};

/// DeepSeek 语音合成：人民币 / 万字符（汉字算 2 个字符）。官方标价 3 元，现为限时 5 折 1.5 元。
pub const DEEPSEEK_TTS_PER_10K_CHARS: f64 = 1.5;
/// DeepSeek 语音识别：人民币 / 小时（按秒计，不足 1 秒按 1 秒）。标价 0.78 元，现为限时 5 折。
pub const DEEPSEEK_ASR_PER_HOUR: f64 = 0.39;
/// DeepSeek 文字识别：人民币 / 千页（只算识别成功的页）。
pub const DEEPSEEK_OCR_PER_1K_PAGES: f64 = 1.5;
/// 搜索与抓取接口：每次成功请求 0.005 元（5 元 / 1000 次），https://api-docs.deepseek.com/zh-cn/guides/search
pub const DEEPSEEK_SEARCH_PER_CALL: f64 = 0.005;

/// DeepSeek 的高峰时段（北京时间工作日 9–12 点、14–18 点；周末全天空闲）。
/// 法定节假日也是空闲，但离线判断不了，按工作日算（费用偏高一点，界面说明是估算）。
pub fn deepseek_peak(at_ms: u64) -> bool {
    use chrono::{Timelike, Weekday};
    let Some(utc) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64) else {
        return true;
    };
    let beijing = utc + chrono::Duration::hours(8);
    if matches!(beijing.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let hour = beijing.hour();
    (9..12).contains(&hour) || (14..18).contains(&hour)
}

/// 模型价格（只收核对过官方价格页的）。`provider` 是连接的服务商 id，`at_ms` 决定高峰 / 空闲。
pub fn price_for(provider: &str, model: &str, at_ms: u64) -> Option<Price> {
    let model = model.trim().to_ascii_lowercase();
    if provider != "deepseek" && !model.starts_with("deepseek-") {
        return None;
    }
    let peak = match model.as_str() {
        // 旧名字：deepseek-v4-flash（及 vision-exp）按 Flash 计费；deepseek-chat / deepseek-reasoner
        // 官方说指向 v4-flash 的非思考 / 思考模式（2026-07-24 起停用），还能用的话按 Flash 估。
        "deepseek-flash"
        | "deepseek-v4-flash"
        | "deepseek-v4-flash-vision-exp"
        | "deepseek-chat"
        | "deepseek-reasoner" => DEEPSEEK_FLASH_PEAK,
        "deepseek-v4-pro" | "deepseek-pro" => DEEPSEEK_PRO_PEAK,
        _ => return None,
    };
    Some(if deepseek_peak(at_ms) {
        peak
    } else {
        peak.half()
    })
}

/// 按价格表估算一次模型调用的费用。缺输入或输出 token 时不估。
pub fn estimate(
    price: &Price,
    input: Option<u64>,
    hit: Option<u64>,
    output: Option<u64>,
) -> Option<f64> {
    let input = input?;
    let output = output?;
    let hit = hit.unwrap_or(0).min(input);
    let miss = input - hit;
    Some(
        (hit as f64 * price.cache_hit_per_m
            + miss as f64 * price.cache_miss_per_m
            + output as f64 * price.output_per_m)
            / 1_000_000.0,
    )
}

/// DeepSeek 语音合成的计费字数：汉字（及其他非 ASCII 字符）算 2 个，其余算 1 个。
pub fn deepseek_tts_characters(text: &str) -> u64 {
    text.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

/// DeepSeek 语音识别的计费秒数：按秒向上取整，至少 1 秒。
pub fn deepseek_asr_cost(audio_ms: u64) -> f64 {
    let seconds = audio_ms.div_ceil(1000).max(1);
    seconds as f64 / 3600.0 * DEEPSEEK_ASR_PER_HOUR
}

/// 账本。进程里一个；写入串行。
pub struct UsageLedger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl UsageLedger {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// `<KOTONE_HOME>/usage.jsonl`。
    pub fn default_path() -> PathBuf {
        crate::settings::kotone_dir().join(FILE_NAME)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一行。失败只返回错误（调用方记日志），不影响正在做的事。
    pub fn append(&self, record: &UsageRecord) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap();
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
            let old = self.path.with_extension("1.jsonl");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&self.path, &old);
        }
        let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
        line.push('\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()))
            .map_err(|e| format!("写入用量账本失败：{e}"))
    }

    /// 读出 `since_ms` 之后的记录（含旧档）。坏行跳过。
    pub fn read_since(&self, since_ms: u64) -> Vec<UsageRecord> {
        let _guard = self.lock.lock().unwrap();
        let mut out = Vec::new();
        for path in [self.path.with_extension("1.jsonl"), self.path.clone()] {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<UsageRecord>(line).ok())
                    .filter(|r| r.at_ms >= since_ms),
            );
        }
        out
    }
}

/// 进程里的账本：宿主（桌面端）启动时 [`enable`] 之后才记；没开（测试、命令行）时什么都不写，
/// 免得测试写进用户的 `~/.kotone`。
static GLOBAL: std::sync::OnceLock<UsageLedger> = std::sync::OnceLock::new();

/// 打开进程里的账本（桌面端启动时调用一次）。
pub fn enable(path: impl Into<PathBuf>) {
    let _ = GLOBAL.set(UsageLedger::new(path));
}

/// 进程里的账本（没开时为 None）。
pub fn global() -> Option<&'static UsageLedger> {
    GLOBAL.get()
}

/// 记一行；没开账本时什么都不做，写失败只记日志。
pub fn record(record: UsageRecord) {
    if let Some(ledger) = global() {
        if let Err(e) = ledger.append(&record) {
            crate::log::log(&format!("usage: {e}"));
        }
    }
}

fn now() -> u64 {
    crate::reminders::now_ms()
}

/// 在线语音合成一次（真的发了请求才记；缓存命中不记）。只有 DeepSeek 有价格。
pub fn record_tts(provider: &str, model: &str, text: &str) {
    let deepseek = provider == "deepseek";
    let characters = if deepseek {
        deepseek_tts_characters(text)
    } else {
        text.chars().count() as u64
    };
    record(UsageRecord {
        at_ms: now(),
        kind: Some(UsageKind::Tts),
        provider: provider.to_string(),
        model: model.to_string(),
        characters: Some(characters),
        cost: deepseek.then(|| characters as f64 / 10_000.0 * DEEPSEEK_TTS_PER_10K_CHARS),
        currency: if deepseek {
            "CNY".into()
        } else {
            String::new()
        },
        ..Default::default()
    });
}

/// 在线语音识别一次。
pub fn record_asr(provider: &str, model: &str, audio_ms: u64) {
    let deepseek = provider == "deepseek";
    record(UsageRecord {
        at_ms: now(),
        kind: Some(UsageKind::Asr),
        provider: provider.to_string(),
        model: model.to_string(),
        audio_ms: Some(audio_ms),
        cost: deepseek.then(|| deepseek_asr_cost(audio_ms)),
        currency: if deepseek {
            "CNY".into()
        } else {
            String::new()
        },
        ..Default::default()
    });
}

/// 在线文字识别一次（`pages`：识别成功的页数，一张截图就是 1）。
pub fn record_ocr(provider: &str, model: &str, pages: u64) {
    let deepseek = provider == "deepseek";
    record(UsageRecord {
        at_ms: now(),
        kind: Some(UsageKind::Ocr),
        provider: provider.to_string(),
        model: model.to_string(),
        images: Some(pages),
        cost: deepseek.then(|| pages as f64 / 1_000.0 * DEEPSEEK_OCR_PER_1K_PAGES),
        currency: if deepseek {
            "CNY".into()
        } else {
            String::new()
        },
        ..Default::default()
    });
}

/// 联网搜索或读网页成功一次（DeepSeek 搜索与抓取接口：每 1000 次成功请求 5 元）。
pub fn record_search(provider: &str, what: &str) {
    let deepseek = provider == "deepseek";
    record(UsageRecord {
        at_ms: now(),
        kind: Some(UsageKind::Search),
        provider: provider.to_string(),
        model: what.to_string(),
        cost: deepseek.then_some(DEEPSEEK_SEARCH_PER_CALL),
        currency: if deepseek {
            "CNY".into()
        } else {
            String::new()
        },
        ..Default::default()
    });
}

/// 一组记录的合计。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub requests: u64,
    pub input_tokens: u64,
    pub cache_hit_tokens: u64,
    pub output_tokens: u64,
    pub characters: u64,
    pub audio_ms: u64,
    pub images: u64,
    /// 按币种的费用合计（只加知道费用的记录）。
    pub cost: std::collections::BTreeMap<String, f64>,
    /// 有多少条记录不知道费用（界面提示“部分调用没有价格”）。
    pub unpriced: u64,
}

impl UsageTotals {
    pub fn add(&mut self, r: &UsageRecord) {
        self.requests += 1;
        self.input_tokens += r.input_tokens.unwrap_or(0);
        self.cache_hit_tokens += r.cache_hit_tokens.unwrap_or(0);
        self.output_tokens += r.output_tokens.unwrap_or(0);
        self.characters += r.characters.unwrap_or(0);
        self.audio_ms += r.audio_ms.unwrap_or(0);
        self.images += r.images.unwrap_or(0);
        match r.cost {
            Some(cost) => {
                let currency = if r.currency.is_empty() {
                    "USD".to_string()
                } else {
                    r.currency.clone()
                };
                *self.cost.entry(currency).or_default() += cost;
            }
            None => self.unpriced += 1,
        }
    }

    /// 命中率 = 命中 / 输入（照 DeepSeek Harness：只算报了 token 的请求）。没有输入时为 None。
    pub fn hit_rate(&self) -> Option<f64> {
        (self.input_tokens > 0).then(|| self.cache_hit_tokens as f64 / self.input_tokens as f64)
    }
}

/// 汇总：今天、本月、按用途、按模型、最近几天。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: UsageTotals,
    pub month: UsageTotals,
    /// 本月按用途。
    pub by_kind: Vec<(UsageKind, UsageTotals)>,
    /// 本月按模型（`provider/model`）。
    pub by_model: Vec<(String, UsageTotals)>,
    /// 最近 [`DAYS`] 天，每天一项（旧的在前；没用量的天不列）。
    pub days: Vec<DayUsage>,
    pub price_source: &'static str,
}

/// 统计最近几天（界面按天列出）。
pub const DAYS: i64 = 60;

/// 一天的用量：合计、按用途、按模型。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayUsage {
    /// `YYYY-MM-DD`（本地时间）。
    pub day: String,
    pub totals: UsageTotals,
    pub by_kind: Vec<(UsageKind, UsageTotals)>,
    pub by_model: Vec<(String, UsageTotals)>,
}

fn model_key(r: &UsageRecord) -> String {
    if r.provider.is_empty() {
        r.model.clone()
    } else if r.model.is_empty() {
        r.provider.clone()
    } else {
        format!("{}/{}", r.provider, r.model)
    }
}

fn local_day(ms: u64) -> String {
    Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 本地时间的今天 0 点、本月 1 日 0 点、[`DAYS`] 天前 0 点（毫秒）。
fn boundaries(now_ms: u64) -> (u64, u64, u64) {
    let now = Local
        .timestamp_millis_opt(now_ms as i64)
        .single()
        .unwrap_or_else(Local::now);
    let midnight = |date: chrono::NaiveDate| -> u64 {
        date.and_hms_opt(0, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
            .map(|t| t.timestamp_millis().max(0) as u64)
            .unwrap_or(0)
    };
    let today = now.date_naive();
    let month = today.with_day(1).unwrap_or(today);
    let days30 = today - chrono::Duration::days(DAYS - 1);
    (midnight(today), midnight(month), midnight(days30))
}

pub fn summarize(records: &[UsageRecord], now_ms: u64) -> UsageSummary {
    let (today, month, days30) = boundaries(now_ms);
    let mut summary = UsageSummary {
        price_source: PRICE_SOURCE,
        ..Default::default()
    };
    let mut by_kind: std::collections::BTreeMap<String, (UsageKind, UsageTotals)> =
        Default::default();
    let mut by_model: std::collections::BTreeMap<String, UsageTotals> = Default::default();
    #[allow(clippy::type_complexity)]
    let mut days: std::collections::BTreeMap<
        String,
        (
            UsageTotals,
            std::collections::BTreeMap<String, (UsageKind, UsageTotals)>,
            std::collections::BTreeMap<String, UsageTotals>,
        ),
    > = Default::default();
    for r in records {
        if r.at_ms >= today {
            summary.today.add(r);
        }
        if r.at_ms >= month {
            summary.month.add(r);
            let kind = r.kind.unwrap_or(UsageKind::Other);
            by_kind
                .entry(kind.label().to_string())
                .or_insert_with(|| (kind, UsageTotals::default()))
                .1
                .add(r);
            by_model.entry(model_key(r)).or_default().add(r);
        }
        if r.at_ms >= days30 {
            let kind = r.kind.unwrap_or(UsageKind::Other);
            let day = days.entry(local_day(r.at_ms)).or_default();
            day.0.add(r);
            day.1
                .entry(kind.label().to_string())
                .or_insert_with(|| (kind, UsageTotals::default()))
                .1
                .add(r);
            day.2.entry(model_key(r)).or_default().add(r);
        }
    }
    summary.by_kind = by_kind.into_values().collect();
    summary.by_model = by_model.into_iter().collect();
    summary.days = days
        .into_iter()
        .map(|(day, (totals, kinds, models))| DayUsage {
            day,
            totals,
            by_kind: kinds.into_values().collect(),
            by_model: models.into_iter().collect(),
        })
        .collect();
    summary
}

/// 汇总时从哪天读起（本月 1 日与 [`DAYS`] 天前取更早的）。
pub fn summary_since(now_ms: u64) -> u64 {
    let (_, month, days30) = boundaries(now_ms);
    month.min(days30)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(at_ms: u64, input: u64, hit: u64, output: u64) -> UsageRecord {
        let cost = estimate(&DEEPSEEK_FLASH_PEAK, Some(input), Some(hit), Some(output));
        UsageRecord {
            at_ms,
            kind: Some(UsageKind::Chat),
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
            input_tokens: Some(input),
            cache_hit_tokens: Some(hit),
            output_tokens: Some(output),
            cost,
            currency: "CNY".into(),
            ..Default::default()
        }
    }

    #[test]
    fn deepseek_cost_splits_hit_and_miss() {
        // 100 万命中 + 100 万未命中 + 100 万输出（Flash 高峰价）= 0.04 + 2 + 8
        let cost = estimate(
            &DEEPSEEK_FLASH_PEAK,
            Some(2_000_000),
            Some(1_000_000),
            Some(1_000_000),
        );
        assert!((cost.unwrap() - 10.04).abs() < 1e-9);
        // 命中数大于输入（不应出现）按输入封顶
        let cost = estimate(&DEEPSEEK_FLASH_PEAK, Some(10), Some(20), Some(0)).unwrap();
        assert!((cost - 10.0 * 0.04 / 1e6).abs() < 1e-12);
        assert_eq!(estimate(&DEEPSEEK_FLASH_PEAK, None, None, Some(1)), None);
    }

    /// 北京时间的某个时刻（毫秒）。
    fn beijing(y: i32, m: u32, d: u32, h: u32) -> u64 {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis() as u64
            - 8 * 3_600_000
    }

    #[test]
    fn deepseek_peak_hours_are_beijing_weekday_daytime() {
        // 2026-11-18 是周三
        assert!(deepseek_peak(beijing(2026, 11, 18, 9)));
        assert!(deepseek_peak(beijing(2026, 11, 18, 17)));
        assert!(!deepseek_peak(beijing(2026, 11, 18, 12)));
        assert!(!deepseek_peak(beijing(2026, 11, 18, 18)));
        assert!(!deepseek_peak(beijing(2026, 11, 18, 23)));
        // 周六全天空闲
        assert!(!deepseek_peak(beijing(2026, 11, 21, 10)));
        let peak = price_for("deepseek", "deepseek-flash", beijing(2026, 11, 18, 10)).unwrap();
        let idle = price_for("deepseek", "deepseek-flash", beijing(2026, 11, 18, 20)).unwrap();
        assert_eq!(peak, DEEPSEEK_FLASH_PEAK);
        assert!((idle.cache_miss_per_m - 1.0).abs() < 1e-12);
        assert!((idle.cache_hit_per_m - 0.02).abs() < 1e-12);
    }

    #[test]
    fn only_checked_models_are_priced() {
        assert!(price_for("deepseek", "deepseek-flash", 0).is_some());
        assert!(price_for("deepseek", "DeepSeek-V4-Pro", 0).is_some());
        assert!(price_for("deepseek", "deepseek-chat", 0).is_some());
        assert!(price_for("openai", "gpt-5", 0).is_none());
        assert!(price_for("deepseek", "deepseek-future", 0).is_none());
    }

    #[test]
    fn asr_is_billed_per_started_second() {
        assert!((deepseek_asr_cost(1) - 0.39 / 3600.0).abs() < 1e-12);
        assert!((deepseek_asr_cost(1_001) - 2.0 * 0.39 / 3600.0).abs() < 1e-12);
    }

    #[test]
    fn tts_characters_count_cjk_twice() {
        assert_eq!(deepseek_tts_characters("好的 ok"), 2 + 2 + 1 + 2);
    }

    #[test]
    fn ledger_appends_and_summarizes() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::new(dir.path().join(FILE_NAME));
        let now = Local::now().timestamp_millis() as u64;
        ledger.append(&rec(now - 1_000, 1_000, 900, 100)).unwrap();
        ledger.append(&rec(now, 1_000, 0, 100)).unwrap();
        ledger
            .append(&UsageRecord {
                at_ms: now,
                kind: Some(UsageKind::Tts),
                provider: "openai".into(),
                characters: Some(30),
                ..Default::default()
            })
            .unwrap();
        // 坏行跳过
        std::fs::OpenOptions::new()
            .append(true)
            .open(ledger.path())
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let records = ledger.read_since(summary_since(now));
        assert_eq!(records.len(), 3);
        let summary = summarize(&records, now);
        assert_eq!(summary.today.requests, 3);
        assert_eq!(summary.today.input_tokens, 2_000);
        assert_eq!(summary.today.cache_hit_tokens, 900);
        assert!((summary.today.hit_rate().unwrap() - 0.45).abs() < 1e-9);
        assert_eq!(summary.today.unpriced, 1);
        assert_eq!(summary.today.characters, 30);
        assert!(summary.today.cost["CNY"] > 0.0);
        assert_eq!(summary.by_kind.len(), 2);
        assert_eq!(summary.days.len(), 1);
        assert_eq!(summary.days[0].totals.requests, 3);
        assert_eq!(summary.days[0].by_kind.len(), 2);
        assert!(summary.days[0]
            .by_model
            .iter()
            .any(|(m, t)| m == "deepseek/deepseek-chat" && t.requests == 2));
    }
}
