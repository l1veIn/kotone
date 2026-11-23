//! Kotone 放歌（ADR 034）：放歌是一个工具，她一步就能放。
//!
//! - [`MusicProvider`]：一个音乐来源（现在只有网易云 [`NeteaseProvider`]）。歌曲 ID 带来源前缀
//!   （`ne:1901371647`），以后加 QQ 音乐只是再写一个实现。
//! - 登录状态（Cookie、设备号）放在 [`SessionStore`] 里，桌面端存 Windows 凭据库。
//! - 只放玩家自己账号能放的：不“解灰”（不从别的平台拉没版权的歌），不下载歌曲文件。
//!
//! 接口和做法照 NeteaseCloudMusicApi 的维护分支 api-enhanced（MIT）、ncmapi（MIT）和
//! YesPlayMusic（MIT），见各文件开头和 `THIRD_PARTY_NOTICES.md`。

mod cache;
pub mod netease;
mod player;
mod qr;
mod random;

use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use netease::NeteaseProvider;
pub use player::{PlayOutcome, Player, PlayerState, Status, DUCK_GAIN, MAX_TRIES};

/// 一首歌。`id` 带来源前缀（`ne:12345`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
    pub playable: Playable,
    /// 放不了的原因（给玩家看的中文）；能完整放时为空。
    pub reason: Option<String>,
}

impl Track {
    /// “歌名 - 歌手”。
    pub fn title(&self) -> String {
        if self.artists.is_empty() {
            self.name.clone()
        } else {
            format!("{} - {}", self.name, self.artists.join("/"))
        }
    }
}

/// 这个账号能不能完整地放这首歌（照 YesPlayMusic 的 `isTrackPlayable`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Playable {
    Full,
    /// 要 VIP（非 VIP 只给试听片段）。
    Vip,
    /// 付费专辑。
    Paid,
    /// 没有版权或已下架。
    Unavailable,
}

/// 一个歌单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub track_count: u32,
    pub creator: String,
    /// “我喜欢的音乐”。
    pub liked: bool,
    /// 自己建的（不是收藏别人的）。
    pub mine: bool,
}

/// 登录的账号。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub vip: bool,
}

/// 音质（ADR 034 D8：默认“极高”，非 VIP 的上限）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Standard,
    #[default]
    Exhigh,
    Lossless,
}

impl Quality {
    /// 取播放链接时要的码率。
    pub fn bitrate(self) -> u32 {
        match self {
            Self::Standard => 128_000,
            Self::Exhigh => 320_000,
            Self::Lossless => 999_000,
        }
    }
}

/// 一首歌的播放链接。
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub url: String,
    pub bitrate: u32,
    /// 只是试听片段（VIP 歌给非 VIP 账号的 30–45 秒）。
    pub trial: bool,
}

/// 扫码登录轮询的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum QrPoll {
    Waiting,
    /// 扫了，等手机上确认。
    Scanned,
    Expired,
    Ok(Account),
    Failed(String),
}

/// 开始扫码登录：`content` 是二维码里的网址，`svg` 是画好的二维码。
#[derive(Debug, Clone, PartialEq)]
pub struct QrStart {
    pub key: String,
    pub content: String,
    pub svg: String,
}

/// 登录状态存在哪里（一段不透明的文字）。桌面端用 Windows 凭据库，命令行用文件。
pub trait SessionStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, blob: &str);
    fn clear(&self);
}

/// 只在内存里（测试、不想留登录状态时）。
#[derive(Default)]
pub struct MemorySessionStore(Mutex<Option<String>>);

impl SessionStore for MemorySessionStore {
    fn load(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, blob: &str) {
        *self.0.lock().unwrap() = Some(blob.to_string());
    }
    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

/// 存在一个文件里（`kotone-cli music --session <文件>`）。
pub struct FileSessionStore(pub PathBuf);

impl SessionStore for FileSessionStore {
    fn load(&self) -> Option<String> {
        std::fs::read_to_string(&self.0)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
    fn save(&self, blob: &str) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.0, blob);
    }
    fn clear(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 出错了（`Display` 是给玩家看的中文）。
#[derive(Debug, Clone, PartialEq)]
pub enum MusicError {
    /// 要先扫码登录。
    NeedLogin,
    /// 网易云说请求太频繁 / 要验证（-460、-462、8821…）：别马上重试。
    RateLimited(String),
    Network(String),
    Api {
        code: i64,
        message: String,
    },
}

impl fmt::Display for MusicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedLogin => write!(f, "要先用网易云音乐 App 扫码登录"),
            Self::RateLimited(message) => {
                write!(f, "网易云音乐说操作太频繁了，等一会儿再试（{message}）")
            }
            Self::Network(message) => write!(f, "连不上网易云音乐：{message}"),
            Self::Api { code, message } if message.is_empty() => {
                write!(f, "网易云音乐返回错误 {code}")
            }
            Self::Api { code, message } => write!(f, "网易云音乐返回错误 {code}：{message}"),
        }
    }
}

