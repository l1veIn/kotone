//! 两个本机接口的 HTTP 客户端：只连 127.0.0.1。读都是 GET；写（POST / PATCH）只有“操作客户端”
//! （[`crate::control`]，数据开关 `control`，默认关）和“在客户端里发聊天”（[`crate::chat`]，数据开关 `chat`）用。
//! 证书是客户端和游戏自签的，不校验（照 LeagueAkari 的 `rejectUnauthorized: false`）。

use std::time::Duration;

use base64::Engine;
use serde_json::Value;

use crate::client::auth::LcuAuth;

/// 局内接口（Riot 官方 Live Client Data API）。
pub const LIVE_BASE: &str = "https://127.0.0.1:2999";

/// 测试用的局内接口地址（`KOTONE_LOL_LIVE`）：只认本机（`127.0.0.1` / `localhost`）的 http(s)，别的返回 None。
pub fn loopback_base(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    let local = matches!(url.host_str()?, "127.0.0.1" | "localhost");
    (local && matches!(url.scheme(), "http" | "https"))
        .then(|| url.as_str().trim_end_matches('/').to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LolError {
    /// 连不上：客户端 / 游戏没开，或者游戏还在加载。
    Unavailable,
    /// 这一次的期限到了还没回话（[`Lcu::get_until`] / [`Lcu::post_until`]）。
    Timeout,
    /// 有回应但不是 200（游戏加载时局内接口返回 404）。
    Status(u16, String),
    Other(String),
}

impl std::fmt::Display for LolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LolError::Unavailable => write!(f, "连不上（没开或还在加载）"),
            LolError::Timeout => write!(f, "没在期限内回话"),
            LolError::Status(code, message) => write!(f, "返回 {code}：{message}"),
            LolError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for LolError {}

fn client(timeout: Duration) -> Result<reqwest::Client, LolError> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .timeout(timeout)
        .build()
        .map_err(|e| LolError::Other(e.to_string()))
}

/// 发请求时的错：连不上、期限到了、别的。
fn send_error(e: reqwest::Error) -> LolError {
    if e.is_connect() {
        LolError::Unavailable
    } else if e.is_timeout() {
        LolError::Timeout
    } else {
        LolError::Other(e.to_string())
    }
}

async fn get_json(request: reqwest::RequestBuilder) -> Result<Value, LolError> {
    let response = request.send().await.map_err(send_error)?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| LolError::Other(e.to_string()))?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        return Err(LolError::Status(status.as_u16(), message));
    }
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(|e| LolError::Other(format!("不是 JSON：{e}")))
}

/// 客户端本地接口。
#[derive(Clone)]
pub struct Lcu {
    http: reqwest::Client,
    base: String,
    authorization: String,
}

impl Lcu {
    pub fn new(auth: &LcuAuth) -> Result<Self, LolError> {
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", auth.token));
        Ok(Self {
            // 别人的战绩第一次查要 1–2 秒，偶尔更慢
            http: client(Duration::from_secs(15))?,
            base: format!("https://127.0.0.1:{}", auth.port),
            authorization: format!("Basic {credentials}"),
        })
    }

    pub async fn get(&self, path: &str) -> Result<Value, LolError> {
        get_json(
            self.http
                .get(format!("{}{path}", self.base))
                .header("Authorization", &self.authorization)
                .header("Accept", "application/json"),
        )
        .await
    }

    /// 读一个文件的原始内容（素材层取图，[`crate::client::assets`]；路径由素材层先检查过，只读）。
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>, LolError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("Authorization", &self.authorization)
            .header("Accept", "image/*")
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    LolError::Unavailable
                } else {
                    LolError::Other(e.to_string())
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(LolError::Status(status.as_u16(), String::new()));
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| LolError::Other(e.to_string()))
    }

    /// LCU 的 POST。除客户端操作外，熟练度 top 查询也使用此方法。
    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, LolError> {
        let mut request = self
            .http
            .post(format!("{}{path}", self.base))
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json");
        if !body.is_null() {
            request = request.json(body);
        }
        get_json(request).await
    }

    /// 读，带这一次的期限（不用建连接时定的 15 秒）：发聊天时截止时间前一定返回。
    pub async fn get_until(&self, path: &str, timeout: Duration) -> Result<Value, LolError> {
        get_json(
            self.http
                .get(format!("{}{path}", self.base))
                .header("Authorization", &self.authorization)
                .header("Accept", "application/json")
                .timeout(timeout),
        )
        .await
    }

    /// 写，带这一次的期限，只看回的状态（发聊天，[`crate::chat`]）：2xx 就是收下了，回的内容不管。
    /// 期限到了是 [`LolError::Timeout`]：这时请求可能已经到了客户端。
    pub async fn post_until(
        &self,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<(), LolError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .map_err(send_error)?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let message = response
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        Err(LolError::Status(status.as_u16(), message))
    }

    /// 测试用：连本机的假客户端（http，不是 https）。
    #[cfg(test)]
    pub(crate) fn with_base(base: &str, token: &str) -> Self {
        let credentials = base64::engine::general_purpose::STANDARD.encode(format!("riot:{token}"));
        Self {
            http: client(Duration::from_secs(15)).unwrap(),
            base: base.trim_end_matches('/').to_string(),
            authorization: format!("Basic {credentials}"),
        }
    }

    /// 写：只给“操作客户端”的一键导入用（符文页、出装方案，[`crate::client::loadout`]）。
    pub async fn put(&self, path: &str, body: &Value) -> Result<Value, LolError> {
        get_json(
            self.http
                .put(format!("{}{path}", self.base))
                .header("Authorization", &self.authorization)
                .header("Accept", "application/json")
                .json(body),
        )
        .await
    }

    /// 写：只给“操作客户端”用（选 / 禁英雄的动作）。
    pub async fn patch(&self, path: &str, body: &Value) -> Result<Value, LolError> {
        get_json(
            self.http
                .patch(format!("{}{path}", self.base))
                .header("Authorization", &self.authorization)
                .header("Accept", "application/json")
                .json(body),
        )
        .await
    }
}

