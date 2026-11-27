//! BetterGI 的运行：推送“BetterGI 的运行”卡片、报事件给自动化（ADR 043 D4、D5、D11；ADR 047 第 3 期）。纯逻辑，测试直接调。
//!
//! 两个来源：
//!
//! - **通知 Webhook**（一条龙、配置组）：BetterGI 0.67 的通知事件（`NotificationEvent.cs`）——`dragon.start` / `dragon.end`、
//!   `group.start` / `group.end`（原话“配置组<组名>启动 / 结束”）、`task.error`、`js.error`、`task.cancel`、`notify.test`……
//!   一条龙里每个配置组也会发一次 `dragon.start`（“配置组任务启动”）和 `group.start` / `group.end`：一条龙开着时这些只算“进行到哪”。
//!   取消时 BetterGI 不发 `group.end`。
//! - **计划**（Pulonia）：BetterGI 不为计划发通知，Kotone 启动的计划由插件看它的运行记录（`history/<计划 ID>/`）。
//!
//! 一次运行一张卡片（key `run:<开始的毫秒>`）：Kotone 启动时就发（“正在启动”），收到开始原地改成“正在跑”，跑完 / 出错原地更新
//! 并给她留一句（`note`），同时报 `genshin.run_finished` / `genshin.run_failed`。15 分钟没收到开始的通知、或者跑了 12 小时还没
//! 结束，就当“不知道结果”收起（多半是 Webhook 没指向 Kotone）。
//!
//! 卡片里的数据（页面 `ui://kotone/genshin/run`，`plugins/genshin/ui/RunCard.svelte` 照它画）就是 [`Run`]（camelCase）。

use std::collections::VecDeque;

use kotone_plugin_api::{Push, PushCard};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::bgi::{clean, first_line, PlanRun};

/// 清单 `[[pushes]]` 的 id。
pub const PUSH_RUNS: &str = "runs";
/// 卡片的页面（清单 `[[pages]]`）。
pub const PAGE_RUN: &str = "ui://kotone/genshin/run";
/// 卡片的名字（和清单里这条推送的 title 一样，测试查）。
pub const TITLE: &str = "BetterGI 的运行";
pub const EVENT_FINISHED: &str = "genshin.run_finished";
pub const EVENT_FAILED: &str = "genshin.run_failed";
/// 事件报在哪个游戏上（游戏配置 id）。
pub const GAME: &str = "genshin";
/// Kotone 启动后多久没收到开始的通知就不等了。
pub const START_WAIT_MS: u64 = 15 * 60 * 1000;
/// 一次运行最长等多久。
pub const RUN_WAIT_MS: u64 = 12 * 60 * 60 * 1000;
/// 记住最近几次。
pub const RECENT_MAX: usize = 20;

/// 跑的是什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Dragon,
    Group,
    Plan,
    /// 出错时不知道是哪次运行（没有开着的运行）。
    Task,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Dragon => "dragon",
            Kind::Group => "group",
            Kind::Plan => "plan",
            Kind::Task => "task",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Dragon => "一条龙",
            Kind::Group => "配置组",
            Kind::Plan => "计划",
            Kind::Task => "任务",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        match text {
            "dragon" => Some(Kind::Dragon),
            "group" => Some(Kind::Group),
            "plan" => Some(Kind::Plan),
            _ => None,
        }
    }
}

/// 现在怎样。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Kotone 刚启动 BetterGI，还没收到开始的通知
    Starting,
    Running,
    Succeeded,
    Partial,
    Cancelled,
    Failed,
    /// 一直没收到结果
    Unknown,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(self, Status::Starting | Status::Running)
    }
}

/// 一次运行（卡片的数据，也存进插件的数据目录）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub key: String,
    pub kind: Kind,
    /// 配置名、组名或计划名；不知道时为空。
    pub name: String,
    pub status: Status,
    /// BetterGI 最后说的话（结束时的原话）。
    #[serde(default)]
    pub message: String,
    /// 进行到哪（一条龙里正在跑的配置组）。
    #[serde(default)]
    pub step: String,
    /// 跑的时候 BetterGI 报的最后一个错。
    #[serde(default)]
    pub error: String,
    /// Kotone 启动的（不是玩家在 BetterGI 里点的）。
    #[serde(default)]
    pub by_kotone: bool,
    pub started_at_ms: u64,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    /// 计划的 ID（计划才有）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan_id: String,
    /// 配置组：还没结束的组（Kotone 一次启动几个组时，全结束才算跑完）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
}

