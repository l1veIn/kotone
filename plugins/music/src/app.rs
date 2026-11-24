//! 放歌插件的宿主一侧（ADR 034、038）：播放器、网易云账号、登录窗口、她的 `music` 工具和音乐页的工具。
//! 原来是桌面端的 `music.rs`，`AppHandle` 换成了插件拿到的 [`Host`]。
//!
//! - 音乐页用只给页面的 `music.*` 工具，她用 `music`，一次调用就出声；
//! - 状态一变就发插件事件 `state`（在放的时候约每秒一次），音乐页、左侧栏的小播放条、聊天页的登录卡片都听它；
//! - 登录用网易云官网的登录页（宿主开的无痕窗口），登录凭据存 Windows 凭据库（宿主的 secret，名字没变）；
//! - 她出声时（边生成边念的回答、应答、提醒）音乐压低到 25%，说完恢复（D7）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kotone_core::log;
use kotone_plugin_api::Host;
use serde_json::{json, Value};

use crate::{
    Account, MusicError, MusicProvider, NeteaseProvider, PlayOutcome, Player, PlayerState, Quality,
    SessionStore, Track,
};

/// 插件事件：音乐的状态（`{provider, account, login, player, quality}`）。
pub const EVENT_STATE: &str = "state";
/// 凭据库里的名字（宿主加上插件 id：`music:netease`，和搬进插件以前一样）。
const SECRET: &str = "netease";
/// 扫码最多等多久。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const LOGIN_POLL: Duration = Duration::from_millis(1500);

/// 插件自己的设置（`plugins.music.settings`；以前是 config.json 的 `music` 段）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MusicSettings {
    /// 音质：`standard` / `exhigh`（默认，非 VIP 的上限）/ `lossless`（要 VIP）。不认识的值按默认。
    pub quality: String,
    /// 音量 0–100。
    pub volume: u8,
}

/// 音质的取值：(值, 名字)。第一个是默认。
pub const QUALITIES: &[(&str, &str)] = &[
    ("exhigh", "极高"),
    ("standard", "标准"),
    ("lossless", "无损（要 VIP）"),
];

impl Default for MusicSettings {
    fn default() -> Self {
        Self {
            quality: QUALITIES[0].0.into(),
            volume: 70,
        }
    }
}

impl MusicSettings {
    pub fn from_settings(settings: &Value) -> Self {
        // 一个字段读不懂不让别的也作废
        let mut out = Self::default();
        if let Some(q) = settings.get("quality").and_then(Value::as_str) {
            out.quality = q.to_string();
        }
        if let Some(v) = settings.get("volume").and_then(Value::as_u64) {
            out.volume = v.min(100) as u8;
        }
        out
    }

    /// 认识的音质（不认识的按默认）。
    pub fn quality(&self) -> &str {
        QUALITIES
            .iter()
            .map(|(value, _)| *value)
            .find(|value| *value == self.quality)
            .unwrap_or(QUALITIES[0].0)
    }
}

/// 登录凭据存在 Windows 凭据库。内容是 `{"cookies":{"MUSIC_U":…,"__csrf":…},"deviceId":…}`，是登录凭据：
/// 不进日志、不进 config.json。从没登录过的玩家不往凭据库里写（只在内存里留一个设备号）。
struct KeyringSession {
    host: Arc<dyn Host>,
    memory: Mutex<Option<String>>,
}

/// 这份登录状态里有没有登录凭据。
fn has_login(blob: &str) -> bool {
    blob.contains("MUSIC_U")
}

impl SessionStore for KeyringSession {
    fn load(&self) -> Option<String> {
        if let Some(blob) = self.memory.lock().unwrap().clone() {
            return Some(blob);
        }
        self.host.secret(SECRET)
    }

    fn save(&self, blob: &str) {
        *self.memory.lock().unwrap() = Some(blob.to_string());
        let stored = self.host.secret(SECRET).is_some();
        if !has_login(blob) && !stored {
            return;
        }
        if let Err(error) = self.host.set_secret(SECRET, blob) {
            log::log(&format!("music: 登录状态没能存进凭据库：{error}"));
        }
    }

    fn clear(&self) {
        *self.memory.lock().unwrap() = None;
        self.host.delete_secret(SECRET);
    }
}