/// 局内接口。
#[derive(Clone)]
pub struct Live {
    http: reqwest::Client,
    base: String,
}

impl Live {
    pub fn new() -> Result<Self, LolError> {
        Self::with_base(LIVE_BASE)
    }

    pub fn with_base(base: &str) -> Result<Self, LolError> {
        Ok(Self {
            http: client(Duration::from_secs(3))?,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    pub async fn get(&self, path: &str) -> Result<Value, LolError> {
        get_json(
            self.http
                .get(format!("{}{path}", self.base))
                .header("Accept", "application/json"),
        )
        .await
    }

    /// 整局的数据（`/liveclientdata/allgamedata`）。
    pub async fn all_game_data(&self) -> Result<Value, LolError> {
        self.get("/liveclientdata/allgamedata").await
    }
}

/// 战绩、段位、游戏资料的路径（照 LeagueAkari 的 LCU 接口）。
pub mod paths {
    pub const PHASE: &str = "/lol-gameflow/v1/gameflow-phase";
    pub const SESSION: &str = "/lol-gameflow/v1/session";
    /// 选人阶段（双方英雄、我方分路和召唤师技能、备选席）。
    pub const CHAMP_SELECT: &str = "/lol-champ-select/v1/session";
    pub const ME: &str = "/lol-summoner/v1/current-summoner";
    pub const CHAMPIONS: &str = "/lol-game-data/assets/v1/champion-summary.json";
    pub const SPELLS: &str = "/lol-game-data/assets/v1/summoner-spells.json";
    pub const ITEMS: &str = "/lol-game-data/assets/v1/items.json";
    pub const AUGMENTS: &str = "/lol-game-data/assets/v1/cherry-augments.json";
    pub const QUEUES: &str = "/lol-game-data/assets/v1/queues.json";
    /// 符文和符文系（页面上的图标、名字）。
    pub const PERKS: &str = "/lol-game-data/assets/v1/perks.json";
    pub const PERK_STYLES: &str = "/lol-game-data/assets/v1/perkstyles.json";

    /// 读一个人的战绩一律要这一段：最近 [`MATCH_WINDOW`] 局（`begIndex=0`）。
    pub const MATCH_WINDOW: u32 = 50;

    /// 这个人最近 [`MATCH_WINDOW`] 局（新的在前，起止索引都含）。客户端按人缓存**第一次被请求的那一段**，之后不管
    /// 要哪一段都给那一份（国服 2026-11-27 实测；rank-analysis #103 / #115 也这样测到，照它所有地方都要 0..49、
    /// 自己切页）。所以插件里读战绩的地方（主页、战绩页、`lol.scout`、`lol.player`、自动卡片、选人的英雄池）
    /// 都用这一个地址；别的工具先要过小的一段时，客户端给的会少，照给的用。
    pub fn matches(puuid: &str) -> String {
        format!(
            "/lol-match-history/v1/products/lol/{puuid}/matches?begIndex=0&endIndex={}",
            MATCH_WINDOW - 1
        )
    }

    pub fn summoner(puuid: &str) -> String {
        format!("/lol-summoner/v2/summoners/puuid/{puuid}")
    }

    pub fn ranked(puuid: &str) -> String {
        format!("/lol-ranked/v1/ranked-stats/{puuid}")
    }

    /// 一局的详情（10 个人的数据）。
    pub fn game(game_id: i64) -> String {
        format!("/lol-match-history/v1/games/{game_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nothing_listening_is_unavailable() {
        // 1 号端口在测试机上不会有服务
        let live = Live::with_base("https://127.0.0.1:1").unwrap();
        assert_eq!(live.get("/x").await, Err(LolError::Unavailable));
    }

    #[test]
    fn live_override_must_be_this_machine() {
        assert_eq!(
            loopback_base("https://127.0.0.1:2998/").as_deref(),
            Some("https://127.0.0.1:2998")
        );
        assert_eq!(
            loopback_base(" http://localhost:8080 ").as_deref(),
            Some("http://localhost:8080")
        );
        for bad in [
            "",
            "127.0.0.1:2998",
            "https://example.com:2999",
            "https://127.0.0.2:2999",
            "ftp://127.0.0.1:21",
        ] {
            assert!(loopback_base(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn every_match_history_read_asks_for_the_same_window() {
        assert_eq!(
            paths::matches("p"),
            "/lol-match-history/v1/products/lol/p/matches?begIndex=0&endIndex=49"
        );
    }
}