/// BetterGI 发来的一条通知（Webhook 的 JSON：`event`、`result`、`timestamp`、`message`、`data`、`screenshot`、`send_to`，
/// `WebhookNotifier.cs`；截图不要）。
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub event: String,
    pub result: Outcome,
    pub message: String,
}

/// 通知里的结果（`NotificationEventResult`：放进字典序列化，所以是数字 0 / 1 / 2；写成名字的也认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Fail,
    Partial,
}

impl Notice {
    pub fn parse(body: &[u8]) -> Result<Notice, String> {
        let v: Value = serde_json::from_slice(body).map_err(|e| format!("不是 JSON：{e}"))?;
        let event = v
            .get("event")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|e| !e.is_empty() && e.len() <= 64)
            .ok_or("没有 event")?
            .to_ascii_lowercase();
        let result = match v.get("result") {
            Some(Value::Number(n)) if n.as_u64() == Some(1) => Outcome::Fail,
            Some(Value::Number(n)) if n.as_u64() == Some(2) => Outcome::Partial,
            Some(Value::String(s)) if s.eq_ignore_ascii_case("fail") => Outcome::Fail,
            Some(Value::String(s)) if s.eq_ignore_ascii_case("partialsuccess") => Outcome::Partial,
            _ => Outcome::Success,
        };
        let message = clean(
            first_line(v.get("message").and_then(Value::as_str).unwrap_or("")),
            300,
        );
        Ok(Notice {
            event,
            result,
            message,
        })
    }
}

/// 一条要做的：一条推送、（结束时）一件事。
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub push: Push,
    pub event: Option<(String, Map<String, Value>)>,
}

/// “配置组锄地启动” → “锄地”。
fn group_name(message: &str) -> String {
    let rest = message.trim().strip_prefix("配置组").unwrap_or("");
    rest.strip_suffix("启动")
        .or_else(|| rest.strip_suffix("结束"))
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 运行的记录：开着的那次（Webhook 管的、计划各一次）和最近几次。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    /// 一条龙或配置组（Webhook 管）
    #[serde(default)]
    pub open: Option<Run>,
    /// 计划（看运行记录）
    #[serde(default)]
    pub plan: Option<Run>,
    /// 结束了的，新的在前
    #[serde(default)]
    pub recent: VecDeque<Run>,
    /// 最后一次收到 BetterGI 的通知
    #[serde(default)]
    pub last_notice_ms: Option<u64>,
    /// 最后一次收到测试通知（BetterGI 通知设置里的“测试”）
    #[serde(default)]
    pub last_test_ms: Option<u64>,
}

impl Tracker {
    fn new_run(kind: Kind, name: &str, by_kotone: bool, status: Status, now: u64) -> Run {
        Run {
            key: format!("run:{now}"),
            kind,
            name: clean(name, 80),
            status,
            message: String::new(),
            step: String::new(),
            error: String::new(),
            by_kotone,
            started_at_ms: now,
            finished_at_ms: None,
            plan_id: String::new(),
            pending: Vec::new(),
        }
    }

    /// 结束一次：记进最近几次，回卡片和事件。
    fn finish(&mut self, mut run: Run, status: Status, message: &str, now: u64) -> Planned {
        run.status = status;
        if !message.is_empty() {
            run.message = message.to_string();
        }
        run.finished_at_ms = Some(now);
        run.pending.clear();
        let planned = planned(&run, true);
        self.recent.push_front(run);
        self.recent.truncate(RECENT_MAX);
        planned
    }