impl std::error::Error for MusicError {}

/// 一个音乐来源。
#[async_trait]
pub trait MusicProvider: Send + Sync {
    /// 来源前缀（`ne`）。
    fn id(&self) -> &'static str;
    /// 登录的账号；没登录是 `None`。
    async fn account(&self) -> Result<Option<Account>, MusicError>;
    async fn login_qr_start(&self) -> Result<QrStart, MusicError>;
    async fn login_qr_poll(&self, key: &str) -> Result<QrPoll, MusicError>;
    async fn logout(&self) -> Result<(), MusicError>;
    /// 搜歌，返回这一页和后面还有没有。
    async fn search(
        &self,
        query: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<Track>, bool), MusicError>;
    /// 播放链接；拿不到（没版权等）是 `None`。
    async fn stream(
        &self,
        track_id: &str,
        quality: Quality,
    ) -> Result<Option<StreamInfo>, MusicError>;
    /// 每日推荐（要登录）。
    async fn daily(&self) -> Result<Vec<Track>, MusicError>;
    /// 我的歌单（自己建的和收藏的，“我喜欢的音乐”排第一；要登录）。
    async fn playlists(&self) -> Result<Vec<Playlist>, MusicError>;
    async fn playlist(&self, id: &str) -> Result<(Playlist, Vec<Track>), MusicError>;
    /// “我喜欢的音乐”里的歌（要登录）。
    async fn liked(&self) -> Result<Vec<Track>, MusicError>;
    /// 私人 FM 的下一批（要登录）。
    async fn fm(&self) -> Result<Vec<Track>, MusicError>;
    /// 红心 / 取消红心（要登录）。
    async fn like(&self, track_id: &str, like: bool) -> Result<(), MusicError>;
    /// 这首歌在不在“我喜欢的音乐”里（没登录是 false）。
    async fn is_liked(&self, track_id: &str) -> Result<bool, MusicError>;
}

/// 拆开带前缀的 ID：`ne:123` → `("ne", "123")`。
pub fn split_id(id: &str) -> Option<(&str, &str)> {
    let (provider, raw) = id.split_once(':')?;
    (!provider.is_empty() && !raw.is_empty()).then_some((provider, raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_serde_shapes_match_the_frontend_contract() {
        assert_eq!(split_id("ne:123"), Some(("ne", "123")));
        assert_eq!(split_id("123"), None);
        assert_eq!(split_id("ne:"), None);
        let track = Track {
            id: "ne:1".into(),
            name: "晴天".into(),
            artists: vec!["周杰伦".into()],
            album: "叶惠美".into(),
            cover_url: None,
            duration_ms: 269_000,
            playable: Playable::Vip,
            reason: Some("要网易云 VIP".into()),
        };
        let v = serde_json::to_value(&track).unwrap();
        assert_eq!(v["coverUrl"], serde_json::Value::Null);
        assert_eq!(v["durationMs"], 269_000);
        assert_eq!(v["playable"], "vip");
        assert_eq!(track.title(), "晴天 - 周杰伦");
        assert_eq!(serde_json::to_value(Quality::Exhigh).unwrap(), "exhigh");
        assert_eq!(Quality::default(), Quality::Exhigh);
        assert_eq!(Quality::Lossless.bitrate(), 999_000);
    }

    #[test]
    fn errors_read_as_chinese_for_the_player() {
        assert!(MusicError::NeedLogin.to_string().contains("扫码登录"));
        assert_eq!(
            MusicError::Api {
                code: 400,
                message: String::new()
            }
            .to_string(),
            "网易云音乐返回错误 400"
        );
    }

    #[test]
    fn session_stores_round_trip() {
        let memory = MemorySessionStore::default();
        assert_eq!(memory.load(), None);
        memory.save("x");
        assert_eq!(memory.load().as_deref(), Some("x"));
        memory.clear();
        assert_eq!(memory.load(), None);
        let dir = tempfile::tempdir().unwrap();
        let file = FileSessionStore(dir.path().join("a").join("session.json"));
        file.save("{}");
        assert_eq!(file.load().as_deref(), Some("{}"));
        file.clear();
        assert_eq!(file.load(), None);
    }
}
