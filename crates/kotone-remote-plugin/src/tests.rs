//! 远程插件的整条路：真的 stdio 子进程（`examples/remote_test_server.rs`）和进程内的 HTTP 服务器
//! （`kotone_mcp::testing`）；权限用 kotone-agent 的判定核对。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_plugin_api::{BoxFuture, Host, Observation, Offer, Plugin};
use serde_json::{json, Map, Value};

use crate::{tools, RemotePlugin, ServerSpec, Status, EFFECT_ACTION, EFFECT_UNTRUSTED_READ};

/// 测试用的宿主：只有远程插件用得到的那几样（开关、设置、Key、后台任务）。
#[derive(Default)]
struct TestHost {
    off: AtomicBool,
    settings: Mutex<Value>,
    secrets: Mutex<BTreeMap<String, String>>,
}

impl Host for TestHost {
    fn plugin(&self) -> &str {
        "rt"
    }
    fn enabled(&self) -> bool {
        !self.off.load(Ordering::SeqCst)
    }
    fn source_enabled(&self, _: &str) -> bool {
        false
    }
    fn settings(&self) -> Value {
        let s = self.settings.lock().unwrap().clone();
        if s.is_null() {
            json!({})
        } else {
            s
        }
    }
    fn data_dir(&self) -> PathBuf {
        PathBuf::new()
    }
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }
    fn process_ids(&self, _: &str) -> Vec<u32> {
        Vec::new()
    }
    fn process_command_line(&self, _: u32) -> Option<String> {
        None
    }
    fn game_observe(&self, _: &str, _: Observation) {}
    fn game_claim(&self, _: &str, _: bool) {}
    fn game_event(&self, _: &str, _: &str, _: &str, _: Map<String, Value>, _: Option<f64>) {}
    fn secret(&self, name: &str) -> Option<String> {
        self.secrets.lock().unwrap().get(name).cloned()
    }
}

/// `target/debug/examples/remote_test_server.exe`（`cargo test -p kotone-remote-plugin` 会先编译 examples）。
///
/// 测试程序自己放在哪随 cargo 版本变：1.100 之前在 `target/debug/deps/`，1.100 起在
/// `target/debug/build/kotone-remote-plugin/<哈希>/out/`（Cargo 1.100 changelog，cargo#17354：build-dir 新布局）；
/// example 一直放到 `target/debug/examples/`。所以不数层数，从测试程序往上一级级找。
fn server_exe() -> PathBuf {
    let name = format!("remote_test_server{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().unwrap();
    let mut tried = Vec::new();
    for dir in exe.ancestors().skip(1) {
        let path = dir.join("examples").join(&name);
        if path.is_file() {
            return path;
        }
        tried.push(path.display().to_string());
    }
    panic!(
        "找不到 {name}：用 `cargo test -p kotone-remote-plugin` 跑（会先编译 examples/remote_test_server.rs）。找过：\n{}",
        tried.join("\n")
    );
}

async fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "等太久了：{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn names(plugin: &RemotePlugin) -> Vec<String> {
    let mut names: Vec<String> = plugin.tools().into_iter().map(|t| t.name).collect();
    names.sort();
    names
}