    /// Kotone 启动了 BetterGI：发一张“正在启动”的卡片。一条龙 / 配置组顶掉还开着的那次（BetterGI 没开着才会走到这里，
    /// 那次一定已经结束了，只是没收到）。
    pub fn launched(
        &mut self,
        kind: Kind,
        name: &str,
        groups: &[String],
        plan_id: &str,
        now: u64,
    ) -> Vec<Planned> {
        let mut out = Vec::new();
        let slot = if kind == Kind::Plan {
            &mut self.plan
        } else {
            &mut self.open
        };
        if let Some(old) = slot.take() {
            out.push(self.finish(old, Status::Unknown, "", now));
        }
        let mut run = Self::new_run(kind, name, true, Status::Starting, now);
        run.plan_id = plan_id.to_string();
        run.pending = groups.to_vec();
        out.push(planned(&run, false));
        if kind == Kind::Plan {
            self.plan = Some(run);
        } else {
            self.open = Some(run);
        }
        out
    }

    /// 收到一条通知。
    pub fn notice(&mut self, notice: &Notice, now: u64) -> Vec<Planned> {
        self.last_notice_ms = Some(now);
        let active = self
            .open
            .as_ref()
            .filter(|r| r.status.active())
            .map(|r| r.kind);
        let message = notice.message.as_str();
        let result = match notice.result {
            Outcome::Success => Status::Succeeded,
            Outcome::Partial => Status::Partial,
            Outcome::Fail => Status::Failed,
        };
        match notice.event.as_str() {
            "dragon.start" => match active {
                Some(Kind::Dragon) => {
                    let run = self.open.as_mut().expect("开着");
                    run.status = Status::Running;
                    vec![planned(run, false)]
                }
                _ => self.replace_open(Kind::Dragon, "", now),
            },
            "group.start" => {
                let name = group_name(message);
                match active {
                    Some(Kind::Dragon) => {
                        let run = self.open.as_mut().expect("开着");
                        run.status = Status::Running;
                        run.step = name;
                        vec![planned(run, false)]
                    }
                    Some(Kind::Group) => {
                        let run = self.open.as_mut().expect("开着");
                        run.status = Status::Running;
                        run.step = name.clone();
                        if run.pending.is_empty() {
                            run.pending.push(name);
                        }
                        vec![planned(run, false)]
                    }
                    _ => self.replace_open(Kind::Group, &name, now),
                }
            }
            "dragon.end" => match active {
                Some(_) => {
                    let run = self.open.take().expect("开着");
                    vec![self.finish(run, result, message, now)]
                }
                None => {
                    let run = Self::new_run(Kind::Dragon, "", false, Status::Running, now);
                    vec![self.finish(run, result, message, now)]
                }
            },
            "group.end" => {
                let name = group_name(message);
                match active {
                    Some(Kind::Dragon) => {
                        let run = self.open.as_mut().expect("开着");
                        run.step = String::new();
                        vec![planned(run, false)]
                    }
                    Some(Kind::Group) => {
                        let run = self.open.as_mut().expect("开着");
                        run.pending.retain(|n| n != &name);
                        if run.pending.is_empty() {
                            let run = self.open.take().expect("开着");
                            vec![self.finish(run, result, message, now)]
                        } else {
                            run.step = String::new();
                            vec![planned(run, false)]
                        }
                    }
                    _ => {
                        let run = Self::new_run(Kind::Group, &name, false, Status::Running, now);
                        vec![self.finish(run, result, message, now)]
                    }
                }
            }
            "task.error" | "js.error" | "album.error" => match active {
                Some(_) => {
                    let run = self.open.as_mut().expect("开着");
                    run.status = Status::Running;
                    run.error = message.to_string();
                    vec![planned(run, false)]
                }
                None => {
                    let run = Self::new_run(Kind::Task, "", false, Status::Running, now);
                    vec![self.finish(run, Status::Failed, message, now)]
                }
            },
            "task.cancel" => match active {
                Some(_) => {
                    let run = self.open.take().expect("开着");
                    vec![self.finish(run, Status::Cancelled, message, now)]
                }
                None => Vec::new(),
            },
            "notify.test" => {
                self.last_test_ms = Some(now);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// 开始的通知来了而没有开着的运行（玩家在 BetterGI 里点的，或者 Kotone 那次早结束了）：另起一次。
    fn replace_open(&mut self, kind: Kind, name: &str, now: u64) -> Vec<Planned> {
        let mut out = Vec::new();
        if let Some(old) = self.open.take() {
            out.push(self.finish(old, Status::Unknown, "", now));
        }
        let mut run = Self::new_run(kind, name, false, Status::Running, now);
        if kind == Kind::Group && !name.is_empty() {
            run.pending = vec![name.to_string()];
        }
        out.push(planned(&run, false));
        self.open = Some(run);
        out
    }

    /// 计划：看它的运行记录。`newest` 是这个计划最新的一条，`running` = BetterGI 现在正在跑这个计划。
    pub fn plan_progress(
        &mut self,
        newest: Option<&PlanRun>,
        running: bool,
        now: u64,
    ) -> Vec<Planned> {
        let Some(run) = self.plan.as_mut() else {
            return Vec::new();
        };
        if let Some(record) = newest {
            // 记录是这次启动以后写的（留 5 秒误差）
            if record.plan_id == run.plan_id && record.recorded_at_ms + 5000 >= run.started_at_ms {
                let status = match record.status.as_str() {
                    "succeeded" => Some(Status::Succeeded),
                    "cancelled" => Some(Status::Cancelled),
                    "failed" | "timed_out" | "interrupted" | "needs_attention" | "expired" => {
                        Some(Status::Failed)
                    }
                    _ => None,
                };
                if let Some(status) = status {
                    let run = self.plan.take().expect("开着");
                    return vec![self.finish(run, status, &record.message, now)];
                }
            }
        }
        if running && run.status == Status::Starting {
            run.status = Status::Running;
            return vec![planned(run, false)];
        }
        Vec::new()
    }

    /// 定时看一眼：太久没开始、跑太久的当“不知道结果”收起。
    pub fn tick(&mut self, now: u64) -> Vec<Planned> {
        let mut out = Vec::new();
        for plan in [false, true] {
            let slot = if plan { &self.plan } else { &self.open };
            let Some(run) = slot.as_ref() else { continue };
            let waited = now.saturating_sub(run.started_at_ms);
            let stale =
                (run.status == Status::Starting && waited > START_WAIT_MS) || waited > RUN_WAIT_MS;
            if !stale {
                continue;
            }
            let note = if run.status == Status::Starting && !plan {
                "15 分钟没收到 BetterGI 的通知：看看它的通知 Webhook 有没有指向 Kotone"
            } else if run.status == Status::Starting {
                "15 分钟没看到这个计划开始"
            } else {
                "12 小时还没结束，不再等了"
            };
            let run = if plan {
                self.plan.take()
            } else {
                self.open.take()
            }
            .expect("刚看过");
            out.push(self.finish(run, Status::Unknown, note, now));
        }
        out
    }

    /// 开着的（先一条龙 / 配置组，再计划）。
    pub fn current(&self) -> Vec<&Run> {
        self.open.iter().chain(self.plan.iter()).collect()
    }
}

/// 给她的一句（跑完、出错时）。
fn note(run: &Run) -> String {
    let name = if run.name.is_empty() {
        String::new()
    } else {
        format!("「{}」", run.name)
    };
    let what = format!("BetterGI 的{}{name}", run.kind.label());
    let message = if run.message.is_empty() {
        String::new()
    } else {
        format!("：{}", run.message)
    };
    match run.status {
        Status::Succeeded => format!("{what}跑完了{message}"),
        Status::Partial => format!("{what}跑完了，有一部分没成{message}"),
        Status::Cancelled => format!("{what}取消了"),
        Status::Failed => {
            let reason = if run.message.is_empty() {
                &run.error
            } else {
                &run.message
            };
            if reason.is_empty() {
                format!("{what}出错了")
            } else {
                format!("{what}出错了：{reason}")
            }
        }
        Status::Unknown => format!("{what}不知道结果{message}"),
        Status::Starting | Status::Running => String::new(),
    }
}

/// 这次运行的卡片；`finished` 时带给她的一句和事件。
fn planned(run: &Run, finished: bool) -> Planned {
    let push = Push {
        trigger: PUSH_RUNS.into(),
        key: run.key.clone(),
        card: Some(PushCard {
            title: TITLE.into(),
            uri: PAGE_RUN.into(),
            params: Value::Null,
            data: serde_json::to_value(run).unwrap_or(Value::Null),
        }),
        note: if finished { note(run) } else { String::new() },
        ..Push::default()
    };
    let event = finished
        .then(|| {
            let mut data = Map::new();
            data.insert("kind".into(), json!(run.kind.as_str()));
            data.insert("name".into(), json!(run.name));
            let reason = if run.message.is_empty() {
                &run.error
            } else {
                &run.message
            };
            match run.status {
                Status::Succeeded | Status::Partial | Status::Cancelled => {
                    let result = match run.status {
                        Status::Succeeded => "success",
                        Status::Partial => "partial",
                        _ => "cancelled",
                    };
                    data.insert("result".into(), json!(result));
                    data.insert("message".into(), json!(run.message));
                    Some((EVENT_FINISHED.to_string(), data))
                }
                Status::Failed => {
                    data.insert("message".into(), json!(reason));
                    Some((EVENT_FAILED.to_string(), data))
                }
                // 不知道结果的不报事件（自动化只认真的跑完 / 出错）
                _ => None,
            }
        })
        .flatten();
    Planned { push, event }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_800_000_000_000;

    fn notice(event: &str, result: Outcome, message: &str) -> Notice {
        Notice {
            event: event.into(),
            result,
            message: message.into(),
        }
    }

    fn data(p: &Planned) -> &Value {
        &p.push.card.as_ref().unwrap().data
    }

    #[test]
    fn notices_parse_like_bettergi_sends_them() {
        // 照 WebhookNotifier：snake_case、结果是数字、截图是 base64（不要）
        let body = "{\"send_to\":\"\",\"event\":\"dragon.end\",\"result\":1,\"timestamp\":\"2026-11-28T09:00:00\",\"screenshot\":\"AAAA\",\"message\":\"\u{200b}一条龙出错\\n   at X\",\"data\":null}";
        let n = Notice::parse(body.as_bytes()).unwrap();
        assert_eq!(
            (n.event.as_str(), n.result, n.message.as_str()),
            ("dragon.end", Outcome::Fail, "一条龙出错")
        );
        let n = Notice::parse(br#"{"event":"Group.End","result":"PartialSuccess"}"#).unwrap();
        assert_eq!(
            (n.event.as_str(), n.result),
            ("group.end", Outcome::Partial)
        );
        assert_eq!(
            Notice::parse(br#"{"event":"x"}"#).unwrap().result,
            Outcome::Success
        );
        assert!(Notice::parse(b"{}").is_err());
        assert!(Notice::parse(b"nope").is_err());
    }

    /// Kotone 启动的一条龙：先发“正在启动”，开始的通知原地改成“正在跑”，中间配置组的通知只算进行到哪，结束原地更新、
    /// 给她一句、报事件。整个过程同一个 key。
    #[test]
    fn a_dragon_started_by_kotone_is_one_card_from_start_to_end() {
        let mut t = Tracker::default();
        let start = t.launched(Kind::Dragon, "默认配置", &[], "", T);
        assert_eq!(start.len(), 1);
        let key = start[0].push.key.clone();
        assert_eq!(data(&start[0])["status"], "starting");
        assert_eq!(data(&start[0])["name"], "默认配置");
        assert_eq!(data(&start[0])["byKotone"], true);
        assert!(start[0].push.note.is_empty() && start[0].event.is_none());
        assert_eq!(start[0].push.card.as_ref().unwrap().title, TITLE);
        assert_eq!(start[0].push.card.as_ref().unwrap().uri, PAGE_RUN);

        let running = t.notice(
            &notice("dragon.start", Outcome::Success, "一条龙启动"),
            T + 10,
        );
        assert_eq!(
            (
                running[0].push.key.as_str(),
                data(&running[0])["status"].as_str()
            ),
            (key.as_str(), Some("running"))
        );
        let step = t.notice(
            &notice("group.start", Outcome::Success, "配置组锄地启动"),
            T + 20,
        );
        assert_eq!(data(&step[0])["step"], "锄地");
        assert!(t.notice(
            &notice("dragon.start", Outcome::Success, "配置组任务启动"),
            T + 25
        )[0]
        .event
        .is_none());
        let err = t.notice(&notice("task.error", Outcome::Fail, "传送失败"), T + 30);
        assert_eq!(data(&err[0])["error"], "传送失败");
        assert!(err[0].event.is_none(), "跑的中间出错不算结束");
        t.notice(
            &notice("group.end", Outcome::Success, "配置组锄地结束"),
            T + 40,
        );

        let end = t.notice(
            &notice("dragon.end", Outcome::Success, "一条龙和配置组任务结束"),
            T + 50,
        );
        assert_eq!(end.len(), 1);
        assert_eq!(end[0].push.key, key);
        assert_eq!(data(&end[0])["status"], "succeeded");
        assert_eq!(data(&end[0])["finishedAtMs"], T + 50);
        assert_eq!(
            end[0].push.note,
            "BetterGI 的一条龙「默认配置」跑完了：一条龙和配置组任务结束"
        );
        let (kind, ev) = end[0].event.clone().unwrap();
        assert_eq!(kind, EVENT_FINISHED);
        assert_eq!(
            Value::Object(ev),
            json!({"kind": "dragon", "name": "默认配置", "result": "success", "message": "一条龙和配置组任务结束"})
        );
        assert!(t.open.is_none());
        assert_eq!(t.recent.len(), 1);
        assert_eq!(t.last_notice_ms, Some(T + 50));
    }

    #[test]
    fn a_failed_dragon_reports_run_failed_with_the_reason() {
        let mut t = Tracker::default();
        t.launched(Kind::Dragon, "", &[], "", T);
        t.notice(
            &notice("dragon.start", Outcome::Success, "一条龙启动"),
            T + 1,
        );
        let end = t.notice(&notice("dragon.end", Outcome::Fail, "进不去游戏"), T + 2);
        assert_eq!(data(&end[0])["status"], "failed");
        assert_eq!(end[0].push.note, "BetterGI 的一条龙出错了：进不去游戏");
        let (kind, ev) = end[0].event.clone().unwrap();
        assert_eq!(kind, EVENT_FAILED);
        assert_eq!(
            Value::Object(ev),
            json!({"kind": "dragon", "name": "", "message": "进不去游戏"})
        );
    }

    /// 一次启动几个配置组：全结束才算跑完；取消时 BetterGI 不发 group.end，task.cancel 收起。
    #[test]
    fn several_groups_finish_together_and_cancel_closes_them() {
        let mut t = Tracker::default();
        t.launched(
            Kind::Group,
            "锄地、钓鱼",
            &["锄地".into(), "钓鱼".into()],
            "",
            T,
        );
        t.notice(
            &notice("group.start", Outcome::Success, "配置组锄地启动"),
            T + 1,
        );
        let half = t.notice(
            &notice("group.end", Outcome::Success, "配置组锄地结束"),
            T + 2,
        );
        assert!(half[0].event.is_none());
        assert_eq!(data(&half[0])["pending"], json!(["钓鱼"]));
        t.notice(
            &notice("group.start", Outcome::Success, "配置组钓鱼启动"),
            T + 3,
        );
        let done = t.notice(
            &notice("group.end", Outcome::Success, "配置组钓鱼结束"),
            T + 4,
        );
        assert_eq!(done[0].event.as_ref().unwrap().0, EVENT_FINISHED);
        assert_eq!(done[0].event.as_ref().unwrap().1["kind"], "group");

        t.launched(Kind::Group, "锄地", &["锄地".into()], "", T + 10);
        t.notice(
            &notice("group.start", Outcome::Success, "配置组锄地启动"),
            T + 11,
        );
        let cancelled = t.notice(&notice("task.cancel", Outcome::Success, ""), T + 12);
        assert_eq!(data(&cancelled[0])["status"], "cancelled");
        assert_eq!(
            cancelled[0].event.as_ref().unwrap().1["result"],
            "cancelled"
        );
        assert_eq!(cancelled[0].push.note, "BetterGI 的配置组「锄地」取消了");
    }

    /// 玩家自己在 BetterGI 里点的：收到开始就另起一张（名字从原话里取），出错时没有开着的就单独报一次。
    #[test]
    fn runs_the_player_started_in_bettergi_are_tracked_too() {
        let mut t = Tracker::default();
        let start = t.notice(
            &notice("group.start", Outcome::Success, "配置组每日启动"),
            T,
        );
        assert_eq!(data(&start[0])["name"], "每日");
        assert_eq!(data(&start[0])["byKotone"], false);
        let end = t.notice(
            &notice("group.end", Outcome::Success, "配置组每日结束"),
            T + 1,
        );
        assert_eq!(end[0].push.key, start[0].push.key);
        assert_eq!(end[0].event.as_ref().unwrap().1["name"], "每日");

        let lone = t.notice(&notice("js.error", Outcome::Fail, "脚本出错"), T + 2);
        assert_eq!(lone[0].event.as_ref().unwrap().0, EVENT_FAILED);
        assert_eq!(lone[0].event.as_ref().unwrap().1["kind"], "task");
        // 没开着的结束：也记一次
        let end = t.notice(&notice("dragon.end", Outcome::Success, ""), T + 3);
        assert_eq!(end[0].event.as_ref().unwrap().1["kind"], "dragon");
        // 测试通知、别的通知：不推
        assert!(t
            .notice(&notice("notify.test", Outcome::Success, ""), T + 4)
            .is_empty());
        assert_eq!(t.last_test_ms, Some(T + 4));
        assert!(t
            .notice(&notice("domain.reward", Outcome::Success, ""), T + 5)
            .is_empty());
        assert_eq!(t.recent.len(), 3);
    }

    /// 计划：看运行记录。这次启动以前的记录不算；正在跑时改成“正在跑”；结束原地更新。
    #[test]
    fn plans_follow_their_history_records() {
        let mut t = Tracker::default();
        let start = t.launched(Kind::Plan, "早上", &[], "pln_a", T);
        let record = |status: &str, recorded_at_ms: u64| PlanRun {
            plan_id: "pln_a".into(),
            plan_name: "早上".into(),
            status: status.into(),
            message: "完成".into(),
            submitted_at: String::new(),
            finished_at: String::new(),
            recorded_at_ms,
        };
        let old = record("succeeded", T - 60_000);
        assert!(
            t.plan_progress(Some(&old), false, T + 1).is_empty(),
            "上一次的记录"
        );
        let running = t.plan_progress(None, true, T + 2);
        assert_eq!(data(&running[0])["status"], "running");
        assert!(t.plan_progress(None, true, T + 3).is_empty(), "没变就不推");
        let failed = record("timed_out", T + 100);
        let end = t.plan_progress(Some(&failed), false, T + 101);
        assert_eq!(end[0].push.key, start[0].push.key);
        assert_eq!(data(&end[0])["status"], "failed");
        assert_eq!(end[0].event.as_ref().unwrap().1["kind"], "plan");
        assert!(t.plan.is_none());
        // 计划和一条龙互不顶掉
        t.launched(Kind::Dragon, "", &[], "", T + 200);
        t.launched(Kind::Plan, "晚上", &[], "pln_b", T + 201);
        assert_eq!(t.current().len(), 2);
    }

    #[test]
    fn stale_runs_are_closed_as_unknown_without_events() {
        let mut t = Tracker::default();
        t.launched(Kind::Dragon, "", &[], "", T);
        assert!(t.tick(T + START_WAIT_MS).is_empty());
        let closed = t.tick(T + START_WAIT_MS + 1);
        assert_eq!(data(&closed[0])["status"], "unknown");
        assert!(closed[0].event.is_none());
        assert!(closed[0].push.note.contains("Webhook"));
        t.notice(&notice("dragon.start", Outcome::Success, ""), T);
        assert!(t.tick(T + RUN_WAIT_MS).is_empty());
        assert_eq!(t.tick(T + RUN_WAIT_MS + 1).len(), 1);
        // 记录存得下、读得回
        let saved = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Tracker>(&saved).unwrap(), t);
        // 最近几次最多 20 次
        for i in 0..30 {
            t.notice(&notice("dragon.end", Outcome::Success, ""), T + 10 + i);
        }
        assert_eq!(t.recent.len(), RECENT_MAX);
    }
}
