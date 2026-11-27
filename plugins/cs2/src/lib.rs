//! CS2 插件（ADR 036 D9、ADR 047 第 3 期“自己做”）：
//!
//! - **收数据**：Valve 官方的游戏状态接口（GSI）。插件开着时在 127.0.0.1 上听一个端口（[`server`]，默认
//!   [`server::DEFAULT_PORT`]，被占了往后找，找到的存进设置 `port`），游戏每份 POST 带 cfg 里的随机令牌（设置 `token`），
//!   不对的丢掉。
//! - **写 cfg**（[`cfg`]）：默认就往 CS2 的 cfg 目录写 `gamestate_integration_kotone.cfg`（设置 `writeCfg`，缺省 true，
//!   插件页上的开关），关了、插件关了就删掉我们写的那个；只在 CS2 没开时写、删。
//! - **游戏中心适配器** `cs2`、**事件** `cs2.*`、**推送**：[`engine`] 把一份份 POST 变成阶段、事件和推送（纯逻辑）。
//! - **工具**：`cs2.state`（这一局现在的样子）、`cs2.rounds`（每回合的结果），给她也给页面；`cs2.status`、`cs2.cfg`
//!   只给页面（收数据和 cfg 的情况、cfg 的开关）。
//!
//! 只说事实：比分、回合、时间、谁赢；不评价玩家、不说该怎么打（判断归她，只经自动化）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

pub mod cfg;
pub mod engine;
pub mod gsi;
pub mod rules;
pub mod server;

use engine::{Action, Engine, GAME, PAGE_MATCH, SOURCE};

pub const TOOL_STATE: &str = "cs2.state";
pub const TOOL_ROUNDS: &str = "cs2.rounds";
pub const TOOL_STATUS: &str = "cs2.status";
pub const TOOL_CFG: &str = "cs2.cfg";
pub const PAGE_HOME: &str = "ui://kotone/cs2/home";
/// CS2 的进程名（和游戏配置一样）。
pub const PROCESS: &str = "cs2.exe";

/// 多久看一次炸弹倒数（还剩 10 秒那一句）。
const TICK: Duration = Duration::from_millis(250);
/// 多久看一次 cfg、游戏开没开、插件开没开。
const CHECK: Duration = Duration::from_secs(5);
/// 多久重新找一次 CS2 装在哪（Steam 库）。
const LOCATE: Duration = Duration::from_secs(120);

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/cs2/plugin.toml")
    })
}

pub fn specs() -> Vec<ToolSpec> {
    let empty = || json!({"type": "object", "properties": {}, "additionalProperties": false});
    vec![
        ToolSpec::new(
            TOOL_STATE,
            "CS2 这一局现在的样子：地图、模式、阶段（热身 / 对局中 / 半场换边 / 结算）、比分（知道玩家在哪边时写“我们 7 : 5 对面”）、第几回合、玩家在哪边、玩家的血量 / 护甲 / 钱和这回合的击杀、这张图的击杀 / 死亡 / 助攻 / MVP、炸弹（下了约几秒后炸、拆了、炸了）。数据来自游戏自己的接口，自己打时只有玩家自己的。没在玩、收不到数据时 status 不是 ok，text 写着为什么（照着告诉玩家），lastMatch 是上一张打完的图。",
            empty(),
        )
        .page(PAGE_MATCH)
        .instant(),
        ToolSpec::new(
            TOOL_ROUNDS,
            "CS2 这张图每回合的结果：第几回合、谁赢（CT / T）、玩家这边赢了没有（won）、怎么赢的（elimination 全灭 / bomb 炸了 / defuse 拆了 / time 时间到）、这回合以后的比分。只有 Kotone 收到数据以后打的回合。",
            empty(),
        )
        .instant(),
        ToolSpec::new(TOOL_STATUS, "CS2 页面：收数据的端口、配置文件写在哪 / 有没有写上、这一局", empty()).app_only(),
        ToolSpec::new(
            TOOL_CFG,
            "CS2 页面：打开 / 关掉“写入游戏的配置文件”（write）。关掉就删掉 Kotone 写的那个文件；CS2 开着时等它关了再写 / 删。",
            json!({"type": "object", "additionalProperties": false, "required": ["write"], "properties": {
                "write": {"type": "boolean", "description": "写不写"}
            }}),
        )
        .app_only(),
    ]
}

