//! 应用级工具与反向通道端到端：真实 loopback HTTP + 客户端 + 假宿主。
//! 覆盖 `user.request` → 长轮询唤醒 → 独占领取 → `agent.reply` 交付 → 事件与状态，
//! 以及游标错误、期限丢弃交付、退出、运行时启停、长轮询不占执行名额、关闭唤醒。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::{Envelope, ToolCallRequest};
use crate::testing::{validate_schema, FakeAppHost, FakeDesktop, FakeHost, ManualClock};
use crate::{RunningToolService, ServiceDeps, ServiceLimits, ToolClient};

const GAME: u64 = 0x1001;

struct H {
    running: RunningToolService,
    host: Arc<FakeAppHost>,
    clock: Arc<ManualClock>,
    client: ToolClient,
    _dir: tempfile::TempDir,
}

impl H {
    async fn new(limits: ServiceLimits) -> Self {
        let desktop = FakeDesktop::new();
        desktop.add_window(
            GAME,
            4242,
            "League of Legends (TM) Client",
            "League of Legends.exe",
            0,
            0,
            64,
            48,
        );
        desktop.set_foreground(Some(GAME));
        let host = FakeAppHost::new();
        let clock = ManualClock::at(1_700_000_000_000);
        let tool_host = FakeHost::new(Some(5));
        let dir = tempfile::tempdir().unwrap();
        let deps = ServiceDeps {
            instance_id: crate::ids::new_id("inst"),
            app_version: "0.0.0-app".into(),
            observer: desktop.clone(),
            capturer: desktop,
            asr_phase: Arc::new(|| "stopped".into()),
            limits,
            clock: clock.clone(),
            ocr_backends: vec![],
            host: host.clone(),
        };
        let running = RunningToolService::start(deps, tool_host.clone(), dir.path())
            .await
            .unwrap();
        let client = ToolClient::discover(dir.path(), tool_host.as_ref()).unwrap();
        Self {
            running,
            host,
            clock,
            client,
            _dir: dir,
        }
    }

    async fn call(&self, tool: &str, arguments: Value, timeout_ms: Option<u64>) -> Envelope {
        self.client
            .call(&ToolCallRequest {
                request_id: None,
                tool: tool.into(),
                arguments,
                timeout_ms,
            })
            .await
            .unwrap()
    }

