//! 兼容测试：“老插件”（插件标准 ADR 047 D4 第 7 条、`docs/plugin-standard.md` 第八节）。每次改宿主都跑（CI 的
//! `cargo test --workspace` 里就有）：以后改了宿主、让按旧标准写的插件不能用了，这里就红。
//!
//! **规矩**
//!
//! 1. `tests/old-plugins/<名字>/snapshot.json` 是插件**原样**的回答（握手、`tools/list`、`resources/read`、观察项那次调用的
//!    结果），`expected.json` 是那时候宿主该有的样子（清单的提示、观察项发出的事件）。**这些快照不许为了让测试过而改。**
//!    测试红了，是宿主改坏了：改宿主，不改快照。
//! 2. 标准真的改了含义（D4 第 5 条：只对 `standard` 写了新版本的插件生效），照新版本**另加**一份老插件
//!    （如 `0.2-minimal/`），旧的留着、照旧要过。
//! 3. 每个标准版本一份最小插件（`0.1-minimal/`，手写，照标准文档第六节的例子），加真实插件的快照
//!    （`steam-snapshot/`：`l1veIn/kotone-plugin-steam`，出处和提交写在快照的 `about` 里）。只录假数据，不录玩家的东西。
//!
//! 每份老插件过三关：宿主读清单（提示和 `expected.json` 记着的一样）→ 宿主的观察项真跑起来（假服务器
//! `examples/old_plugin_server.rs` 照快照回答：第一次记底，第二次换一份结果）发出预期的事件 → 检查命令
//! （[`kotone_remote_plugin::check`]，就是 `kotone-cli plugin check`）对它没有 ✗。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_plugin_api::{BoxFuture, Host, Observation, Plugin};
use kotone_remote_plugin::check::{self, Level};
use kotone_remote_plugin::manifest::{self, Source};
use kotone_remote_plugin::watch::{digest, Timing, EVENT_SOURCE_PREFIX};
use kotone_remote_plugin::{RemotePlugin, ServerSpec, TRUST_KEY};
use serde_json::{json, Map, Value};

/// 收到的一件事：游戏、类型、来源、数据。
type Sent = (String, String, String, Map<String, Value>);

/// 测试用的宿主：开关开着、设置（信任和观察项的决定）、收事件。
struct OldHost {
    id: String,
    settings: Mutex<Value>,
    events: Mutex<Vec<Sent>>,
}