/// 插件自己的设置（`plugins.cs2.settings`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    /// 收数据的端口。
    pub port: u16,
    /// cfg 里的令牌（32 个十六进制字符；没有或写法不对时插件生成一个存进去）。
    pub token: String,
    /// 往游戏目录写 cfg（ADR 036 D9：默认写）。
    pub write_cfg: bool,
}

impl Prefs {
    pub fn from_settings(settings: &Value) -> Self {
        Self {
            port: settings
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p >= 1024)
                .unwrap_or(server::DEFAULT_PORT),
            token: settings
                .get("token")
                .and_then(Value::as_str)
                .filter(|t| valid_token(t))
                .unwrap_or("")
                .to_string(),
            write_cfg: settings
                .get("writeCfg")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }
}

/// 令牌的写法：32 个小写十六进制字符。
pub fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// 新的随机令牌。
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        // 系统随机数用不了（几乎不会）：用时间和地址凑，照样不是固定的
        let seed = now_ms() ^ (bytes.as_ptr() as usize as u64);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (seed.rotate_left(i as u32 * 5) as u8) ^ (i as u8).wrapping_mul(151);
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// cfg 的情况（页面显示）。
#[derive(Debug, Clone, Default)]
struct CfgView {
    /// `dir` / `not_installed` / `isolated`
    found: &'static str,
    path: String,
    state: Option<cfg::CfgState>,
    error: String,
}

#[derive(Default)]
struct Inner {
    engine: Engine,
    /// 现在在听的端口。
    listening: Option<u16>,
    bind_error: String,
    /// 收数据用的令牌（和 cfg 里的一样）。
    token: String,
    cfg: CfgView,
    located: Option<(Instant, cfg::Where)>,
    game_running: bool,
    /// 令牌不对、被丢掉的几份。
    rejected: u64,
}

/// CS2 插件。
#[derive(Default)]
pub struct Cs2Plugin {
    inner: Mutex<Inner>,
    started: AtomicBool,
}

impl Cs2Plugin {
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 收到一份 POST（服务器线程里调）。
    fn receive(&self, host: &dyn Host, payload: Value) -> server::Verdict {
        let actions = {
            let mut inner = self.inner();
            if !gsi::token_ok(&payload, &inner.token) {
                inner.rejected += 1;
                return server::Verdict::BadToken;
            }
            if !gsi::from_cs2(&payload) {
                return server::Verdict::NotCs2;
            }
            inner.engine.feed(gsi::parse(&payload), now_ms())
        };
        run(host, actions);
        server::Verdict::Accepted
    }

    /// 令牌：设置里没有（或写法不对）就生成一个存进去。返回现在的设置。
    fn prefs(&self, host: &dyn Host) -> Prefs {
        let mut prefs = Prefs::from_settings(&host.settings());
        if prefs.token.is_empty() {
            prefs.token = new_token();
            if let Err(e) = host.update_settings(json!({"token": prefs.token})) {
                self.inner().bind_error = format!("存令牌失败：{e}");
            }
        }
        self.inner().token = prefs.token.clone();
        prefs
    }

    /// CS2 装在哪（缓存 [`LOCATE`]；`fresh` 时重新找）。
    fn locate(&self, fresh: bool) -> cfg::Where {
        if let Some((at, found)) = self.inner().located.as_ref() {
            if !fresh && at.elapsed() < LOCATE {
                return found.clone();
            }
        }
        let found = cfg::locate();
        self.inner().located = Some((Instant::now(), found.clone()));
        found
    }

    /// 让 cfg 变成该有的样子（插件开着、`writeCfg` 开着 = 要；否则不要），记下情况给页面。
    fn apply_cfg(&self, host: &dyn Host, prefs: &Prefs, fresh: bool) {
        let running = !host.process_ids(PROCESS).is_empty();
        let found = self.locate(fresh);
        let want = (host.enabled() && prefs.write_cfg && !prefs.token.is_empty())
            .then(|| cfg::text(prefs.port, &prefs.token));
        let view = match &found {
            cfg::Where::Dir(dir) => {
                let result = cfg::apply(
                    dir,
                    want.as_deref(),
                    running,
                    &host.data_dir().join("backups"),
                    now_ms(),
                );
                CfgView {
                    found: "dir",
                    path: dir.join(cfg::FILE).display().to_string(),
                    state: result.as_ref().ok().copied(),
                    error: result.err().unwrap_or_default(),
                }
            }
            cfg::Where::NotInstalled => CfgView {
                found: "not_installed",
                ..CfgView::default()
            },
            cfg::Where::Isolated => CfgView {
                found: "isolated",
                ..CfgView::default()
            },
        };
        let mut inner = self.inner();
        inner.game_running = running;
        inner.cfg = view;
    }

