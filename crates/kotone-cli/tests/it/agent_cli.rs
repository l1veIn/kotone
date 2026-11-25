//! Agent 接入的 CLI 集成：真实 `kotone-cli` 二进制 → loopback HTTP → 工具服务（假宿主）。
//! 覆盖 `app status/launch/quit`、`tools events`（含 --follow）与 `kotone-cli mcp`（stdio JSON-RPC）。
//! 使用临时 KOTONE_HOME 与运行目录；不启动 kotone.exe、不读取用户配置。

#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use kotone_platform_windows::tool_host::WindowsToolHost;
use kotone_tools::artifacts::SystemClock;
use kotone_tools::discovery;
use kotone_tools::testing::{FakeAppHost, FakeDesktop};
use kotone_tools::{RunningToolService, ServiceDeps, ServiceLimits};

fn cli(run_dir: &Path, home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_kotone-cli"))
        .args(args)
        .env(discovery::DIR_ENV, run_dir)
        .env("KOTONE_HOME", home)
        .env_remove("KOTONE_EXE")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

async fn run(run_dir: &Path, home: &Path, args: &[&str]) -> (i32, Value) {
    let (r, h) = (run_dir.to_path_buf(), home.to_path_buf());
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = cli(&r, &h, &refs);
        let value = serde_json::from_str(&stdout)
            .unwrap_or_else(|_| json!({"raw": stdout, "stderr": stderr}));
        (code, value)
    })
    .await
    .unwrap()
}

async fn start(run_dir: &Path) -> (RunningToolService, Arc<FakeAppHost>) {
    let desktop = FakeDesktop::new();
    desktop.add_window(0x1001, 4242, "Fake Game", "game.exe", 0, 0, 40, 30);
    desktop.set_foreground(Some(0x1001));
    let host = FakeAppHost::new();
    let deps = ServiceDeps {
        instance_id: kotone_tools::ids::new_id("inst"),
        app_version: "0.0.0-agent".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(SystemClock),
        ocr_backends: vec![],
        host: host.clone(),
    };
    let running = RunningToolService::start(deps, Arc::new(WindowsToolHost), run_dir)
        .await
        .unwrap();
    (running, host)
}

