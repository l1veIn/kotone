//! 游戏中心（ADR 036 D5）：把各适配器看到的合成一份状态，变化时发事件，分给关心的地方。
//!
//! - 适配器：`generic`（这里：每 2 秒看一次进程和前台窗口，所有有进程名的游戏配置都有）；
//!   `lol`（`lol.rs`：英雄联盟客户端开着时由它报阶段和局内的事，见 [`claim`]）。
//! - 分发：界面（`kotone://game`）；Rust 内部订阅（[`subscribe`]，对局复盘用）；自动化的 `on`；
//!   游戏一启动就把当前游戏配置切过去（D7，热词跟着变）。
//! - 纯逻辑（状态机、匹配、`for` / 每局次数 / 冷却）在 `kotone_core::games`，这里只管读进程和分发。

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use kotone_core::automation::library::{Automation, AutomationBody};
use kotone_core::games::{Armed, Fire, GameEvent, GameState, Hub, Observation, Phase, Triggers};
use kotone_core::log;
use kotone_core::profile;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast;

/// 进程和前台窗口多久看一次。
const POLL: Duration = Duration::from_secs(2);
/// “让她做”每局最多触发几次（D8：每次花一次 AI 调用）。
pub const PROMPT_PER_MATCH: u32 = 3;

#[derive(Default)]
struct Inner {
    hub: Hub,
    triggers: Triggers,
    /// 由专门的适配器报阶段的游戏（英雄联盟客户端开着时）：进程那一路不管它。
    claimed: HashSet<String>,
}

fn inner() -> &'static Mutex<Inner> {
    static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();
    INNER.get_or_init(Default::default)
}

fn channel() -> &'static broadcast::Sender<GameEvent> {
    static TX: OnceLock<broadcast::Sender<GameEvent>> = OnceLock::new();
    TX.get_or_init(|| broadcast::channel(256).0)
}

fn now_ms() -> u64 {
    kotone_ai::agent::now_ms()
}

/// Rust 内部订阅游戏事件（只收订阅以后的）。
pub fn subscribe() -> broadcast::Receiver<GameEvent> {
    channel().subscribe()
}

/// 现在在跑的游戏。
pub fn states() -> Vec<GameState> {
    inner().lock().unwrap().hub.states()
}

/// 这个游戏最近的事（旧的在前）。
pub fn recent(game: &str, limit: usize) -> Vec<GameEvent> {
    inner().lock().unwrap().hub.recent(game, limit)
}

/// 专门的适配器接手 / 放手这个游戏的阶段。放手前它自己先报一次 `Stopped`。
pub fn claim(game: &str, claimed: bool) {
    let mut inner = inner().lock().unwrap();
    if claimed {
        inner.claimed.insert(game.to_string());
    } else {
        inner.claimed.remove(game);
    }
}

/// 适配器报一个游戏这一轮的样子。
pub fn observe(app: &AppHandle, game: &str, seen: Observation) {
    let events = inner().lock().unwrap().hub.observe(game, seen, now_ms());
    dispatch(app, events);
}

/// 适配器报一件游戏专用的事（`lol.dragon_kill`……）。
pub fn push(
    app: &AppHandle,
    game: &str,
    kind: &str,
    source: &str,
    data: Map<String, Value>,
    game_time: Option<f64>,
) {
    let event = inner()
        .lock()
        .unwrap()
        .hub
        .push(game, kind, source, data, game_time, now_ms());
    dispatch(app, event.into_iter().collect());
}

/// 有事件触发的自动化；“让她做”每局有上限（D8）。
fn armed(automations: &[Automation]) -> Vec<Armed<'_>> {
    automations
        .iter()
        .filter_map(|a| {
            let trigger = a.on.as_ref()?;
            Some(Armed {
                id: &a.id,
                trigger,
                per_match: matches!(a.action, AutomationBody::Prompt(_))
                    .then_some(PROMPT_PER_MATCH),
            })
        })
        .collect()
}

fn dispatch(app: &AppHandle, events: Vec<GameEvent>) {
    if events.is_empty() {
        return;
    }
    let now = now_ms();
    let automations = crate::automation::saved_list(app);
    let fires: Vec<Fire> = {
        let armed = armed(&automations);
        let mut inner = inner().lock().unwrap();
        events
            .iter()
            .flat_map(|e| inner.triggers.on_event(&armed, e, now))
            .collect()
    };
    for event in &events {
        log::log(&format!(
            "game: {} {} {}",
            event.game,
            event.kind,
            Value::Object(event.data.clone())
        ));
        let _ = app.emit("kotone://game", event);
        let _ = channel().send(event.clone());
        if event.kind == "game.started" {
            switch_profile(app, &event.game);
        }
    }
    run(app, fires);
}