    /// 收不到数据时为什么（给她的一句，照着告诉玩家）。
    fn off_reason(&self, prefs: &Prefs) -> String {
        let inner = self.inner();
        if inner.listening.is_none() {
            return format!(
                "Kotone 收 CS2 数据的端口没开起来{}",
                if inner.bind_error.is_empty() {
                    String::new()
                } else {
                    format!("：{}", inner.bind_error)
                }
            );
        }
        match inner.cfg.found {
            "isolated" => return "这是隔离的测试实例，没去找这台电脑上的 CS2".into(),
            "not_installed" => return "这台电脑上没找到 Steam 里的 CS2，收不到它的数据".into(),
            _ => {}
        }
        if !prefs.write_cfg {
            return "CS2 插件页上关了“写入游戏的配置文件”，收不到 CS2 的数据".into();
        }
        if !inner.cfg.error.is_empty() {
            return format!("CS2 的配置文件没写上：{}", inner.cfg.error);
        }
        match (inner.cfg.state, inner.game_running) {
            (Some(cfg::CfgState::WaitWrite), _) => {
                "CS2 开着，配置文件要等它关了才写上；下次打开 CS2 才有数据".into()
            }
            (Some(cfg::CfgState::Foreign), _) => {
                "CS2 的 cfg 目录里有一个同名的文件不是 Kotone 写的".into()
            }
            (_, false) => "CS2 没开".into(),
            (_, true) => {
                "CS2 开着，但还没收到它的数据：配置文件是这次 CS2 启动以后才写上的，要重启 CS2"
                    .into()
            }
        }
    }

    /// 页面的情况（`cs2.status`）。
    fn status(&self, host: &dyn Host) -> Value {
        let prefs = Prefs::from_settings(&host.settings());
        let reason = self.off_reason(&prefs);
        let now = now_ms();
        let inner = self.inner();
        json!({
            "enabled": host.enabled(),
            "listening": inner.listening,
            "bindError": inner.bind_error,
            "connected": inner.engine.live(),
            "lastPostMs": inner.engine.last_ms(),
            "rejected": inner.rejected,
            "gameRunning": inner.game_running,
            "offReason": if inner.engine.live() { String::new() } else { reason },
            "cfg": {
                "writeCfg": prefs.write_cfg,
                "found": inner.cfg.found,
                "path": inner.cfg.path,
                "state": inner.cfg.state.map(cfg::CfgState::as_str),
                "error": inner.cfg.error,
                "port": prefs.port,
            },
            "match": inner.engine.match_view(now),
        })
    }
}

/// 照引擎说的做（不拿着插件的锁：推送、报事件会进宿主）。
fn run(host: &dyn Host, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Claim(on) => host.game_claim(GAME, on),
            Action::Observe(o) => host.game_observe(GAME, o),
            Action::Event { kind, data } => host.game_event(GAME, kind, SOURCE, data, None),
            Action::Push(push) => {
                // 关着的推送宿主丢掉（Off），没推出去的宿主记日志
                let _ = host.push(push);
            }
        }
    }
}

