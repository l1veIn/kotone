//! 英雄技能（被动、Q、W、E、R）的名字和图标路径：英雄页加点那块的图（[对标调研](../../../../docs/research/2026-11-25-lol-app-parity.md)
//! 第三节 1“英雄技能”，设计文档第一节 7）。
//!
//! 资料是 `/lol-game-data/assets/v1/champions/{id}.json` 的 `passive`、`spells[]`（[`gamedata::abilities`] 解析）。顺序：
//!
//! 1. 内存；
//! 2. 客户端：开着、而且“读客户端数据”开着时（语言跟客户端，国服是中文）；
//! 3. 磁盘缓存：插件目录下的 `abilities/`，7 天内的；
//! 4. CommunityDragon 上同一个文件（`…/global/zh_cn/v1/champions/{id}.json`，中文）：在后台取，最多等 [`WAIT`]。
//!    慢了这次先不带（页面显示字母格子），后台取完存进缓存，下次就有；同一个英雄同时只取一份；失败的 10 分钟内不再试；
//! 5. 磁盘上过期的；都没有就是空的（页面显示字母格子）。
//!
//! 磁盘上只存解析好的五条（一个英雄几百字节），不存整个文件（一个英雄 60–90 KB，大半是皮肤）。图标路径过了素材层的
//! 检查，图本身照素材层的规则取（[`crate::client::assets`]：缓存 → 客户端 → CommunityDragon）。只读公开资料，请求里
//! 不带玩家的任何数据（ADR 039 D6）。不用腾讯的 `js/hero/{id}.js`：它给的图标是腾讯 CDN 的地址，素材层和 App 的 CSP
//! 都不认（调研第三节 4 第 5 条）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use kotone_plugin_api::BoxFuture;
use serde_json::Value;

use crate::client::assets::{normalize, CDRAGON};
use crate::client::gamedata::{self, Ability};
use crate::client::http::{loopback_base, Lcu};

/// 工具最多等多久（客户端、CommunityDragon 各一次）。给琴音的回答不因为图标慢下来。
pub const WAIT: Duration = Duration::from_secs(2);
/// 磁盘缓存多久以后重新取。
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// CommunityDragon 取失败的，多久内不再试。
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(15);
/// 一个英雄的资料最大多少（现在 60–90 KB）。
const MAX_FILE: usize = 4 * 1024 * 1024;

/// 客户端上的路径。
pub fn client_path(id: i64) -> String {
    format!("/lol-game-data/assets/v1/champions/{id}.json")
}

/// CommunityDragon 上的地址（中文）。`base` 和素材层一样（`https://raw.communitydragon.org/latest`）。
pub fn cdragon_url(base: &str, id: i64) -> String {
    format!(
        "{}/plugins/rcp-be-lol-game-data/global/zh_cn/v1/champions/{id}.json",
        base.trim_end_matches('/')
    )
}

/// CommunityDragon 的地址：平时是 [`CDRAGON`]；测试设了 `KOTONE_LOL_CDRAGON`（和素材层同一个变量）时用本机的
/// 假服务器，写的不是本机地址时 None（不连网）。
pub fn online_base() -> Option<String> {
    match std::env::var("KOTONE_LOL_CDRAGON") {
        Err(_) => Some(CDRAGON.to_string()),
        Ok(value) => loopback_base(&value),
    }
}

/// 取 `champions/{id}.json` 的一个来源（客户端；测试里换成假的）。拿不到给 None。
pub trait Source: Send + Sync {
    fn champion<'a>(&'a self, id: i64) -> BoxFuture<'a, Option<Value>>;
}

/// 客户端：本机接口，只读（[`Lcu::get`]）。
impl Source for Lcu {
    fn champion<'a>(&'a self, id: i64) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async move { self.get(&client_path(id)).await.ok() })
    }
}

/// 英雄技能的缓存和取法。进程里一份（[`crate::LolPlugin`] 用时建）。
pub struct Abilities {
    dir: PathBuf,
    /// CommunityDragon 的地址；None = 不连网。
    online: Option<String>,
    http: reqwest::Client,
    wait: Duration,
    memory: Mutex<HashMap<i64, Vec<Ability>>>,
    /// 正在后台取的英雄。
    inflight: Mutex<HashSet<i64>>,
    /// CommunityDragon 取失败的英雄和时间。
    failed: Mutex<HashMap<i64, Instant>>,
    tmp: AtomicU64,
}

