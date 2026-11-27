//! 插件层的测试：清单和工具对得上、文案对得上；用合成的假安装（`tempfile` 下）走一遍 run → 通知 → 推送 / 事件 → status，
//! 一键填 Webhook 的备份和只改那两个键、BetterGI 开着时不写。不碰玩家的 BetterGI。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kotone_plugin_api::{BoxFuture, Host, Observation, Plugin, Push, PushOutcome};
use serde_json::{json, Map, Value};

use super::*;

#[derive(Default)]
struct Seen {
    pushes: Vec<Push>,
    events: Vec<(String, String, Map<String, Value>)>,
    spawned: Vec<(PathBuf, Vec<String>)>,
}

struct FakeHost {
    settings: Value,
    data: PathBuf,
    running: Mutex<Vec<u32>>,
    source_on: bool,
    seen: Arc<Mutex<Seen>>,
}

impl Host for FakeHost {
    fn plugin(&self) -> &str {
        "genshin"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn source_enabled(&self, _source: &str) -> bool {
        self.source_on
    }
    fn settings(&self) -> Value {
        self.settings.clone()
    }
    fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }
    fn spawn(&self, _task: BoxFuture<'static, ()>) {}
    fn process_ids(&self, exe: &str) -> Vec<u32> {
        assert_eq!(exe, BGI_PROCESS);
        self.running.lock().unwrap().clone()
    }
    fn process_command_line(&self, _pid: u32) -> Option<String> {
        None
    }
    fn game_observe(&self, _game: &str, _observation: Observation) {}
    fn game_claim(&self, _game: &str, _claimed: bool) {}
    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        _t: Option<f64>,
    ) {
        assert_eq!(source, "bettergi");
        assert!(manifest().declares_event(kind), "{kind} 要在清单里登记");
        self.seen
            .lock()
            .unwrap()
            .events
            .push((game.into(), kind.into(), data));
    }
    fn push(&self, push: Push) -> Result<PushOutcome, String> {
        push.check(manifest())?;
        self.seen.lock().unwrap().pushes.push(push);
        Ok(PushOutcome::Posted)
    }
}