/// 后台：开着就听端口、每 250 毫秒看炸弹、每 5 秒对一下 cfg；关了就不听、交回游戏中心、删 cfg。
async fn supervise(plugin: Arc<Cs2Plugin>, host: Arc<dyn Host>) {
    let mut listener: Option<(tokio::net::TcpListener, u16)> = None;
    let mut last_check: Option<Instant> = None;
    let mut last_bind_try: Option<Instant> = None;
    loop {
        let enabled = host.enabled();
        let check = last_check.map_or(true, |at| at.elapsed() >= CHECK);
        if !enabled {
            if listener.take().is_some() || plugin.inner().listening.is_some() {
                plugin.inner().listening = None;
            }
            let running = plugin.inner().game_running;
            let actions = plugin.inner().engine.release(running);
            run(host.as_ref(), actions);
            if check {
                last_check = Some(Instant::now());
                let prefs = Prefs::from_settings(&host.settings());
                plugin.apply_cfg(host.as_ref(), &prefs, false);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        if check {
            last_check = Some(Instant::now());
            let prefs = plugin.prefs(host.as_ref());
            // 端口：设置里的变了（或者还没听）就重新找
            let bound = listener.as_ref().map(|(_, p)| *p);
            // 没听上的时候 30 秒再试一次；听着别的端口（设置改了）马上换
            let retry = bound.is_some()
                || last_bind_try.map_or(true, |at| at.elapsed() >= Duration::from_secs(30));
            if bound != Some(prefs.port) && retry {
                last_bind_try = Some(Instant::now());
                listener = None;
                match server::bind_from(prefs.port).await {
                    Ok((l, port)) => {
                        {
                            let mut inner = plugin.inner();
                            inner.listening = Some(port);
                            inner.bind_error.clear();
                        }
                        listener = Some((l, port));
                        if port != prefs.port {
                            // 被占了换了一个：存进设置（settings_changed 会照新端口重写 cfg）
                            let _ = host.update_settings(json!({"port": port}));
                        }
                    }
                    Err(e) => {
                        let mut inner = plugin.inner();
                        inner.listening = None;
                        inner.bind_error = format!(
                            "{}–{} 都用不了（{e}）",
                            prefs.port,
                            prefs.port.saturating_add(server::TRIES - 1)
                        );
                    }
                }
            }
            let prefs = Prefs::from_settings(&host.settings());
            plugin.apply_cfg(host.as_ref(), &prefs, false);
        }
        let running = plugin.inner().game_running;
        let actions = plugin.inner().engine.tick(now_ms(), running);
        run(host.as_ref(), actions);
        match &listener {
            Some((l, _)) => {
                tokio::select! {
                    accepted = l.accept() => {
                        if let Ok((stream, _)) = accepted {
                            let (plugin, conn_host) = (plugin.clone(), host.clone());
                            host.spawn(Box::pin(server::serve(stream, move |payload| {
                                plugin.receive(conn_host.as_ref(), payload)
                            })));
                        }
                    }
                    _ = tokio::time::sleep(TICK) => {}
                }
            }
            None => tokio::time::sleep(TICK).await,
        }
    }
}

impl Plugin for Cs2Plugin {
    fn manifest(&self) -> &Manifest {
        manifest()
    }

    fn tools(&self) -> Vec<ToolSpec> {
        specs()
    }

    fn call<'a>(
        &'a self,
        host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            let now = now_ms();
            match tool {
                TOOL_STATE => {
                    let prefs = Prefs::from_settings(&host.settings());
                    let reason = self.off_reason(&prefs);
                    let inner = self.inner();
                    let content = inner.engine.state_for_her(now, &reason);
                    let view = inner.engine.match_view(now);
                    drop(inner);
                    let playing = view["playing"] == true;
                    let result = ToolResult::content(content);
                    Ok(if playing {
                        result.with_page(PAGE_MATCH, view)
                    } else {
                        result
                    })
                }
                TOOL_ROUNDS => {
                    let prefs = Prefs::from_settings(&host.settings());
                    let reason = self.off_reason(&prefs);
                    Ok(ToolResult::content(
                        self.inner().engine.rounds_for_her(&reason),
                    ))
                }
                TOOL_STATUS => Ok(ToolResult::content(self.status(host.as_ref()))),
                TOOL_CFG => {
                    let Some(write) = arguments.get("write").and_then(Value::as_bool) else {
                        return Ok(ToolResult::content(
                            json!({"status": "error", "text": "write 要是 true / false"}),
                        ));
                    };
                    host.update_settings(json!({"writeCfg": write}))?;
                    let prefs = Prefs::from_settings(&host.settings());
                    self.apply_cfg(host.as_ref(), &prefs, true);
                    let mut status = self.status(host.as_ref());
                    status["status"] = json!("ok");
                    Ok(ToolResult::content(status))
                }
                _ => Err(format!("CS2 插件没有工具 {tool}")),
            }
        })
    }

    fn start(self: Arc<Self>, host: Arc<dyn Host>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let task = supervise(self, host.clone());
        host.spawn(Box::pin(task));
    }

    fn settings_changed(&self, host: Arc<dyn Host>) {
        // 端口、令牌、cfg 开关、插件开关：马上对一下 cfg（端口由后台的循环重新听）
        let prefs = Prefs::from_settings(&host.settings());
        if !prefs.token.is_empty() {
            self.inner().token = prefs.token.clone();
        }
        self.apply_cfg(host.as_ref(), &prefs, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_tools_are_valid() {
        let m = manifest();
        assert_eq!(m.id, "cs2");
        assert_eq!(m.adapter.as_deref(), Some("cs2"));
        assert_eq!(m.games, ["cs2"]);
        m.check_tools(&specs()).unwrap();
        for push in &m.pushes {
            for page in [engine::PAGE_BOMB, engine::PAGE_MATCH] {
                assert!(m.page(page).is_some(), "{page}");
            }
            assert!(!push.title.is_empty());
        }
        // 给她的两个工具马上返回、主会话里有
        for name in [TOOL_STATE, TOOL_ROUNDS] {
            let spec = specs().into_iter().find(|t| t.name == name).unwrap();
            assert!(spec.instant && spec.for_model());
        }
        for name in [TOOL_STATUS, TOOL_CFG] {
            let spec = specs().into_iter().find(|t| t.name == name).unwrap();
            assert!(!spec.for_model(), "{name} 只给页面");
        }
        // 没有数据开关、没有风险和许可（GSI 是官方接口，ADR 043 D8：没有就不弹）
        assert!(m.sources.is_empty() && m.notices.is_empty());
    }

    #[test]
    fn prefs_default_to_writing_the_cfg_on_the_default_port() {
        let p = Prefs::from_settings(&json!({}));
        assert_eq!(p.port, server::DEFAULT_PORT);
        assert!(p.write_cfg);
        assert!(p.token.is_empty());
        let p = Prefs::from_settings(
            &json!({"port": 24001, "writeCfg": false, "token": "0123456789abcdef0123456789abcdef"}),
        );
        assert_eq!((p.port, p.write_cfg), (24001, false));
        assert_eq!(p.token.len(), 32);
        // 写法不对的令牌、端口当没有
        let p = Prefs::from_settings(&json!({"port": 80, "token": "short"}));
        assert_eq!(p.port, server::DEFAULT_PORT);
        assert!(p.token.is_empty());
    }

    #[test]
    fn tokens_are_random_hex() {
        let a = new_token();
        let b = new_token();
        assert!(valid_token(&a) && valid_token(&b));
        assert_ne!(a, b);
        assert!(!valid_token("0123456789ABCDEF0123456789ABCDEF"));
    }

    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        let mut expect = vec![
            ("plugin_cs2_name".to_string(), m.name.clone()),
            ("plugin_cs2_description".to_string(), m.description.clone()),
        ];
        for p in &m.pushes {
            let id = p.id.replace('-', "_");
            expect.push((format!("plugin_cs2_push_{id}_title"), p.title.clone()));
            expect.push((format!("plugin_cs2_push_{id}_when"), p.when.clone()));
            expect.push((format!("plugin_cs2_push_{id}_what"), p.what.clone()));
        }
        for e in &m.events {
            let name = e.kind.trim_start_matches("cs2.");
            expect.push((format!("plugin_cs2_event_{name}"), e.label.clone()));
        }
        for (key, text) in expect {
            assert_eq!(
                zh.get(&key).and_then(Value::as_str),
                Some(text.as_str()),
                "{key}"
            );
            assert!(
                en.get(&key)
                    .and_then(Value::as_str)
                    .is_some_and(|t| !t.is_empty()),
                "en {key}"
            );
        }
        for key in zh.keys() {
            assert!(
                key.starts_with("plugin_cs2_"),
                "插件的文案要以 plugin_cs2_ 开头：{key}"
            );
            assert!(en.contains_key(key), "en 缺 {key}");
        }
        for key in en.keys() {
            assert!(zh.contains_key(key), "zh-CN 缺 {key}");
        }
    }
}
