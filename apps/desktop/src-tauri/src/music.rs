//! 放歌（ADR 034）：网易云接口 + Kotone 自己的播放器（`kotone-music`）。
//!
//! - 音乐页用 `music_*` 命令（[`crate::music_cmd`]），她用 `music` 工具（[`tool`]），一次调用就出声；
//! - 状态一变就发 `kotone://music`（在放的时候约每秒一次），音乐页、左侧栏的小播放条、聊天页的扫码卡片都听它；
//! - 登录只用扫码（`type:3`），登录凭据和 Key 一样存 Windows 凭据库（隔离的 KOTONE_HOME 分开存）；
//! - 她出声时（边生成边念的回答、应答、提醒）音乐压低到 25%，说完恢复（D7）。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use kotone_core::connection::SecretStore;
use kotone_core::log;
use kotone_music::{
    Account, MusicError, MusicProvider, NeteaseProvider, PlayOutcome, Player, PlayerState, QrPoll,
    Quality, SessionStore, Track,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT: &str = "kotone://music";
/// 凭据库里的名字（KeyringSecretStore 会加上 `connection:` 前缀）。
const SECRET_ID: &str = "music:netease";
/// 扫码最多等多久（二维码本身也会过期，网易云返回 800）。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
const LOGIN_POLL: Duration = Duration::from_secs(2);

/// 登录凭据存在 Windows 凭据库（和连接的 Key 同一个地方、同一套隔离规则）。内容是
/// `{"cookies":{"MUSIC_U":…,"__csrf":…},"deviceId":…}`，是登录凭据：不进日志、不进 config.json。
/// 从没登录过的玩家不往凭据库里写（只在内存里留一个设备号），免得没用过放歌也多一条凭据。
#[derive(Default)]
struct KeyringSession {
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
        kotone_ai::secrets::KeyringSecretStore
            .get(SECRET_ID)
            .ok()
            .flatten()
    }

    fn save(&self, blob: &str) {
        *self.memory.lock().unwrap() = Some(blob.to_string());
        let stored = kotone_ai::secrets::KeyringSecretStore
            .get(SECRET_ID)
            .ok()
            .flatten()
            .is_some();
        if !has_login(blob) && !stored {
            return;
        }
        if let Err(error) = kotone_ai::secrets::KeyringSecretStore.set(SECRET_ID, blob) {
            log::log(&format!("music: 登录状态没能存进凭据库：{error}"));
        }
    }

    fn clear(&self) {
        *self.memory.lock().unwrap() = None;
        let _ = kotone_ai::secrets::KeyringSecretStore.delete(SECRET_ID);
    }
}

#[derive(Clone, Default)]
struct Login {
    /// `idle` / `waiting` / `scanned` / `expired` / `failed`
    stage: &'static str,
    qr_svg: Option<String>,
    message: Option<String>,
}

