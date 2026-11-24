//! 英雄联盟插件的“用网上的统计”部分（数据开关 `stats`，[调研](../../../../docs/research/2026-11-23-lol-data-sources.md)）：
//!
//! - [`qq101`]：腾讯 101 / 掌盟（国服的峡谷强度榜、克制、出装、符文、加点、召唤师技能，海斗的英雄榜和海克斯榜），首选；
//! - [`aramhex`]：czgreat/aramhex-data（海斗英雄 × 海克斯胜率，CC BY-NC，页面上署名）；
//! - [`nicknames`]：腾讯 `hero_list.js` 的外号。
//!
//! 都是没有文档的公开接口，随时可能改：解析照 LeagueAkari 容错，请求失败时用磁盘上旧的那份（标出是哪天的）。
//! 都按需请求（打开页面、她问的时候），不批量抓；内存和磁盘各缓存一份（101 的 6 小时，aramhex 按 `latest.json` 的
//! SHA-256，外号 1 天）。只读，不带任何账号信息。

pub mod aramhex;
pub mod nicknames;
pub mod opgg;
pub mod qq101;
pub mod statics;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use sha2::{Digest, Sha256};

/// 101 的数据一天更新一次：6 小时内用缓存。
const TTL_101: Duration = Duration::from_secs(6 * 3600);
/// aramhex 的 `latest.json` 多久再看一次。
const TTL_LATEST: Duration = Duration::from_secs(6 * 3600);
/// 外号。
const TTL_NICKNAMES: Duration = Duration::from_secs(24 * 3600);
/// OP.GG：和 OP.GG 说好的 24 小时（issue #26）。
const TTL_OPGG: Duration = Duration::from_secs(24 * 3600);
const TIMEOUT: Duration = Duration::from_secs(15);

/// 拿到的数据和它是不是新的（网络失败时用了磁盘上旧的）。
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub value: T,
    /// 用的是过期的缓存（网络失败）。
    pub stale: bool,
}

pub struct Stats {
    http: reqwest::Client,
    dir: PathBuf,
    /// OP.GG 的 MCP 连接：第一次调用时连上，之后的调用复用同一个会话；断了下次再连。
    opgg: tokio::sync::Mutex<Option<Arc<kotone_mcp::McpClient>>>,
    opgg_endpoint: String,
    memory: Mutex<HashMap<String, (SystemTime, Value)>>,
    aramhex: Mutex<Option<(SystemTime, Arc<aramhex::Dataset>)>>,
    nicknames: Mutex<Option<(SystemTime, Arc<nicknames::Nicknames>)>>,
    statics: Mutex<Option<(SystemTime, Arc<statics::Statics>)>>,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fresh(at: SystemTime, ttl: Duration) -> bool {
    SystemTime::now()
        .duration_since(at)
        .is_ok_and(|age| age < ttl)
}

impl Stats {
    /// `dir`：插件的存储目录下的 `stats/`（用时才建）。
    pub fn new(dir: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("kotone")
            .build()
            .unwrap_or_default();
        Self {
            http,
            dir,
            opgg: tokio::sync::Mutex::new(None),
            opgg_endpoint: opgg::ENDPOINT.to_string(),
            memory: Mutex::new(HashMap::new()),
            aramhex: Mutex::new(None),
            nicknames: Mutex::new(None),
            statics: Mutex::new(None),
        }
    }

    /// 测试用：OP.GG 换成本地的测试服务器。
    #[cfg(test)]
    fn with_opgg_endpoint(mut self, endpoint: &str) -> Self {
        self.opgg_endpoint = endpoint.to_string();
        self
    }

    fn disk(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.json", hash(url)))
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("连不上 {}：{e}", host_of(url)))?;
        if !response.status().is_success() {
            return Err(format!("{} 返回 {}", host_of(url), response.status()));
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("{} 的数据没读完：{e}", host_of(url)))
    }

