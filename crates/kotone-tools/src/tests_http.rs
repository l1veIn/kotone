//! 端到端：真实 loopback HTTP 服务 + 真实客户端 + 假窗口/捕获后端与临时运行目录。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::catalog::IMPLEMENTED_TOOLS;
use crate::discovery::{self, Descriptor};
use crate::protocol::{Envelope, ToolCallRequest};
use crate::testing::{
    decode_png, pattern_pixel, validate_schema, FakeDesktop, FakeHost, ManualClock,
};
use crate::{RunningToolService, ServiceDeps, ServiceLimits, ToolClient};

const GAME: u64 = 0x1001;
const OTHER: u64 = 0x2002;
const GAME_PID: u32 = 4242;

struct Harness {
    running: RunningToolService,
    desktop: Arc<FakeDesktop>,
    clock: Arc<ManualClock>,
    host: Arc<FakeHost>,
    dir: tempfile::TempDir,
    client: ToolClient,
}

impl Harness {
    async fn new(limits: ServiceLimits) -> Self {
        let desktop = FakeDesktop::new();
        // 客户区位于屏幕 (100, 50)，64x48 物理像素
        desktop.add_window(
            GAME,
            GAME_PID,
            "League of Legends (TM) Client",
            "League of Legends.exe",
            100,
            50,
            64,
            48,
        );
        desktop.add_window(OTHER, 777, "Notes", "notepad.exe", 0, 0, 32, 32);
        desktop.add_window(0x3003, 778, "", "untitled.exe", 0, 0, 8, 8);
        desktop.set_foreground(Some(GAME));
        let clock = ManualClock::at(1_700_000_000_000);
        let host = FakeHost::new(Some(7));
        let dir = tempfile::tempdir().unwrap();
        let deps = ServiceDeps {
            instance_id: crate::ids::new_id("inst"),
            app_version: "0.0.0-test".into(),
            observer: desktop.clone(),
            capturer: desktop.clone(),
            asr_phase: Arc::new(|| "stopped".to_string()),
            limits,
            clock: clock.clone(),
        };
        let running = RunningToolService::start(deps, host.clone(), dir.path())
            .await
            .unwrap();
        let client = ToolClient::discover(dir.path(), host.as_ref()).unwrap();
        Self {
            running,
            desktop,
            clock,
            host,
            dir,
            client,
        }
    }

