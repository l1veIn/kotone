//! 网易云音乐（非官方接口，ADR 034 D2）。
//!
//! 每个接口用哪种加密、带哪些参数照 ncmapi 1.9.1（https://github.com/two-mountains/ncmapi-rs，MIT，
//! Copyright (c) 2021 TMs；2026-11-17 还在维护）：搜索、账号、歌单、每日推荐、私人 FM、红心走 weapi，
//! 扫码登录（`type: 3`，旧的 weapi `type: 1` 扫完会被 8821 风控）和取播放链接走 eapi。
//! eapi 请求头里那份 Cookie（设备号、版本号……）照 ncmapi 的 `eapi_header_cookies`；歌曲详情的 `c`
//! 参数照 api-enhanced（https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced，MIT）。
//!
//! 账号安全（照 api-enhanced）：同一个请求 2 分钟内用缓存；两次请求之间至少隔 [`MIN_GAP`]；
//! 不批量取歌词；歌单最多取前 [`MAX_PLAYLIST_TRACKS`] 首。

pub(crate) mod crypto;
pub(crate) mod parse;
pub(crate) mod session;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::cache::{TtlCache, TTL};
use crate::{
    Account, MusicError, MusicProvider, Playlist, QrPoll, QrStart, Quality, SessionStore,
    StreamInfo, Track,
};
use parse::Ctx;
use session::Session;

const BASE: &str = "https://music.163.com";
/// 取播放链接走这个域名（ncmapi 的 `STREAM_URLS`）。
const STREAM_HOST: &str = "https://interface3.music.163.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// 两次请求之间至少隔这么久（防“IP 高频”）。
pub const MIN_GAP: Duration = Duration::from_millis(150);
/// 歌单最多取这么多首（一首一首查详情，太多了容易被限制）。
pub const MAX_PLAYLIST_TRACKS: usize = 1000;
const DETAIL_CHUNK: usize = 500;

/// 网易云音乐。`store` 存登录状态（Cookie、设备号）。
pub struct NeteaseProvider {
    http: reqwest::Client,
    store: Arc<dyn SessionStore>,
    session: Mutex<Session>,
    cache: TtlCache,
    /// 账号：外层 None = 还没问过。
    account: Mutex<Option<Option<Account>>>,
    /// “我喜欢的音乐”的歌曲 ID（红心判断用）。
    liked_ids: Mutex<Option<HashSet<u64>>>,
    last_request: tokio::sync::Mutex<Option<Instant>>,
}

enum Crypto {
    Weapi,
    Eapi,
}

