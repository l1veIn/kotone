//! 用量与费用（[`kotone_core::usage`]）的桌面端接线：每次花钱的调用记一行，界面和工具读汇总。
//!
//! 记账失败只写日志，不影响正在做的事；只记数字和模型名，不记内容。

use kotone_ai::agent::{now_ms, TokenUsage};
use kotone_core::usage::{self, UsageKind, UsageLedger, UsageRecord};
use serde_json::{json, Value};

/// 启动时打开账本（`<KOTONE_HOME>/usage.jsonl`）；之后各处的在线调用都记到这里。
pub fn start() {
    usage::enable(UsageLedger::default_path());
}

fn append(record: UsageRecord) {
    usage::record(record);
}

/// 一次模型调用（琴音、子会话、压缩、复盘……）。
pub fn record_model(
    kind: UsageKind,
    provider: &str,
    model: &str,
    conversation: &str,
    usage: &TokenUsage,
) {
    let (cost, currency) = match (usage.cost, &usage.currency, usage.cost_usd) {
        (Some(cost), Some(currency), _) => (Some(cost), currency.clone()),
        (_, _, Some(usd)) => (Some(usd), "USD".to_string()),
        _ => (None, String::new()),
    };
    append(UsageRecord {
        at_ms: now_ms(),
        kind: Some(kind),
        provider: provider.to_string(),
        model: model.to_string(),
        conversation: (!conversation.is_empty()).then(|| conversation.to_string()),
        input_tokens: usage.input_tokens,
        cache_hit_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cost,
        currency,
        ..Default::default()
    });
}

/// 同 [`record_model`]，明确记到某段对话的某一轮（交给后台的事：记到派它出去的那一轮，ADR 029 第二期）。
pub fn record_model_for(
    kind: UsageKind,
    provider: &str,
    model: &str,
    owner: &usage::Owner,
    usage: &TokenUsage,
) {
    let (cost, currency) = match (usage.cost, &usage.currency, usage.cost_usd) {
        (Some(cost), Some(currency), _) => (Some(cost), currency.clone()),
        (_, _, Some(usd)) => (Some(usd), "USD".to_string()),
        _ => (None, String::new()),
    };
    append(UsageRecord {
        at_ms: now_ms(),
        kind: Some(kind),
        provider: provider.to_string(),
        model: model.to_string(),
        conversation: Some(owner.conversation.clone()),
        turn: owner.turn,
        input_tokens: usage.input_tokens,
        cache_hit_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cost,
        currency,
        ..Default::default()
    });
}

fn totals_json(t: &usage::UsageTotals) -> Value {
    json!({
        "requests": t.requests,
        "inputTokens": t.input_tokens,
        "cacheHitTokens": t.cache_hit_tokens,
        "outputTokens": t.output_tokens,
        "characters": t.characters,
        "audioMs": t.audio_ms,
        "images": t.images,
        "hitRate": t.hit_rate(),
        "cost": t.cost,
        "unpriced": t.unpriced,
    })
}

/// 今天、本月、本月按用途 / 按模型、最近 30 天（界面「用量」和工具 `usage.summary` 共用）。
pub fn summary() -> Value {
    let now = now_ms();
    let records = usage::global()
        .map(|l| l.read_since(usage::summary_since(now)))
        .unwrap_or_default();
    let s = usage::summarize(&records, now);
    json!({
        "today": totals_json(&s.today),
        "month": totals_json(&s.month),
        "byKind": s.by_kind.iter().map(|(kind, t)| json!({"kind": kind, "label": kind.label(), "totals": totals_json(t)})).collect::<Vec<_>>(),
        "byModel": s.by_model.iter().map(|(model, t)| json!({"model": model, "totals": totals_json(t)})).collect::<Vec<_>>(),
        "days": s.days.iter().map(|d| json!({
            "day": d.day,
            "totals": totals_json(&d.totals),
            "byKind": d.by_kind.iter().map(|(kind, t)| json!({"kind": kind, "label": kind.label(), "totals": totals_json(t)})).collect::<Vec<_>>(),
            "byModel": d.by_model.iter().map(|(model, t)| json!({"model": model, "totals": totals_json(t)})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "priceSource": s.price_source,
        "note": "费用按 Kotone 内置的价格表估算（DeepSeek 分高峰 / 空闲时段，节假日按工作日算，会略偏高），以服务商账单为准；没有价格的模型只记 token。",
        "ledger": usage::global().map(|l| l.path().display().to_string()),
    })
}

fn by_category_json(list: &[(usage::CostCategory, usage::UsageTotals)]) -> Vec<Value> {
    list.iter()
        .map(|(category, t)| json!({"category": category, "label": category.label(), "totals": totals_json(t)}))
        .collect()
}

/// 一段对话的费用（玩家要求，2026-11-20）：整段按类别（对话 / 语音 / 读屏 / 搜索），
/// 以及每一轮的（`turn` = 那一轮玩家那句话的时间，聊天里每轮的页脚用）。`since_ms`：从哪天读起
/// （对话开始的时间），不用读整本账。
pub fn conversation(id: &str, since_ms: u64) -> Value {
    let records = usage::global()
        .map(|l| l.read_since(since_ms))
        .unwrap_or_default();
    let costs = usage::conversation_costs(&records, id);
    json!({
        "conversationId": id,
        "total": by_category_json(&costs.total),
        "turns": costs.turns.iter().map(|(turn, by)| json!({"turn": turn, "byCategory": by_category_json(by)})).collect::<Vec<_>>(),
    })
}

#[tauri::command]
pub fn usage_summary() -> Value {
    summary()
}

#[tauri::command]
pub fn usage_conversation(id: String, since_ms: Option<u64>) -> Value {
    conversation(&id, since_ms.unwrap_or(0))
}