/// stdio 的整条路：开着就连、工具名、只给页面的、信任前后的询问分类、结果剥掉看不见的字符、
/// 它的 MCP App 页面、`tools/list_changed`、工具清单记在磁盘上、掉线和重连、关掉就断开。
#[tokio::test]
async fn a_stdio_server_is_a_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ServerSpec::parse(&json!({"command": server_exe().to_string_lossy()})).unwrap();
    let fingerprint = spec.fingerprint();
    let plugin = Arc::new(RemotePlugin::new(
        "rt",
        Ok(spec.clone()),
        Some(dir.path().to_path_buf()),
        "0.2.0-test",
    ));
    assert!(plugin.tools().is_empty(), "还没连过");
    let host = Arc::new(TestHost::default());
    let dyn_host: Arc<dyn Host> = host.clone();
    // 开着、还不知道有哪些工具：启动时就连
    plugin.clone().start(dyn_host.clone());
    until("连上并列出工具", || {
        (!plugin.tools().is_empty()).then_some(())
    })
    .await;
    assert_eq!(plugin.server().status(), Status::Connected);
    assert_eq!(
        names(&plugin),
        [
            "rt.add",
            "rt.change_tools",
            "rt.echo",
            "rt.exit",
            "rt.hidden",
            "rt.refresh"
        ],
        "名字不合规矩的（bad name）不要"
    );
    let describe = plugin.server().describe();
    assert_eq!(describe["skippedTools"], json!(["bad name"]));
    assert_eq!(
        describe["server"]["title"], "Remote Test Server",
        "{describe}"
    );
    assert_eq!(
        describe["server"]["icons"].as_array().unwrap().len(),
        1,
        "javascript: 的图标不要"
    );
    let spec_of = |name: &str| plugin.tools().into_iter().find(|t| t.name == name).unwrap();
    assert!(!spec_of("rt.refresh").for_model(), "只给页面的不进她的清单");
    assert!(spec_of("rt.add").untrusted && spec_of("rt.echo").untrusted);
    assert_eq!(spec_of("rt.add").side_effects, vec![EFFECT_ACTION]);
    assert_eq!(spec_of("rt.echo").side_effects, vec![EFFECT_UNTRUSTED_READ]);
    assert!(
        !spec_of("rt.echo")
            .parameters
            .to_string()
            .contains('\u{E0041}'),
        "参数说明里藏的字剥掉"
    );
    // 玩家信任了这个服务器（设置里记下这份配置的指纹）：只读的不再问，没标只读的照样问
    *host.settings.lock().unwrap() = json!({"trusted": fingerprint});
    plugin.settings_changed(dyn_host.clone());
    assert!(spec_of("rt.echo").side_effects.is_empty());
    assert_eq!(spec_of("rt.add").side_effects, vec![EFFECT_ACTION]);

    // 调用：结果给她的文字、给页面的数据、用哪个页面
    let echo = plugin
        .call(
            dyn_host.clone(),
            "rt.echo",
            &json!({"text": "你好\u{200B}"}),
        )
        .await
        .unwrap();
    assert_eq!(echo.content, json!("你好"));
    assert_eq!(echo.structured_content, Some(json!({"echo": "你好"})));
    assert_eq!(echo.ui.unwrap().uri, "ui://remote-test/page");
    let hidden = plugin
        .call(dyn_host.clone(), "rt.hidden", &json!({}))
        .await
        .unwrap();
    assert_eq!(hidden.content, json!("visible part"));
    assert!(plugin
        .call(dyn_host.clone(), "lol.scout", &json!({}))
        .await
        .is_err());

    // 它的 MCP App：归属按“哪个服务器给的”，CSP 用它声明的（写法不对的丢掉）
    let server = plugin.server();
    assert!(server.owns_uri("ui://remote-test/page"));
    assert!(!server.owns_uri("ui://kotone/lol/tier"));
    let page = server
        .read_app(&dyn_host, "ui://remote-test/page")
        .await
        .unwrap();
    assert_eq!(page.mime_type, kotone_plugin_api::APP_MIME);
    assert!(page.text.contains("remote test page"));
    assert_eq!(
        page.ui.csp.resource_domains,
        vec!["https://cdn.example.com"]
    );
    assert_eq!(page.ui.prefers_border, Some(true));
    assert!(server.read_app(&dyn_host, "ui://other/page").await.is_err());
    let slug = crate::page_slug("ui://remote-test/page");
    assert_eq!(
        server.uri_for_slug(&slug).as_deref(),
        Some("ui://remote-test/page")
    );

    // 服务器说工具变了：重新列（从下一段对话起生效，由宿主每段对话取一次清单）
    plugin
        .call(dyn_host.clone(), "rt.change_tools", &json!({}))
        .await
        .unwrap();
    until("tools/list_changed 之后多了 extra", || {
        names(&plugin)
            .contains(&"rt.extra".to_string())
            .then_some(())
    })
    .await;

    // 工具清单记在磁盘上：Kotone 重开以后不用先连就知道有哪些工具；配置变了不认
    let reopened = RemotePlugin::new("rt", Ok(spec.clone()), Some(dir.path().to_path_buf()), "x");
    assert!(names(&reopened).contains(&"rt.extra".to_string()));
    assert_eq!(reopened.server().status(), Status::NotConnected);
    let other =
        ServerSpec::parse(&json!({"command": server_exe().to_string_lossy(), "args": ["--x"]}))
            .unwrap();
    let changed = RemotePlugin::new("rt", Ok(other), Some(dir.path().to_path_buf()), "x");
    assert!(changed.tools().is_empty());
    // 配置变了：认得出上次连的是哪份（宿主据此清掉“总是允许”），同一份配置不算
    assert_eq!(
        changed.server().replaced_fingerprint(),
        Some(fingerprint.as_str())
    );
    assert_eq!(reopened.server().replaced_fingerprint(), None);

    // 掉线：进程退出了，这一次回答“没连上”（工具还在清单里）；下一次用的时候重新连
    let pid_before = describe_pid(&plugin).await;
    let lost = plugin
        .call(dyn_host.clone(), "rt.exit", &json!({}))
        .await
        .unwrap_err();
    assert!(lost.contains("现在没连上"), "{lost}");
    assert!(names(&plugin).contains(&"rt.echo".to_string()));
    let again = plugin
        .call(dyn_host.clone(), "rt.echo", &json!({"text": "again"}))
        .await
        .unwrap();
    assert_eq!(again.content, json!("again"));
    assert_ne!(describe_pid(&plugin).await, pid_before, "是新起的进程");

    // 关掉开关：断开
    host.off.store(true, Ordering::SeqCst);
    plugin.settings_changed(dyn_host.clone());
    until("关掉以后断开", || {
        (plugin.server().status() == Status::NotConnected).then_some(())
    })
    .await;
}