#[derive(Clone, Default)]
struct Login {
    /// `idle` / `waiting` / `needed` / `expired` / `failed`
    stage: &'static str,
    qr_svg: Option<String>,
    message: Option<String>,
}

pub struct Music {
    host: Arc<dyn Host>,
    provider: Arc<NeteaseProvider>,
    player: Player,
    account: Mutex<Option<Account>>,
    /// 启动时查过一次账号了吗（没查完时不急着说“没登录”）。
    checked: Mutex<bool>,
    login: Mutex<Login>,
    login_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 上次用到播放器上的设置（设置变了才改播放器）。
    applied: Mutex<MusicSettings>,
}

fn quality_of(value: &str) -> Quality {
    match value {
        "standard" => Quality::Standard,
        "lossless" => Quality::Lossless,
        _ => Quality::Exhigh,
    }
}

fn quality_name(quality: Quality) -> &'static str {
    match quality {
        Quality::Standard => "standard",
        Quality::Lossless => "lossless",
        Quality::Exhigh => "exhigh",
    }
}

fn settings(host: &dyn Host) -> MusicSettings {
    MusicSettings::from_settings(&host.settings())
}

impl Music {
    /// 建播放器、查一次账号（不阻塞启动）、开始“她出声时压低”的检查。要在 tokio 运行时里调（播放器的下载、
    /// 每秒的进度都在上面跑）。
    pub fn start(host: Arc<dyn Host>) -> Arc<Self> {
        let session = Arc::new(KeyringSession {
            host: host.clone(),
            memory: Mutex::new(None),
        });
        let logged_in_before = session.load().is_some_and(|blob| has_login(&blob));
        let provider = Arc::new(NeteaseProvider::new(session));
        let config = settings(host.as_ref());
        let music = Arc::new_cyclic(|weak: &std::sync::Weak<Music>| {
            let weak = weak.clone();
            let on_state: Arc<dyn Fn(PlayerState) + Send + Sync> = Arc::new(move |player| {
                if let Some(music) = weak.upgrade() {
                    music.emit_with(Some(player));
                }
            });
            let player = Player::new(provider.clone() as Arc<dyn MusicProvider>, on_state);
            player.set_quality(quality_of(config.quality()));
            player.set_volume(config.volume.min(100));
            Music {
                host: host.clone(),
                provider,
                player,
                account: Mutex::new(None),
                checked: Mutex::new(false),
                login: Mutex::new(Login {
                    stage: "idle",
                    ..Login::default()
                }),
                login_task: Mutex::new(None),
                applied: Mutex::new(config),
            }
        });
        // 登录过、插件也开着才联网查账号（没用过放歌、或关了这个插件的玩家，启动时不连网易云）。
        // 关着时先不查：打开插件（[`Music::settings_changed`]）或第一次要放时再查。
        let first = music.clone();
        let enabled = host.enabled();
        tokio::spawn(async move {
            if !logged_in_before {
                *first.checked.lock().unwrap() = true;
            } else if enabled {
                first.refresh_account().await;
            }
        });
        let ducking = music.clone();
        tokio::spawn(async move { ducking.duck_while_she_speaks().await });
        music
    }

    async fn refresh_account(&self) {
        match self.provider.account().await {
            Ok(account) => *self.account.lock().unwrap() = account,
            Err(error) => log::log(&format!("music: 查账号失败：{error}")),
        }
        *self.checked.lock().unwrap() = true;
        self.emit();
    }