    async fn ok(&self, tool: &str, arguments: Value) -> Value {
        let env = self.call(tool, arguments, Some(10_000)).await;
        assert!(env.ok, "{tool} failed: {:?}", env.error);
        let result = env.result.unwrap();
        let caps = self.running.service().capabilities();
        let schema = caps["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == tool)
            .unwrap()["outputSchema"]
            .clone();
        validate_schema(&schema, &result, tool)
            .unwrap_or_else(|e| panic!("{tool} schema: {e}\n{result}"));
        result
    }

    async fn err(&self, tool: &str, arguments: Value) -> crate::protocol::ToolError {
        let env = self.call(tool, arguments, Some(10_000)).await;
        assert!(!env.ok, "{tool} unexpectedly ok: {:?}", env.result);
        env.error.unwrap()
    }
}

/// 从游标读到至少 `n` 个事件（后台交付的完成事件异步到达）。
async fn wait_for_events(h: &H, cursor: &str, n: usize) -> Value {
    for _ in 0..50 {
        let got = h.ok("event.read", json!({"cursor": cursor})).await;
        if got["events"].as_array().unwrap().len() >= n {
            return got;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("等待 {n} 个事件超时");
}

fn types(v: &Value) -> Vec<String> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn user_request_long_poll_claim_and_reply_round_trip() {
    let h = H::new(ServiceLimits::default()).await;
    // 启动时已有 app.started
    let first = h.ok("event.read", json!({})).await;
    assert_eq!(types(&first), vec!["app.started"]);
    let cursor = first["cursor"].as_str().unwrap().to_string();
    // 插件长轮询（HTTP 端点）先挂起，随后桌面端转交一条用户请求
    let client = ToolClient::from_descriptor(h.client.descriptor().clone()).unwrap();
    let query = format!("cursor={cursor}&types=user.request&waitMs=5000");
    let poll = tokio::spawn(async move { client.events(&query, 5000).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let request = h
        .running
        .service()
        .submit_user_request("  我队友在说啥？ ", "assistant-voice")
        .unwrap();
    let env = poll.await.unwrap();
    assert!(env.ok, "{:?}", env.error);
    let got = env.result.unwrap();
    assert_eq!(types(&got), vec!["user.request"]);
    let event = &got["events"][0];
    assert_eq!(event["data"]["text"], "我队友在说啥？");
    assert_eq!(event["data"]["source"], "assistant-voice");
    assert_eq!(event["data"]["status"], "pending");
    assert_eq!(
        event["data"]["foregroundWindow"]["processName"], "League of Legends.exe",
        "附带当时的前台窗口引用"
    );
    let id = request["requestId"].as_str().unwrap().to_string();
    // 独占领取：同一消费者幂等，其他消费者被拒
    let claimed = h
        .ok(
            "agent.request.claim",
            json!({"requestId": id, "consumer": "dsh:session-1"}),
        )
        .await;
    assert_eq!(claimed["newlyClaimed"], true);
    let again = h
        .ok(
            "agent.request.claim",
            json!({"requestId": id, "consumer": "dsh:session-1"}),
        )
        .await;
    assert_eq!(again["newlyClaimed"], false);
    let e = h
        .err(
            "agent.request.claim",
            json!({"requestId": id, "consumer": "claude-code"}),
        )
        .await;
    assert_eq!(e.code, "AGENT_REQUEST_CLAIMED");
    // 回复：经宿主交付，记录回复并发布事件
    let reply = h
        .ok(
            "agent.reply",
            json!({"requestId": id, "text": "队友说他去中路", "outputs": ["danmaku"], "wait": true}),
        )
        .await;
    assert_eq!(reply["status"], "delivered");
    assert_eq!(reply["receipt"]["delivered"], json!(["danmaku"]));
    assert_eq!(reply["request"]["status"], "replied");
    assert_eq!(
        h.host.deliveries(),
        vec![("队友说他去中路".to_string(), vec!["danmaku"])]
    );
    // 主动通知（无 requestId）：默认受理即返回，交付结果见事件
    let notice = h.ok("agent.reply", json!({"text": "打野有闪了"})).await;
    assert!(notice["request"].is_null());
    assert_eq!(notice["status"], "accepted");
    assert!(notice["receipt"].is_null());
    let all = wait_for_events(&h, &cursor, 4).await;
    assert_eq!(
        all["events"][3]["data"]["deliveryId"], notice["deliveryId"],
        "完成事件带受理时返回的 deliveryId"
    );
    assert_eq!(
        types(&all),
        vec![
            "user.request",
            "agent.request.claimed",
            "agent.reply.delivered",
            "agent.reply.delivered"
        ]
    );
    let list = h.ok("agent.requests", json!({"status": "replied"})).await;
    assert_eq!(list["matched"], 1);
    // 未知请求、非法参数
    assert_eq!(
        h.err("agent.reply", json!({"requestId": "areq_x", "text": "x"}))
            .await
            .code,
        "AGENT_REQUEST_NOT_FOUND"
    );
    assert_eq!(
        h.err("agent.reply", json!({"text": "x", "outputs": ["email"]}))
            .await
            .code,
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        h.err("agent.reply", json!({"text": "   "})).await.code,
        "INVALID_ARGUMENTS"
    );
    assert!(h
        .running
        .service()
        .submit_user_request("   ", "tool")
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn cursors_never_skip_silently_and_requests_expire() {
    let limits = ServiceLimits {
        max_events: 3,
        maintenance_interval: Duration::from_millis(20),
        agent_request_ttl_ms: 1_000,
        ..Default::default()
    };
    let h = H::new(limits).await;
    let start = h.ok("event.read", json!({"from": "latest"})).await;
    let old_cursor = start["cursor"].as_str().unwrap().to_string();
    for i in 0..4 {
        h.running
            .service()
            .publish_event("test.tick", json!({"i": i}));
    }
    assert_eq!(
        h.err("event.read", json!({"cursor": old_cursor}))
            .await
            .code,
        "EVENT_CURSOR_EXPIRED"
    );
    let fake_instance = format!("ev1.000000000000.{}", 1);
    assert_eq!(
        h.err("event.read", json!({"cursor": fake_instance}))
            .await
            .code,
        "EVENT_CURSOR_STALE"
    );
    assert_eq!(
        h.err("event.read", json!({"cursor": "nope"})).await.code,
        "EVENT_CURSOR_INVALID"
    );
    assert_eq!(
        h.err("event.read", json!({"cursor": "x", "from": "latest"}))
            .await
            .code,
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        h.err("event.read", json!({"limit": 0})).await.code,
        "INVALID_ARGUMENTS"
    );
    // 未领取的请求过期：维护任务发布 agent.request.expired，领取被拒
    let r = h
        .running
        .service()
        .submit_user_request("还在吗", "assistant-text")
        .unwrap();
    let id = r["requestId"].as_str().unwrap().to_string();
    let head = h.ok("event.read", json!({"from": "latest"})).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    h.clock.advance(1_000);
    let got = h
        .ok(
            "event.read",
            json!({"cursor": head, "types": ["agent.*"], "waitMs": 2000}),
        )
        .await;
    assert_eq!(types(&got), vec!["agent.request.expired"]);
    assert_eq!(
        h.err(
            "agent.request.claim",
            json!({"requestId": id, "consumer": "x"})
        )
        .await
        .code,
        "AGENT_REQUEST_EXPIRED"
    );
    // 过期请求仍可回复，但带警告
    let late = h
        .ok(
            "agent.reply",
            json!({"requestId": id, "text": "抱歉，刚才没看到"}),
        )
        .await;
    assert_eq!(late["warnings"][0]["code"], "REQUEST_EXPIRED");
}

#[tokio::test(flavor = "multi_thread")]
async fn late_delivery_is_cancelled_and_not_recorded() {
    let h = H::new(ServiceLimits::default()).await;
    let r = h
        .running
        .service()
        .submit_user_request("说点什么", "tool")
        .unwrap();
    let id = r["requestId"].as_str().unwrap().to_string();
    h.host.set_delay(Duration::from_secs(3));
    let env = h
        .call(
            "agent.reply",
            json!({"requestId": id, "text": "很长的一段话", "wait": true}),
            Some(200),
        )
        .await;
    assert_eq!(env.error.unwrap().code, "TIMEOUT");
    for _ in 0..100 {
        if h.host.cancelled_deliveries() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        h.host.cancelled_deliveries(),
        1,
        "期限到达时丢弃交付 future（宿主据此停止播报）"
    );
    assert!(h.host.deliveries().is_empty());
    let request = h.ok("agent.requests", json!({})).await["requests"][0].clone();
    assert_eq!(request["status"], "pending", "未交付成功不记录回复");
    let failed = h
        .ok("event.read", json!({"types": ["agent.reply.*"]}))
        .await;
    assert_eq!(types(&failed), vec!["agent.reply.failed"]);
    // 宿主失败 → HOST_ACTION_FAILED
    h.host.set_delay(Duration::ZERO);
    h.host.set_failure(Some("扬声器不可用"));
    assert_eq!(
        h.err("agent.reply", json!({"text": "x", "wait": true}))
            .await
            .code,
        "HOST_ACTION_FAILED"
    );
    assert_eq!(
        h.err("runtime.start", json!({})).await.code,
        "HOST_ACTION_FAILED"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_replies_deliver_in_background_and_report_by_event() {
    let limits = ServiceLimits {
        max_pending_replies: 2,
        ..ServiceLimits::default()
    };
    let h = H::new(limits).await;
    let cursor = h.ok("event.read", json!({"from": "latest"})).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let r = h
        .running
        .service()
        .submit_user_request("委托做完没有", "assistant-voice")
        .unwrap();
    let id = r["requestId"].as_str().unwrap().to_string();
    // 交付很慢：受理立即返回，请求已记为 replied，名额被占用
    h.host.set_delay(Duration::from_millis(400));
    let t = std::time::Instant::now();
    let first = h
        .ok(
            "agent.reply",
            json!({"requestId": id, "text": "还剩两个", "outputs": ["speech"]}),
        )
        .await;
    assert!(t.elapsed() < Duration::from_millis(300), "受理不等交付");
    assert_eq!(first["status"], "accepted");
    assert_eq!(first["request"]["status"], "replied");
    let second = h.ok("agent.reply", json!({"text": "第二条"})).await;
    assert_ne!(first["deliveryId"], second["deliveryId"]);
    // 名额用尽：明确 BUSY，而不是无限排队
    assert_eq!(
        h.err("agent.reply", json!({"text": "第三条"})).await.code,
        "BUSY"
    );
    // user.request + 两条完成事件（假宿主不串行交付，顺序不定，按 deliveryId 匹配）
    let all = wait_for_events(&h, &cursor, 3).await;
    let delivered = all["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["data"]["deliveryId"] == first["deliveryId"])
        .unwrap()
        .clone();
    assert_eq!(delivered["type"], "agent.reply.delivered");
    assert_eq!(delivered["data"]["requestId"], id);
    assert_eq!(delivered["data"]["receipt"]["delivered"], json!(["speech"]));
    // 后台交付失败 → agent.reply.failed（带 deliveryId），不影响受理结果
    let after = all["cursor"].as_str().unwrap().to_string();
    h.host.set_delay(Duration::ZERO);
    h.host.set_failure(Some("扬声器不可用"));
    let failing = h.ok("agent.reply", json!({"text": "x"})).await;
    let got = h
        .ok(
            "event.read",
            json!({"cursor": after, "types": ["agent.reply.*"], "waitMs": 3000}),
        )
        .await;
    assert_eq!(types(&got), vec!["agent.reply.failed"]);
    assert_eq!(
        got["events"][0]["data"]["deliveryId"],
        failing["deliveryId"]
    );
    assert_eq!(got["events"][0]["data"]["code"], "HOST_ACTION_FAILED");
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_cancels_background_replies() {
    let h = H::new(ServiceLimits::default()).await;
    h.host.set_delay(Duration::from_secs(10));
    let accepted = h.ok("agent.reply", json!({"text": "很长的话"})).await;
    assert_eq!(accepted["status"], "accepted");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let host = h.host.clone();
    let mut running = h.running;
    running.shutdown_now();
    for _ in 0..100 {
        if host.cancelled_deliveries() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        host.cancelled_deliveries(),
        1,
        "服务关闭时丢弃后台交付（宿主停止播报）"
    );
    assert!(host.deliveries().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_tools_and_long_poll_do_not_block_execution_slots() {
    let h = H::new(ServiceLimits {
        max_concurrent_calls: 1,
        ..Default::default()
    })
    .await;
    let status = h.ok("app.status", json!({})).await;
    assert_eq!(status["app"]["host"], "fake");
    assert_eq!(status["runtime"]["phase"], "stopped");
    let started = h.ok("runtime.start", json!({})).await;
    assert_eq!(started["phase"], "running");
    assert_eq!(h.ok("runtime.status", json!({})).await["phase"], "running");
    assert_eq!(h.ok("runtime.stop", json!({})).await["phase"], "stopped");
    let submitted = h
        .ok("assistant.submit", json!({"text": "帮我看看队友在说啥"}))
        .await;
    assert_eq!(submitted["route"], "agent");
    assert_eq!(h.host.submitted(), vec!["帮我看看队友在说啥"]);
    // 唯一的执行名额下：长轮询挂起期间其他工具照常执行
    let client = ToolClient::from_descriptor(h.client.descriptor().clone()).unwrap();
    let poll = tokio::spawn(async move {
        client
            .events("from=latest&types=never.happens&waitMs=800", 800)
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let t = std::time::Instant::now();
    h.ok("app.status", json!({})).await;
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "长轮询不占执行名额"
    );
    let timed_out = poll.await.unwrap().result.unwrap();
    assert!(
        timed_out["events"].as_array().unwrap().is_empty(),
        "期限内无匹配事件返回空页而不是错误"
    );
    // 查询串错误
    let bad = h.client.events("waitMs=abc", 0).await.unwrap();
    assert_eq!(bad.error.unwrap().code, "INVALID_ARGUMENTS");
    // 退出：提交闸门后才调用宿主
    let quit = h.ok("app.quit", json!({})).await;
    assert_eq!(quit["status"], "quitting");
    assert_eq!(h.host.quits(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_wakes_long_polls_and_announces_stopping() {
    let h = H::new(ServiceLimits::default()).await;
    let head = h.ok("event.read", json!({"from": "latest"})).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let service = h.running.service().clone();
    let poll = tokio::spawn(async move {
        service
            .read_events(
                crate::service::EventReadArgs {
                    cursor: Some(head),
                    wait_ms: Some(10_000),
                    ..Default::default()
                },
                10_000,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let t = std::time::Instant::now();
    let mut running = h.running;
    running.shutdown_now();
    let result = poll.await.unwrap().unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "关闭唤醒长轮询");
    assert_eq!(types(&result), vec!["app.stopping"]);
    assert_eq!(result["closed"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_desktop_host_app_tools_fail_explicitly() {
    let desktop = FakeDesktop::new();
    let tool_host = FakeHost::new(Some(6));
    let dir = tempfile::tempdir().unwrap();
    let deps = ServiceDeps {
        instance_id: crate::ids::new_id("inst"),
        app_version: "t".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: ManualClock::at(0),
        ocr_backends: vec![],
        host: Arc::new(crate::host::NoAppHost),
    };
    let running = RunningToolService::start(deps, tool_host.clone(), dir.path())
        .await
        .unwrap();
    let client = ToolClient::discover(dir.path(), tool_host.as_ref()).unwrap();
    for tool in [
        "runtime.start",
        "agent.reply",
        "assistant.submit",
        "app.quit",
    ] {
        let args = if tool == "agent.reply" || tool == "assistant.submit" {
            json!({"text": "x"})
        } else {
            json!({})
        };
        let env = client
            .call(&ToolCallRequest {
                request_id: None,
                tool: tool.into(),
                arguments: args,
                timeout_ms: Some(2000),
            })
            .await
            .unwrap();
        assert_eq!(env.error.unwrap().code, "HOST_ACTION_FAILED", "{tool}");
    }
    drop(running);
}