/// 现在连着的子进程的进程号（连着的话）。
async fn describe_pid(plugin: &RemotePlugin) -> Option<u32> {
    let host: Arc<dyn Host> = Arc::new(TestHost::default());
    plugin
        .server()
        .connect(&host, false)
        .await
        .ok()
        .and_then(|c| c.process_id())
}

/// HTTP：Key 不在配置里，连接时从凭据库填进请求头；没填时说清缺哪个 Key、在哪填。
#[tokio::test]
async fn an_http_server_gets_its_key_from_the_credential_store() {
    use kotone_mcp::testing::{handler, text_reply, TestServer};
    let server = TestServer::start(
        json!([{"name": "weather", "description": "天气", "inputSchema": {"type": "object"},
                "annotations": {"readOnlyHint": true}}]),
        json!([]),
        handler(|_, args| async move {
            text_reply(&format!("晴 {}", args["city"].as_str().unwrap_or("")))
        }),
    )
    .await;
    let spec = ServerSpec::parse(&json!({
        "type": "http", "url": server.url(), "headers": {"X-Test-Key": "${input:token}"}
    }))
    .unwrap();
    assert_eq!(spec.keys(), vec!["token"]);
    let plugin = Arc::new(RemotePlugin::new("weather", Ok(spec), None, "0.2.0-test"));
    let host = Arc::new(TestHost::default());
    let dyn_host: Arc<dyn Host> = host.clone();
    let missing = plugin.server().connect(&dyn_host, true).await.unwrap_err();
    assert!(missing.contains("还没填 Key：token"), "{missing}");
    assert_eq!(plugin.server().status(), Status::Failed);
    host.secrets
        .lock()
        .unwrap()
        .insert("token".into(), "k-123".into());
    plugin.server().connect(&dyn_host, true).await.unwrap();
    let result = plugin
        .call(
            dyn_host.clone(),
            "weather.weather",
            &json!({"city": "上海"}),
        )
        .await
        .unwrap();
    assert_eq!(result.content, json!("晴 上海"));
    let log = server.log();
    assert_eq!(
        log.custom_headers.get("x-test-key").map(String::as_str),
        Some("k-123")
    );
    assert_eq!(
        log.calls,
        vec![("weather".to_string(), json!({"city": "上海"}))]
    );
}

/// 地址里的 Key（`?token=${input:token}`）连不上时不出现在给模型的“没连上”和卡片的错误里（审查后加的，2026-11-25）。
#[tokio::test]
async fn keys_in_the_url_stay_out_of_errors() {
    let (held, port) = nobody_listening();
    let spec = ServerSpec::parse(&json!({
        "url": format!("http://127.0.0.1:{port}/mcp?token=${{input:token}}"),
        "headers": {"X-Key": "${input:header}"}
    }))
    .unwrap();
    let known = crate::Known {
        fingerprint: spec.fingerprint(),
        tools: vec![serde_json::from_value(json!({"name": "ping", "inputSchema": {}})).unwrap()],
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(crate::CACHE_FILE),
        serde_json::to_string(&known).unwrap(),
    )
    .unwrap();
    let plugin = RemotePlugin::new("leaky", Ok(spec), Some(dir.path().to_path_buf()), "x");
    let host = Arc::new(TestHost::default());
    host.secrets
        .lock()
        .unwrap()
        .insert("token".into(), "sk-url-secret-1".into());
    host.secrets
        .lock()
        .unwrap()
        .insert("header".into(), "hdr-secret-22".into());
    let dyn_host: Arc<dyn Host> = host.clone();
    let error = plugin
        .call(dyn_host.clone(), "leaky.ping", &json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("现在没连上"), "{error}");
    let card = plugin.server().describe().to_string();
    for text in [&error, &card] {
        assert!(
            !text.contains("sk-url-secret-1") && !text.contains("hdr-secret-22"),
            "{text}"
        );
    }
    drop(held);
}