    /// 读一个 JSON：内存里新的 → 磁盘上新的 → 网络（存到内存和磁盘）→ 网络失败时磁盘上旧的。
    pub async fn json(&self, url: &str, ttl: Duration) -> Result<Fetched<Value>, String> {
        if let Some((at, v)) = self.memory.lock().unwrap().get(url).cloned() {
            if fresh(at, ttl) {
                return Ok(Fetched {
                    value: v,
                    stale: false,
                });
            }
        }
        let path = self.disk(url);
        let on_disk = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| {
                let text = std::fs::read_to_string(&path).ok()?;
                Some((at, serde_json::from_str::<Value>(&text).ok()?))
            });
        if let Some((at, v)) = &on_disk {
            if fresh(*at, ttl) {
                self.memory
                    .lock()
                    .unwrap()
                    .insert(url.into(), (*at, v.clone()));
                return Ok(Fetched {
                    value: v.clone(),
                    stale: false,
                });
            }
        }
        match self.download(url).await {
            Ok(bytes) => {
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| format!("{} 返回的不是 JSON", host_of(url)))?;
                let _ = std::fs::create_dir_all(&self.dir);
                let _ = std::fs::write(&path, &bytes);
                self.memory
                    .lock()
                    .unwrap()
                    .insert(url.into(), (SystemTime::now(), value.clone()));
                Ok(Fetched {
                    value,
                    stale: false,
                })
            }
            Err(error) => match on_disk {
                Some((_, v)) => Ok(Fetched {
                    value: v,
                    stale: true,
                }),
                None => Err(error),
            },
        }
    }

    // ---------- 腾讯 101 ----------

    /// 最新的版本号（“16.23”）。
    pub async fn patch(&self) -> Result<String, String> {
        let v = self
            .json(&format!("{}{}", qq101::BASE, qq101::VERSIONS), TTL_101)
            .await?;
        qq101::parse_versions(&v.value)
            .into_iter()
            .next()
            .map(|(_, name)| name)
            .ok_or_else(|| "腾讯的版本列表是空的".into())
    }

    fn rift_url(path: &str, patch: &str, lane: &str, champion: Option<i64>, extra: &str) -> String {
        let mut url = format!(
            "{}{}?itier={}&version_id={patch}&lane={lane}{extra}",
            qq101::BASE,
            path,
            qq101::ALL_TIERS
        );
        if let Some(id) = champion {
            url.push_str(&format!("&championid={id}"));
        }
        url
    }

    /// 峡谷某一路的强度榜。
    pub async fn tier_list(
        &self,
        lane: &str,
    ) -> Result<Fetched<(String, Vec<qq101::TierRow>)>, String> {
        let patch = self.patch().await?;
        let url = Self::rift_url(
            qq101::RIFT,
            &patch,
            lane,
            None,
            "&sort_metric=1&sort_order=2",
        );
        let v = self.json(&url, TTL_101).await?;
        Ok(Fetched {
            value: qq101::parse_tier_list(&v.value),
            stale: v.stale,
        })
    }

    /// 一个英雄在某一路的克制、出装、符文、加点、召唤师技能（并发请求；哪项失败就空着）。
    pub async fn champion(&self, champion: i64, lane: &str) -> Result<ChampionStats, String> {
        let patch = self.patch().await?;
        let url = |suffix: &str| {
            Self::rift_url(
                &format!("{}{suffix}", qq101::RIFT),
                &patch,
                lane,
                Some(champion),
                "",
            )
        };
        let urls = [
            url("_confront"),
            url("_build"),
            url("_runeinfo"),
            url("_skill_point"),
            url("_skill"),
        ];
        let (confront, build, runes, skill_point, skill) = tokio::join!(
            self.json(&urls[0], TTL_101),
            self.json(&urls[1], TTL_101),
            self.json(&urls[2], TTL_101),
            self.json(&urls[3], TTL_101),
            self.json(&urls[4], TTL_101),
        );
        let stale = [&confront, &build, &runes, &skill_point, &skill]
            .iter()
            .any(|r| r.as_ref().is_ok_and(|f| f.stale));
        let value = |r: Result<Fetched<Value>, String>| r.map(|f| f.value).unwrap_or(Value::Null);
        let (good, bad) = qq101::parse_matchups(&value(confront));
        let (priority, order) = qq101::parse_skill_order(&value(skill_point));
        Ok(ChampionStats {
            patch,
            lane: lane.to_string(),
            good_against: good,
            bad_against: bad,
            build: qq101::parse_build(&value(build)),
            runes: qq101::parse_runes(&value(runes)),
            skill_priority: priority,
            skill_order: order,
            spells: qq101::parse_summoner_spells(&value(skill)),
            stale,
        })
    }

    /// 海斗的英雄榜（往前找最近有数据的一天，最多 7 天）。`today`：`YYYYMMDD` 的日期，往前数。
    pub async fn mayhem_champions(
        &self,
        days: &[String],
    ) -> Result<Fetched<(String, Vec<qq101::MayhemChampion>)>, String> {
        let mut last_error = String::from("腾讯的海斗数据这几天都是空的");
        for day in days.iter().take(7) {
            let url = format!(
                "{}{}/fuwen_aram_hero_rank_v2?dtstatdate={day}",
                qq101::BASE,
                qq101::MAYHEM
            );
            match self.json(&url, TTL_101).await {
                Ok(v) => {
                    let rows = qq101::parse_mayhem_champions(&v.value);
                    if !rows.is_empty() {
                        return Ok(Fetched {
                            value: (day.clone(), rows),
                            stale: v.stale,
                        });
                    }
                }
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// 海斗的海克斯榜。
    pub async fn mayhem_augments(&self) -> Result<Fetched<Vec<qq101::MayhemAugment>>, String> {
        let url = format!(
            "{}{}/fuwen_aram_rune_rank_v2?augmentid_level=255",
            qq101::BASE,
            qq101::MAYHEM
        );
        let v = self.json(&url, TTL_101).await?;
        Ok(Fetched {
            value: qq101::parse_mayhem_augments(&v.value),
            stale: v.stale,
        })
    }

    // ---------- aramhex-data ----------

    /// 海斗英雄 × 海克斯（文件按 SHA-256 存在磁盘上，内容对不上的不用）。
    pub async fn aramhex(&self) -> Result<Arc<aramhex::Dataset>, String> {
        if let Some((at, d)) = self.aramhex.lock().unwrap().clone() {
            if fresh(at, TTL_LATEST) {
                return Ok(d);
            }
        }
        let latest = self
            .json(&format!("{}/latest.json", aramhex::RAW), TTL_LATEST)
            .await?;
        let latest =
            aramhex::parse_latest(&latest.value).ok_or("aramhex-data 的 latest.json 不对")?;
        let mut files = Vec::new();
        for name in ["heroes.json", "augments.json", "hero_augments.json"] {
            files.push(self.aramhex_file(&latest, name).await?);
        }
        let dataset = Arc::new(aramhex::Dataset::from_json(
            &latest, &files[0], &files[1], &files[2],
        ));
        if dataset.is_empty() {
            return Err("aramhex-data 的数据是空的".into());
        }
        *self.aramhex.lock().unwrap() = Some((SystemTime::now(), dataset.clone()));
        Ok(dataset)
    }

    async fn aramhex_file(&self, latest: &aramhex::Latest, name: &str) -> Result<Value, String> {
        let sha = latest
            .files
            .get(name)
            .ok_or_else(|| format!("aramhex-data 没列出 {name}"))?;
        let path = self.dir.join("aramhex").join(format!("{sha}.json"));
        if let Ok(bytes) = std::fs::read(&path) {
            if aramhex::verify(latest, name, &bytes) {
                return serde_json::from_slice(&bytes).map_err(|e| e.to_string());
            }
        }
        let bytes = self
            .download(&format!("{}/{}/{name}", aramhex::RAW, latest.path))
            .await?;
        if !aramhex::verify(latest, name, &bytes) {
            return Err(format!("aramhex-data 的 {name} 校验没过（SHA-256 对不上）"));
        }
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&self.dir));
        let _ = std::fs::write(&path, &bytes);
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }

    // ---------- OP.GG（默认关，见 [`opgg`]） ----------

    /// 调一次 OP.GG 的工具，结果（文字）在磁盘上存 24 小时。`version`：Kotone 的版本（写进 `client_app`）。
    pub async fn opgg(
        &self,
        tool: &str,
        arguments: Value,
        version: &str,
    ) -> Result<Fetched<String>, String> {
        let key = format!("opgg:{tool}:{arguments}");
        let path = self.dir.join("opgg").join(format!("{}.txt", hash(&key)));
        let cached = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| Some((at, std::fs::read_to_string(&path).ok()?)));
        if let Some((at, text)) = &cached {
            if fresh(*at, TTL_OPGG) {
                return Ok(Fetched {
                    value: text.clone(),
                    stale: false,
                });
            }
        }
        match self.opgg_call(tool, arguments, version).await {
            Ok(text) => {
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(&self.dir));
                let _ = std::fs::write(&path, &text);
                Ok(Fetched {
                    value: text,
                    stale: false,
                })
            }
            Err(e) => match cached {
                Some((_, text)) => Ok(Fetched {
                    value: text,
                    stale: true,
                }),
                None => Err(e),
            },
        }
    }

    /// 调一次 OP.GG 的工具（经 `kotone-mcp`，复用会话）。会话没了、连接断了，就丢掉这个连接，下次重新连。
    async fn opgg_call(
        &self,
        tool: &str,
        arguments: Value,
        version: &str,
    ) -> Result<String, String> {
        let client = {
            let mut slot = self.opgg.lock().await;
            match slot.as_ref() {
                Some(client) if client.is_connected() => client.clone(),
                _ => {
                    let config = opgg::config(&self.opgg_endpoint, version);
                    let client = Arc::new(
                        kotone_mcp::McpClient::connect(&config)
                            .await
                            .map_err(|e| opgg::describe(&e))?,
                    );
                    *slot = Some(client.clone());
                    client
                }
            }
        };
        match client
            .call_tool(tool, opgg::arguments(arguments, version))
            .await
        {
            Ok(result) => opgg::result_text(&result),
            Err(error) => {
                if opgg::should_reconnect(&error) {
                    let mut slot = self.opgg.lock().await;
                    if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &client)) {
                        *slot = None;
                    }
                }
                Err(opgg::describe(&error))
            }
        }
    }

    // ---------- 名字表 ----------

    /// 装备、符文、召唤师技能的名字（哪个文件读不到就空着，显示时用编号）。
    pub async fn statics(&self) -> Arc<statics::Statics> {
        if let Some((at, s)) = self.statics.lock().unwrap().clone() {
            if fresh(at, TTL_NICKNAMES) {
                return s;
            }
        }
        let (items, runes, spells) = tokio::join!(
            self.json(statics::ITEMS, TTL_NICKNAMES),
            self.json(statics::RUNES, TTL_NICKNAMES),
            self.json(statics::SPELLS, TTL_NICKNAMES),
        );
        let value = |r: Result<Fetched<Value>, String>| r.map(|f| f.value).unwrap_or(Value::Null);
        let s = Arc::new(statics::Statics {
            items: statics::parse_items(&value(items)),
            runes: statics::parse_named_map(&value(runes), "rune"),
            spells: statics::parse_named_map(&value(spells), "summonerskill"),
        });
        if !s.items.is_empty() {
            *self.statics.lock().unwrap() = Some((SystemTime::now(), s.clone()));
        }
        s
    }

    // ---------- 外号 ----------

    pub async fn nicknames(&self) -> Result<Arc<nicknames::Nicknames>, String> {
        if let Some((at, n)) = self.nicknames.lock().unwrap().clone() {
            if fresh(at, TTL_NICKNAMES) {
                return Ok(n);
            }
        }
        let path = self.dir.join("hero_list.js");
        let cached = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .filter(|at| fresh(*at, TTL_NICKNAMES))
            .and_then(|_| std::fs::read_to_string(&path).ok());
        let text = match cached {
            Some(text) => text,
            None => match self.download(nicknames::URL).await {
                Ok(bytes) => {
                    let _ = std::fs::create_dir_all(&self.dir);
                    let _ = std::fs::write(&path, &bytes);
                    String::from_utf8_lossy(&bytes).into_owned()
                }
                Err(e) => std::fs::read_to_string(&path).map_err(|_| e)?,
            },
        };
        let n = Arc::new(nicknames::Nicknames::parse(&text));
        if n.is_empty() {
            return Err("腾讯的英雄外号表读不懂".into());
        }
        *self.nicknames.lock().unwrap() = Some((SystemTime::now(), n.clone()));
        Ok(n)
    }
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
}

