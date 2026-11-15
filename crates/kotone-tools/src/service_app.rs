//! 应用级工具：生命周期（`app.*`、`runtime.*`）、事件读取（`event.read` 与 `GET /v1/events`）、
//! Agent 请求（`agent.requests`、`agent.request.claim`）、回复（`agent.reply`）与助手提交
//! （`assistant.submit`）。是 `service` 的子模块，可访问服务私有状态。
//!
//! - 宿主动作（启停运行时、交付回复、助手提交）在工作线程上以当前 tokio 运行时等待，
//!   与调用期限、服务关闭竞争；到期即丢弃宿主 future（宿主保证丢弃时停止原生工作）。
//! - 状态变更（领取、记录回复、退出）经 `Prepared::Commit` 在提交闸门之后执行。
//! - 事件读取不占工具执行名额：长轮询只在等待者上限内异步等待。

use super::*;

use crate::agent_requests::{RequestError, RequestStatus};
use crate::events::{CursorError, ReadFrom};
use crate::host::{HostFuture, ReplyOutput};

/// 提交闸门之后执行的短小状态变更。
pub(super) type CommitFn = Box<dyn FnOnce(&ToolService) -> Result<Value, ToolError> + Send>;

const MAX_REQUEST_TEXT: usize = 4000;
const MAX_REPLY_TEXT: usize = 2000;
const MAX_EVENT_LIMIT: usize = 500;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventReadArgs {
    pub cursor: Option<String>,
    pub from: Option<String>,
    pub types: Option<Vec<String>>,
    pub limit: Option<usize>,
    pub wait_ms: Option<u64>,
}

impl EventReadArgs {
    /// 从 `GET /v1/events` 的查询串解析（`types` 逗号分隔）。
    pub fn from_query(query: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v)?;
            match k {
                "cursor" => out.cursor = Some(v),
                "from" => out.from = Some(v),
                "types" => {
                    out.types = Some(
                        v.split(',')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(str::to_string)
                            .collect(),
                    )
                }
                "limit" => out.limit = Some(v.parse().map_err(|_| "limit 必须是整数")?),
                "waitMs" => out.wait_ms = Some(v.parse().map_err(|_| "waitMs 必须是整数")?),
                other => return Err(format!("未知查询参数 {other}")),
            }
        }
        Ok(out)
    }
}