fn run(app: &AppHandle, fires: Vec<Fire>) {
    for fire in fires {
        crate::automation::run_on_event(app, &fire);
    }
}

/// D7：游戏一启动，当前游戏配置切过去（热词跟着变）；退出后不切回。
fn switch_profile(app: &AppHandle, game: &str) {
    let current = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .active_profile_id
        .clone();
    if current.as_deref() == Some(game) || profile::get(game).is_none() {
        return;
    }
    match crate::settings_cmd::update_settings_from_backend(
        app,
        serde_json::json!({"activeProfileId": game}),
    ) {
        Ok(_) => log::log(&format!("game: 当前游戏配置自动切到 {game}")),
        Err(e) => log::log(&format!("game: 切换游戏配置失败：{e}")),
    }
}

/// 启动时：开始看进程和前台窗口（`generic` 适配器），以及 `for` 的计时。
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("kotone-games".into())
        .spawn(move || loop {
            std::thread::sleep(POLL);
            poll(&app);
        });
}

fn poll(app: &AppHandle) {
    let running: HashSet<String> = kotone_platform_windows::inject::process_list()
        .into_iter()
        .map(|(_, _, name)| name.to_ascii_lowercase())
        .collect();
    let games: Vec<profile::GameProfile> = profile::list()
        .into_iter()
        .filter(|g| !g.process_names.is_empty())
        .collect();
    let claimed = inner().lock().unwrap().claimed.clone();
    for game in &games {
        if claimed.contains(&game.id) {
            continue;
        }
        let on = game
            .process_names
            .iter()
            .any(|n| running.contains(&n.to_ascii_lowercase()));
        observe(
            app,
            &game.id,
            Observation {
                phase: if on { Phase::Running } else { Phase::Stopped },
                source: "process".into(),
                ..Observation::default()
            },
        );
    }
    let foreground = kotone_platform_windows::inject::foreground_process_name().and_then(|name| {
        games
            .iter()
            .find(|g| profile::matches_process(g, &name))
            .map(|g| g.id.clone())
    });
    let now = now_ms();
    let events = inner()
        .lock()
        .unwrap()
        .hub
        .focus(foreground.as_deref(), now);
    dispatch(app, events);
    let automations = crate::automation::saved_list(app);
    let fires = {
        let armed = armed(&automations);
        inner().lock().unwrap().triggers.tick(&armed, now)
    };
    run(app, fires);
}

/// `game.state` 工具：现在在玩什么、在哪个阶段、这局多久了、最近几件事，以及每个游戏能给哪些事。
pub fn state_json(game: Option<&str>) -> Value {
    let now = now_ms();
    let states: Vec<Value> = states()
        .into_iter()
        .filter(|s| game.map_or(true, |g| g == s.game))
        .map(|s| {
            let name = profile::get(&s.game)
                .map(|p| p.display_name)
                .unwrap_or_else(|| s.game.clone());
            let recent: Vec<Value> = recent(&s.game, 8)
                .into_iter()
                .filter(|e| e.kind != "game.focused" && e.kind != "game.unfocused")
                .map(|e| {
                    serde_json::json!({
                        "type": e.kind,
                        "secondsAgo": now.saturating_sub(e.time) / 1000,
                        "gameTime": e.game_time,
                        "data": e.data,
                    })
                })
                .collect();
            serde_json::json!({
                "game": s.game,
                "name": name,
                "phase": s.phase,
                "phaseLabel": s.phase.label(),
                "nativePhase": s.native_phase,
                "focused": s.focused,
                "runningSeconds": now.saturating_sub(s.started) / 1000,
                "matchSeconds": s.match_started.map(|t| now.saturating_sub(t) / 1000),
                "attrs": s.attrs,
                "recent": recent,
            })
        })
        .collect();
    let catalog: Vec<Value> = kotone_core::games::catalog()
        .into_iter()
        .map(|(adapter, specs)| {
            serde_json::json!({
                "adapter": adapter,
                "games": if adapter == "generic" { "所有有进程名的游戏配置" } else { adapter },
                "events": specs.iter().map(|s| serde_json::json!({
                    "type": s.kind,
                    "label": s.label,
                    "data": s.fields.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect::<Map<_, _>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({"playing": states, "events": catalog})
}

/// 界面：现在在玩什么（和 `game.state` 工具同一份）。
#[tauri::command]
pub fn game_state() -> Value {
    state_json(None)
}