/// 一个肯定没人听的端口：占着但不 listen，连过去是“拒绝连接”，拿着它的时候别的测试也绑不上。
/// 不能绑了再放掉：Windows 连一个没人听的端口要重试约 2 秒，这期间同时在跑的测试在这个端口上
/// listen 就连上了（2026-11-25 一次全量跑里 `an_offline_server_says_it_is_not_connected` 看到 NotConnected：
/// 连上了别人的服务器，调用时才断）。
fn nobody_listening() -> (tokio::net::TcpSocket, u16) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}

/// 服务器掉线（连不上）：工具还在（一段对话里清单不变），调用回答“这个插件现在没连上”；
/// 连不上以后一会儿之内再用马上回答，不每次都等。
#[tokio::test]
async fn an_offline_server_says_it_is_not_connected() {
    let dir = tempfile::tempdir().unwrap();
    let (held, port) = nobody_listening();
    let spec = ServerSpec::parse(&json!({"url": format!("http://127.0.0.1:{port}/mcp")})).unwrap();
    // 上次连上时记下的工具清单
    let known = crate::Known {
        fingerprint: spec.fingerprint(),
        tools: vec![serde_json::from_value(json!({"name": "ping", "inputSchema": {}})).unwrap()],
        ..Default::default()
    };
    std::fs::write(
        dir.path().join(crate::CACHE_FILE),
        serde_json::to_string(&known).unwrap(),
    )
    .unwrap();
    let plugin = RemotePlugin::new("gone", Ok(spec), Some(dir.path().to_path_buf()), "x");
    assert_eq!(names(&plugin), ["gone.ping"], "记着的工具还在");
    let host: Arc<dyn Host> = Arc::new(TestHost::default());
    let first = plugin
        .call(host.clone(), "gone.ping", &json!({}))
        .await
        .unwrap_err();
    assert!(first.starts_with("「gone」插件现在没连上"), "{first}");
    assert!(first.contains("「插件」里重新连接"), "说下一步：{first}");
    assert_eq!(plugin.server().status(), Status::Failed);
    let started = Instant::now();
    let second = plugin
        .call(host.clone(), "gone.ping", &json!({}))
        .await
        .unwrap_err();
    assert!(second.contains("现在没连上"), "{second}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "一会儿之内不再重试"
    );
    drop(held);
}