/// 一个英雄的统计（[`Stats::champion`]）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampionStats {
    pub patch: String,
    pub lane: String,
    pub good_against: Vec<qq101::Matchup>,
    pub bad_against: Vec<qq101::Matchup>,
    pub build: qq101::Build,
    pub runes: Vec<qq101::RunePage>,
    pub skill_priority: Vec<String>,
    pub skill_order: Vec<String>,
    pub spells: Vec<qq101::SpellPair>,
    /// 有一项用的是过期的缓存。
    pub stale: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_and_hosts() {
        assert_eq!(hash("a"), hash("a"));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(host_of("https://mlol.qt.qq.com/go/x?y=1"), "mlol.qt.qq.com");
        assert!(fresh(SystemTime::now(), Duration::from_secs(5)));
        assert!(!fresh(
            SystemTime::now() - Duration::from_secs(10),
            Duration::from_secs(5)
        ));
    }

    /// 磁盘上有旧的、网络连不上：用旧的并标出来。
    #[tokio::test]
    async fn stale_disk_copy_when_offline() {
        let dir = tempfile_dir();
        let stats = Stats::new(dir.clone());
        // 一个连不上的本机端口
        let url = "http://127.0.0.1:9/never.json";
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(stats.disk(url), r#"{"old": true}"#).unwrap();
        // 把文件时间设成很久以前：写不了 mtime 时就当 ttl 为 0
        let got = stats.json(url, Duration::from_secs(0)).await.unwrap();
        assert!(got.stale);
        assert_eq!(got.value["old"], true);
        let missing = stats
            .json("http://127.0.0.1:9/none.json", Duration::from_secs(0))
            .await;
        assert!(missing.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// OP.GG 经 `kotone-mcp`：一个会话里调多次（以前每次调用三个请求），`client_app` 和 `clientInfo` 照旧，
    /// 24 小时的磁盘缓存照旧，出错时的说法照旧。服务器是本地的测试服务器（rmcp 的服务端，回复走 SSE，和 OP.GG 一样）。
    #[tokio::test]
    async fn opgg_reuses_one_session() {
        use kotone_mcp::testing::{handler, text_reply, TestServer};
        use serde_json::json;

        let server = TestServer::start(
            json!([{"name": "lol_get_champion_analysis", "inputSchema": {"type": "object"}}]),
            json!([]),
            handler(|_, args| async move {
                match args["champion"].as_str() {
                    Some("NOPE") => Ok(json!({"isError": true,
                        "content": [{"type": "text", "text": "no such champion"}]})),
                    Some("BAD") => Err((-32602, "bad".to_string())),
                    Some(c) => text_reply(&format!("class {c} {{}}")),
                    None => text_reply(""),
                }
            }),
        )
        .await;
        let dir = std::env::temp_dir().join(format!("kotone-lol-opgg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stats = Stats::new(dir.clone()).with_opgg_endpoint(server.url());
        let call = |champion: &'static str| {
            let stats = &stats;
            async move {
                stats
                    .opgg(
                        "lol_get_champion_analysis",
                        json!({"champion": champion, "position": "top"}),
                        "0.2.0",
                    )
                    .await
            }
        };

        let irelia = call("IRELIA").await.unwrap();
        assert_eq!(irelia.value, "class IRELIA {}");
        assert!(!irelia.stale);
        assert_eq!(call("YASUO").await.unwrap().value, "class YASUO {}");
        // 同样的参数 24 小时内从磁盘拿，不再请求
        assert_eq!(call("IRELIA").await.unwrap().value, "class IRELIA {}");

        let log = server.log();
        assert_eq!(log.initializes, 1, "只握手一次：{log:?}");
        assert_eq!(log.calls.len(), 2);
        // initialize、notifications/initialized、两次 tools/call
        assert_eq!(log.posts, 4, "{log:?}");
        assert_eq!(log.calls[0].1["client_app"], "kotone/0.2.0");
        assert_eq!(log.calls[0].1["champion"], "IRELIA");
        let init = log.initialize.unwrap();
        assert_eq!(init["clientInfo"]["name"], "kotone");
        assert_eq!(init["clientInfo"]["version"], "0.2.0");
        assert_eq!(log.user_agent.as_deref(), Some("kotone"));

        // 出错时的说法照旧；工具出错、JSON-RPC 出错都不用重新握手
        assert_eq!(call("NOPE").await.unwrap_err(), "OP.GG：no such champion");
        assert_eq!(call("BAD").await.unwrap_err(), "OP.GG：bad");
        assert_eq!(call("AHRI").await.unwrap().value, "class AHRI {}");
        assert_eq!(server.log().initializes, 1);

        // 服务器没了：说连不上；磁盘上有的照样给（标成旧的要等 24 小时，这里还是新的）
        drop(server);
        let offline = call("ZED").await.unwrap_err();
        assert!(offline.starts_with("连不上 OP.GG："), "{offline}");
        assert!(
            stats.opgg.lock().await.is_none(),
            "连不上以后丢掉这个连接，下次重新连"
        );
        assert_eq!(call("IRELIA").await.unwrap().value, "class IRELIA {}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真调 OP.GG：一个工具（查一个英雄），两次调用，第二次复用第一次的会话。OP.GG 回复 issue #26 前开关默认关，
    /// 这个测试只在验收时手动跑：`cargo test -p kotone-plugin-lol opgg_real -- --ignored --nocapture`。
    #[tokio::test]
    #[ignore = "连真的 OP.GG，手动跑"]
    async fn opgg_real_call() {
        use serde_json::json;
        use std::time::Instant;

        let dir = std::env::temp_dir().join(format!("kotone-lol-opgg-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stats = Stats::new(dir.clone());
        let args = |position: &str| {
            json!({"champion": opgg::champion_key("Irelia"), "position": position,
                   "game_mode": "ranked", "lang": "zh_CN"})
        };
        let started = Instant::now();
        let first = stats
            .opgg("lol_get_champion_analysis", args("top"), "0.2.0")
            .await;
        let first_ms = started.elapsed().as_millis();
        let client = stats.opgg.lock().await.clone();
        let started = Instant::now();
        let second = stats
            .opgg("lol_get_champion_analysis", args("mid"), "0.2.0")
            .await;
        let second_ms = started.elapsed().as_millis();
        let same_session = match (&client, stats.opgg.lock().await.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) && b.is_connected(),
            _ => false,
        };
        let show = |r: &Result<Fetched<String>, String>| match r {
            Ok(f) => format!(
                "ok, {} 字：{}",
                f.value.chars().count(),
                f.value.chars().take(200).collect::<String>()
            ),
            Err(e) => format!("出错：{e}"),
        };
        eprintln!(
            "OP.GG 服务器：{:?}",
            client.as_ref().map(|c| c.server_info().clone())
        );
        eprintln!("第一次（握手 + 调用）{first_ms} ms，{}", show(&first));
        eprintln!("第二次（复用会话）{second_ms} ms，{}", show(&second));
        eprintln!("同一个会话：{same_session}");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first.is_ok() && second.is_ok());
        assert!(same_session);
    }

    fn tempfile_dir() -> PathBuf {
        std::env::temp_dir().join(format!("kotone-lol-stats-{}", std::process::id()))
    }
}