struct Music {
    app: AppHandle,
    provider: Arc<NeteaseProvider>,
    player: Player,
    account: Mutex<Option<Account>>,
    /// 启动时查过一次账号了吗（没查完时不急着说“没登录”）。
    checked: Mutex<bool>,
    login: Mutex<Login>,
    login_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

static MUSIC: OnceLock<Arc<Music>> = OnceLock::new();

fn music() -> Result<&'static Arc<Music>, String> {
    MUSIC.get().ok_or_else(|| "放歌还没准备好".to_string())
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

fn settings(app: &AppHandle) -> kotone_core::settings::MusicConfig {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .music
        .clone()
}

/// 启动时：建播放器、查一次账号（不阻塞启动）、开始“她出声时压低”的检查。
pub fn init(app: &AppHandle) {
    // 播放器用 Tauri 的 tokio 运行时（下载、每秒的进度都在上面跑）。
    let _runtime = tauri::async_runtime::handle().inner().enter();
    let session = Arc::new(KeyringSession::default());
    let logged_in_before = session.load().is_some_and(|blob| has_login(&blob));
    let provider = Arc::new(NeteaseProvider::new(session));
    let emitter = app.clone();
    let on_state: Arc<dyn Fn(PlayerState) + Send + Sync> =
        Arc::new(move |player| emit_with(&emitter, Some(player)));
    let player = Player::new(provider.clone() as Arc<dyn MusicProvider>, on_state);
    let config = settings(app);
    player.set_quality(quality_of(config.quality()));
    player.set_volume(config.volume.min(100));
    let music = Arc::new(Music {
        app: app.clone(),
        provider,
        player,
        account: Mutex::new(None),
        checked: Mutex::new(false),
        login: Mutex::new(Login {
            stage: "idle",
            ..Login::default()
        }),
        login_task: Mutex::new(None),
    });
    if MUSIC.set(music.clone()).is_err() {
        return;
    }
    // 登录过才联网查账号（没用过放歌的玩家，启动时不连网易云）。
    tauri::async_runtime::spawn(async move {
        if logged_in_before {
            refresh_account(&music).await;
        } else {
            *music.checked.lock().unwrap() = true;
        }
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move { duck_while_she_speaks(app).await });
}

async fn refresh_account(music: &Arc<Music>) {
    match music.provider.account().await {
        Ok(account) => *music.account.lock().unwrap() = account,
        Err(error) => log::log(&format!("music: 查账号失败：{error}")),
    }
    *music.checked.lock().unwrap() = true;
    emit(&music.app);
}

/// 她出声时音乐压低（D7）：每 150 毫秒看一眼，变了才通知播放器。
async fn duck_while_she_speaks(app: AppHandle) {
    let mut ducked = false;
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let Some(music) = MUSIC.get() else { continue };
        let audible = crate::voice_reply::active()
            || crate::assistant::speaking_now()
            || crate::assistant::output_busy(&app);
        if audible != ducked {
            ducked = audible;
            music.player.duck(audible);
        }
    }
}

// ---------- 状态 ----------

fn player_json(state: &PlayerState) -> Value {
    serde_json::to_value(state).unwrap_or(Value::Null)
}

fn state_with(music: &Music, player: Option<PlayerState>) -> Value {
    let player = player.unwrap_or_else(|| music.player.state());
    let login = music.login.lock().unwrap().clone();
    json!({
        "provider": "netease",
        "account": *music.account.lock().unwrap(),
        "login": {"stage": login.stage, "qrSvg": login.qr_svg, "message": login.message},
        "player": player_json(&player),
        "quality": settings(&music.app).quality(),
    })
}

pub fn state(_app: &AppHandle) -> Result<Value, String> {
    Ok(state_with(music()?, None))
}

fn emit_with(app: &AppHandle, player: Option<PlayerState>) {
    if let Some(music) = MUSIC.get() {
        let _ = app.emit(EVENT, state_with(music, player));
    }
}

fn emit(app: &AppHandle) {
    emit_with(app, None);
}

// ---------- 登录 ----------

fn logged_in(music: &Music) -> bool {
    music.account.lock().unwrap().is_some()
}

/// 开始扫码登录（已经在等就不重来）；后台每 2 秒问一次，进展经事件通知。
pub async fn login_start(app: &AppHandle) -> Result<Value, String> {
    let music = music()?.clone();
    if logged_in(&music) {
        return Ok(state_with(&music, None));
    }
    let waiting = matches!(music.login.lock().unwrap().stage, "waiting" | "scanned");
    if waiting {
        return Ok(state_with(&music, None));
    }
    let start = music
        .provider
        .login_qr_start()
        .await
        .map_err(|e| e.to_string())?;
    // 页面用 {@html} 嵌进去：只要 <svg …> 本身（去掉 XML 声明）
    let svg = start
        .svg
        .find("<svg")
        .map_or(start.svg.as_str(), |at| &start.svg[at..])
        .to_string();
    *music.login.lock().unwrap() = Login {
        stage: "waiting",
        qr_svg: Some(svg),
        message: None,
    };
    log::log("music: 开始扫码登录");
    let task_music = music.clone();
    let key = start.key.clone();
    let task = tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(LOGIN_POLL).await;
            if started.elapsed() > LOGIN_TIMEOUT {
                set_login(&task_music, "expired", None);
                break;
            }
            match task_music.provider.login_qr_poll(&key).await {
                Ok(QrPoll::Waiting) => {}
                Ok(QrPoll::Scanned) => set_login(&task_music, "scanned", None),
                Ok(QrPoll::Expired) => {
                    set_login(&task_music, "expired", None);
                    break;
                }
                Ok(QrPoll::Ok(account)) => {
                    log::log("music: 扫码登录好了");
                    *task_music.account.lock().unwrap() = Some(account);
                    *task_music.login.lock().unwrap() = Login {
                        stage: "idle",
                        ..Login::default()
                    };
                    emit(&task_music.app);
                    break;
                }
                Ok(QrPoll::Failed(message)) => {
                    log::log(&format!("music: 扫码登录没成功：{message}"));
                    set_login(&task_music, "failed", Some(message));
                    break;
                }
                // 网络抖一下不算失败，接着问
                Err(error) => log::log(&format!("music: 查扫码结果失败：{error}")),
            }
        }
    });
    if let Some(old) = music.login_task.lock().unwrap().replace(task) {
        old.abort();
    }
    emit(app);
    Ok(state_with(&music, None))
}

fn set_login(music: &Music, stage: &'static str, message: Option<String>) {
    {
        let mut login = music.login.lock().unwrap();
        login.stage = stage;
        login.message = message;
        if stage == "idle" {
            login.qr_svg = None;
        }
    }
    emit(&music.app);
}