impl Abilities {
    /// `dir`：缓存目录（用时才建）；`online`：CommunityDragon 的地址（[`online_base`]）。
    pub fn new(dir: PathBuf, online: Option<String>) -> Self {
        let mut builder = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("kotone");
        if online.as_deref().and_then(loopback_base).is_some() {
            builder = builder.no_proxy();
        }
        Self {
            dir,
            online: online.map(|base| base.trim_end_matches('/').to_string()),
            http: builder.build().unwrap_or_default(),
            wait: WAIT,
            memory: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashSet::new()),
            failed: Mutex::new(HashMap::new()),
            tmp: AtomicU64::new(0),
        }
    }

    /// 换掉最多等多久（测试用）。
    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// 一个英雄的技能（按 P Q W E R 排）：内存 → 客户端（`client`，没开时 None）→ 新的磁盘缓存 →
    /// CommunityDragon（后台取，最多等 [`WAIT`]）→ 过期的磁盘缓存 → 空的。
    pub async fn get(&'static self, client: Option<&dyn Source>, id: i64) -> Vec<Ability> {
        if id <= 0 {
            return Vec::new();
        }
        if let Some(list) = self.memory.lock().unwrap().get(&id).cloned() {
            return list;
        }
        if let Some(client) = client {
            if let Ok(Some(value)) = tokio::time::timeout(self.wait, client.champion(id)).await {
                let list = gamedata::abilities(&value);
                if !list.is_empty() {
                    self.keep(id, &list);
                    return list;
                }
            }
        }
        let stale = match self.read(id) {
            Some((list, true)) => {
                self.memory.lock().unwrap().insert(id, list.clone());
                return list;
            }
            Some((list, false)) => Some(list),
            None => None,
        };
        if let Some(task) = self.start_download(id) {
            if let Ok(Ok(Some(list))) = tokio::time::timeout(self.wait, task).await {
                return list;
            }
        }
        stale.unwrap_or_default()
    }

    /// 在后台从 CommunityDragon 取（不连网、10 分钟内失败过、已经在取的，不取）。取到的存进内存和磁盘。
    fn start_download(
        &'static self,
        id: i64,
    ) -> Option<tokio::task::JoinHandle<Option<Vec<Ability>>>> {
        let base = self.online.clone()?;
        let failed_recently = self
            .failed
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|at| at.elapsed() < RETRY_AFTER);
        if failed_recently || !self.inflight.lock().unwrap().insert(id) {
            return None;
        }
        Some(tokio::spawn(async move {
            let result = self.download(&base, id).await;
            match &result {
                Some(list) => {
                    self.failed.lock().unwrap().remove(&id);
                    self.keep(id, list);
                }
                None => {
                    self.failed.lock().unwrap().insert(id, Instant::now());
                }
            }
            self.inflight.lock().unwrap().remove(&id);
            result
        }))
    }

    async fn download(&self, base: &str, id: i64) -> Option<Vec<Ability>> {
        let response = self.http.get(cdragon_url(base, id)).send().await.ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|n| n > MAX_FILE as u64)
        {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if bytes.len() > MAX_FILE {
            return None;
        }
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        let list = gamedata::abilities(&value);
        (!list.is_empty()).then_some(list)
    }

    fn file(&self, id: i64) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 记下来：内存一份，磁盘一份（先写临时文件再改名，写不了就算了）。
    fn keep(&self, id: i64, list: &[Ability]) {
        self.memory.lock().unwrap().insert(id, list.to_vec());
        let Ok(bytes) = serde_json::to_vec(list) else {
            return;
        };
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join(format!(
            "{id}.json.{}-{}.tmp",
            std::process::id(),
            self.tmp.fetch_add(1, Ordering::Relaxed)
        ));
        if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, self.file(id)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// 磁盘上的这个英雄和它是不是新的（7 天内）。读出来的图标路径再过一遍素材层的检查。
    fn read(&self, id: i64) -> Option<(Vec<Ability>, bool)> {
        let file = self.file(id);
        let meta = std::fs::metadata(&file).ok()?;
        let mut list: Vec<Ability> = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
        for a in &mut list {
            a.icon = normalize(&a.icon).unwrap_or_default();
        }
        list.retain(|a| gamedata::ABILITY_KEYS.contains(&a.key.as_str()));
        if list.is_empty() {
            return None;
        }
        // 时钟往回拨过（修改时间在将来）也算新的
        let fresh = meta
            .modified()
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .map_or(true, |age| age < MAX_AGE);
        Some((list, fresh))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gamedata::tests::yasuo_json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kotone-lol-abilities-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn leak(abilities: Abilities) -> &'static Abilities {
        Box::leak(Box::new(abilities))
    }

    /// 假客户端：按 id 回固定的资料，记下被问了几次。
    #[derive(Default)]
    struct FakeClient {
        files: HashMap<i64, Value>,
        calls: AtomicUsize,
    }

    impl Source for FakeClient {
        fn champion<'a>(&'a self, id: i64) -> BoxFuture<'a, Option<Value>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.files.get(&id).cloned()
            })
        }
    }

    /// 本机的假 CommunityDragon：只认亚索的中文资料（可以慢一点），别的 404；记下被问的路径。
    async fn fake_cdragon(delay: Duration) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let body = serde_json::to_vec(&yasuo_json()).unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    log.lock().unwrap().push(path.clone());
                    tokio::time::sleep(delay).await;
                    let reply = if path
                        == "/latest/plugins/rcp-be-lol-game-data/global/zh_cn/v1/champions/157.json"
                    {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&body);
                        r
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    };
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (format!("http://127.0.0.1:{port}/latest/"), seen)
    }

    #[test]
    fn urls_follow_the_client_and_community_dragon() {
        assert_eq!(
            client_path(157),
            "/lol-game-data/assets/v1/champions/157.json"
        );
        assert_eq!(
            cdragon_url(CDRAGON, 157),
            "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/zh_cn/v1/champions/157.json"
        );
        assert_eq!(
            cdragon_url("http://127.0.0.1:9/latest/", 1),
            "http://127.0.0.1:9/latest/plugins/rcp-be-lol-game-data/global/zh_cn/v1/champions/1.json"
        );
    }

    #[tokio::test]
    async fn the_client_comes_first_and_is_remembered() {
        let dir = temp_dir("client");
        let client = FakeClient {
            files: HashMap::from([(157, yasuo_json())]),
            ..FakeClient::default()
        };
        // 不连网：只有客户端
        let abilities = leak(Abilities::new(dir.clone(), None));
        let list = abilities.get(Some(&client), 157).await;
        assert_eq!(list.len(), 5);
        assert_eq!(list[4].name, "狂风绝息斩");
        // 第二次走内存，不再问客户端
        assert_eq!(abilities.get(Some(&client), 157).await, list);
        assert_eq!(client.calls.load(Ordering::SeqCst), 1);
        // 重启以后（新的一份）：客户端没开也有，磁盘上存的只是这五条
        let restarted = leak(Abilities::new(dir.clone(), None));
        assert_eq!(restarted.get(None, 157).await, list);
        let saved = std::fs::read(dir.join("157.json")).unwrap();
        assert!(saved.len() < 1024, "{}", saved.len());
        // 客户端里没有的、不是英雄的 id：空的
        assert!(abilities.get(Some(&client), 103).await.is_empty());
        assert!(abilities.get(Some(&client), 0).await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_the_client_it_asks_community_dragon_once() {
        let dir = temp_dir("online");
        let (base, seen) = fake_cdragon(Duration::ZERO).await;
        let abilities = leak(Abilities::new(dir.clone(), Some(base)));
        let list = abilities.get(None, 157).await;
        assert_eq!(list.len(), 5);
        assert!(list[1].icon.ends_with("/Yasuo_Q1.png"));
        assert_eq!(abilities.get(None, 157).await, list);
        // 没有的英雄：404 → 空的，10 分钟内不再问
        assert!(abilities.get(None, 999).await.is_empty());
        assert!(abilities.get(None, 999).await.is_empty());
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(
            seen[1].ends_with("/zh_cn/v1/champions/999.json"),
            "{seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn slow_community_dragon_fills_the_cache_for_next_time() {
        let dir = temp_dir("slow");
        let (base, seen) = fake_cdragon(Duration::from_millis(300)).await;
        let abilities =
            leak(Abilities::new(dir.clone(), Some(base)).with_wait(Duration::from_millis(50)));
        // 这次等不及：先空着（页面显示字母）；在取的时候再要，不再多取一份
        assert!(abilities.get(None, 157).await.is_empty());
        assert!(abilities.get(None, 157).await.is_empty());
        // 后台取完了：下次就有
        let mut list = Vec::new();
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            list = abilities.get(None, 157).await;
            if !list.is_empty() {
                break;
            }
        }
        assert_eq!(list.len(), 5);
        assert!(dir.join("157.json").exists());
        assert_eq!(seen.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn old_or_tampered_cache_files_are_handled() {
        let dir = temp_dir("disk");
        std::fs::create_dir_all(&dir).unwrap();
        // 磁盘上的路径被改过：不放行的写成空的；不认识的键丢掉
        std::fs::write(
            dir.join("157.json"),
            r#"[{"key":"Q","name":"斩钢闪","icon":"/lol-champ-select/v1/session"},
                {"key":"W","name":"风之障壁","icon":"/lol-game-data/assets/ASSETS/Characters/Yasuo/HUD/Icons2D/Yasuo_W.png"},
                {"key":"X","name":"?","icon":""}]"#,
        )
        .unwrap();
        // 坏了的文件当没有
        std::fs::write(dir.join("103.json"), b"not json").unwrap();
        let abilities = leak(Abilities::new(dir.clone(), None));
        let list = abilities.get(None, 157).await;
        let keys: Vec<(&str, &str)> = list
            .iter()
            .map(|a| (a.key.as_str(), a.icon.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("Q", ""),
                (
                    "W",
                    "/lol-game-data/assets/ASSETS/Characters/Yasuo/HUD/Icons2D/Yasuo_W.png"
                )
            ]
        );
        assert!(abilities.get(None, 103).await.is_empty());
        // 过期了、又取不到（不连网）：用旧的那份
        let old = leak(Abilities::new(dir.clone(), None));
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("157.json"))
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 3600))
            .unwrap();
        drop(f);
        assert_eq!(old.get(None, 157).await.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