fn percent_decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex =
                    std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| "查询串编码无效")?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| "查询串编码无效")?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "查询串不是 UTF-8".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestsArgs {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimArgs {
    request_id: String,
    consumer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyArgs {
    request_id: Option<String>,
    text: String,
    outputs: Option<Vec<String>>,
    /// true：等交付完成（播完 / 弹幕滚完）再返回；默认受理即返回，结果见事件。
    wait: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitArgs {
    text: String,
}

fn cursor_error(error: CursorError) -> ToolError {
    match error {
        CursorError::Invalid(message) => ToolError::new(ErrorCode::EventCursorInvalid, message),
        CursorError::Stale => ToolError::new(
            ErrorCode::EventCursorStale,
            "游标来自另一个服务实例（桌面端已重启）；不带游标重新读取（from=oldest 取回仍保留的事件）",
        ),
        CursorError::Expired { oldest_seq } => ToolError::new(
            ErrorCode::EventCursorExpired,
            "游标之后的部分事件已因容量被回收；不带游标重新读取，并用 agent.requests 补齐未处理的请求",
        )
        .with_details(json!({"oldestSeq": oldest_seq})),
    }
}

fn request_error(id: &str, error: RequestError) -> ToolError {
    match error {
        RequestError::NotFound => ToolError::new(
            ErrorCode::AgentRequestNotFound,
            format!("未知 Agent 请求 {id}（只在发放它的实例内有效，且有容量上限）"),
        ),
        RequestError::Expired => ToolError::new(
            ErrorCode::AgentRequestExpired,
            format!("请求 {id} 已过期，用户可能不再等待"),
        ),
        RequestError::AlreadyClaimed { by } => ToolError::new(
            ErrorCode::AgentRequestClaimed,
            format!("请求 {id} 已被 {by} 领取"),
        )
        .with_details(json!({"claimedBy": by})),
        RequestError::Closed => closed_error(),
    }
}

fn host_failed(message: String) -> ToolError {
    ToolError::new(ErrorCode::HostActionFailed, message)
}

fn commit(f: impl FnOnce(&ToolService) -> Result<Value, ToolError> + Send + 'static) -> ToolResult {
    Ok(Outcome {
        prepared: Prepared::Commit(Box::new(f)),
        phases: BTreeMap::new(),
    })
}

impl ToolService {
    /// 发布事件（关闭后忽略）。桌面端用它报告运行时相位等变化。
    pub fn publish_event(&self, kind: &str, data: Value) -> Option<u64> {
        self.events.publish(kind, data, self.store.now())
    }

    pub(super) fn expire_agent_requests(&self, now: u64) {
        for request in self.requests.expire(now) {
            self.publish_event(
                "agent.request.expired",
                json!({"requestId": request.request_id}),
            );
        }
    }

    /// 用户经 Kotone 发给 Agent 的请求（助手语音/文字入口转交时调用）：记录请求、附上当时的
    /// 前台窗口引用，并发布 `user.request` 事件。返回请求 JSON。
    pub fn submit_user_request(&self, text: &str, source: &str) -> Result<Value, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("请求内容为空".into());
        }
        if text.chars().count() > MAX_REQUEST_TEXT {
            return Err(format!("请求内容超过 {MAX_REQUEST_TEXT} 字符"));
        }
        if self.store.is_closed() {
            return Err("工具服务已关闭".into());
        }
        let window = match self.observer.foreground_window() {
            Ok(Some(info)) => window_json(&self.windows.id_for(info.identity), &info),
            _ => Value::Null,
        };
        let request = self
            .requests
            .submit(text.to_string(), source, window, self.store.now())
            .map_err(|_| "工具服务已关闭".to_string())?;
        let value = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        self.publish_event("user.request", value.clone());
        Ok(value)
    }

    /// 读取事件；`waitMs>0` 且暂无新事件时异步等待（不占工具执行名额）。
    pub async fn read_events(
        &self,
        args: EventReadArgs,
        max_wait_ms: u64,
    ) -> Result<Value, ToolError> {
        let from = match args.from.as_deref() {
            None | Some("oldest") => ReadFrom::Oldest,
            Some("latest") => ReadFrom::Latest,
            Some(other) => {
                return Err(invalid_args(format!(
                    "from 只接受 oldest / latest（收到 {other}）"
                )))
            }
        };
        if args.cursor.is_some() && args.from.is_some() {
            return Err(invalid_args("cursor 与 from 只能二选一"));
        }
        let limit = args.limit.unwrap_or(100);
        if limit == 0 || limit > MAX_EVENT_LIMIT {
            return Err(invalid_args(format!(
                "limit 必须在 1-{MAX_EVENT_LIMIT} 之间"
            )));
        }
        let wait_ms = args.wait_ms.unwrap_or(0);
        if wait_ms > self.limits.max_timeout_ms {
            return Err(invalid_args(format!(
                "waitMs 不能超过 {}",
                self.limits.max_timeout_ms
            )));
        }
        let types = args.types.unwrap_or_default();
        if types.len() > 32 || types.iter().any(|t| t.is_empty() || t.len() > 64) {
            return Err(invalid_args("types 最多 32 项，每项 1-64 字符"));
        }
        let after = self
            .events
            .resolve(args.cursor.as_deref(), from)
            .map_err(cursor_error)?;
        let first = self.events.read_after(after, limit, &types);
        let empty = first["events"].as_array().is_some_and(|e| e.is_empty());
        if !empty || wait_ms == 0 || self.events.is_closed() {
            return Ok(first);
        }
        // 长轮询：在等待者上限内等待新事件；只要有新事件（即使不匹配过滤）就重读一次
        let Ok(_permit) = self.event_waiters.clone().try_acquire_owned() else {
            return Err(ToolError::new(
                ErrorCode::Busy,
                format!(
                    "事件长轮询等待者已满（上限 {}）",
                    self.limits.max_event_waiters
                ),
            ));
        };
        let wait = Duration::from_millis(wait_ms.min(max_wait_ms));
        let deadline = tokio::time::Instant::now() + wait;
        let mut seen = self.events.head();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() || !self.events.wait_after(seen, remaining).await {
                // 期限内没有匹配事件：返回推进到头部的游标（跳过不匹配的事件）
                return Ok(self.events.read_after(after, limit, &types));
            }
            let next = self.events.read_after(after, limit, &types);
            if next["events"].as_array().is_some_and(|e| !e.is_empty()) || self.events.is_closed() {
                return Ok(next);
            }
            seen = self.events.head();
        }
    }

    pub(super) async fn event_read_envelope(
        self: &Arc<Self>,
        request_id: String,
        arguments: Value,
        timeout_ms: u64,
        started: Instant,
    ) -> Envelope {
        let tool = catalog::TOOL_EVENT_READ.to_string();
        let args: EventReadArgs = match parse_args(arguments) {
            Ok(a) => a,
            Err(e) => return self.envelope_failure(request_id, Some(tool), e, started),
        };
        // 工具调用形式：等待不超过调用期限（留 50ms 写回余量）
        let max_wait = timeout_ms.saturating_sub(50);
        self.event_envelope(request_id, args, max_wait, started)
            .await
    }

    /// `GET /v1/events`：查询串参数，等待上限为服务的最大期限。
    pub async fn events_http(self: &Arc<Self>, query: &str, request_id: String) -> Envelope {
        let started = Instant::now();
        let tool = catalog::TOOL_EVENT_READ.to_string();
        if self.store.is_closed() {
            return self.envelope_failure(request_id, Some(tool), closed_error(), started);
        }
        let args = match EventReadArgs::from_query(query) {
            Ok(a) => a,
            Err(e) => {
                return self.envelope_failure(request_id, Some(tool), invalid_args(e), started)
            }
        };
        let max_wait = self.limits.max_timeout_ms;
        self.event_envelope(request_id, args, max_wait, started)
            .await
    }

    async fn event_envelope(
        self: &Arc<Self>,
        request_id: String,
        args: EventReadArgs,
        max_wait_ms: u64,
        started: Instant,
    ) -> Envelope {
        let tool = catalog::TOOL_EVENT_READ.to_string();
        match self.read_events(args, max_wait_ms).await {
            Ok(result) => Envelope::success(
                request_id,
                Some(self.instance_id.clone()),
                Some(tool),
                result,
                Timing {
                    total_ms: ms_since(started),
                    ..Default::default()
                },
            ),
            Err(e) => self.envelope_failure(request_id, Some(tool), e, started),
        }
    }

    /// 在工作线程上等待宿主 future，与期限、取消（期限计时器 / 服务关闭）竞争。
    fn run_host<T>(
        &self,
        ctx: &CallCtx,
        future: HostFuture<'_, T>,
        tool: &str,
        timeout_ms: u64,
    ) -> Result<T, ToolError> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| ToolError::new(ErrorCode::Internal, "没有可用的异步运行时"))?;
        let notify = Arc::new(tokio::sync::Notify::new());
        let waker = notify.clone();
        let _registration = ctx.cancel.on_cancel(move || waker.notify_one());
        let deadline = tokio::time::Instant::from_std(ctx.deadline);
        let outcome = handle.block_on(async {
            tokio::select! {
                biased;
                _ = notify.notified() => Err(None),
                _ = tokio::time::sleep_until(deadline) => Err(None),
                result = future => result.map_err(Some),
            }
        });
        outcome.map_err(|e| match e {
            Some(message) => host_failed(message),
            None if self.store.is_closed() => closed_error(),
            None => timeout_error(tool, timeout_ms, "running"),
        })
    }

    /// `agent.reply` 默认模式：受理后立即返回，在后台交付（与其他输出互斥排队），
    /// 完成后发布 `agent.reply.delivered` / `agent.reply.failed`（带 deliveryId）。
    /// 服务关闭时丢弃交付 future（宿主随之停止播报、收起弹幕）。
    fn accept_reply(
        &self,
        request_id: Option<String>,
        text: String,
        outputs: Vec<ReplyOutput>,
        warnings: Vec<Value>,
    ) -> ToolResult {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| ToolError::new(ErrorCode::Internal, "没有可用的异步运行时"))?;
        let Ok(slot) = self.reply_slots.clone().try_acquire_owned() else {
            return Err(ToolError::new(
                ErrorCode::Busy,
                format!(
                    "待交付的回复已满（上限 {}）；稍后重试，或用 wait=true 逐条等待",
                    self.limits.max_pending_replies
                ),
            ));
        };
        let host = self.host.clone();
        let events = self.events.clone();
        let clock = self.store.clock();
        let mut closing = self.closing.subscribe();
        let delivery_id = crate::ids::new_id("dlv");
        commit(move |service| {
            let request = match &request_id {
                Some(id) => Some(
                    service
                        .requests
                        .record_reply(id, service.store.now())
                        .map_err(|e| request_error(id, e))?,
                ),
                None => None,
            };
            let task_id = delivery_id.clone();
            let task_request = request_id.clone();
            handle.spawn(async move {
                let _slot = slot;
                let started = Instant::now();
                let result = tokio::select! {
                    biased;
                    _ = closing.wait_for(|closed| *closed) => return,
                    result = host.deliver(text.clone(), outputs) => result,
                };
                let data = match result {
                    Ok(receipt) => (
                        "agent.reply.delivered",
                        json!({"deliveryId": task_id, "requestId": task_request, "text": text, "receipt": receipt, "deliverMs": ms_since(started)}),
                    ),
                    Err(message) => (
                        "agent.reply.failed",
                        json!({"deliveryId": task_id, "requestId": task_request, "code": ErrorCode::HostActionFailed.as_str(), "message": message}),
                    ),
                };
                events.publish(data.0, data.1, clock.now_unix_ms());
            });
            Ok(json!({
                "requestId": request_id,
                "deliveryId": delivery_id,
                "status": "accepted",
                "receipt": null,
                "request": request,
                "warnings": warnings,
            }))
        })
    }

    pub(super) fn execute_app(
        &self,
        tool: &str,
        arguments: Value,
        ctx: &CallCtx,
        timeout_ms: u64,
    ) -> ToolResult {
        match tool {
            catalog::TOOL_APP_STATUS => {
                let _: NoArgs = parse_args(arguments)?;
                let pending = self
                    .requests
                    .list(Some(RequestStatus::Pending), self.store.now())
                    .len();
                Self::value(json!({
                    "instanceId": self.instance_id,
                    "appVersion": self.app_version,
                    "pid": std::process::id(),
                    "startedAtUnixMs": self.started_at_unix_ms,
                    "uptimeMs": self.started.elapsed().as_millis() as u64,
                    "app": self.host.app_info(),
                    "runtime": self.host.runtime_status(),
                    "events": {"headSeq": self.events.head(), "retained": self.events.len()},
                    "agentRequests": {"pending": pending, "retained": self.requests.len()},
                }))
            }
            catalog::TOOL_APP_QUIT => {
                let _: NoArgs = parse_args(arguments)?;
                // 退出在提交闸门之后安排（宿主在响应送出后才真正退出）
                commit(|service| {
                    service.host.request_quit().map_err(host_failed)?;
                    Ok(json!({"status": "quitting", "instanceId": service.instance_id}))
                })
            }
            catalog::TOOL_RUNTIME_STATUS => {
                let _: NoArgs = parse_args(arguments)?;
                Self::value(self.host.runtime_status())
            }
            catalog::TOOL_RUNTIME_START | catalog::TOOL_RUNTIME_STOP => {
                let _: NoArgs = parse_args(arguments)?;
                let started = Instant::now();
                let future = if tool == catalog::TOOL_RUNTIME_START {
                    self.host.runtime_start()
                } else {
                    self.host.runtime_stop()
                };
                let status = self.run_host(ctx, future, tool, timeout_ms)?;
                let mut phases = BTreeMap::new();
                phases.insert("hostMs".into(), ms_since(started));
                Ok(Outcome {
                    prepared: Prepared::Value(status),
                    phases,
                })
            }
            catalog::TOOL_AGENT_REQUESTS => {
                let args: RequestsArgs = parse_args(arguments)?;
                let status = match args.status.as_deref() {
                    None => None,
                    Some(s) => Some(RequestStatus::parse(s).ok_or_else(|| {
                        invalid_args("status 只接受 pending / claimed / replied / expired")
                    })?),
                };
                let limit = args.limit.unwrap_or(50);
                if limit == 0 || limit > 200 {
                    return Err(invalid_args("limit 必须在 1-200 之间"));
                }
                let mut list = self.requests.list(status, self.store.now());
                let total = list.len();
                // 最新的在前
                list.reverse();
                list.truncate(limit);
                Self::value(json!({"requests": list, "matched": total, "truncated": total > limit}))
            }
            catalog::TOOL_AGENT_REQUEST_CLAIM => {
                let args: ClaimArgs = parse_args(arguments)?;
                let consumer = args.consumer.trim().to_string();
                if consumer.is_empty() || consumer.chars().count() > 64 {
                    return Err(invalid_args(
                        "consumer 必须是 1-64 个字符（如 dsh:<会话> / claude-code）",
                    ));
                }
                let id = args.request_id;
                commit(move |service| {
                    let now = service.store.now();
                    let (request, newly) = service
                        .requests
                        .claim(&id, &consumer, now)
                        .map_err(|e| request_error(&id, e))?;
                    if newly {
                        service.publish_event(
                            "agent.request.claimed",
                            json!({"requestId": id, "consumer": consumer}),
                        );
                    }
                    Ok(json!({"request": request, "newlyClaimed": newly}))
                })
            }
            catalog::TOOL_AGENT_REPLY => {
                let args: ReplyArgs = parse_args(arguments)?;
                let text = args.text.trim().to_string();
                if text.is_empty() || text.chars().count() > MAX_REPLY_TEXT {
                    return Err(invalid_args(format!(
                        "text 必须是 1-{MAX_REPLY_TEXT} 个字符"
                    )));
                }
                let mut outputs = Vec::new();
                for o in args.outputs.unwrap_or_default() {
                    let parsed = ReplyOutput::parse(&o)
                        .ok_or_else(|| invalid_args("outputs 只接受 speech / danmaku"))?;
                    if !outputs.contains(&parsed) {
                        outputs.push(parsed);
                    }
                }
                let mut warnings = Vec::new();
                if let Some(id) = &args.request_id {
                    let request = self
                        .requests
                        .get(id, self.store.now())
                        .map_err(|e| request_error(id, e))?;
                    if request.status == RequestStatus::Expired {
                        warnings.push(json!({"code": "REQUEST_EXPIRED", "message": "请求已过期，仍按要求交付"}));
                    }
                }
                if !self.host.can_deliver() {
                    return Err(host_failed(crate::host::NO_HOST.into()));
                }
                if !args.wait.unwrap_or(false) {
                    return self.accept_reply(args.request_id, text, outputs, warnings);
                }
                let started = Instant::now();
                let delivered = self.run_host(
                    ctx,
                    self.host.deliver(text.clone(), outputs),
                    tool,
                    timeout_ms,
                );
                let receipt = match delivered {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        self.publish_event(
                            "agent.reply.failed",
                            json!({"requestId": args.request_id, "code": error.code, "message": error.message}),
                        );
                        return Err(error);
                    }
                };
                let deliver_ms = ms_since(started);
                let request_id = args.request_id;
                let delivery_id = crate::ids::new_id("dlv");
                let mut outcome = commit(move |service| {
                    let request = match &request_id {
                        Some(id) => Some(
                            service
                                .requests
                                .record_reply(id, service.store.now())
                                .map_err(|e| request_error(id, e))?,
                        ),
                        None => None,
                    };
                    service.publish_event(
                        "agent.reply.delivered",
                        json!({"deliveryId": delivery_id, "requestId": request_id, "text": text, "receipt": receipt, "deliverMs": deliver_ms}),
                    );
                    Ok(json!({
                        "requestId": request_id,
                        "deliveryId": delivery_id,
                        "status": "delivered",
                        "receipt": receipt,
                        "request": request,
                        "warnings": warnings,
                    }))
                })?;
                outcome.phases.insert("deliverMs".into(), deliver_ms);
                Ok(outcome)
            }
            catalog::TOOL_ASSISTANT_SUBMIT => {
                let args: SubmitArgs = parse_args(arguments)?;
                let text = args.text.trim().to_string();
                if text.is_empty() || text.chars().count() > MAX_REQUEST_TEXT {
                    return Err(invalid_args(format!(
                        "text 必须是 1-{MAX_REQUEST_TEXT} 个字符"
                    )));
                }
                let result =
                    self.run_host(ctx, self.host.assistant_submit(text), tool, timeout_ms)?;
                Self::value(result)
            }
            other => Err(ToolError::new(
                ErrorCode::ToolNotFound,
                format!("工具 {other} 未实现"),
            )),
        }
    }
}