#[tokio::test(flavor = "multi_thread")]
async fn app_lifecycle_and_events_over_the_real_cli() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // 未运行：status 退出码 1；launch 在工具服务关闭时拒绝；开启后找不到 exe 也明确报错
    let (code, value) = run(&run_dir, &home, &["app", "status", "--json"]).await;
    assert_eq!(
        (code, value["running"].clone()),
        (1, json!(false)),
        "{value}"
    );
    let (code, value) = run(
        &run_dir,
        &home,
        &["app", "launch", "--exe", "Z:/missing/kotone.exe"],
    )
    .await;
    assert_eq!(code, 2, "{value}");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("toolService.enabled"));
    let (code, value) = run(
        &run_dir,
        &home,
        &[
            "app",
            "launch",
            "--enable-tool-service",
            "--exe",
            "Z:/missing/kotone.exe",
        ],
    )
    .await;
    assert_eq!(code, 2, "{value}");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("kotone.exe"));
    let config: Value =
        serde_json::from_slice(&std::fs::read(home.join("config.json")).unwrap()).unwrap();
    assert_eq!(
        config["toolService"]["enabled"], true,
        "配置写在隔离的 KOTONE_HOME"
    );

    let (running, host) = start(&run_dir).await;
    let (code, value) = run(&run_dir, &home, &["app", "status", "--json"]).await;
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["status"]["instanceId"], running.instance_id());
    // 已在运行：launch 直接返回现有实例（不需要 exe）
    let (code, value) = run(
        &run_dir,
        &home,
        &["app", "launch", "--exe", "Z:/missing/kotone.exe"],
    )
    .await;
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["result"]["alreadyRunning"], true);
    // 事件：读取起点、用户请求、类型过滤
    let (code, value) = run(&run_dir, &home, &["tools", "events", "--from", "oldest"]).await;
    assert_eq!(code, 0);
    let cursor = value["result"]["cursor"].as_str().unwrap().to_string();
    assert_eq!(value["result"]["events"][0]["type"], "app.started");
    running
        .service()
        .submit_user_request("我队友在说啥", "assistant-voice")
        .unwrap();
    let (code, value) = run(
        &run_dir,
        &home,
        &[
            "tools",
            "events",
            "--cursor",
            &cursor,
            "--types",
            "user.*",
            "--wait-ms",
            "2000",
        ],
    )
    .await;
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["result"]["events"][0]["data"]["text"], "我队友在说啥");
    let (code, value) = run(
        &run_dir,
        &home,
        &["tools", "events", "--cursor", "ev1.000000000000.1"],
    )
    .await;
    assert_eq!(
        (code, value["error"]["code"].clone()),
        (1, json!("EVENT_CURSOR_STALE"))
    );
    // --follow：逐行输出，服务关闭时退出
    let follow = {
        let (r, h, c) = (run_dir.clone(), home.clone(), cursor.clone());
        std::thread::spawn(move || {
            let mut child = Command::new(env!("CARGO_BIN_EXE_kotone-cli"))
                .args(["tools", "events", "--follow", "--cursor", &c])
                .env(discovery::DIR_ENV, &r)
                .env("KOTONE_HOME", &h)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let reader = BufReader::new(child.stdout.take().unwrap());
            let lines: Vec<Value> = reader
                .lines()
                .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
                .collect();
            (child.wait().unwrap().code(), lines)
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    running
        .service()
        .publish_event("runtime.phase", json!({"phase": "running"}));
    tokio::time::sleep(Duration::from_millis(300)).await;
    // quit：调用宿主；假宿主不会真的退出进程，CLI 等待超时后报告
    let (code, value) = run(&run_dir, &home, &["app", "quit", "--wait-ms", "300"]).await;
    assert_eq!(code, 1, "{value}");
    assert_eq!(host.quits(), 1);
    let mut running = running;
    running.shutdown_now();
    let (code, lines) = tokio::task::spawn_blocking(move || follow.join().unwrap())
        .await
        .unwrap();
    assert_eq!(code, Some(0));
    let kinds: Vec<&str> = lines.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["user.request", "runtime.phase", "app.stopping"]);
}

struct Mcp {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(run_dir: &Path, home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kotone-cli"))
            .arg("mcp")
            .env(discovery::DIR_ENV, run_dir)
            .env("KOTONE_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut buf = String::new();
            self.stdout.read_line(&mut buf).unwrap();
            let msg: Value = serde_json::from_str(&buf).unwrap();
            if msg["id"] == json!(id) {
                return msg;
            }
            assert!(
                msg["method"] == "notifications/tools/list_changed"
                    || msg["method"] == "notifications/resources/list_changed",
                "意外消息 {msg}"
            );
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> (Value, bool) {
        let msg = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &msg["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (
            serde_json::from_str(text).unwrap(),
            result["isError"] == true,
        )
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_server_exposes_kotone_tools_and_the_reverse_channel() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (rd, h) = (run_dir.clone(), home.clone());
    // Kotone 未运行：只有固定工具，状态如实报告
    let offline = tokio::task::spawn_blocking(move || {
        let mut mcp = Mcp::start(&rd, &h);
        let init = mcp.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
        writeln!(mcp.stdin, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).unwrap();
        let tools = mcp.request("tools/list", json!({}));
        let (status, is_error) = mcp.call("kotone_service_status", json!({}));
        let (call, call_error) = mcp.call("kotone_call", json!({"tool": "app.status"}));
        drop(mcp.stdin);
        let _ = mcp.child.wait();
        (init, tools, status, is_error, call, call_error)
    })
    .await
    .unwrap();
    let (init, tools, status, is_error, call, call_error) = offline;
    assert_eq!(init["result"]["serverInfo"]["name"], "kotone");
    assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "kotone_service_status",
            "kotone_launch",
            "kotone_quit",
            "kotone_capabilities",
            "kotone_call",
            "kotone_events"
        ]
    );
    assert_eq!((status["running"].clone(), is_error), (json!(false), false));
    assert!(call_error);
    assert!(call["hint"].as_str().unwrap().contains("kotone_launch"));

    // 运行中：映射全部工具，经 MCP 完成“领取 → 回复”
    let (running, host) = start(&run_dir).await;
    let request = running
        .service()
        .submit_user_request("我队友在说啥", "assistant-voice")
        .unwrap();
    let id = request["requestId"].as_str().unwrap().to_string();
    let (rd, h) = (run_dir.clone(), home.clone());
    let online = tokio::task::spawn_blocking(move || {
        let mut mcp = Mcp::start(&rd, &h);
        mcp.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
        let tools = mcp.request("tools/list", json!({}));
        let events = mcp.call("kotone_events", json!({"types": ["user.request"]}));
        let claim = mcp.call("kotone_agent_request_claim", json!({"requestId": id, "consumer": "mcp-test"}));
        let reply = mcp.call("kotone_agent_reply", json!({"requestId": id, "text": "他说他去中路", "outputs": ["danmaku"]}));
        let generic = mcp.call("kotone_call", json!({"tool": "agent.requests", "arguments": {"status": "replied"}}));
        let unknown = mcp.request("completion/complete", json!({}));
        drop(mcp.stdin);
        let _ = mcp.child.wait();
        (tools, events, claim, reply, generic, unknown)
    })
    .await
    .unwrap();
    let (tools, events, claim, reply, generic, unknown) = online;
    let list = tools["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "kotone_agent_reply",
        "kotone_agent_request_claim",
        "kotone_screen_capture",
        "kotone_ocr_recognize",
        "kotone_chat_track_read",
        "kotone_runtime_start",
    ] {
        assert!(names.contains(&expected), "{expected} missing in {names:?}");
    }
    assert!(
        !names.contains(&"kotone_event_read") && !names.contains(&"kotone_system_capabilities")
    );
    assert!(
        list.iter().all(|t| t["inputSchema"]["type"] == "object"),
        "MCP inputSchema 必须是 object"
    );
    assert_eq!(
        events.0["result"]["events"][0]["data"]["text"],
        "我队友在说啥"
    );
    assert!(!claim.1, "{}", claim.0);
    assert_eq!(claim.0["result"]["newlyClaimed"], true);
    assert!(!reply.1, "{}", reply.0);
    // 默认受理即返回，交付在后台完成
    assert_eq!(reply.0["result"]["status"], "accepted");
    assert!(reply.0["result"]["deliveryId"]
        .as_str()
        .unwrap()
        .starts_with("dlv_"));
    for _ in 0..100 {
        if !host.deliveries().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        host.deliveries(),
        vec![("他说他去中路".to_string(), vec!["danmaku"])]
    );
    assert_eq!(generic.0["result"]["matched"], 1);
    assert_eq!(unknown["error"]["code"], -32601);
    drop(running);
}

/// 插件的工具名里本来就有下划线（`voice_chat.status`、`game_chat.send`）：MCP 名照旧，调用时还原对；
/// 没先 tools/list 也能调（现取 capabilities）；撞名的不列、调用时说清楚；没有的工具说下一步。
/// 假宿主只记下调用，不打字、不出声。
#[tokio::test(flavor = "multi_thread")]
async fn mcp_maps_underscored_plugin_tool_names_back() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (running, host) = start(&run_dir).await;
    let spec = |name: &str| {
        json!({
            "name": name, "version": 1, "status": "implemented", "description": format!("{name} 的说明"),
            "sideEffects": [], "dataEgress": "none", "ready": true, "notReadyReason": null,
            "inputSchema": {"type": "object", "properties": {}}, "outputSchema": {"type": "object"},
            "plugin": "test", "untrustedResult": false
        })
    };
    host.set_plugin_tools(
        [
            "voice_chat.status",
            "voice_chat.default_mic",
            "game_chat.send",
            "x_y.z",
            "x.y_z",
        ]
        .into_iter()
        .map(spec)
        .collect(),
    );
    let (rd, h) = (run_dir.clone(), home.clone());
    let outcome = tokio::task::spawn_blocking(move || {
        let mut mcp = Mcp::start(&rd, &h);
        mcp.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
        // 不先列工具就调
        let status = mcp.call("kotone_voice_chat_status", json!({}));
        let tools = mcp.request("tools/list", json!({}));
        let send = mcp.call("kotone_game_chat_send", json!({"text": "gg"}));
        let mic = mcp.call("kotone_voice_chat_default_mic", json!({"use": "restore"}));
        let ambiguous = mcp.call("kotone_x_y_z", json!({}));
        let unknown = mcp.call("kotone_voice_chat_nope", json!({}));
        drop(mcp.stdin);
        let _ = mcp.child.wait();
        (status, tools, send, mic, ambiguous, unknown)
    })
    .await
    .unwrap();
    let (status, tools, send, mic, ambiguous, unknown) = outcome;
    assert!(!status.1, "{}", status.0);
    assert!(!send.1, "{}", send.0);
    assert!(!mic.1, "{}", mic.0);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "kotone_voice_chat_status",
        "kotone_voice_chat_default_mic",
        "kotone_game_chat_send",
    ] {
        assert!(names.contains(&expected), "{expected} missing in {names:?}");
    }
    assert!(!names.contains(&"kotone_x_y_z"), "{names:?}");
    // 宿主收到的是原来的 Kotone 名和原样的参数
    let calls: Vec<(String, Value)> = host.invocations();
    assert_eq!(
        calls,
        vec![
            ("voice_chat.status".to_string(), json!({})),
            ("game_chat.send".to_string(), json!({"text": "gg"})),
            (
                "voice_chat.default_mic".to_string(),
                json!({"use": "restore"})
            ),
        ]
    );
    assert!(ambiguous.1);
    assert_eq!(ambiguous.0["candidates"], json!(["x.y_z", "x_y.z"]));
    assert!(ambiguous.0["hint"]
        .as_str()
        .unwrap()
        .contains("kotone_call"));
    assert!(unknown.1);
    assert!(unknown.0["error"]
        .as_str()
        .unwrap()
        .contains("kotone_voice_chat_nope"));
    assert!(unknown.0["hint"].as_str().unwrap().contains("tools/list"));
    drop(running);
}