pub fn login_cancel(app: &AppHandle) -> Result<Value, String> {
    let music = music()?;
    if let Some(task) = music.login_task.lock().unwrap().take() {
        task.abort();
    }
    *music.login.lock().unwrap() = Login {
        stage: "idle",
        ..Login::default()
    };
    emit(app);
    Ok(state_with(music, None))
}

pub async fn logout(app: &AppHandle) -> Result<Value, String> {
    let music = music()?.clone();
    music.player.stop();
    music.provider.logout().await.map_err(|e| e.to_string())?;
    *music.account.lock().unwrap() = None;
    log::log("music: 退出登录");
    emit(app);
    Ok(state_with(&music, None))
}

/// 要登录才能放（D5）：没登录就开始扫码，返回给她 / 界面的说明。
async fn need_login(app: &AppHandle, music: &Arc<Music>) -> Option<Value> {
    if !*music.checked.lock().unwrap() {
        refresh_account(music).await;
    }
    if logged_in(music) {
        return None;
    }
    let stage = match login_start(app).await {
        Ok(_) => "waiting",
        Err(error) => {
            return Some(json!({"status": "need_login", "error": error,
                "note": "网易云的扫码登录没能开始：把原因告诉玩家"}))
        }
    };
    Some(json!({
        "status": "need_login",
        "login": stage,
        "note": "玩家还没登录网易云：扫码卡片已经显示在聊天页和音乐页，请玩家用网易云音乐 App 扫一下、在手机上确认，好了再放",
    }))
}

fn error_text(error: MusicError) -> String {
    error.to_string()
}

// ---------- 浏览 ----------

pub async fn search(query: &str, offset: u32) -> Result<Value, String> {
    let music = music()?;
    let (tracks, has_more) = music
        .provider
        .search(query, offset, 30)
        .await
        .map_err(error_text)?;
    Ok(json!({"tracks": tracks, "hasMore": has_more}))
}

pub async fn home() -> Result<Value, String> {
    let music = music()?;
    let daily = music.provider.daily().await.map_err(error_text)?;
    let playlists = music.provider.playlists().await.map_err(error_text)?;
    Ok(json!({"daily": daily, "playlists": playlists}))
}

pub async fn playlist(id: &str) -> Result<Value, String> {
    let music = music()?;
    let (playlist, tracks) = music.provider.playlist(id).await.map_err(error_text)?;
    Ok(json!({"playlist": playlist, "tracks": tracks}))
}

pub fn queue() -> Result<Value, String> {
    let (tracks, index) = music()?.player.queue();
    Ok(json!({"tracks": tracks, "index": index}))
}

// ---------- 放 ----------

/// 放什么：音乐页传曲目列表；她传 query 或 from。
pub struct PlayRequest {
    pub tracks: Option<Vec<Track>>,
    pub index: usize,
    pub from: Option<String>,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub query: Option<String>,
}

async fn play_request(music: &Music, request: PlayRequest) -> Result<PlayOutcome, String> {
    if let Some(tracks) = request.tracks {
        return music
            .player
            .play(tracks, request.index, None)
            .await
            .map_err(error_text);
    }
    if let Some(query) = request.query.filter(|q| !q.trim().is_empty()) {
        return music
            .player
            .play_query(query.trim())
            .await
            .map_err(error_text);
    }
    let provider = &music.provider;
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
            return music
                .player
                .play_fm(Some("私人 FM".to_string()))
                .await
                .map_err(error_text)
        }
        Some("playlist") => {
            let id = match (request.playlist_id, request.playlist_name) {
                (Some(id), _) => id,
                (None, Some(name)) => find_playlist(music, &name).await?,
                (None, None) => return Err("放歌单要说是哪个歌单".into()),
            };
            let (playlist, tracks) = provider.playlist(&id).await.map_err(error_text)?;
            (tracks, playlist.name)
        }
        Some(other) => return Err(format!("不认识的 from：{other}")),
        None => {
            return Err("要放什么：搜歌（query）、每日推荐、我喜欢的、私人 FM 或某个歌单".into())
        }
    };
    if tracks.is_empty() {
        return Err(format!("{source}里没有歌"));
    }
    // 音乐页点了列表里的某一首：从那首开始放这一列
    let index = request.index.min(tracks.len() - 1);
    music
        .player
        .play(tracks, index, Some(source))
        .await
        .map_err(error_text)
}