impl NeteaseProvider {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        let session = Session::from_blob(store.load().as_deref());
        store.save(&session.to_blob());
        Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
            store,
            session: Mutex::new(session),
            cache: TtlCache::new(TTL),
            account: Mutex::new(None),
            liked_ids: Mutex::new(None),
            last_request: tokio::sync::Mutex::new(None),
        }
    }

    fn logged_in(&self) -> bool {
        self.session.lock().unwrap().logged_in()
    }

    fn persist(&self) {
        let blob = self.session.lock().unwrap().to_blob();
        self.store.save(&blob);
    }

    /// 登录 / 退出以后：缓存的结果都是另一个身份的。
    fn forget(&self) {
        self.cache.clear();
        *self.account.lock().unwrap() = None;
        *self.liked_ids.lock().unwrap() = None;
    }

    async fn ctx(&self) -> Ctx {
        match self.account().await {
            Ok(Some(account)) => Ctx {
                logged_in: true,
                vip: account.vip,
            },
            _ => Ctx::default(),
        }
    }

    async fn uid(&self) -> Result<u64, MusicError> {
        match self.account().await? {
            Some(account) => account.id.parse().map_err(|_| MusicError::NeedLogin),
            None => Err(MusicError::NeedLogin),
        }
    }

    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(at) = *last {
            let since = at.elapsed();
            if since < MIN_GAP {
                tokio::time::sleep(MIN_GAP - since).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// eapi 请求里那份 Cookie（也放进 JSON 的 `header`）。照 ncmapi `eapi_header_cookies`。
    fn eapi_header(&self, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let session = self.session.lock().unwrap();
        let device = session.device_id.clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut header: BTreeMap<String, String> = [
            ("osver", "undefined"),
            ("deviceId", device.as_str()),
            ("appver", "8.0.0"),
            ("versioncode", "140"),
            ("mobilename", device.as_str()),
            ("resolution", "1920x1080"),
            ("__csrf", ""),
            ("os", "android"),
            ("channel", "undefined"),
        ]
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                session
                    .cookie(k)
                    .map(str::to_string)
                    .unwrap_or(v.to_string()),
            )
        })
        .collect();
        header.insert("buildver".into(), now.as_secs().to_string());
        header.insert(
            "requestId".into(),
            format!("{}_{:04}", now.as_millis(), crate::random::u64() % 1000),
        );
        for name in ["MUSIC_U", "MUSIC_A"] {
            if let Some(v) = session.cookie(name) {
                header.insert(name.into(), v.to_string());
            }
        }
        for (k, v) in extra {
            header.insert(k.to_string(), v.to_string());
        }
        header
    }

    /// 发一个请求，返回原始 JSON（不看 `code`）。`path` 是 `/api/…`。
    async fn request(
        &self,
        crypto: Crypto,
        host: &str,
        path: &str,
        mut payload: Value,
        extra_cookies: &[(&str, &str)],
        cacheable: bool,
    ) -> Result<Value, MusicError> {
        let key = format!("{host}{path}{payload}");
        if cacheable {
            if let Some(hit) = self.cache.get(&key) {
                return Ok(hit);
            }
        }
        let (url, form, cookie) = match crypto {
            Crypto::Weapi => {
                let (cookie, csrf) = {
                    let session = self.session.lock().unwrap();
                    let mut cookies: Vec<String> = session
                        .cookies
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    cookies.extend(extra_cookies.iter().map(|(k, v)| format!("{k}={v}")));
                    (
                        cookies.join("; "),
                        session.cookie("__csrf").unwrap_or("").to_string(),
                    )
                };
                if let Some(object) = payload.as_object_mut() {
                    object.insert("csrf_token".into(), Value::String(csrf));
                }
                let form = crypto::weapi(&payload.to_string());
                (
                    format!("{host}{}", path.replacen("/api/", "/weapi/", 1)),
                    vec![("params", form.params), ("encSecKey", form.enc_sec_key)],
                    cookie,
                )
            }
            Crypto::Eapi => {
                let header = self.eapi_header(extra_cookies);
                let cookie = header
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                if let Some(object) = payload.as_object_mut() {
                    object.insert("header".into(), json!(header));
                }
                (
                    format!("{host}{}", path.replacen("/api/", "/eapi/", 1)),
                    vec![("params", crypto::eapi(path, &payload.to_string()))],
                    cookie,
                )
            }
        };
        self.throttle().await;
        let response = self
            .http
            .post(&url)
            .header(reqwest::header::REFERER, BASE)
            .header(reqwest::header::ORIGIN, BASE)
            .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
            .header(reqwest::header::COOKIE, cookie)
            .form(&form)
            .send()
            .await
            .map_err(|e| MusicError::Network(e.without_url().to_string()))?;
        let mut changed = false;
        {
            let mut session = self.session.lock().unwrap();
            for value in response.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(text) = value.to_str() {
                    changed |= session.absorb(text);
                }
            }
        }
        if changed {
            self.persist();
        }
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|e| MusicError::Network(e.without_url().to_string()))?;
        let value: Value = serde_json::from_slice(&body).map_err(|_| {
            MusicError::Network(format!("{} 返回的不是 JSON（HTTP {status}）", path))
        })?;
        if cacheable && code_of(&value) == 200 {
            self.cache.insert(key, value.clone());
        }
        Ok(value)
    }

    async fn weapi(
        &self,
        path: &str,
        payload: Value,
        cacheable: bool,
    ) -> Result<Value, MusicError> {
        let v = self
            .request(Crypto::Weapi, BASE, path, payload, &[], cacheable)
            .await?;
        check(v)
    }

    async fn song_details(&self, ids: &[u64], ctx: Ctx) -> Result<Vec<Track>, MusicError> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(DETAIL_CHUNK) {
            let c = format!(
                "[{}]",
                chunk
                    .iter()
                    .map(|id| format!("{{\"id\":{id}}}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let v = self
                .weapi("/api/v3/song/detail", json!({ "c": c }), true)
                .await?;
            out.extend(parse::tracks(&v["songs"], Some(&v["privileges"]), ctx));
        }
        Ok(out)
    }

    async fn liked_set(&self) -> Result<HashSet<u64>, MusicError> {
        if let Some(set) = self.liked_ids.lock().unwrap().clone() {
            return Ok(set);
        }
        let uid = self.uid().await?;
        let v = self
            .weapi("/api/song/like/get", json!({ "uid": uid }), false)
            .await?;
        let set: HashSet<u64> = v["ids"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        *self.liked_ids.lock().unwrap() = Some(set.clone());
        Ok(set)
    }
}

fn code_of(v: &Value) -> i64 {
    v["code"].as_i64().unwrap_or(200)
}

/// 看 `code`：200 正常；301 要登录；-460 / -462 / 405 / 8821 是风控。
fn check(v: Value) -> Result<Value, MusicError> {
    let code = code_of(&v);
    if code == 200 {
        return Ok(v);
    }
    let message = ["message", "msg"]
        .iter()
        .find_map(|k| v[*k].as_str())
        .unwrap_or("")
        .to_string();
    Err(match code {
        301 => MusicError::NeedLogin,
        -460 | -462 | 405 | 8821 => MusicError::RateLimited(if message.is_empty() {
            code.to_string()
        } else {
            message
        }),
        _ => MusicError::Api { code, message },
    })
}

fn raw_id(id: &str) -> Result<u64, MusicError> {
    let raw = match crate::split_id(id) {
        Some((parse::PREFIX, raw)) => raw,
        Some(_) => {
            return Err(MusicError::Api {
                code: 0,
                message: format!("不是网易云的歌：{id}"),
            })
        }
        None => id,
    };
    raw.parse().map_err(|_| MusicError::Api {
        code: 0,
        message: format!("歌曲 ID 不对：{id}"),
    })
}

#[async_trait]
impl MusicProvider for NeteaseProvider {
    fn id(&self) -> &'static str {
        parse::PREFIX
    }

    async fn account(&self) -> Result<Option<Account>, MusicError> {
        if !self.logged_in() {
            return Ok(None);
        }
        if let Some(account) = self.account.lock().unwrap().clone() {
            return Ok(account);
        }
        let v = self.weapi("/api/nuser/account/get", json!({}), false).await;
        let account = match v {
            Ok(v) => parse::account(&v),
            Err(MusicError::NeedLogin) => None,
            Err(e) => return Err(e),
        };
        *self.account.lock().unwrap() = Some(account.clone());
        Ok(account)
    }

    async fn login_qr_start(&self) -> Result<QrStart, MusicError> {
        let v = self
            .request(
                Crypto::Eapi,
                BASE,
                "/api/login/qrcode/unikey",
                json!({ "type": 3, "noCheckToken": true }),
                &[],
                false,
            )
            .await?;
        let v = check(v)?;
        let key = v["unikey"]
            .as_str()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| MusicError::Api {
                code: code_of(&v),
                message: "没拿到二维码".into(),
            })?
            .to_string();
        let content = format!("{BASE}/login?codekey={key}");
        let svg =
            crate::qr::svg(&content).map_err(|message| MusicError::Api { code: 0, message })?;
        Ok(QrStart { key, content, svg })
    }

    async fn login_qr_poll(&self, key: &str) -> Result<QrPoll, MusicError> {
        let v = self
            .request(
                Crypto::Eapi,
                BASE,
                "/api/login/qrcode/client/login",
                json!({ "key": key, "type": 3, "noCheckToken": true }),
                &[],
                false,
            )
            .await?;
        // 有的版本把 Cookie 也放在正文里（api-enhanced 的 `cookie` 字段）。
        if let Some(text) = v["cookie"].as_str() {
            let mut changed = false;
            {
                let mut session = self.session.lock().unwrap();
                for part in text.split(";;") {
                    changed |= session.absorb(part);
                }
            }
            if changed {
                self.persist();
            }
        }
        let message = v["message"].as_str().unwrap_or("").to_string();
        Ok(match code_of(&v) {
            800 => QrPoll::Expired,
            801 => QrPoll::Waiting,
            802 => QrPoll::Scanned,
            803 => {
                self.forget();
                match self.account().await? {
                    Some(account) => QrPoll::Ok(account),
                    None => QrPoll::Failed("扫码成功了，但没拿到账号信息，再扫一次试试".into()),
                }
            }
            -460 | -462 | 8821 => QrPoll::Failed(format!(
                "网易云要求验证（{}），换个网络或过一会儿再扫",
                if message.is_empty() {
                    code_of(&v).to_string()
                } else {
                    message
                }
            )),
            code => QrPoll::Failed(if message.is_empty() {
                format!("网易云音乐返回 {code}")
            } else {
                message
            }),
        })
    }

    async fn logout(&self) -> Result<(), MusicError> {
        if self.logged_in() {
            // 告诉网易云退出（失败也照样在本机退出）。
            let _ = self
                .request(Crypto::Weapi, BASE, "/api/logout", json!({}), &[], false)
                .await;
        }
        self.session.lock().unwrap().logout();
        self.persist();
        self.forget();
        Ok(())
    }

    async fn search(
        &self,
        query: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<Track>, bool), MusicError> {
        let ctx = self.ctx().await;
        let limit = limit.clamp(1, 100);
        let v = self
            .weapi(
                "/api/cloudsearch/pc",
                json!({ "s": query, "type": 1, "limit": limit, "offset": offset, "total": true }),
                true,
            )
            .await?;
        let tracks = parse::tracks(&v["result"]["songs"], None, ctx);
        let total = v["result"]["songCount"].as_u64().unwrap_or(0);
        let more = (offset as u64 + tracks.len() as u64) < total && !tracks.is_empty();
        Ok((tracks, more))
    }

    async fn stream(
        &self,
        track_id: &str,
        quality: Quality,
    ) -> Result<Option<StreamInfo>, MusicError> {
        let id = raw_id(track_id)?;
        // 链接 20 分钟过期；不缓存，免得重试时拿到同一个过期链接。
        let v = self
            .request(
                Crypto::Eapi,
                STREAM_HOST,
                "/api/song/enhance/player/url",
                json!({ "ids": [id], "br": quality.bitrate() }),
                &[("os", "pc")],
                false,
            )
            .await?;
        let v = check(v)?;
        Ok(v["data"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["id"].as_u64() == Some(id))
                    .or(items.first())
            })
            .and_then(parse::stream))
    }

    async fn daily(&self) -> Result<Vec<Track>, MusicError> {
        if !self.logged_in() {
            return Err(MusicError::NeedLogin);
        }
        let ctx = self.ctx().await;
        let v = self
            .request(
                Crypto::Weapi,
                BASE,
                "/api/v3/discovery/recommend/songs",
                json!({}),
                &[("os", "ios")],
                true,
            )
            .await?;
        let v = check(v)?;
        Ok(parse::tracks(&v["data"]["dailySongs"], None, ctx))
    }

    async fn playlists(&self) -> Result<Vec<Playlist>, MusicError> {
        let uid = self.uid().await?;
        let v = self
            .weapi(
                "/api/user/playlist",
                json!({ "uid": uid, "limit": 1000, "offset": 0, "includeVideo": true }),
                true,
            )
            .await?;
        let mut lists: Vec<Playlist> = v["playlist"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| parse::playlist(p, Some(uid)))
                    .collect()
            })
            .unwrap_or_default();
        // “我喜欢的音乐”排第一（ADR 034 第五节）。
        lists.sort_by_key(|p| !p.liked);
        Ok(lists)
    }

    async fn playlist(&self, id: &str) -> Result<(Playlist, Vec<Track>), MusicError> {
        let raw = raw_id(id)?;
        let ctx = self.ctx().await;
        let uid = self.uid().await.ok();
        let v = self
            .weapi(
                "/api/v6/playlist/detail",
                json!({ "id": raw, "n": 100000, "s": 8 }),
                true,
            )
            .await?;
        let detail = &v["playlist"];
        let playlist = parse::playlist(detail, uid).ok_or_else(|| MusicError::Api {
            code: code_of(&v),
            message: "没找到这个歌单".into(),
        })?;
        let ids: Vec<u64> = detail["trackIds"]
            .as_array()
            .map(|a| a.iter().filter_map(|t| t["id"].as_u64()).collect())
            .unwrap_or_default();
        let inline = parse::tracks(&detail["tracks"], Some(&v["privileges"]), ctx);
        let tracks = if ids.len() > inline.len() {
            let ids: Vec<u64> = ids.into_iter().take(MAX_PLAYLIST_TRACKS).collect();
            self.song_details(&ids, ctx).await?
        } else {
            inline
        };
        Ok((playlist, tracks))
    }

    async fn liked(&self) -> Result<Vec<Track>, MusicError> {
        let lists = self.playlists().await?;
        let Some(liked) = lists.into_iter().find(|p| p.liked) else {
            return Ok(Vec::new());
        };
        Ok(self.playlist(&liked.id).await?.1)
    }

    async fn fm(&self) -> Result<Vec<Track>, MusicError> {
        if !self.logged_in() {
            return Err(MusicError::NeedLogin);
        }
        let ctx = self.ctx().await;
        let v = self.weapi("/api/v1/radio/get", json!({}), false).await?;
        Ok(parse::tracks(&v["data"], None, ctx))
    }

    async fn like(&self, track_id: &str, like: bool) -> Result<(), MusicError> {
        if !self.logged_in() {
            return Err(MusicError::NeedLogin);
        }
        let id = raw_id(track_id)?;
        let v = self
            .request(
                Crypto::Weapi,
                BASE,
                "/api/radio/like",
                json!({ "alg": "itembased", "time": 3, "like": like, "trackId": id }),
                &[("os", "pc")],
                false,
            )
            .await?;
        check(v)?;
        if let Some(set) = self.liked_ids.lock().unwrap().as_mut() {
            if like {
                set.insert(id);
            } else {
                set.remove(&id);
            }
        }
        // 歌单里的“我喜欢的音乐”变了：缓存作废。
        self.cache.clear();
        Ok(())
    }

    async fn is_liked(&self, track_id: &str) -> Result<bool, MusicError> {
        if !self.logged_in() {
            return Ok(false);
        }
        let id = raw_id(track_id)?;
        Ok(self.liked_set().await?.contains(&id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySessionStore;

    #[test]
    fn codes_map_to_errors() {
        assert!(check(json!({"code": 200, "x": 1})).is_ok());
        assert!(check(json!({"x": 1})).is_ok(), "no code = ok");
        assert_eq!(
            check(json!({"code": 301})).unwrap_err(),
            MusicError::NeedLogin
        );
        assert!(
            matches!(check(json!({"code": -460, "message": "Cheating"})).unwrap_err(), MusicError::RateLimited(m) if m == "Cheating")
        );
        assert!(
            matches!(check(json!({"code": 8821})).unwrap_err(), MusicError::RateLimited(m) if m == "8821")
        );
        assert_eq!(
            check(json!({"code": 404, "msg": "没有"})).unwrap_err(),
            MusicError::Api {
                code: 404,
                message: "没有".into()
            }
        );
    }

    #[test]
    fn ids_must_be_netease_ids() {
        assert_eq!(raw_id("ne:12").unwrap(), 12);
        assert_eq!(raw_id("12").unwrap(), 12);
        assert!(raw_id("qq:12").is_err());
        assert!(raw_id("ne:abc").is_err());
    }

    #[test]
    fn a_new_provider_persists_a_device_id_and_starts_logged_out() {
        let store = Arc::new(MemorySessionStore::default());
        let provider = NeteaseProvider::new(store.clone());
        assert!(!provider.logged_in());
        let blob = store.load().unwrap();
        let session: Session = serde_json::from_str(&blob).unwrap();
        assert_eq!(session.device_id.len(), 52);
        // 同一份存储再建一次：设备号不变
        let again = NeteaseProvider::new(store.clone());
        assert_eq!(again.session.lock().unwrap().device_id, session.device_id);
        let header = again.eapi_header(&[("os", "pc")]);
        assert_eq!(header["os"], "pc");
        assert_eq!(header["deviceId"], session.device_id);
        assert!(header.contains_key("requestId") && !header.contains_key("MUSIC_U"));
    }

    #[tokio::test]
    async fn logged_out_calls_that_need_login_say_so_without_a_request() {
        let provider = NeteaseProvider::new(Arc::new(MemorySessionStore::default()));
        assert_eq!(provider.account().await.unwrap(), None);
        assert_eq!(provider.daily().await.unwrap_err(), MusicError::NeedLogin);
        assert_eq!(provider.fm().await.unwrap_err(), MusicError::NeedLogin);
        assert_eq!(
            provider.playlists().await.unwrap_err(),
            MusicError::NeedLogin
        );
        assert_eq!(
            provider.like("ne:1", true).await.unwrap_err(),
            MusicError::NeedLogin
        );
        assert!(!provider.is_liked("ne:1").await.unwrap());
        provider.logout().await.unwrap();
    }
}