/// MCP Apps（ADR 039 D5）：initialize 声明 resources 和 `io.modelcontextprotocol/ui`；resources/list、resources/read
/// 经工具服务读到插件的 App 页面；工具带 `_meta.ui`（resourceUri 只给真是资源的页面）；带页面的结果照规范交出。
#[tokio::test(flavor = "multi_thread")]
async fn mcp_serves_app_resources_and_page_results() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (running, host) = start(&run_dir).await;
    let spec = |name: &str, ui: Value| {
        json!({
            "name": name, "version": 1, "status": "implemented", "description": format!("{name} 的说明"),
            "sideEffects": [], "dataEgress": "none", "ready": true, "notReadyReason": null,
            "inputSchema": {"type": "object", "properties": {}}, "outputSchema": {"type": "object"},
            "plugin": "lol", "untrustedResult": true, "_meta": {"ui": ui}
        })
    };
    host.set_plugin_tools(vec![
        spec(
            "lol.tier",
            json!({"visibility": ["model", "app"], "resourceUri": "ui://kotone/lol/tier"}),
        ),
        spec(
            "lol.scout",
            json!({"visibility": ["model", "app"], "resourceUri": "ui://kotone/lol/match"}),
        ),
        spec("lol.champions", json!({"visibility": ["app"]})),
    ]);
    let meta = json!({"ui": {"csp": {"resourceDomains": ["https://raw.communitydragon.org"]}, "prefersBorder": false}});
    let html = "<!doctype html><html><head><meta name=\"kotone:page\" content=\"tier\"></head><body></body></html>";
    host.set_resources(vec![(
        json!({"uri": "ui://kotone/lol/tier", "name": "强度榜", "mimeType": "text/html;profile=mcp-app", "_meta": meta}),
        json!({"uri": "ui://kotone/lol/tier", "mimeType": "text/html;profile=mcp-app", "text": html, "_meta": meta}),
    )]);
    host.set_invoke_result(
        "lol.tier",
        Ok(json!({
            "content": {"status": "ok", "text": "上单：剑魔 T1"},
            "structuredContent": {"mode": "rift", "lanes": {"top": []}},
            "_meta": {"ui": {"resourceUri": "ui://kotone/lol/tier", "params": null}}
        })),
    );
    let (rd, h) = (run_dir.clone(), home.clone());
    let outcome = tokio::task::spawn_blocking(move || {
        let mut mcp = Mcp::start(&rd, &h);
        let init = mcp.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {"extensions": {"io.modelcontextprotocol/ui": {"mimeTypes": ["text/html;profile=mcp-app"]}}}, "clientInfo": {"name": "test", "version": "1"}}));
        let list = mcp.request("resources/list", json!({}));
        let templates = mcp.request("resources/templates/list", json!({}));
        let read = mcp.request("resources/read", json!({"uri": "ui://kotone/lol/tier"}));
        let missing = mcp.request("resources/read", json!({"uri": "ui://kotone/lol/match"}));
        let tools = mcp.request("tools/list", json!({}));
        let tier = mcp.request("tools/call", json!({"name": "kotone_lol_tier", "arguments": {}}));
        drop(mcp.stdin);
        let _ = mcp.child.wait();
        (init, list, templates, read, missing, tools, tier)
    })
    .await
    .unwrap();
    let (init, list, templates, read, missing, tools, tier) = outcome;
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["resources"]["listChanged"], true);
    assert_eq!(
        caps["extensions"]["io.modelcontextprotocol/ui"]["mimeTypes"],
        json!(["text/html;profile=mcp-app"])
    );
    assert_eq!(
        list["result"]["resources"][0]["uri"],
        "ui://kotone/lol/tier"
    );
    assert_eq!(
        list["result"]["resources"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    assert_eq!(templates["result"]["resourceTemplates"], json!([]));
    let content = &read["result"]["contents"][0];
    assert_eq!(content["text"], html);
    assert_eq!(content["_meta"], meta);
    assert_eq!(missing["error"]["code"], -32002);
    let listed = tools["result"]["tools"].as_array().unwrap();
    let by = |n: &str| listed.iter().find(|t| t["name"] == n).unwrap().clone();
    assert_eq!(
        by("kotone_lol_tier")["_meta"]["ui"]["resourceUri"],
        "ui://kotone/lol/tier"
    );
    assert_eq!(
        by("kotone_lol_tier")["_meta"]["ui/resourceUri"],
        "ui://kotone/lol/tier"
    );
    assert!(by("kotone_lol_scout")["_meta"]["ui"]
        .get("resourceUri")
        .is_none());
    assert_eq!(
        by("kotone_lol_champions")["_meta"]["ui"]["visibility"],
        json!(["app"])
    );
    let result = &tier["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(
        result["structuredContent"],
        json!({"mode": "rift", "lanes": {"top": []}})
    );
    assert_eq!(result["_meta"]["ui"]["resourceUri"], "ui://kotone/lol/tier");
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("剑魔"));
    drop(running);
}