    fn descriptor(&self) -> Descriptor {
        let path = discovery::descriptor_path(self.dir.path(), Some(7));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn base(&self) -> String {
        format!("http://{}", self.running.local_addr())
    }

    async fn call(&self, tool: &str, arguments: Value) -> Envelope {
        self.call_with(tool, arguments, None).await
    }

    async fn call_with(&self, tool: &str, arguments: Value, timeout_ms: Option<u64>) -> Envelope {
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
        let envelope = self.call(tool, arguments).await;
        assert!(envelope.ok, "{tool} failed: {:?}", envelope.error);
        envelope.result.unwrap()
    }

    async fn err(&self, tool: &str, arguments: Value) -> (String, bool) {
        let envelope = self.call(tool, arguments).await;
        assert!(!envelope.ok, "{tool} unexpectedly succeeded");
        let error = envelope.error.unwrap();
        (error.code, error.retryable)
    }

    async fn window_id(&self, title: &str) -> String {
        let list = self
            .ok("window.list", json!({"titleContains": title}))
            .await;
        list["windows"][0]["windowId"].as_str().unwrap().to_string()
    }

    async fn capture_game(&self) -> Value {
        let id = self.window_id("League").await;
        self.ok("screen.capture", json!({"windowId": id})).await
    }

    async fn artifact_png(&self, id: &str) -> (Value, Vec<u8>) {
        let got = self.ok("artifact.get", json!({"artifactId": id})).await;
        let data = got["content"]["data"].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        (got, bytes)
    }

    fn output_schema(&self, tool: &str) -> Value {
        let caps = self.running.service().capabilities();
        caps["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == tool)
            .unwrap()["outputSchema"]
            .clone()
    }
}

fn raw_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_endpoint_requires_the_session_token() {
    let h = Harness::new(ServiceLimits::default()).await;
    let http = raw_client();
    let token = h.descriptor().token;
    for (method, path) in [
        ("GET", "/v1/capabilities"),
        ("GET", "/v1/status"),
        ("POST", "/v1/tools/call"),
        ("GET", "/v1/unknown"),
        ("GET", "/"),
    ] {
        let url = format!("{}{path}", h.base());
        let req = if method == "GET" {
            http.get(&url)
        } else {
            http.post(&url).body(r#"{"tool":"system.status"}"#)
        };
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 401, "{method} {path} without token");
        let env: Envelope = resp.json().await.unwrap();
        assert_eq!(env.error.unwrap().code, "UNAUTHORIZED");

        let req = if method == "GET" {
            http.get(&url)
        } else {
            http.post(&url).body(r#"{"tool":"system.status"}"#)
        };
        let resp = req.bearer_auth("0".repeat(64)).send().await.unwrap();
        assert_eq!(resp.status(), 401, "{method} {path} with wrong token");
    }
    // 正确凭证：未知路径才暴露 404，已知路径错误方法 405
    let resp = http
        .get(format!("{}/v1/unknown", h.base()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let resp = http
        .post(format!("{}/v1/status", h.base()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    // 浏览器 Origin 与非 loopback Host 被拒绝（即使凭证正确）
    let resp = http
        .get(format!("{}/v1/status", h.base()))
        .bearer_auth(&token)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let resp = http
        .get(format!("{}/v1/status", h.base()))
        .bearer_auth(&token)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_envelope_and_body_limits_are_enforced() {
    let h = Harness::new(ServiceLimits::default()).await;
    let http = raw_client();
    let token = h.descriptor().token;
    let url = format!("{}/v1/tools/call", h.base());
    let big = format!(
        r#"{{"tool":"system.status","arguments":{{"pad":"{}"}}}}"#,
        "x".repeat(70 * 1024)
    );
    let resp = http
        .post(&url)
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let resp = http
        .post(&url)
        .bearer_auth(&token)
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    // 信封不接受额外字段（如文件路径）
    let resp = http
        .post(&url)
        .bearer_auth(&token)
        .body(r#"{"requestId":"r-1","tool":"artifact.get","path":"C:/Windows/win.ini"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let env: Envelope = resp.json().await.unwrap();
    assert_eq!(env.request_id, "r-1");
    assert_eq!(env.error.unwrap().code, "INVALID_REQUEST");
    let resp = http
        .post(&url)
        .bearer_auth(&token)
        .header("content-type", "text/plain")
        .body(r#"{"tool":"system.status"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
    // 工具参数也不接受路径字段
    let (code, _) = h
        .err(
            "artifact.get",
            json!({"artifactId": "x", "path": "C:/x.png"}),
        )
        .await;
    assert_eq!(code, "INVALID_ARGUMENTS");
    // timeoutMs 范围
    for bad in [0, 30_001] {
        let env = h.call_with("system.status", json!({}), Some(bad)).await;
        assert_eq!(env.error.unwrap().code, "INVALID_ARGUMENTS");
    }
    // requestId 回显与校验
    let env = h
        .client
        .call(&ToolCallRequest {
            request_id: Some("agent-job.1:step".into()),
            tool: "system.status".into(),
            arguments: Value::Null,
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(env.request_id, "agent-job.1:step");
    let env = h
        .client
        .call(&ToolCallRequest {
            request_id: Some("bad id with spaces".into()),
            tool: "system.status".into(),
            arguments: Value::Null,
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(env.error.unwrap().code, "INVALID_REQUEST");
}

#[tokio::test(flavor = "multi_thread")]
async fn capabilities_list_exactly_the_implemented_tools() {
    let h = Harness::new(ServiceLimits::default()).await;
    let env = h.client.capabilities().await.unwrap();
    assert!(env.ok);
    assert_eq!(env.instance_id.as_deref(), Some(h.running.instance_id()));
    let caps = env.result.unwrap();
    validate_schema(
        &crate::catalog::capabilities_schema(),
        &caps,
        "capabilities",
    )
    .unwrap();
    let names: Vec<&str> = caps["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, IMPLEMENTED_TOOLS);
    for tool in caps["tools"].as_array().unwrap() {
        assert_eq!(tool["status"], "implemented");
        assert_eq!(tool["dataEgress"], "none");
        assert_eq!(tool["ready"], true, "{}", tool["name"]);
    }
    assert_eq!(caps["capture"]["method"], "fake-test-capture");
    // 路线图工具不冒充已实现
    for planned in [
        "ocr.recognize",
        "ocr.backends",
        "image.preprocess",
        "chat.track.read",
        "hotkey.register",
        "workflow.run",
        "llm.complete",
        "tts.speak",
        "overlay.push",
        "chat.send",
        "asr.start",
        "runtime.start",
    ] {
        assert!(!names.contains(&planned));
        let (code, retryable) = h.err(planned, json!({})).await;
        assert_eq!(code, "TOOL_NOT_FOUND", "{planned}");
        assert!(!retryable);
    }
    // 工具形式的 capabilities 与 GET 端点一致，并校验协议版本
    let via_tool = h
        .ok("system.capabilities", json!({"protocolVersion": 1}))
        .await;
    assert_eq!(via_tool["tools"], caps["tools"]);
    let (code, _) = h
        .err("system.capabilities", json!({"protocolVersion": 2}))
        .await;
    assert_eq!(code, "UNSUPPORTED_PROTOCOL_VERSION");
    // 后端不可用时如实标注未就绪
    h.desktop.set_unavailable(Some("no display"));
    let caps = h.running.service().capabilities();
    let capture = caps["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "screen.capture")
        .unwrap()
        .clone();
    assert_eq!(capture["ready"], false);
    assert_eq!(capture["notReadyReason"], "no display");
    let id = h.window_id("League").await;
    let (code, _) = h.err("screen.capture", json!({"windowId": id})).await;
    assert_eq!(code, "PLATFORM_UNSUPPORTED");
}

#[tokio::test(flavor = "multi_thread")]
async fn full_flow_matches_advertised_schemas_with_asr_stopped() {
    let h = Harness::new(ServiceLimits::default()).await;
    let status = h.client.status().await.unwrap();
    let status = status.result.unwrap();
    validate_schema(&crate::catalog::status_schema(), &status, "status").unwrap();
    assert_eq!(status["asr"]["runtimePhase"], "stopped");
    assert_eq!(status["asr"]["requiredForTools"], false);
    let token = h.descriptor().token;
    let raw = serde_json::to_string(&status).unwrap();
    assert!(!raw.contains(&token), "status 不得包含凭证");
    let caps = serde_json::to_string(&h.running.service().capabilities()).unwrap();
    assert!(!caps.contains(&token), "capabilities 不得包含凭证");
    assert!(!format!("{:?}", h.client).contains(&token));

    let mut checks: Vec<(&str, Value)> = Vec::new();
    checks.push(("system.status", h.ok("system.status", json!({})).await));
    let list = h.ok("window.list", json!({})).await;
    assert_eq!(list["matched"], 2, "默认不含无标题窗口");
    assert_eq!(
        h.ok("window.list", json!({"includeUntitled": true})).await["matched"],
        3
    );
    let filtered = h
        .ok(
            "window.list",
            json!({"processName": "LEAGUE OF LEGENDS.EXE"}),
        )
        .await;
    assert_eq!(filtered["matched"], 1);
    let limited = h.ok("window.list", json!({"limit": 1})).await;
    assert_eq!(limited["truncated"], true);
    checks.push(("window.list", list.clone()));
    let active = h.ok("window.active", json!({})).await;
    assert_eq!(active["window"]["title"], "League of Legends (TM) Client");
    assert_eq!(
        active["window"]["clientRect"],
        json!({"x":100,"y":50,"width":64,"height":48})
    );
    assert_eq!(active["window"]["scale"], 1.5);
    // 同一窗口两次观察得到同一引用
    assert_eq!(
        active["window"]["windowId"],
        filtered["windows"][0]["windowId"]
    );
    checks.push(("window.active", active.clone()));
    let capture = h.capture_game().await;
    checks.push(("screen.capture", capture.clone()));
    let crop = h
        .ok(
            "image.crop",
            json!({"artifactId": capture["artifactId"], "rect": {"x": 1, "y": 2, "width": 3, "height": 4}}),
        )
        .await;
    checks.push(("image.crop", crop.clone()));
    let (got, _) = h.artifact_png(crop["artifactId"].as_str().unwrap()).await;
    checks.push(("artifact.get", got));
    let meta_only = h
        .ok(
            "artifact.get",
            json!({"artifactId": crop["artifactId"], "includeContent": false}),
        )
        .await;
    assert!(meta_only.get("content").is_none());
    checks.push(("artifact.get", meta_only));
    let released = h
        .ok(
            "artifact.release",
            json!({"artifactId": crop["artifactId"]}),
        )
        .await;
    checks.push(("artifact.release", released));
    checks.push((
        "system.capabilities",
        h.ok("system.capabilities", json!({})).await,
    ));
    for (tool, result) in checks {
        validate_schema(&h.output_schema(tool), &result, tool)
            .unwrap_or_else(|e| panic!("{tool} result violates its outputSchema: {e}"));
    }
    h.desktop.set_foreground(None);
    let none = h.ok("window.active", json!({})).await;
    assert_eq!(
        none,
        json!({"window": null, "reason": "NO_FOREGROUND_WINDOW"})
    );
    validate_schema(&h.output_schema("window.active"), &none, "window.active").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_and_crop_lineage_and_exact_pixels() {
    let h = Harness::new(ServiceLimits::default()).await;
    let capture = h.capture_game().await;
    let root_id = capture["artifactId"].as_str().unwrap().to_string();
    assert_eq!(capture["kind"], "capture");
    assert_eq!(
        (capture["width"].as_u64(), capture["height"].as_u64()),
        (Some(64), Some(48))
    );
    assert_eq!(capture["parentArtifactId"], Value::Null);
    assert_eq!(capture["rootArtifactId"], root_id.as_str());
    assert_eq!(capture["coordinateSpace"], "artifact-physical-pixels");
    assert_eq!(capture["source"]["captureMethod"], "fake-test-capture");
    assert_eq!(
        capture["source"]["clientRectScreen"],
        json!({"x":100,"y":50,"width":64,"height":48})
    );
    assert_eq!(capture["source"]["dpi"], 144);
    assert_eq!(capture["source"]["processName"], "League of Legends.exe");
    assert_eq!(capture["source"]["warnings"][0]["code"], "FAKE");
    assert_eq!(
        capture["screenRect"],
        json!({"x":100,"y":50,"width":64,"height":48})
    );
    assert_eq!(
        capture["expiresAtUnixMs"].as_u64().unwrap() - capture["createdAtUnixMs"].as_u64().unwrap(),
        300_000
    );
    let (_, root_png) = h.artifact_png(&root_id).await;
    let root_sha = crate::image::sha256_hex(&root_png);
    assert_eq!(capture["sha256"], root_sha.as_str());
    assert_eq!(capture["byteLength"].as_u64(), Some(root_png.len() as u64));
    let (w, hgt, px) = decode_png(&root_png);
    assert_eq!((w, hgt), (64, 48));
    assert_eq!(&px[..4], &pattern_pixel(0, 0));

    let a = h
        .ok(
            "image.crop",
            json!({"artifactId": root_id, "rect": {"x": 10, "y": 5, "width": 30, "height": 20}, "ttlMs": 60_000}),
        )
        .await;
    assert_eq!(a["kind"], "crop");
    assert_eq!(a["parentArtifactId"], root_id.as_str());
    assert_eq!(a["rootArtifactId"], root_id.as_str());
    assert_eq!(
        a["rootTransform"],
        json!({"offsetX":10,"offsetY":5,"scaleX":1.0,"scaleY":1.0})
    );
    assert_eq!(a["ttlMs"], 60_000);
    let a_id = a["artifactId"].as_str().unwrap().to_string();
    let b = h
        .ok(
            "image.crop",
            json!({"artifactId": a_id, "rect": {"x": 3, "y": 4, "width": 5, "height": 6}}),
        )
        .await;
    assert_eq!(b["parentArtifactId"], a_id.as_str());
    assert_eq!(b["rootArtifactId"], root_id.as_str());
    assert_eq!(b["rootTransform"]["offsetX"], 13);
    assert_eq!(b["rootTransform"]["offsetY"], 9);
    assert_eq!(b["rootRect"], json!({"x":13,"y":9,"width":5,"height":6}));
    assert_eq!(
        b["screenRect"],
        json!({"x":113,"y":59,"width":5,"height":6})
    );
    assert_eq!(b["source"]["frameId"], capture["source"]["frameId"]);
    let (_, b_png) = h.artifact_png(b["artifactId"].as_str().unwrap()).await;
    let (bw, bh, bpx) = decode_png(&b_png);
    assert_eq!((bw, bh), (5, 6));
    for y in 0..6u32 {
        for x in 0..5u32 {
            let i = ((y * 5 + x) * 4) as usize;
            assert_eq!(
                &bpx[i..i + 4],
                &pattern_pixel(x + 13, y + 9),
                "pixel ({x},{y})"
            );
        }
    }
    // 原图保持不变（哈希不变），再次捕获得到新的 frameId 与新产物
    let (_, root_again) = h.artifact_png(&root_id).await;
    assert_eq!(crate::image::sha256_hex(&root_again), root_sha);
    let second = h.capture_game().await;
    assert_ne!(second["artifactId"], capture["artifactId"]);
    assert_ne!(second["source"]["frameId"], capture["source"]["frameId"]);
    // 父产物释放后，子产物仍可读取且谱系不变
    h.ok("artifact.release", json!({"artifactId": a_id})).await;
    let (b_meta, _) = h.artifact_png(b["artifactId"].as_str().unwrap()).await;
    assert_eq!(b_meta["parentArtifactId"], a_id.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn crop_rejects_invalid_rectangles_without_clamping() {
    let h = Harness::new(ServiceLimits::default()).await;
    let capture = h.capture_game().await;
    let id = capture["artifactId"].clone();
    let cases = [
        (
            json!({"x": -1, "y": 0, "width": 1, "height": 1}),
            Some("negative-origin"),
        ),
        (
            json!({"x": 0, "y": 0, "width": 0, "height": 1}),
            Some("empty-rect"),
        ),
        (
            json!({"x": 60, "y": 0, "width": 5, "height": 1}),
            Some("out-of-bounds"),
        ),
        (
            json!({"x": 0, "y": 48, "width": 1, "height": 1}),
            Some("out-of-bounds"),
        ),
        (
            json!({"x": i64::MAX, "y": 0, "width": 1, "height": 1}),
            Some("overflow"),
        ),
        (json!({"x": 0.5, "y": 0, "width": 1, "height": 1}), None),
        (json!({"x": 1e30, "y": 0, "width": 1, "height": 1}), None),
        (json!({"x": 0, "y": 0, "width": 1}), None),
        (
            json!({"x": 0, "y": 0, "width": 1, "height": 1, "scale": 2}),
            None,
        ),
    ];
    for (rect, reason) in cases {
        let env = h
            .call("image.crop", json!({"artifactId": id, "rect": rect}))
            .await;
        let error = env.error.unwrap_or_else(|| panic!("accepted {rect}"));
        assert_eq!(error.code, "INVALID_ARGUMENTS", "{rect}");
        if let Some(reason) = reason {
            assert_eq!(error.details.unwrap()["reason"], reason, "{rect}");
        }
    }
    let (code, _) = h
        .err(
            "image.crop",
            json!({"artifactId": "art_unknown", "rect": {"x":0,"y":0,"width":1,"height":1}}),
        )
        .await;
    assert_eq!(code, "ARTIFACT_NOT_FOUND");
    for ttl in [0, 999, 1_800_001] {
        let (code, _) = h
            .err(
                "image.crop",
                json!({"artifactId": id, "rect": {"x":0,"y":0,"width":1,"height":1}, "ttlMs": ttl}),
            )
            .await;
        assert_eq!(code, "INVALID_ARGUMENTS");
    }
    assert_eq!(h.running.service().status()["artifacts"]["count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn window_failures_have_distinct_errors() {
    let h = Harness::new(ServiceLimits::default()).await;
    let game = h.window_id("League").await;
    let notes = h.window_id("Notes").await;
    let (code, retryable) = h.err("screen.capture", json!({"windowId": notes})).await;
    assert_eq!(code, "WINDOW_NOT_FOREGROUND");
    assert!(retryable);
    h.desktop.set_minimized(GAME, true);
    let (code, _) = h.err("screen.capture", json!({"windowId": game})).await;
    assert_eq!(code, "WINDOW_MINIMIZED");
    h.desktop.set_minimized(GAME, false);
    h.desktop.reuse_handle(GAME, 5555);
    let (code, retryable) = h.err("screen.capture", json!({"windowId": game})).await;
    assert_eq!(code, "WINDOW_IDENTITY_CHANGED");
    assert!(!retryable);
    h.desktop.close(GAME);
    let (code, _) = h.err("screen.capture", json!({"windowId": game})).await;
    assert_eq!(code, "WINDOW_GONE");
    let (code, _) = h
        .err(
            "screen.capture",
            json!({"windowId": "win_from_another_instance"}),
        )
        .await;
    assert_eq!(code, "WINDOW_NOT_FOUND");
    let (code, _) = h.err("screen.capture", json!({})).await;
    assert_eq!(code, "INVALID_ARGUMENTS");
    let (code, _) = h.err("window.list", json!({"limit": 0})).await;
    assert_eq!(code, "INVALID_ARGUMENTS");
    assert_eq!(h.desktop.capture_count(), 0, "失败路径不应产出像素");
    assert_eq!(h.running.service().status()["artifacts"]["count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_pixel_limit_is_enforced() {
    let limits = ServiceLimits {
        max_image_pixels: 64 * 48 - 1,
        ..Default::default()
    };
    let h = Harness::new(limits).await;
    let id = h.window_id("League").await;
    let (code, _) = h.err("screen.capture", json!({"windowId": id})).await;
    assert_eq!(code, "IMAGE_TOO_LARGE");
}

#[tokio::test(flavor = "multi_thread")]
async fn ttl_release_and_unknown_artifacts() {
    let h = Harness::new(ServiceLimits::default()).await;
    let id = h.window_id("League").await;
    let capture = h
        .ok("screen.capture", json!({"windowId": id, "ttlMs": 1_000}))
        .await;
    let art = capture["artifactId"].clone();
    h.clock.advance(999);
    assert!(h.call("artifact.get", json!({"artifactId": art})).await.ok);
    h.clock.advance(1);
    let env = h.call("artifact.get", json!({"artifactId": art})).await;
    let error = env.error.unwrap();
    assert_eq!(error.code, "ARTIFACT_EXPIRED");
    assert_eq!(error.details.unwrap()["reason"], "ttl");
    let (code, _) = h
        .err(
            "image.crop",
            json!({"artifactId": art, "rect": {"x":0,"y":0,"width":1,"height":1}}),
        )
        .await;
    assert_eq!(code, "ARTIFACT_EXPIRED");
    let rel = h.ok("artifact.release", json!({"artifactId": art})).await;
    assert_eq!(rel["status"], "already-expired");

    let fresh = h.capture_game().await;
    let fresh_id = fresh["artifactId"].clone();
    assert_eq!(
        h.ok("artifact.release", json!({"artifactId": fresh_id}))
            .await["status"],
        "released"
    );
    assert_eq!(
        h.ok("artifact.release", json!({"artifactId": fresh_id}))
            .await["status"],
        "already-released"
    );
    let (code, _) = h.err("artifact.get", json!({"artifactId": fresh_id})).await;
    assert_eq!(code, "ARTIFACT_RELEASED");
    let (code, _) = h
        .err(
            "artifact.release",
            json!({"artifactId": "art_never_issued"}),
        )
        .await;
    assert_eq!(code, "ARTIFACT_NOT_FOUND");
    let (code, _) = h
        .err(
            "artifact.get",
            json!({"artifactId": "C:\\Users\\someone\\secret.png"}),
        )
        .await;
    assert_eq!(
        code, "ARTIFACT_NOT_FOUND",
        "路径字符串只是未知 ID，不会被读取"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capacity_evicts_oldest_artifacts() {
    let limits = ServiceLimits {
        max_artifacts: 2,
        ..Default::default()
    };
    let h = Harness::new(limits).await;
    let first = h.capture_game().await;
    let second = h.capture_game().await;
    assert_eq!(second["evictedArtifactIds"], json!([]));
    let third = h.capture_game().await;
    assert_eq!(third["evictedArtifactIds"], json!([first["artifactId"]]));
    let env = h
        .call("artifact.get", json!({"artifactId": first["artifactId"]}))
        .await;
    let error = env.error.unwrap();
    assert_eq!(error.code, "ARTIFACT_EXPIRED");
    assert_eq!(error.details.unwrap()["reason"], "evicted");
    assert!(
        h.call("artifact.get", json!({"artifactId": second["artifactId"]}))
            .await
            .ok
    );
    let status = h.ok("system.status", json!({})).await;
    assert_eq!(status["artifacts"]["count"], 2);
    assert_eq!(status["artifacts"]["maxArtifacts"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeouts_discard_late_results_and_concurrency_is_bounded() {
    let limits = ServiceLimits {
        max_concurrent_calls: 1,
        ..Default::default()
    };
    let h = Arc::new(Harness::new(limits).await);
    let id = h.window_id("League").await;
    h.desktop.set_capture_delay(Duration::from_millis(600));
    let env = h
        .call_with("screen.capture", json!({"windowId": id}), Some(100))
        .await;
    let error = env.error.unwrap();
    assert_eq!(error.code, "TIMEOUT");
    assert!(error.retryable);
    assert!(env.timing.total_ms < 600, "期限到即返回，不等待后台完成");
    // 超时调用仍占着唯一的执行名额 → 新调用在期限内拿不到名额
    let busy = h.call_with("system.status", json!({}), Some(100)).await;
    assert_eq!(busy.error.unwrap().code, "BUSY");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(h.desktop.capture_count(), 1, "后台捕获确实执行完毕");
    let status = h.ok("system.status", json!({})).await;
    assert_eq!(status["artifacts"]["count"], 0, "迟到结果不得进入产物存储");
    assert_eq!(status["activeCalls"], 1, "status 自身占用一个名额");
    // 名额释放后正常调用可以完成
    h.desktop.set_capture_delay(Duration::ZERO);
    assert!(h.call("screen.capture", json!({"windowId": id})).await.ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_rejects_stale_insecure_and_foreign_descriptors() {
    let h = Harness::new(ServiceLimits::default()).await;
    let path = discovery::descriptor_path(h.dir.path(), Some(7));
    let original = h.descriptor();
    let write = |d: &Descriptor| std::fs::write(&path, serde_json::to_vec(d).unwrap()).unwrap();
    let discover_code = || {
        ToolClient::discover(h.dir.path(), h.host.as_ref())
            .unwrap_err()
            .code
    };

    // 另一个会话看不到本会话的描述文件
    let other_session = FakeHost::new(Some(8));
    assert_eq!(
        ToolClient::discover(h.dir.path(), other_session.as_ref())
            .unwrap_err()
            .code,
        "SERVICE_NOT_RUNNING"
    );
    // 会话字段不符
    let mut d = original.clone();
    d.session_id = Some(9);
    write(&d);
    assert_eq!(discover_code(), "STALE_DESCRIPTOR");
    // 进程已退出
    let mut d = original.clone();
    d.pid = 999_999;
    write(&d);
    assert_eq!(discover_code(), "STALE_DESCRIPTOR");
    // PID 被复用（创建时间不同）
    let mut d = original.clone();
    d.process_started_at = Some(1);
    write(&d);
    assert_eq!(discover_code(), "STALE_DESCRIPTOR");
    // 非 loopback 地址
    let mut d = original.clone();
    d.address = "10.0.0.1:80".into();
    write(&d);
    assert_eq!(discover_code(), "DESCRIPTOR_INVALID");
    // 协议版本不符
    let mut raw = serde_json::to_value(&original).unwrap();
    raw["protocolVersion"] = json!(2);
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(discover_code(), "UNSUPPORTED_PROTOCOL_VERSION");
    // 不安全权限
    write(&original);
    *h.host.insecure.lock().unwrap() = true;
    assert_eq!(discover_code(), "DESCRIPTOR_INSECURE");
    *h.host.insecure.lock().unwrap() = false;
    // 凭证被替换：同一实例拒绝 → STALE_DESCRIPTOR
    let mut d = original.clone();
    d.token = "f".repeat(64);
    write(&d);
    let client = ToolClient::discover(h.dir.path(), h.host.as_ref()).unwrap();
    assert_eq!(client.status().await.unwrap_err().code, "STALE_DESCRIPTOR");
    // 描述文件指向另一个实例 ID → INSTANCE_MISMATCH
    let mut d = original.clone();
    d.instance_id = "inst_somebody_else".into();
    write(&d);
    let client = ToolClient::discover(h.dir.path(), h.host.as_ref()).unwrap();
    assert_eq!(client.status().await.unwrap_err().code, "INSTANCE_MISMATCH");
    // 端口上没有服务 → SERVICE_UNREACHABLE
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let mut d = original.clone();
    d.address = format!("127.0.0.1:{port}");
    write(&d);
    let client = ToolClient::discover(h.dir.path(), h.host.as_ref()).unwrap();
    assert_eq!(
        client.status().await.unwrap_err().code,
        "SERVICE_UNREACHABLE"
    );
    // 恢复原文件后可以正常使用
    write(&original);
    let client = ToolClient::discover(h.dir.path(), h.host.as_ref()).unwrap();
    assert!(client.status().await.unwrap().ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_removes_only_owned_descriptor() {
    let mut h = Harness::new(ServiceLimits::default()).await;
    let path = discovery::descriptor_path(h.dir.path(), Some(7));
    assert!(path.exists());
    h.running.shutdown_now();
    assert!(!path.exists(), "退出删除本实例描述文件");
    // 接受循环的中止是协作式的：关闭后短时间内仍可能有连接被接受，但任何响应都只能是
    // SERVICE_CLOSED（或传输被中止），绝不返回结果；随后监听端口消失。
    let mut unreachable = false;
    for _ in 0..100 {
        match h.client.status().await {
            Ok(envelope) => {
                assert!(!envelope.ok, "关闭后不得返回结果");
                assert_eq!(envelope.error.unwrap().code, "SERVICE_CLOSED");
            }
            Err(error) if error.code == "SERVICE_UNREACHABLE" => {
                unreachable = true;
                break;
            }
            Err(error) => assert_eq!(error.code, "TRANSPORT_ERROR", "{error:?}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(unreachable, "退出后监听端口应关闭");
    // 关闭后的服务直接调用也被拒绝
    let env = h
        .running
        .service()
        .call(ToolCallRequest {
            request_id: None,
            tool: "system.status".into(),
            arguments: Value::Null,
            timeout_ms: None,
        })
        .await;
    assert_eq!(env.error.unwrap().code, "SERVICE_CLOSED");

    // 描述文件已被另一实例覆盖时不删除
    let h2 = Harness::new(ServiceLimits::default()).await;
    let path2 = discovery::descriptor_path(h2.dir.path(), Some(7));
    let mut foreign = h2.descriptor();
    foreign.instance_id = "inst_newer".into();
    std::fs::write(&path2, serde_json::to_vec(&foreign).unwrap()).unwrap();
    h2.running.shutdown().await;
    assert!(path2.exists(), "不得删除其他实例的描述文件");
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_refuses_to_replace_a_live_foreign_instance() {
    let h = Harness::new(ServiceLimits::default()).await;
    let path = discovery::descriptor_path(h.dir.path(), Some(7));
    let mut foreign = h.descriptor();
    foreign.pid = 31_337;
    foreign.process_started_at = Some(5);
    foreign.instance_id = "inst_foreign".into();
    std::fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
    h.host.set_process(31_337, Some(5));
    let desktop = FakeDesktop::new();
    let deps = ServiceDeps {
        instance_id: "inst_second".into(),
        app_version: "test".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "running".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(crate::artifacts::SystemClock),
    };
    let err = RunningToolService::start(deps, h.host.clone(), h.dir.path())
        .await
        .err()
        .unwrap();
    assert!(err.contains("31337"), "{err}");
    // 外部实例退出后，过期描述文件可被替换
    h.host.set_process(31_337, None);
    let desktop = FakeDesktop::new();
    let deps = ServiceDeps {
        instance_id: "inst_third".into(),
        app_version: "test".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "running".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(crate::artifacts::SystemClock),
    };
    let third = RunningToolService::start(deps, h.host.clone(), h.dir.path())
        .await
        .unwrap();
    let client = ToolClient::discover(h.dir.path(), h.host.as_ref()).unwrap();
    assert_eq!(client.instance_id(), "inst_third");
    drop(third);
    assert!(!path.exists());
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn windows_host_writes_private_descriptor_and_client_verifies_it() {
    use kotone_core::tools::ToolHost;
    use kotone_platform_windows::tool_host::WindowsToolHost;
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    let desktop = FakeDesktop::new();
    desktop.add_window(GAME, GAME_PID, "Game", "game.exe", 0, 0, 4, 4);
    desktop.set_foreground(Some(GAME));
    let host: Arc<dyn ToolHost> = Arc::new(WindowsToolHost);
    let deps = ServiceDeps {
        instance_id: crate::ids::new_id("inst"),
        app_version: "test".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(crate::artifacts::SystemClock),
    };
    let running = RunningToolService::start(deps, host.clone(), &run)
        .await
        .unwrap();
    let path = running.descriptor_path().unwrap();
    host.verify_private_file(&path).unwrap();
    let descriptor: Descriptor = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(descriptor.pid, std::process::id());
    assert_eq!(descriptor.session_id, host.session_id());
    assert!(descriptor.process_started_at.is_some());
    assert!(!format!("{descriptor:?}").contains(&descriptor.token));
    let client = ToolClient::discover(&run, host.as_ref()).unwrap();
    assert!(client.status().await.unwrap().ok);
    // 放宽权限后客户端拒绝使用
    let out = std::process::Command::new("icacls")
        .arg(&path)
        .args(["/grant", "*S-1-5-32-545:R"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        ToolClient::discover(&run, host.as_ref()).unwrap_err().code,
        "DESCRIPTOR_INSECURE"
    );
    drop(running);
    assert!(!path.exists());
}