impl Host for OldHost {
    fn plugin(&self) -> &str {
        &self.id
    }
    fn enabled(&self) -> bool {
        true
    }
    fn source_enabled(&self, _: &str) -> bool {
        false
    }
    fn settings(&self) -> Value {
        self.settings.lock().unwrap().clone()
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

/// `target/debug/examples/old_plugin_server.exe`（`cargo test -p kotone-remote-plugin` / `--workspace` 会先编译 examples；
/// 测试程序放在哪随 cargo 版本变，所以从测试程序往上一级级找，同 `src/tests.rs` 的 `server_exe`）。
fn server_exe() -> PathBuf {
    let name = format!("old_plugin_server{}", std::env::consts::EXE_SUFFIX);
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
        "找不到 {name}：用 `cargo test -p kotone-remote-plugin` 跑（会先编译 examples/old_plugin_server.rs）。找过：\n{}",
        tried.join("\n")
    );
}

/// 一秒 = 1 毫秒；下限 20 / 40 毫秒（照 `src/watch_tests.rs`）。
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "等太久了：{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}：{e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}：{e}", path.display()))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 全部老插件（`tests/old-plugins/` 下每个目录一份）。
fn old_plugins() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("old-plugins");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("snapshot.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// 第一关：宿主读清单（服务器给的原文 → 和宿主一样合成，预设照样认）。
fn host_reads_the_manifest(name: &str, snapshot: &Value, expected: &Value) -> manifest::Effective {
    let text = snapshot["reads"][manifest::MANIFEST_URI][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}：快照里没有 {} 的原文", manifest::MANIFEST_URI));
    let server_name = snapshot["serverInfo"]["name"].as_str();
    let preset = manifest::preset_for(None, server_name).map(|(_, parsed)| parsed);
    let effective = manifest::merge(Some(manifest::parse_text(text)), preset, None);
    assert_eq!(effective.source, Source::Server, "{name}：清单是服务器给的");
    let texts = |notes: &[manifest::Note]| notes.iter().map(|n| n.text.clone()).collect::<Vec<_>>();
    assert_eq!(
        texts(&effective.unknown),
        strings(&expected["unknown"]),
        "{name}：不认识的字段和那时候不一样了"
    );
    assert_eq!(
        texts(&effective.problems),
        strings(&expected["problems"]),
        "{name}：清单的问题和那时候不一样了"
    );
    effective
}

/// 第二关：宿主的观察项真跑起来，发出预期的事件（之后结果不变，不再发）。
async fn watches_send_the_events(
    name: &str,
    id: &str,
    spec: &ServerSpec,
    effective: &manifest::Effective,
    expected: &Value,
) {
    let dir = tempfile::tempdir().unwrap();
    let fp = spec.fingerprint();
    let plugin = Arc::new(RemotePlugin::with_timing(
        id,
        Ok(spec.clone()),
        Some(dir.path().to_path_buf()),
        "old-plugins",
        fast(),
    ));
    let games = effective.manifest.games.clone();
    plugin.set_games_probe(Arc::new(move || games.clone()));
    let host = Arc::new(OldHost {
        id: id.to_string(),
        settings: Mutex::new(json!({ TRUST_KEY: fp })),
        events: Mutex::new(Vec::new()),
    });
    let dyn_host: Arc<dyn Host> = host.clone();
    plugin.clone().start(dyn_host.clone());
    until(&format!("{name}：连上、读到清单"), || {
        (plugin.kotone_manifest().source == Source::Server).then_some(())
    })
    .await;
    // 玩家把每个观察项都打开、确认了（确认的就是这份内容）
    let mut choices = Map::new();
    for watch in &plugin.kotone_manifest().manifest.watches {
        choices.insert(
            watch.event.clone(),
            json!({"on": true, "confirmed": digest(&fp, watch)}),
        );
    }
    *host.settings.lock().unwrap() = json!({ TRUST_KEY: fp, "watches": choices });
    plugin.settings_changed(dyn_host.clone());

    let wanted = expected["events"].as_array().cloned().unwrap_or_default();
    until(&format!("{name}：发出 {} 件事", wanted.len()), || {
        (host.events.lock().unwrap().len() >= wanted.len()).then_some(())
    })
    .await;
    // 结果不再变：多等几轮，不该再发
    tokio::time::sleep(Duration::from_millis(300)).await;
    let sent = host.events.lock().unwrap().clone();
    let game = effective
        .manifest
        .games
        .first()
        .cloned()
        .unwrap_or_default();
    for (g, _, source, _) in &sent {
        assert_eq!(*g, game, "{name}：事件挂在插件的游戏上");
        assert_eq!(*source, format!("{EVENT_SOURCE_PREFIX}{id}"), "{name}");
    }
    let mut got: Vec<Value> = sent
        .into_iter()
        .map(|(_, kind, _, data)| json!({"type": kind, "data": data}))
        .collect();
    let mut want = wanted;
    let key = |v: &Value| v["type"].as_str().unwrap_or_default().to_string();
    got.sort_by_key(key);
    want.sort_by_key(key);
    assert_eq!(got, want, "{name}：观察项发出的事件和那时候不一样了");
    for view in plugin.watcher().view() {
        assert_eq!(view["status"]["state"], "polling", "{name}：{view}");
        assert_eq!(view["failures"], 0, "{name}：{view}");
    }
    plugin.server().disconnect().await;
}

/// 第三关：检查命令（`kotone-cli plugin check` 用的同一份）没有 ✗。
async fn check_has_no_failures(name: &str, id: &str, spec: &ServerSpec) {
    let config = spec
        .resolve(&|_| None)
        .expect("老插件不要 Key")
        .client("kotone", "old-plugins");
    let report = check::run(&check::Options {
        id: id.to_string(),
        spec: spec.clone(),
        config,
        known_games: None,
        modern: true,
    })
    .await;
    let failures: Vec<String> = report
        .lines()
        .filter(|(_, l)| l.level == Level::Fail)
        .map(|(section, l)| format!("{section}：{}", l.text))
        .collect();
    assert!(
        failures.is_empty(),
        "{name}：检查命令报了 ✗：\n{}",
        failures.join("\n")
    );
    assert!(report.manifest.is_some(), "{name}：连上了");
    assert_eq!(report.exit_code(false), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_plugins_still_work() {
    let dirs = old_plugins();
    let names: Vec<String> = dirs
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for required in ["0.1-minimal", "steam-snapshot"] {
        assert!(
            names.iter().any(|n| n == required),
            "少了老插件 {required}：{names:?}"
        );
    }
    let exe = server_exe();
    for (dir, name) in dirs.iter().zip(&names) {
        let snapshot = read_json(&dir.join("snapshot.json"));
        let expected = read_json(&dir.join("expected.json"));
        let id = expected["plugin"].as_str().unwrap_or("plugin").to_string();
        let effective = host_reads_the_manifest(name, &snapshot, &expected);
        let spec = ServerSpec::parse(&json!({
            "command": exe.to_string_lossy(),
            "args": [dir.join("snapshot.json").to_string_lossy()],
        }))
        .unwrap();
        watches_send_the_events(name, &id, &spec, &effective, &expected).await;
        check_has_no_failures(name, &id, &spec).await;
    }
}
