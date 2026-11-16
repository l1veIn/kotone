//! 提醒的意图、视图与投递端口。调度已并入自动化调度器（`crate::automation`，ADR 013）：
//! 提醒 = “延迟 + 播报”的一次执行；这里保留 AI 提出的提醒意图、界面沿用的提醒视图、
//! 投递端口，以及只读的旧 `reminders.json` 格式（首次启动时导入）。

use std::{
    future::Future,
    pin::Pin,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::cancel::CancellationToken;

pub const MISSED_GRACE_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryKind {
    Voice,
    Danmaku,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderIntent {
    pub delay_seconds: u64,
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
}

impl ReminderIntent {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=604_800).contains(&self.delay_seconds) {
            return Err("提醒时间必须在 1 秒到 7 天之间".into());
        }
        validate_content(&self.text, &self.delivery)
    }
}

pub(crate) fn validate_content(text: &str, delivery: &[DeliveryKind]) -> Result<(), String> {
    if !(1..=300).contains(&text.trim().chars().count()) {
        return Err("提醒内容必须为 1 到 300 个字符".into());
    }
    if delivery.is_empty()
        || delivery.len() > 2
        || (delivery.len() == 2 && delivery[0] == delivery[1])
    {
        return Err("提醒方式需为语音、弹幕或两者，不能重复".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReminderStatus {
    Pending,
    Delivering,
    Delivered,
    Failed,
    Cancelled,
    Missed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
    pub created_at_ms: u64,
    pub due_at_ms: u64,
    pub status: ReminderStatus,
    pub error: Option<String>,
}

pub type DeliveryFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait ReminderDelivery: Send + Sync {
    /// 投递操作必须尊重取消；返回表示该投递已完成，不能悄悄丢给无人管理的任务。
    fn deliver<'a>(
        &'a self,
        reminder: &'a Reminder,
        cancel: CancellationToken,
    ) -> DeliveryFuture<'a>;
}

/// 旧版（0.1.x）`reminders.json`：只读，首次启动时导入未到期的提醒；文件本身不改。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyReminderStore {
    pub version: u32,
    pub reminders: Vec<Reminder>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