/// 权限（MCP 方案第七节第 3 条）：用 kotone-agent 的判定核对。
/// 没标只读的第一次问、“总是允许这个工具”以后不问；标了只读的，信任这个服务器之前也问；
/// 只读模式下第三方的操作不让做；结果一律不可信；不超过上限时主会话也拿，超过了只给后台。
#[test]
fn asking_rules_for_third_party_tools() {
    use kotone_agent::catalog::{main_offers, subagent_offers, Catalog};
    use kotone_agent::permissions::{Decide, Decision, Mode, Policy, Rule};

    let tool = |value: Value| serde_json::from_value::<kotone_mcp::Tool>(value).unwrap();
    let write = tool(json!({"name": "add", "inputSchema": {"type": "object"}}));
    let read =
        tool(json!({"name": "get", "inputSchema": {}, "annotations": {"readOnlyHint": true}}));
    let offer = |trusted: bool| Offer {
        plugin: "srv".into(),
        name: "srv".into(),
        text: crate::manifest("srv", &Err(String::new())).agent,
        tools: vec![
            tools::spec("srv", &write, trusted).unwrap(),
            tools::spec("srv", &read, trusted).unwrap(),
        ],
    };
    let untrusted = Catalog::build(&[], &[], &[offer(false)]);
    let trusted = Catalog::build(&[], &[], &[offer(true)]);
    let policy = Policy::default();
    let args = json!({});

    let add = untrusted.get("srv.add").unwrap();
    assert!(add.untrusted, "结果一律不可信");
    match policy.decide(add, &args) {
        Decision::Ask { reason } => assert_eq!(reason, "第三方插件的操作"),
        other => panic!("没标只读的第一次要问：{other:?}"),
    }
    // 信任了服务器也照样问（只读的注解才因为信任而算数）
    assert!(matches!(
        policy.decide(trusted.get("srv.add").unwrap(), &args),
        Decision::Ask { .. }
    ));
    // 总是允许这个工具 = 权限里这个工具一条“允许”
    let mut always = Policy::default();
    always.rules.insert("srv.add".into(), Rule::Allow);
    assert_eq!(always.decide(add, &args), Decision::Allow);

    let get = untrusted.get("srv.get").unwrap();
    assert!(get.untrusted);
    match policy.decide(get, &args) {
        Decision::Ask { reason } => assert!(reason.contains("还没信任"), "{reason}"),
        other => panic!("只读的，信任前也问：{other:?}"),
    }
    assert_eq!(
        policy.decide(trusted.get("srv.get").unwrap(), &args),
        Decision::Allow,
        "信任以后只读的不问"
    );
    // 只读模式：第三方的操作不让做，信任过的只读工具可以
    let read_only = Policy {
        mode: Mode::ReadOnly,
        ..Policy::default()
    };
    assert!(matches!(
        read_only.decide(add, &args),
        Decision::Deny { .. }
    ));
    assert_eq!(
        read_only.decide(trusted.get("srv.get").unwrap(), &args),
        Decision::Allow
    );

    // 主会话：不超过上限时也拿（超过 3 秒自动转后台）；超过了只给后台（宿主把 instant 去掉）
    assert!(main_offers(add) && subagent_offers(add));
    let mut capped = offer(false);
    for t in &mut capped.tools {
        t.instant = false;
    }
    let capped = Catalog::build(&[], &[], &[capped]);
    let add = capped.get("srv.add").unwrap();
    assert!(!main_offers(add) && subagent_offers(add));
    assert!(
        capped
            .find(Some("srv"), None)
            .iter()
            .any(|e| e.name == "srv.add"),
        "后台按关键词找得到"
    );
}

/// 原则 19：`config.json` 的 `plugins.<id>` 里有第三方插件（旧版本不认识的 id、多出来的字段）照样读；
/// 信任记的是配置的指纹，配置变了就不算。
#[test]
fn settings_with_third_party_plugins_still_load() {
    let spec = ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "x"]})).unwrap();
    let fingerprint = spec.fingerprint();
    let mut config = serde_json::to_value(kotone_core::settings::Settings::default()).unwrap();
    config["plugins"] = json!({
        "everything": {"enabled": false, "settings": {"trusted": fingerprint}, "futureField": 1},
        "lol": {"enabled": true}
    });
    let settings: kotone_core::settings::Settings = serde_json::from_value(config).unwrap();
    assert!(!settings.plugin_enabled("everything", true));
    let stored = Value::Object(settings.plugins["everything"].settings.clone());
    assert!(crate::trusted_in(&stored, &fingerprint));
    let changed = ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "y"]})).unwrap();
    assert!(!crate::trusted_in(&stored, &changed.fingerprint()));
    assert!(!crate::trusted_in(&json!({}), &fingerprint));
    assert!(!crate::trusted_in(&json!({"trusted": ""}), ""));
}

/// 写错的配置也是一张卡片（说清哪里不对），不连、没有工具。
#[test]
fn a_broken_entry_is_a_plugin_that_explains_itself() {
    let plugin = RemotePlugin::new("broken", Err("要写 command".into()), None, "x");
    assert!(plugin.tools().is_empty());
    assert_eq!(plugin.server().status(), Status::Failed);
    assert!(plugin.manifest().description.contains("配置不对"));
    assert!(plugin.server().error().unwrap().contains("要写 command"));
    let m = plugin.manifest();
    assert!(m.enabled_by_default && m.sources.is_empty() && m.pages.is_empty());
    assert!(m.agent.overview.contains("只当数据"));
}

/// 第三方插件不能提供发到游戏的发法（ADR 041 D7，单向门）：合成的清单里这一项永远是空的。
#[test]
fn third_party_plugins_never_provide_send_to_game() {
    for spec in [
        Err("要写 command".to_string()),
        ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "x"]})),
        ServerSpec::parse(&json!({"url": "https://example.com/mcp"})),
    ] {
        let m = crate::manifest("srv", &spec);
        assert!(m.send_to_game.is_empty(), "{m:?}");
    }
}
