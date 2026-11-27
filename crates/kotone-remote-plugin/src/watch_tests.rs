//! 观察项的整条路（ADR 047 D5）：进程内的 HTTP 测试服务器装成绝区零一条龙（工具名、返回格式照它的
//! `docs/develop/zzz/backend/mcp.md` 和 `schemas.py` 的 `RunStatusResult`），状态按脚本变：空闲 → 运行 → 跑完 / 出错。
//! 定时用调小了的 [`Timing`]（“一秒”是 1 毫秒）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_mcp::testing::{handler, TestServer};
use kotone_plugin_api::{BoxFuture, Host, Observation, Plugin};
use serde_json::{json, Map, Value};

use crate::manifest::{self, Source};
use crate::watch::{digest, Timing, EVENT_SOURCE_PREFIX};
use crate::{RemotePlugin, ServerSpec, TRUST_KEY};

/// 收到的一件事：游戏、类型、来源、数据。
type Sent = (String, String, String, Map<String, Value>);

#[derive(Default)]
struct WatchHost {
    settings: Mutex<Value>,
    events: Mutex<Vec<Sent>>,
}

impl Host for WatchHost {
    fn plugin(&self) -> &str {
        "zzz-od"
    }
    fn enabled(&self) -> bool {
        true
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
    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        _: Option<f64>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push((game.into(), kind.into(), source.into(), data));
    }
}

/// 一秒 = 1 毫秒（一条龙的 30 秒 = 30 毫秒）；下限 20 / 40 毫秒；宽限 300 毫秒；退避最多 200 毫秒。
fn fast() -> Timing {
    Timing {
        second: Duration::from_millis(1),
        local_floor: Duration::from_millis(20),
        remote_floor: Duration::from_millis(40),
        grace: Duration::from_millis(300),
        max_backoff: Duration::from_millis(200),
        idle: Duration::from_millis(10),
    }
}

async fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "等太久了：{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 一条龙的 `get_run_status`：`RunStatusResult`（Python 的 dataclass，MCPServer 序列化成 structuredContent，
/// 文字里是同一份 JSON）。字段照 `src/zzz_od/backend/schemas.py`。
fn run_status(state: &str) -> Value {
    let status = json!({
        "state": state,
        "source": if state == "idle" { Value::Null } else { json!("mcp") },
        "app": if state == "idle" { Value::Null } else { json!("OneDragonApp") },
        "started_at": if state == "idle" { Value::Null } else { json!("2026-11-28T10:00:00") },
        "duration_seconds": if state == "idle" { Value::Null } else { json!(321.5) },
        "current_node": if state == "running" { json!("体力刷本") } else { Value::Null },
        "retry_count": if state == "running" { json!(0) } else { Value::Null },
        "last_status": match state { "success" => json!("全部完成\u{200B}"), "failed" => json!("识别不到画面"), _ => Value::Null },
        "failed_node": if state == "failed" { json!("进入副本") } else { Value::Null },
    });
    json!({"content": [{"type": "text", "text": status.to_string()}], "structuredContent": status})
}

/// 装成一条龙的服务器：它自己在 `kotone://manifest` 给出和 Kotone 预设一样的清单（`games: ["zzz"]`）。
async fn onedragon(state: Arc<Mutex<String>>, failing: Arc<Mutex<bool>>) -> TestServer {
    let preset: Value =
        serde_json::from_str(include_str!("../presets/zzz-onedragon.json")).unwrap();
    TestServer::start(
        json!([
            {"name": "get_run_status", "title": "查询运行状态", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true, "title": "查询运行状态"}},
            {"name": "run_one_dragon", "inputSchema": {"type": "object", "properties": {"block": {"type": "boolean"}}},
             "annotations": {"title": "运行一条龙"}},
        ]),
        json!([{"uri": manifest::MANIFEST_URI, "name": "manifest", "mimeType": "application/json",
                "text": preset["manifest"].to_string()}]),
        handler(move |name, _| {
            let state = state.lock().unwrap().clone();
            let failing = *failing.lock().unwrap();
            async move {
                match name.as_str() {
                    "get_run_status" if failing => Err((-32603, "后端未就绪".to_string())),
                    "get_run_status" => Ok(run_status(&state)),
                    _ => Ok(json!({"content": [{"type": "text", "text": "{\"started\": true}"}]})),
                }
            }
        }),
    )
    .await
}