    /// 她出声时音乐压低（D7）：每 150 毫秒看一眼，变了才通知播放器。
    async fn duck_while_she_speaks(&self) {
        let mut ducked = false;
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let audible = self.host.she_is_audible();
            if audible != ducked {
                ducked = audible;
                self.player.duck(audible);
            }
        }
    }

    // ---------- 状态 ----------

    fn state_with(&self, player: Option<PlayerState>) -> Value {
        let player = player.unwrap_or_else(|| self.player.state());
        let login = self.login.lock().unwrap().clone();
        json!({
            "provider": "netease",
            "account": *self.account.lock().unwrap(),
            "login": {"stage": login.stage, "qrSvg": login.qr_svg, "message": login.message},
            "player": serde_json::to_value(&player).unwrap_or(Value::Null),
            "quality": settings(self.host.as_ref()).quality(),
        })
    }

    pub fn state(&self) -> Value {
        self.state_with(None)
    }

    fn emit_with(&self, player: Option<PlayerState>) {
        self.host.emit(EVENT_STATE, self.state_with(player));
    }

    fn emit(&self) {
        self.emit_with(None);
    }

    // ---------- 登录：网易云官网的登录页（ADR 034 实现情况） ----------
    //
    // 第三方扫码（`type: 3`）在玩家手机上提示“检测到当前设备环境异常”（2026-11-23 真机，带 chainId 也一样；
    // api-enhanced #1 / #6、YesPlayMusic #2493）。官网的扫码可以：宿主弹一个小窗口打开 music.163.com 的登录页，
    // 玩家用网易云音乐 App 扫官网的二维码，Kotone 从这个窗口的 Cookie 里拿 MUSIC_U（SPlayer 也这么做）。
    // 窗口是无痕的：网页数据关窗口就没了，登录凭据只在 Kotone 自己这边（凭据库）。

    fn logged_in(&self) -> bool {
        self.account.lock().unwrap().is_some()
    }

    /// 登录窗口里 music.163.com 的 Cookie。
    async fn login_cookies(&self) -> Result<Vec<(String, String)>, String> {
        self.host.web_window_cookies("https://music.163.com/").await
    }

    /// 打开官网登录窗口，等玩家扫码（已经在等就只是把窗口调到前面）。登好了关窗口、发事件。
    pub async fn login_start(self: &Arc<Self>) -> Result<Value, String> {
        if self.logged_in() {
            return Ok(self.state());
        }
        self.host.open_web_window(LOGIN_URL, "登录网易云音乐")?;
        let waiting = self.login.lock().unwrap().stage == "waiting";
        if waiting {
            return Ok(self.state());
        }
        self.set_login(
            "waiting",
            Some("在弹出的网易云音乐窗口里，用网易云音乐 App 扫码登录".into()),
        );
        log::log("music: 打开网易云官网登录窗口");
        let music = self.clone();
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut seen = String::new();
            loop {
                tokio::time::sleep(LOGIN_POLL).await;
                if !music.host.web_window_open() {
                    // 玩家把窗口关了：不登了
                    log::log(&format!(
                        "music: 登录窗口关了（{} 秒后），不登了",
                        started.elapsed().as_secs()
                    ));
                    music.set_login("idle", None);
                    break;
                }
                if started.elapsed() > LOGIN_TIMEOUT {
                    log::log("music: 登录窗口开了 5 分钟没登上，关掉");
                    music.host.close_web_window();
                    music.set_login("expired", Some("等太久了，再点一次登录".into()));
                    break;
                }
                // 诊断：读到的 Cookie 名字变了才记一次（只记名字，不记值：MUSIC_U 是登录凭据）
                let cookies = match music.login_cookies().await {
                    Ok(cookies) => cookies,
                    Err(error) => {
                        if seen != error {
                            log::log(&format!("music: 登录窗口 {error}"));
                            seen = error;
                        }
                        continue;
                    }
                };
                let mut names: Vec<&str> = cookies.iter().map(|(name, _)| name.as_str()).collect();
                names.sort_unstable();
                let names = names.join(",");
                if seen != names {
                    log::log(&format!("music: 登录窗口里的 Cookie：{names}"));
                    seen = names;
                }
                if !cookies
                    .iter()
                    .any(|(name, value)| name == "MUSIC_U" && !value.is_empty())
                {
                    continue;
                }
                match music.provider.login_with_cookies(&cookies).await {
                    Ok(account) => {
                        log::log("music: 网易云登录好了");
                        *music.account.lock().unwrap() = Some(account);
                        music.host.close_web_window();
                        music.set_login("idle", None);
                    }
                    Err(error) => {
                        log::log(&format!("music: 拿到了登录凭据但登录没成功：{error}"));
                        music.host.close_web_window();
                        music.set_login("failed", Some(error.to_string()));
                    }
                }
                break;
            }
        });
        if let Some(old) = self.login_task.lock().unwrap().replace(task) {
            old.abort();
        }
        Ok(self.state())
    }

    fn set_login(&self, stage: &'static str, message: Option<String>) {
        {
            let mut login = self.login.lock().unwrap();
            login.stage = stage;
            login.message = message;
            login.qr_svg = None;
        }
        self.emit();
    }

    pub fn login_cancel(&self) -> Value {
        log::log("music: 取消登录");
        if let Some(task) = self.login_task.lock().unwrap().take() {
            task.abort();
        }
        self.host.close_web_window();
        self.set_login("idle", None);
        self.state()
    }

    /// 退出登录：Kotone 这边忘掉（登录窗口是无痕的，网页那边本来就没留东西；下次登录要重新扫码）。
    pub async fn logout(&self) -> Result<Value, String> {
        self.player.stop();
        self.provider.logout().await.map_err(|e| e.to_string())?;
        *self.account.lock().unwrap() = None;
        self.host.close_web_window();
        log::log("music: 退出登录");
        self.emit();
        Ok(self.state())
    }

    /// 要登录才能放（D5）。`open_window`：玩家在音乐页点的（打开官网登录窗口）；她的工具不弹窗口
    /// （通话里玩家多半在游戏里，弹窗会抢游戏的前台），只把状态设成“要登录”，聊天页出一个登录按钮。
    async fn need_login(self: &Arc<Self>, open_window: bool) -> Option<Value> {
        if !*self.checked.lock().unwrap() {
            self.refresh_account().await;
        }
        if self.logged_in() {
            return None;
        }
        if open_window {
            if let Err(error) = self.login_start().await {
                return Some(json!({"status": "need_login", "error": error}));
            }
            return Some(json!({"status": "need_login",
                "note": "要先登录网易云：已经打开网易云音乐的登录窗口，用网易云音乐 App 扫码"}));
        }
        if self.login.lock().unwrap().stage != "waiting" {
            self.set_login("needed", None);
        }
        Some(json!({
            "status": "need_login",
            "note": "玩家还没登录网易云（只要登一次）：请玩家在 Kotone 的网易云音乐插件页面或聊天页点“登录网易云音乐”，在弹出的网易云页面里用网易云音乐 App 扫码。登好了再放",
        }))
    }

    // ---------- 浏览 ----------

    pub async fn search(&self, query: &str, offset: u32) -> Result<Value, String> {
        let (tracks, has_more) = self
            .provider
            .search(query, offset, 30)
            .await
            .map_err(error_text)?;
        Ok(json!({"tracks": tracks, "hasMore": has_more}))
    }

    pub async fn home(&self) -> Result<Value, String> {
        let daily = self.provider.daily().await.map_err(error_text)?;
        let playlists = self.provider.playlists().await.map_err(error_text)?;
        Ok(json!({"daily": daily, "playlists": playlists}))
    }

    pub async fn playlist(&self, id: &str) -> Result<Value, String> {
        let (playlist, tracks) = self.provider.playlist(id).await.map_err(error_text)?;
        Ok(json!({"playlist": playlist, "tracks": tracks}))
    }

    pub fn queue(&self) -> Value {
        let (tracks, index) = self.player.queue();
        json!({"tracks": tracks, "index": index})
    }

    // ---------- 放 ----------

    async fn play_request(&self, request: PlayRequest) -> Result<PlayOutcome, String> {
        if let Some(tracks) = request.tracks {
            return self
                .player
                .play(tracks, request.index, None)
                .await
                .map_err(error_text);
        }
        if let Some(query) = request.query.filter(|q| !q.trim().is_empty()) {
            return self
                .player
                .play_query(query.trim())
                .await
                .map_err(error_text);
        }
        let provider = &self.provider;
        let (tracks, source) = match request.from.as_deref() {
            Some("daily") => (
                provider.daily().await.map_err(error_text)?,
                "每日推荐".to_string(),
            ),
            Some("liked") => (
                provider.liked().await.map_err(error_text)?,
                "我喜欢的音乐".to_string(),
            ),
            // 私人 FM：放完这一批播放器自己要下一批
            Some("fm") => {
                return self
                    .player
                    .play_fm(Some("私人 FM".to_string()))
                    .await
                    .map_err(error_text)
            }
            Some("playlist") => {
                let id = match (request.playlist_id, request.playlist_name) {
                    (Some(id), _) => id,
                    (None, Some(name)) => self.find_playlist(&name).await?,
                    (None, None) => return Err("放歌单要说是哪个歌单".into()),
                };
                let (playlist, tracks) = provider.playlist(&id).await.map_err(error_text)?;
                (tracks, playlist.name)
            }
            Some(other) => return Err(format!("不认识的 from：{other}")),
            None => {
                return Err(
                    "要放什么：搜歌（query）、每日推荐、我喜欢的、私人 FM 或某个歌单".into(),
                )
            }
        };
        if tracks.is_empty() {
            return Err(format!("{source}里没有歌"));
        }
        // 音乐页点了列表里的某一首：从那首开始放这一列
        let index = request.index.min(tracks.len() - 1);
        self.player
            .play(tracks, index, Some(source))
            .await
            .map_err(error_text)
    }

    /// 按名字找歌单（玩家自己的和收藏的）：完全一样的优先，再找包含的。
    async fn find_playlist(&self, name: &str) -> Result<String, String> {
        let lists = self.provider.playlists().await.map_err(error_text)?;
        let want = name.trim().trim_matches(['《', '》', '“', '”', '"']);
        lists
            .iter()
            .find(|p| p.name == want)
            .or_else(|| {
                lists
                    .iter()
                    .find(|p| p.name.contains(want) || want.contains(&p.name))
            })
            .map(|p| p.id.clone())
            .ok_or_else(|| {
                let names: Vec<&str> = lists.iter().take(12).map(|p| p.name.as_str()).collect();
                format!("没找到叫“{want}”的歌单；玩家的歌单有：{}", names.join("、"))
            })
    }

    /// 音乐页的“放”：要登录（D5）；玩家就在音乐页上，没登录就直接打开官网登录窗口。
    pub async fn play(self: &Arc<Self>, request: PlayRequest) -> Result<Value, String> {
        if let Some(login) = self.need_login(true).await {
            return Err(login["note"]
                .as_str()
                .or(login["error"].as_str())
                .unwrap_or("要先登录网易云")
                .to_string());
        }
        let outcome = self.play_request(request).await?;
        if outcome.started.is_none() {
            return Err(nothing_playable(&outcome));
        }
        Ok(self.state())
    }

    /// 播放条和她的控制：`pause` / `resume` / `toggle` / `next` / `previous` / `stop` / `seek` / `volume` / `like`。
    pub async fn control(&self, action: &str, value: Option<&Value>) -> Result<Value, String> {
        let number = value.and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
        match action {
            "pause" => self.player.pause(),
            "resume" => self.player.resume(),
            "toggle" => self.player.toggle(),
            "stop" => self.player.stop(),
            "next" => {
                self.player.next().await.map_err(error_text)?;
            }
            "previous" => {
                self.player.previous().await.map_err(error_text)?;
            }
            "seek" => self.player.seek(number as u64),
            "volume" => {
                let volume = number.min(100.0) as u8;
                self.player.set_volume(volume);
                self.applied.lock().unwrap().volume = volume;
                let _ = self.host.update_settings(json!({"volume": volume}));
            }
            "like" => {
                if self.player.state().track.is_none() {
                    return Err("现在没在放歌".into());
                }
                let like = value.and_then(Value::as_bool).unwrap_or(true);
                self.player.like(like).await.map_err(error_text)?;
            }
            other => return Err(format!("不认识的操作：{other}")),
        }
        self.emit();
        Ok(self.state())
    }

    pub fn set_quality(&self, quality: &str) -> Result<Value, String> {
        let quality = quality_of(quality);
        self.applied.lock().unwrap().quality = quality_name(quality).to_string();
        self.host
            .update_settings(json!({"quality": quality_name(quality)}))?;
        self.player.set_quality(quality);
        self.emit();
        Ok(self.state())
    }

    /// 设置改了（音乐页、她用 settings.update、插件开关）：音质、音量跟着变；关掉插件就停下，
    /// 打开插件时还没查过账号就查一次。
    pub fn settings_changed(self: &Arc<Self>) {
        if !self.host.enabled() {
            if self.player.state().track.is_some() {
                log::log("music: 插件关了，停下");
                self.player.stop();
            }
        } else if !*self.checked.lock().unwrap() {
            let music = self.clone();
            tokio::spawn(async move { music.refresh_account().await });
        }
        let new = settings(self.host.as_ref());
        let mut applied = self.applied.lock().unwrap();
        if applied.quality() != new.quality() {
            self.player.set_quality(quality_of(new.quality()));
        }
        if applied.volume != new.volume {
            self.player.set_volume(new.volume.min(100));
        }
        *applied = new;
    }

    // ---------- 她的 `music` 工具 ----------

    fn now_json(&self) -> Value {
        let state = self.player.state();
        json!({
            "status": state.status,
            "playing": state.track.as_ref().map(Track::title),
            "source": state.source,
            "position": format!("{} / {}", clock(state.position_ms), clock(state.duration_ms)),
            "volume": state.volume,
            "liked": state.liked,
            "queue": format!("第 {} 首，共 {} 首", state.index + 1, state.queue_len),
        })
    }

    /// 主会话的 `music`（ADR 034 第一节）。
    pub async fn tool(self: &Arc<Self>, arguments: &Value) -> Result<Value, String> {
        let action = str_arg(arguments, "action").unwrap_or("now");
        match action {
            "play" => {
                if let Some(login) = self.need_login(false).await {
                    return Ok(login);
                }
                let request = PlayRequest {
                    tracks: None,
                    index: 0,
                    from: str_arg(arguments, "from").map(str::to_string),
                    playlist_id: None,
                    playlist_name: str_arg(arguments, "playlist").map(str::to_string),
                    query: str_arg(arguments, "query").map(str::to_string),
                };
                let outcome = self.play_request(request).await?;
                if outcome.started.is_none() {
                    return Err(nothing_playable(&outcome));
                }
                let mut out = outcome_json(&outcome);
                out["status"] = json!("playing");
                out["source"] = json!(self.player.state().source);
                Ok(out)
            }
            "now" => Ok(self.now_json()),
            "volume" => {
                let Some(value) = arguments.get("value") else {
                    return Err("volume 要带 value（0–100）".into());
                };
                self.control("volume", Some(value)).await?;
                Ok(self.now_json())
            }
            "like" => {
                if let Some(login) = self.need_login(false).await {
                    return Ok(login);
                }
                self.control("like", Some(&json!(true))).await?;
                Ok(self.now_json())
            }
            "pause" | "resume" | "next" | "previous" | "stop" => {
                if self.player.state().track.is_none() && action != "stop" {
                    return Err("现在没在放歌".into());
                }
                self.control(action, None).await?;
                Ok(self.now_json())
            }
            other => Err(format!("action 不认识：{other}")),
        }
    }
}