/// 按名字找歌单（玩家自己的和收藏的）：完全一样的优先，再找包含的。
async fn find_playlist(music: &Music, name: &str) -> Result<String, String> {
    let lists = music.provider.playlists().await.map_err(error_text)?;
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

fn outcome_json(outcome: &PlayOutcome) -> Value {
    json!({
        "playing": outcome.started.as_ref().map(Track::title),
        "skipped": outcome.skipped.iter().map(|(track, reason)| json!({"title": track.title(), "reason": reason})).collect::<Vec<_>>(),
    })
}

/// 音乐页的“放”：要登录（D5）。
pub async fn play(app: &AppHandle, request: PlayRequest) -> Result<Value, String> {
    let music = music()?.clone();
    if let Some(login) = need_login(app, &music).await {
        return Err(login["note"]
            .as_str()
            .unwrap_or("要先扫码登录网易云")
            .to_string());
    }
    let outcome = play_request(&music, request).await?;
    if outcome.started.is_none() {
        return Err(nothing_playable(&outcome));
    }
    Ok(state_with(&music, None))
}

fn nothing_playable(outcome: &PlayOutcome) -> String {
    match outcome.skipped.first() {
        Some((track, reason)) => format!("{}放不了：{reason}", track.title()),
        None => "没有能放的歌".into(),
    }
}

/// 播放条和她的控制：`pause` / `resume` / `toggle` / `next` / `previous` / `stop` / `seek` / `volume` / `like`。
pub async fn control(
    app: &AppHandle,
    action: &str,
    value: Option<&Value>,
) -> Result<Value, String> {
    let music = music()?.clone();
    let number = value.and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    match action {
        "pause" => music.player.pause(),
        "resume" => music.player.resume(),
        "toggle" => music.player.toggle(),
        "stop" => music.player.stop(),
        "next" => {
            music.player.next().await.map_err(error_text)?;
        }
        "previous" => {
            music.player.previous().await.map_err(error_text)?;
        }
        "seek" => music.player.seek(number as u64),
        "volume" => {
            let volume = number.min(100.0) as u8;
            music.player.set_volume(volume);
            let _ = crate::settings_cmd::update_settings_from_backend(
                app,
                json!({"music": {"volume": volume}}),
            );
        }
        "like" => {
            if music.player.state().track.is_none() {
                return Err("现在没在放歌".into());
            }
            let like = value.and_then(Value::as_bool).unwrap_or(true);
            music.player.like(like).await.map_err(error_text)?;
        }
        other => return Err(format!("不认识的操作：{other}")),
    }
    emit(app);
    Ok(state_with(&music, None))
}

pub fn set_quality(app: &AppHandle, quality: &str) -> Result<Value, String> {
    let music = music()?;
    let quality = quality_of(quality);
    crate::settings_cmd::update_settings_from_backend(
        app,
        json!({"music": {"quality": quality_name(quality)}}),
    )?;
    music.player.set_quality(quality);
    emit(app);
    Ok(state_with(music, None))
}

/// 设置改了（设置页、她用 settings.update）：音质、音量跟着变。
pub fn settings_changed(
    old: &kotone_core::settings::MusicConfig,
    new: &kotone_core::settings::MusicConfig,
) {
    let Some(music) = MUSIC.get() else { return };
    if old.quality() != new.quality() {
        music.player.set_quality(quality_of(new.quality()));
    }
    if old.volume != new.volume {
        music.player.set_volume(new.volume.min(100));
    }
}

// ---------- 她的 `music` 工具 ----------

fn now_json(music: &Music) -> Value {
    let state = music.player.state();
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

/// 主会话的 `music`（ADR 034 第一节）。
pub async fn tool(app: &AppHandle, arguments: &Value) -> Result<Value, String> {
    let music = music()?.clone();
    let action = str_arg(arguments, "action").unwrap_or("now");
    match action {
        "play" => {
            if let Some(login) = need_login(app, &music).await {
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
            let outcome = play_request(&music, request).await?;
            if outcome.started.is_none() {
                return Err(nothing_playable(&outcome));
            }
            let mut out = outcome_json(&outcome);
            out["status"] = json!("playing");
            out["source"] = json!(music.player.state().source);
            Ok(out)
        }
        "now" => Ok(now_json(&music)),
        "volume" => {
            let Some(value) = arguments.get("value") else {
                return Err("volume 要带 value（0–100）".into());
            };
            control(app, "volume", Some(value)).await?;
            Ok(now_json(&music))
        }
        "like" => {
            if let Some(login) = need_login(app, &music).await {
                return Ok(login);
            }
            control(app, "like", Some(&json!(true))).await?;
            Ok(now_json(&music))
        }
        "pause" | "resume" | "next" | "previous" | "stop" => {
            if music.player.state().track.is_none() && action != "stop" {
                return Err("现在没在放歌".into());
            }
            control(app, action, None).await?;
            Ok(now_json(&music))
        }
        other => Err(format!("action 不认识：{other}")),
    }
}