fn calls(server: &TestServer, tool: &str) -> usize {
    server.log().calls.iter().filter(|(n, _)| n == tool).count()
}

fn view_of(plugin: &RemotePlugin, event: &str) -> Value {
    plugin
        .watcher()
        .view()
        .into_iter()
        .find(|w| w["event"] == event)
        .unwrap_or_else(|| panic!("没有观察项 {event}"))
}

/// 一条龙的整条路：没打开、没确认不查；游戏没开不查；开着才查，第一次只记底；跑完、出错各发一件事（来源是第三方，
/// 字段照登记的、剥掉看不见的字符）；两个观察项调同一个工具时一轮只调一次；游戏退出后宽限期里还查、过了就不查；
/// 观察项的内容变了要重新确认。
#[tokio::test]
async fn a_status_tool_becomes_finished_and_failed_events() {
    let state = Arc::new(Mutex::new("idle".to_string()));
    let failing = Arc::new(Mutex::new(false));
    let server = onedragon(state.clone(), failing.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let spec = ServerSpec::parse(&json!({"url": server.url()})).unwrap();
    let fp = spec.fingerprint();
    let plugin = Arc::new(RemotePlugin::with_timing(
        "zzz-od",
        Ok(spec),
        Some(dir.path().to_path_buf()),
        "t",
        fast(),
    ));
    let running = Arc::new(Mutex::new(Vec::<String>::new()));
    let probe = running.clone();
    plugin.set_games_probe(Arc::new(move || probe.lock().unwrap().clone()));
    let host = Arc::new(WatchHost::default());
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp});
    let dyn_host: Arc<dyn Host> = host.clone();
    plugin.clone().start(dyn_host.clone());
    until("连上、读到清单", || {
        (plugin.kotone_manifest().source == Source::Server).then_some(())
    })
    .await;
    let effective = plugin.kotone_manifest();
    assert_eq!(effective.manifest.games, ["zzz"]);
    assert_eq!(effective.manifest.watches.len(), 2);
    assert_eq!(view_of(&plugin, "run_finished")["status"]["state"], "off");
    assert_eq!(view_of(&plugin, "run_finished")["readOnly"], true);

    // 没打开：不查
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(calls(&server, "get_run_status"), 0, "没打开的观察项不查");
    // 打开了但确认的不是这份内容：不查
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp, "watches": {
        "run_finished": {"on": true, "confirmed": "别的"}, "run_failed": {"on": true, "confirmed": "别的"}}});
    plugin.settings_changed(dyn_host.clone());
    until("要重新确认", || {
        (view_of(&plugin, "run_finished")["status"]["state"] == "confirm").then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(calls(&server, "get_run_status"), 0, "没确认不查");

    // 确认了这份内容：游戏没开，等着
    let digests: Vec<(String, String)> = effective
        .manifest
        .watches
        .iter()
        .map(|w| (w.event.clone(), digest(&fp, w)))
        .collect();
    let mut watches = Map::new();
    for (event, d) in &digests {
        watches.insert(event.clone(), json!({"on": true, "confirmed": d}));
    }
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp, "watches": watches});
    plugin.settings_changed(dyn_host.clone());
    until("等游戏开", || {
        (view_of(&plugin, "run_finished")["status"]["state"] == "waiting").then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(calls(&server, "get_run_status"), 0, "游戏没开不查");

    // 绝区零开了：查，第一次只记底
    running.lock().unwrap().push("zzz".into());
    until("开始查", || {
        (calls(&server, "get_run_status") >= 3).then_some(())
    })
    .await;
    assert_eq!(
        view_of(&plugin, "run_finished")["status"]["state"],
        "polling"
    );
    assert!(
        host.events.lock().unwrap().is_empty(),
        "第一次只记底，之后没变不发"
    );
    // 两个观察项看同一个工具、同样的参数：一轮只调一次（两个观察项的间隔一样，所以调用数和轮数一样）
    let before = calls(&server, "get_run_status");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rounds = calls(&server, "get_run_status") - before;
    assert!(rounds <= 300 / 30 + 3, "一轮调了不止一次：{rounds}");

    // 跑起来了：running 不在 to 里，不发
    *state.lock().unwrap() = "running".into();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(host.events.lock().unwrap().is_empty());
    // 跑完了
    *state.lock().unwrap() = "success".into();
    let (game, kind, source, data) =
        until("跑完了", || host.events.lock().unwrap().first().cloned()).await;
    assert_eq!(game, "zzz");
    assert_eq!(kind, "zzz-od.run_finished");
    assert_eq!(source, format!("{EVENT_SOURCE_PREFIX}zzz-od"));
    assert_eq!(
        Value::Object(data),
        json!({"from": "running", "to": "success", "app": "OneDragonApp", "lastStatus": "全部完成",
               "durationSeconds": 321.5}),
        "只带登记过的字段（snake_case 对上 camelCase），剥掉看不见的字符"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(host.events.lock().unwrap().len(), 1, "跑完了只发一次");

    // 游戏退出：宽限期里还查（再跑一次，出错了）
    running.lock().unwrap().clear();
    *state.lock().unwrap() = "running".into();
    tokio::time::sleep(Duration::from_millis(60)).await;
    *state.lock().unwrap() = "failed".into();
    let failed = until("出错了", || {
        host.events
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.1 == "zzz-od.run_failed")
            .cloned()
    })
    .await;
    assert_eq!(failed.0, "zzz", "宽限期里的事照样挂在绝区零上");
    assert_eq!(failed.3["failedNode"], "进入副本");
    assert_eq!(failed.3["lastStatus"], "识别不到画面");
    // 宽限过了：不查
    until("宽限过了", || {
        (view_of(&plugin, "run_failed")["status"]["state"] == "waiting").then_some(())
    })
    .await;
    let after_grace = calls(&server, "get_run_status");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        calls(&server, "get_run_status"),
        after_grace,
        "游戏没开、宽限过了不查"
    );

    // 观察项的内容变了（玩家本地加不了同名的；这里模拟配置变了：指纹不同）→ 要重新确认
    let other = digest("另一份配置", &effective.manifest.watches[0]);
    assert_ne!(other, digests[0].1);
}