/// 合成的假安装：一个假的 BetterGI.exe、一条龙配置、配置组、计划、配置文件（键名照 0.67.0，值是假的）。
fn fake_install(root: &Path) -> PathBuf {
    let dir = root.join("BetterGI");
    for (path, text) in [
        ("BetterGI.exe", "MZ"),
        ("User/OneDragon/默认配置.json", "{}"),
        ("User/ScriptGroup/锄地.json", "{\"name\":\"锄地\",\"projects\":[{}]}"),
        ("User/Pulonia/plans/pln_a.json", "{\"id\":\"pln_a\",\"name\":\"早上\"}"),
        (
            "User/config.json",
            "{\n  \"notificationConfig\": {\n    \"webhookEnabled\": false,\n    \"webhookEndpoint\": \"\",\n    \"webhookSendTo\": \"\"\n  },\n  \"selectedOneDragonFlowConfigName\": \"默认配置\",\n  \"hotKeyConfig\": {\n    \"cancelTaskHotkey\": \"\"\n  }\n}",
        ),
        ("log/better-genshin-impact20261128.log", "[09:00:00.000] [INF] [Primary:S1:P1:T1] X\n一条龙启动\n"),
    ] {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

struct Rig {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    host: Arc<FakeHost>,
    plugin: Arc<GenshinPlugin>,
    seen: Arc<Mutex<Seen>>,
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let dir = fake_install(tmp.path());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let host = Arc::new(FakeHost {
        settings: json!({"path": dir.display().to_string()}),
        data: tmp.path().join("plugin-data"),
        running: Mutex::new(Vec::new()),
        source_on: true,
        seen: seen.clone(),
    });
    let spawned = seen.clone();
    let plugin = Arc::new(GenshinPlugin::with_spawner(Box::new(move |exe, args| {
        spawned
            .lock()
            .unwrap()
            .spawned
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(4242)
    })));
    Rig {
        _tmp: tmp,
        dir,
        host,
        plugin,
        seen,
    }
}

fn call(rig: &Rig, tool: &str, args: Value) -> ToolResult {
    let host: Arc<dyn Host> = rig.host.clone();
    let fut = rig.plugin.call(host, tool, &args);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
        .unwrap()
}

#[test]
fn manifest_and_tools_agree() {
    let m = manifest();
    assert_eq!(m.id, "genshin");
    assert_eq!(m.games, ["genshin"]);
    assert!(!m.enabled_by_default, "默认关：要玩家自己装了 BetterGI");
    assert!(
        m.source(SOURCE).is_some_and(|s| !s.risk.is_empty()),
        "风险说明在数据开关上（一个弹窗）"
    );
    m.check_tools(&specs()).unwrap();
    for t in specs() {
        assert_eq!(t.needs, [SOURCE], "{}", t.name);
        match t.name.as_str() {
            TOOL_WEBHOOK => assert!(!t.for_model() && t.for_app()),
            TOOL_RUN => {
                assert!(t.for_model() && t.instant);
                assert_eq!(
                    t.side_effects,
                    ["launches-program", "sends-game-input"],
                    "要确认"
                );
            }
            _ => assert!(
                t.for_model() && t.instant && t.side_effects.is_empty(),
                "{}",
                t.name
            ),
        }
    }
    assert!(
        specs()
            .iter()
            .find(|t| t.name == TOOL_STATUS)
            .unwrap()
            .untrusted,
        "日志里可能有别人写的字"
    );
    for word in [
        TOOL_RUN,
        TOOL_LIST,
        TOOL_STATUS,
        TOOL_STOP,
        "不要说已经停了",
    ] {
        assert!(m.agent.rule.contains(word), "{word}");
    }
    assert!(m.agent.rule.ends_with('\n') && !m.agent.rule.contains('\\'));
    // 推送和事件
    assert_eq!(m.push_spec(runs::PUSH_RUNS).unwrap().title, runs::TITLE);
    assert!(m.page(runs::PAGE_RUN).is_some() && m.page(PAGE_HOME).is_some());
    for kind in [runs::EVENT_FINISHED, runs::EVENT_FAILED] {
        assert!(m.declares_event(kind), "{kind}");
    }
}

#[test]
fn messages_match_the_manifest() {
    let zh: Map<String, Value> =
        serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
    let en: Map<String, Value> = serde_json::from_str(include_str!("../messages/en.json")).unwrap();
    let m = manifest();
    assert_eq!(zh["plugin_genshin_name"], m.name.as_str());
    assert_eq!(zh["plugin_genshin_description"], m.description.as_str());
    let source = m.source(SOURCE).unwrap();
    assert_eq!(
        zh["plugin_genshin_source_bettergi_name"],
        source.name.as_str()
    );
    assert_eq!(
        zh["plugin_genshin_source_bettergi_desc"],
        source.description.as_str()
    );
    assert_eq!(
        zh["plugin_genshin_source_bettergi_risk"],
        source.risk.as_str()
    );
    for push in &m.pushes {
        for (part, want) in [
            ("title", &push.title),
            ("when", &push.when),
            ("what", &push.what),
        ] {
            let key = format!("plugin_genshin_push_{}_{part}", push.id);
            assert_eq!(zh[&key], want.as_str(), "{key}");
        }
    }
    for event in &m.events {
        let name = event.kind.rsplit('.').next().unwrap();
        let key = format!("plugin_genshin_event_{name}");
        assert_eq!(zh[&key], event.label.as_str(), "{key}");
    }
    for notice in &m.notices {
        assert_eq!(
            zh[&format!("plugin_genshin_notice_{}_title", notice.id)],
            notice.title.as_str()
        );
        assert_eq!(
            zh[&format!("plugin_genshin_notice_{}_body", notice.id)],
            notice.body.as_str()
        );
    }
    for key in zh.keys() {
        assert!(key.starts_with("plugin_genshin_"), "{key}");
        assert!(en.contains_key(key), "en 缺 {key}");
    }
}

#[test]
fn list_reads_the_install() {
    let rig = rig();
    let v = call(&rig, TOOL_LIST, json!({})).content;
    assert_eq!(v["status"], "ok");
    assert_eq!(
        v["oneDragon"],
        json!([{"name": "默认配置", "selected": true}])
    );
    assert_eq!(v["groups"], json!([{"name": "锄地", "projects": 1}]));
    assert_eq!(v["plans"][0]["id"], "pln_a");
    // 假的 exe 读不出版本：有没有计划照文件夹
    assert_eq!(v["plansSupported"], true);
}

#[test]
fn nothing_works_without_bettergi_or_with_the_source_off() {
    let mut rig = rig();
    let missing = rig.dir.join("none");
    Arc::get_mut(&mut rig.host).unwrap().settings = json!({"path": missing.display().to_string()});
    for tool in [TOOL_LIST, TOOL_RUN, TOOL_WEBHOOK] {
        let v = call(&rig, tool, json!({"kind": "dragon"})).content;
        assert_eq!(v["status"], "not_found", "{tool}");
        assert!(v["text"].as_str().unwrap().contains("没找到 BetterGI"));
    }
    assert_eq!(
        call(&rig, TOOL_STATUS, json!({})).content["status"],
        "not_found"
    );
    Arc::get_mut(&mut rig.host).unwrap().source_on = false;
    assert_eq!(
        call(&rig, TOOL_RUN, json!({"kind": "dragon"})).content["status"],
        "disabled"
    );
    assert!(rig.seen.lock().unwrap().spawned.is_empty());
}

/// 跑一条龙：命令行只有 startOneDragon 和配置名；推一张“正在启动”的卡片；BetterGI 的通知一路更新同一张、
/// 跑完报 genshin.run_finished、给她一句；status 里看得到。
#[test]
fn run_then_notices_then_status() {
    let rig = rig();
    let v = call(
        &rig,
        TOOL_RUN,
        json!({"kind": "dragon", "name": "默认配置"}),
    )
    .content;
    assert_eq!(v["status"], "started", "{v}");
    assert_eq!(
        (v["pid"].as_u64(), v["forwarded"].as_bool()),
        (Some(4242), Some(false))
    );
    assert_eq!(v["webhookToKotone"], false);
    assert!(v["notes"].to_string().contains("Webhook"));
    {
        let seen = rig.seen.lock().unwrap();
        assert_eq!(seen.spawned.len(), 1);
        assert_eq!(seen.spawned[0].0, rig.dir.join("BetterGI.exe"));
        assert_eq!(seen.spawned[0].1, ["startOneDragon", "默认配置"]);
        assert_eq!(seen.pushes.len(), 1);
        assert_eq!(
            seen.pushes[0].card.as_ref().unwrap().data["status"],
            "starting"
        );
    }
    let host: &dyn Host = rig.host.as_ref();
    for (event, result, message) in [
        ("dragon.start", 0, "一条龙启动"),
        ("dragon.end", 0, "一条龙和配置组任务结束"),
    ] {
        let body = json!({"send_to": "", "event": event, "result": result, "timestamp": "2026-11-28T09:00:00", "screenshot": null, "message": message, "data": null});
        let notice = runs::Notice::parse(body.to_string().as_bytes()).unwrap();
        rig.plugin
            .with_tracker(host, |t| ((), t.notice(&notice, now_ms())));
    }
    {
        let seen = rig.seen.lock().unwrap();
        assert_eq!(seen.pushes.len(), 3);
        let keys: Vec<&str> = seen.pushes.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.iter().all(|k| *k == keys[0]), "同一张卡片：{keys:?}");
        assert_eq!(
            seen.pushes[2].card.as_ref().unwrap().data["status"],
            "succeeded"
        );
        assert!(seen.pushes[2].note.contains("跑完了"));
        assert_eq!(seen.events.len(), 1);
        let (game, kind, data) = &seen.events[0];
        assert_eq!(
            (game.as_str(), kind.as_str()),
            ("genshin", runs::EVENT_FINISHED)
        );
        assert_eq!(data["name"], "默认配置");
        assert_eq!(data["result"], "success");
    }
    // 存进了插件的数据目录：换一个插件实例（重启 Kotone）还记得
    let saved = std::fs::read_to_string(rig.host.data.join("runs.json")).unwrap();
    assert!(saved.contains("默认配置"));
    let status = call(&rig, TOOL_STATUS, json!({}));
    assert_eq!(status.content["recent"][0]["status"], "succeeded");
    assert_eq!(status.content["running"], false);
    let full = status.structured_content.unwrap();
    assert_eq!(full["install"]["found"], "settings");
    assert_eq!(full["log"][0]["text"], "一条龙启动");
    assert!(full["webhook"]["url"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    assert!(full["stop"]
        .as_str()
        .unwrap()
        .contains("没有给外部停止的接口"));
}

/// BetterGI 开着：一条龙、配置组不启动（开着的那个只会把窗口拿到前面）；计划交给它（转过去）。
#[test]
fn when_bettergi_is_open_only_plans_go_through() {
    let rig = rig();
    *rig.host.running.lock().unwrap() = vec![100];
    let v = call(&rig, TOOL_RUN, json!({"kind": "dragon"})).content;
    assert_eq!(v["status"], "busy");
    let v = call(&rig, TOOL_RUN, json!({"kind": "group", "name": "锄地"})).content;
    assert_eq!(v["status"], "busy");
    assert!(rig.seen.lock().unwrap().spawned.is_empty());
    let v = call(&rig, TOOL_RUN, json!({"kind": "plan", "name": "早上"})).content;
    assert_eq!(
        (v["status"].as_str(), v["forwarded"].as_bool()),
        (Some("started"), Some(true))
    );
    assert_eq!(
        rig.seen.lock().unwrap().spawned[0].1,
        ["--pulonia-run", "pln_a"]
    );
    // 名字不对：不启动
    let v = call(&rig, TOOL_RUN, json!({"kind": "plan", "name": "晚上"})).content;
    assert_eq!(v["status"], "not_found");
    assert_eq!(rig.seen.lock().unwrap().spawned.len(), 1);
}

#[test]
fn stop_says_there_is_no_stop_api_and_kills_nothing() {
    let rig = rig();
    *rig.host.running.lock().unwrap() = vec![100];
    let v = call(&rig, TOOL_STOP, json!({})).content;
    assert_eq!(v["status"], "no_stop_api");
    assert!(v["text"].as_str().unwrap().contains("不会结束它的进程"));
    assert!(v["text"].as_str().unwrap().contains("热键默认没设"));
    assert!(rig.seen.lock().unwrap().spawned.is_empty());
    assert!(GenshinPlugin::stop_text("Ctrl + F12").contains("Ctrl + F12"));
}

/// 一键填 Webhook：BetterGI 开着时不写；没开着时先备份、只改那两个键，地址是插件的接收端。
#[test]
fn filling_the_webhook_backs_up_and_changes_only_two_keys() {
    let rig = rig();
    let config = rig.dir.join("User/config.json");
    let before = std::fs::read_to_string(&config).unwrap();
    *rig.host.running.lock().unwrap() = vec![100];
    assert_eq!(
        call(&rig, TOOL_WEBHOOK, json!({"action": "fill"})).content["status"],
        "busy"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        before,
        "开着时不写"
    );
    rig.host.running.lock().unwrap().clear();

    let v = call(&rig, TOOL_WEBHOOK, json!({"action": "fill"})).content;
    assert_eq!(v["status"], "filled", "{v}");
    let url = v["url"].as_str().unwrap().to_string();
    let after = std::fs::read_to_string(&config).unwrap();
    let mut want: Value = serde_json::from_str(&before).unwrap();
    want["notificationConfig"]["webhookEnabled"] = json!(true);
    want["notificationConfig"]["webhookEndpoint"] = json!(url);
    assert_eq!(serde_json::from_str::<Value>(&after).unwrap(), want);
    assert_eq!(
        after.lines().count(),
        before.lines().count(),
        "原地改，不重排"
    );
    let backup = PathBuf::from(v["backup"].as_str().unwrap());
    assert!(backup.starts_with(rig.host.data.join("backups")));
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), before);
    // status 认得：指向 Kotone
    let status = call(&rig, TOOL_STATUS, json!({}));
    assert_eq!(status.content["webhookToKotone"], true);
    // 备份最多留 5 份
    for _ in 0..7 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        call(&rig, TOOL_WEBHOOK, json!({"action": "fill"}));
    }
    assert_eq!(
        std::fs::read_dir(rig.host.data.join("backups"))
            .unwrap()
            .count(),
        BACKUPS_KEPT
    );
}