const LOGIN_URL: &str = "https://music.163.com/#/login";

/// 放什么：音乐页传曲目列表；她传 query 或 from。
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    pub tracks: Option<Vec<Track>>,
    #[serde(default)]
    pub index: usize,
    pub from: Option<String>,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub query: Option<String>,
}

fn error_text(error: MusicError) -> String {
    error.to_string()
}

fn outcome_json(outcome: &PlayOutcome) -> Value {
    json!({
        "playing": outcome.started.as_ref().map(Track::title),
        "skipped": outcome.skipped.iter().map(|(track, reason)| json!({"title": track.title(), "reason": reason})).collect::<Vec<_>>(),
    })
}

fn nothing_playable(outcome: &PlayOutcome) -> String {
    match outcome.skipped.first() {
        Some((track, reason)) => format!("{}放不了：{reason}", track.title()),
        None => "没有能放的歌".into(),
    }
}

fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn str_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_unknown_values() {
        let s = MusicSettings::from_settings(&json!({}));
        assert_eq!((s.quality(), s.volume), ("exhigh", 70));
        let s = MusicSettings::from_settings(&json!({"quality": "lossless", "volume": 300}));
        assert_eq!((s.quality(), s.volume), ("lossless", 100));
        // 不认识的音质按默认，音量照样读
        let s = MusicSettings::from_settings(&json!({"quality": "best", "volume": 12}));
        assert_eq!((s.quality(), s.volume), ("exhigh", 12));
    }

    #[test]
    fn a_page_play_request_reads_camel_case() {
        let r: PlayRequest =
            serde_json::from_value(json!({"from": "playlist", "playlistId": "1", "index": 2}))
                .unwrap();
        assert_eq!(r.playlist_id.as_deref(), Some("1"));
        assert_eq!(r.index, 2);
        assert!(r.tracks.is_none());
    }
}