/// 失败：退避、连续 5 次停下并给出原因（卡片上显示）；点“重试”重新开始。
#[tokio::test]
async fn failures_back_off_then_stop_until_retried() {
    let state = Arc::new(Mutex::new("idle".to_string()));
    let failing = Arc::new(Mutex::new(true));
    let server = onedragon(state.clone(), failing.clone()).await;
    let spec = ServerSpec::parse(&json!({"url": server.url()})).unwrap();
    let fp = spec.fingerprint();
    let dir = tempfile::tempdir().unwrap();
    let plugin = Arc::new(RemotePlugin::with_timing(
        "zzz-od",
        Ok(spec),
        Some(dir.path().to_path_buf()),
        "t",
        fast(),
    ));
    plugin.set_games_probe(Arc::new(|| vec!["zzz".to_string()]));
    let host = Arc::new(WatchHost::default());
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp});
    let dyn_host: Arc<dyn Host> = host.clone();
    plugin.clone().start(dyn_host.clone());
    until("读到清单", || {
        (plugin.kotone_manifest().source == Source::Server).then_some(())
    })
    .await;
    let watch = plugin.kotone_manifest().manifest.watches[0].clone();
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp, "watches": {"run_finished": {"on": true, "confirmed": digest(&fp, &watch)}}});
    let started = Instant::now();
    plugin.settings_changed(dyn_host.clone());
    let reason = until("停下", || {
        let v = view_of(&plugin, "run_finished");
        (v["status"]["state"] == "stopped")
            .then(|| v["status"]["reason"].as_str().unwrap().to_string())
    })
    .await;
    assert!(
        reason.contains("连续 5 次") && reason.contains("后端未就绪"),
        "{reason}"
    );
    assert_eq!(calls(&server, "get_run_status"), 5);
    // 退避：间隔 30 毫秒的 ×2、×4、×8（最多 200）→ 至少 60+120+200+200 毫秒
    assert!(
        started.elapsed() >= Duration::from_millis(560),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(plugin.watcher().stopped().unwrap().0, "一条龙跑完了");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(calls(&server, "get_run_status"), 5, "停下以后不查");
    // 好了，点重试
    *failing.lock().unwrap() = false;
    plugin.watcher().retry("run_finished");
    until("又在查", || {
        (view_of(&plugin, "run_finished")["status"]["state"] == "polling"
            && calls(&server, "get_run_status") > 5)
            .then_some(())
    })
    .await;
    assert!(plugin.watcher().stopped().is_none());
}

/// 只读：工具没标只读的观察项要玩家明确同意；服务器给的观察项要工具返回 structuredContent，否则直接停下；
/// Kotone 预设里的观察项可以读文字里的 JSON（测试服务器叫 kotone-test-server，只给测试的预设按名字认它）。
#[tokio::test]
async fn unmarked_tools_need_consent_and_text_results_only_work_for_presets() {
    let notes = Arc::new(Mutex::new(vec![json!({"id": 1, "text": "第一条"})]));
    let list = notes.clone();
    let server = TestServer::start(
        json!([{"name": "notes", "inputSchema": {"type": "object"}},
               {"name": "status", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}}]),
        json!([]),
        handler(move |name, _| {
            let items = list.lock().unwrap().clone();
            async move {
                match name.as_str() {
                    // 只有文字（JSON 写在文字里），没有 structuredContent
                    "notes" => Ok(json!({"content": [{"type": "text", "text": json!({"notes": items}).to_string()}]})),
                    _ => Ok(json!({"content": [{"type": "text", "text": "{\"s\": 1}"}]})),
                }
            }
        }),
    )
    .await;
    let spec = ServerSpec::parse(&json!({"url": server.url()})).unwrap();
    let fp = spec.fingerprint();
    let dir = tempfile::tempdir().unwrap();
    // 玩家本地加一个观察项：工具 status（标了只读）但只回文字 → 停下
    std::fs::write(
        dir.path().join(manifest::LOCAL_FILE),
        json!({"events": [{"name": "status_changed", "label": "状态变了"}],
               "watches": [{"event": "status_changed", "tool": "status", "compare": "changed", "field": "s"}]})
        .to_string(),
    )
    .unwrap();
    let plugin = Arc::new(RemotePlugin::with_timing(
        "notes-x",
        Ok(spec),
        Some(dir.path().to_path_buf()),
        "t",
        fast(),
    ));
    plugin.set_games_probe(Arc::new(|| vec!["generic".to_string()]));
    let host = Arc::new(WatchHost::default());
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp});
    let dyn_host: Arc<dyn Host> = host.clone();
    plugin.clone().start(dyn_host.clone());
    until("认出预设", || {
        (plugin.kotone_manifest().source == Source::Preset).then_some(())
    })
    .await;
    let effective = plugin.kotone_manifest();
    assert_eq!(effective.watch_from["note_added"], Source::Preset);
    assert_eq!(effective.watch_from["status_changed"], Source::Local);
    let d = |event: &str| {
        digest(
            &fp,
            effective
                .manifest
                .watches
                .iter()
                .find(|w| w.event == event)
                .unwrap(),
        )
    };
    // notes 没标只读：只确认了内容、没明确同意 → 要确认，不查
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp, "watches": {
        "note_added": {"on": true, "confirmed": d("note_added")},
        "status_changed": {"on": true, "confirmed": d("status_changed")}}});
    plugin.settings_changed(dyn_host.clone());
    let local = until("本地的停下", || {
        let v = view_of(&plugin, "status_changed");
        (v["status"]["state"] == "stopped")
            .then(|| v["status"]["reason"].as_str().unwrap().to_string())
    })
    .await;
    assert!(local.contains("structuredContent"), "{local}");
    assert_eq!(
        calls(&server, "status"),
        1,
        "给不了结构化结果的直接停下，不重试"
    );
    let v = view_of(&plugin, "note_added");
    assert_eq!(v["status"]["state"], "confirm");
    assert_eq!(v["readOnly"], false);
    assert_eq!(calls(&server, "notes"), 0, "没标只读、没明确同意不查");
    // 明确同意了：预设的可以读文字里的 JSON；第一次记底，之后新的一条发一件事
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp, "watches": {
        "note_added": {"on": true, "confirmed": d("note_added"), "allowUnmarked": true}}});
    plugin.settings_changed(dyn_host.clone());
    until("记底", || (calls(&server, "notes") >= 2).then_some(())).await;
    assert!(host.events.lock().unwrap().is_empty());
    notes
        .lock()
        .unwrap()
        .push(json!({"id": 2, "text": "第二条", "extra": "不带"}));
    let (game, kind, _, data) = until("新的一条", || {
        host.events.lock().unwrap().first().cloned()
    })
    .await;
    assert_eq!(game, "generic");
    assert_eq!(kind, "notes-x.note_added");
    assert_eq!(Value::Object(data), json!({"id": 2, "text": "第二条"}));
    // 关掉：不查了，再打开重新记底
    *host.settings.lock().unwrap() = json!({TRUST_KEY: fp});
    plugin.settings_changed(dyn_host.clone());
    until("关了", || {
        (view_of(&plugin, "note_added")["status"]["state"] == "off").then_some(())
    })
    .await;
    let off = calls(&server, "notes");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(calls(&server, "notes"), off);
    // 不信任：查不了
    *host.settings.lock().unwrap() = json!({"watches": {
        "note_added": {"on": true, "confirmed": d("note_added"), "allowUnmarked": true}}});
    plugin.settings_changed(dyn_host.clone());
    until("没信任", || {
        (view_of(&plugin, "note_added")["status"]["state"] == "blocked").then_some(())
    })
    .await;
    assert!(plugin
        .watcher()
        .event_off(&host.settings(), true, "note_added")
        .unwrap()
        .contains("信任"));
}

/// 玩家在插件页加观察项：写进本地 manifest.json，只加；重名、写错的不写；只能删自己加的。
#[test]
fn players_add_and_remove_their_own_watches() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(manifest::LOCAL_FILE),
        "{\"games\": [\"lol\"], \"note\": \"我手写的\"}",
    )
    .unwrap();
    let event = json!({"name": "farm_done", "label": "体力用完了", "data": {"message": "它说的"}});
    let watch = json!({"event": "farm_done", "tool": "farm_status", "everySeconds": 30, "compare": "changed",
                       "field": "status", "to": ["finished"]});
    let (decl, w) = manifest::add_local(dir.path(), event.clone(), watch.clone(), &[]).unwrap();
    assert_eq!(decl.name, "farm_done");
    assert_eq!(w.every_seconds, 30);
    let parsed = manifest::read_local(dir.path()).unwrap().unwrap();
    assert_eq!(parsed.manifest.games, ["lol"], "别的不丢");
    assert_eq!(parsed.manifest.watches.len(), 1);
    assert_eq!(parsed.unknown, ["note"]);
    // 重名
    let err = manifest::add_local(
        dir.path(),
        event.clone(),
        watch.clone(),
        &["farm_done".into()],
    )
    .unwrap_err();
    assert!(err.contains("已经有"), "{err}");
    // 写错的：不写
    let bad = json!({"event": "x_done", "tool": "t", "compare": "sometimes"});
    assert!(manifest::add_local(
        dir.path(),
        json!({"name": "x_done", "label": "x"}),
        bad,
        &[]
    )
    .is_err());
    let mismatch = json!({"event": "other", "tool": "t", "compare": "new"});
    assert!(manifest::add_local(
        dir.path(),
        json!({"name": "y_done", "label": "y"}),
        mismatch,
        &[]
    )
    .is_err());
    assert_eq!(
        manifest::read_local(dir.path())
            .unwrap()
            .unwrap()
            .manifest
            .events
            .len(),
        1
    );
    // 删
    assert!(manifest::remove_local(dir.path(), "farm_done").unwrap());
    assert!(!manifest::remove_local(dir.path(), "farm_done").unwrap());
    let parsed = manifest::read_local(dir.path()).unwrap().unwrap();
    assert!(parsed.manifest.events.is_empty() && parsed.manifest.watches.is_empty());
    assert_eq!(parsed.manifest.games, ["lol"]);
}

/// 预设：一条龙按它的默认地址或者它自报的名字（`zzz_od`）认；建议的插件 id 是 `zzz-onedragon`。
#[test]
fn the_onedragon_preset_matches_its_default_url_and_name() {
    for (url, name) in [
        (Some("http://127.0.0.1:23001/mcp"), None),
        (Some("http://localhost:23001/mcp/"), None),
        (None, Some("zzz_od")),
    ] {
        let (file, parsed) = manifest::preset_for(url, name).unwrap();
        assert_eq!(file, "zzz-onedragon.json");
        assert_eq!(parsed.manifest.games, ["zzz"]);
        assert_eq!(
            manifest::preset_id(url, name).as_deref(),
            Some("zzz-onedragon")
        );
        let names: Vec<&str> = parsed
            .manifest
            .events
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["run_finished", "run_failed"]);
        assert!(parsed
            .manifest
            .watches
            .iter()
            .all(|w| w.tool == "get_run_status"));
        assert!(parsed.manifest.risk.is_some());
    }
    assert!(manifest::preset_for(Some("http://127.0.0.1:23002/mcp"), Some("zzz")).is_none());
}
