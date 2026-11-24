//! 素材层（ADR 039 D6；[对标调研](../../../../docs/research/2026-11-25-lol-app-parity.md)第三节 4、差距表 G0）：
//! 英雄头像、召唤师头像、段位徽章、海克斯、装备、召唤师技能、符文的图。
//!
//! 安装包里不带图。页面数据里只放客户端的资源路径（例如 `/lol-game-data/assets/v1/champion-icons/103.png`，
//! [`champion_icon`] 这几个函数给），图按需取，顺序是：
//!
//! 1. 磁盘缓存：插件目录下的 `asset-cache/`，上限 64 MB，超了先删最久没用的；14 天后重新取；
//! 2. 客户端：开着、而且“读客户端数据”开着时，带本机令牌，只读；
//! 3. CommunityDragon：`https://raw.communitydragon.org/latest/…`，路径换成小写；失败的 10 分钟内不再试（照 AramHex）；
//! 4. 都没有：[`AssetError::NotFound`]，页面显示占位。重新取失败时，用缓存里过期的那份。
//!
//! **只放行三个前缀下的图片**（[`PREFIXES`]；png / jpg / svg / webp，按内容认，不是图片的不给），只读。
//! 不能像 LeagueAkari 那样把整个客户端接口转给页面：页面在沙盒里，转了等于让页面能操作客户端（接受对局、
//! 选英雄）。路径里有 `..`、`%`、查询串、反斜杠、空白、非 ASCII 的一律拒绝（[`normalize`]）。
//! 只取公开的图，请求里不带玩家的任何数据（ADR 039 D6）。同一个路径同时来几次只取一次；写缓存先写临时文件再改名，
//! 不会写坏。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use kotone_plugin_api::BoxFuture;
use sha2::{Digest, Sha256};

use crate::client::http::{loopback_base, Lcu};

/// 只放行这三个前缀（客户端的资源路径）。
pub const PREFIXES: [&str; 3] = [
    "/lol-game-data/assets/",
    "/fe/lol-static-assets/",
    "/fe/lol-shared-components/",
];

/// CommunityDragon 上对应的目录（和 [`PREFIXES`] 一一对应；调研 2026-11-25 用十几个样本验证过）。
const CDRAGON_DIRS: [&str; 3] = [
    "plugins/rcp-be-lol-game-data/global/default/",
    "plugins/rcp-fe-lol-static-assets/global/default/",
    "plugins/rcp-fe-lol-shared-components/global/default/",
];

/// CommunityDragon（跟最新版本）。页面在别的宿主里显示时也用它，域名写在 App 的 CSP 里。
pub const CDRAGON: &str = "https://raw.communitydragon.org/latest";
/// 磁盘缓存的上限。
pub const CACHE_LIMIT: u64 = 64 * 1024 * 1024;
/// 多久以后重新取。
pub const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);
/// CommunityDragon 取失败的，多久内不再试。
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// 一张图最大多少（段位大图约 160 KB，原画也不到 1 MB）。
const MAX_ASSET: usize = 8 * 1024 * 1024;
const MAX_PATH: usize = 256;
const TIMEOUT: Duration = Duration::from_secs(10);
/// 缓存目录里没写完的临时文件，多久以后当成残留删掉。
const STALE_TMP: Duration = Duration::from_secs(3600);

// ---------- 页面数据里的路径 ----------

/// 英雄方形头像（`championId`；没有英雄时空）。
pub fn champion_icon(id: i64) -> String {
    if id > 0 {
        format!("/lol-game-data/assets/v1/champion-icons/{id}.png")
    } else {
        String::new()
    }
}

/// 召唤师头像（`profileIconId`，0 也是一个头像；负数时空）。
pub fn profile_icon(id: i64) -> String {
    if id >= 0 {
        format!("/lol-game-data/assets/v1/profile-icons/{id}.jpg")
    } else {
        String::new()
    }
}

/// 段位小徽章（`GOLD` → `…/ranked-mini-crests/gold.svg`；没定级用 `unranked`）。
pub fn ranked_crest(tier: &str) -> String {
    let tier = tier.trim().to_ascii_lowercase();
    let tier = if tier.is_empty() || !tier.bytes().all(|b| b.is_ascii_lowercase()) {
        "unranked"
    } else {
        tier.as_str()
    };
    format!("/fe/lol-static-assets/images/ranked-mini-crests/{tier}.svg")
}

// ---------- 路径检查 ----------

/// 一张图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// `image/png`、`image/jpeg`、`image/svg+xml`、`image/webp`（按内容认的）。
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    /// 路径不放行（不在三个前缀下、不是图片、写法不对）。
    Refused(&'static str),
    /// 哪儿都没取到。
    NotFound,
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::Refused(why) => write!(f, "不放行：{why}"),
            AssetError::NotFound => write!(f, "客户端和 CommunityDragon 都没有这张图"),
        }
    }
}

impl std::error::Error for AssetError {}

/// 页面给的路径 → 规范的客户端路径（`/lol-game-data/assets/…`）。前面的 `/` 可有可无（宿主的代理会去掉它）。
///
/// 每段非空、不是 `.` / `..`，只有字母、数字和 `._-`（所以 `%` 编码、查询串、反斜杠、空白、非 ASCII 都不行），
/// 一共不超过 256 个字符；前缀是 [`PREFIXES`] 之一；扩展名是 png / jpg / jpeg / svg / webp。
pub fn normalize(path: &str) -> Result<String, AssetError> {
    let rest = path.strip_prefix('/').unwrap_or(path);
    if rest.is_empty() || rest.len() > MAX_PATH {
        return Err(AssetError::Refused("路径是空的或太长"));
    }
    let clean = rest.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    });
    if !clean {
        return Err(AssetError::Refused("路径的写法不对"));
    }
    let path = format!("/{rest}");
    if !PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
        return Err(AssetError::Refused("不在放行的三个前缀下"));
    }
    if extension(&path).is_none() {
        return Err(AssetError::Refused("不是图片"));
    }
    Ok(path)
}

/// 规范的扩展名（`jpeg` 算 `jpg`）；不是放行的图片类型时 None。
fn extension(path: &str) -> Option<&'static str> {
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        "svg" => "svg",
        "webp" => "webp",
        _ => return None,
    })
}

/// 客户端路径 → CommunityDragon 地址（前缀后面全部小写：CommunityDragon 上的文件名都是小写的）。
pub fn cdragon_url(base: &str, path: &str) -> Option<String> {
    PREFIXES.iter().zip(CDRAGON_DIRS).find_map(|(prefix, dir)| {
        let rest = path.strip_prefix(prefix)?;
        Some(format!(
            "{}/{dir}{}",
            base.trim_end_matches('/'),
            rest.to_ascii_lowercase()
        ))
    })
}

// ---------- 内容检查 ----------

/// 按内容认图片类型；认不出（例如客户端回了一段 JSON）时 None。
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if safe_svg(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

/// 是 SVG，而且里面没有能跑的东西（脚本、事件属性、`javascript:`、嵌别的文档）。段位徽章是这种 SVG。
fn safe_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if !(text.starts_with('<') && text.contains("<svg")) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let active = [
        "<script",
        "javascript:",
        "<foreignobject",
        "<iframe",
        "<embed",
        "<object",
    ];
    !active.iter().any(|a| lower.contains(a)) && !has_event_attribute(&lower)
}

/// `<svg onload=…>` 这类事件属性（空白或 `/` 后面跟 `on<字母>…=`）。
fn has_event_attribute(lower: &str) -> bool {
    let b = lower.as_bytes();
    (0..b.len().saturating_sub(3)).any(|i| {
        if !(b[i].is_ascii_whitespace() || b[i] == b'/') || &b[i + 1..i + 3] != b"on" {
            return false;
        }
        let mut j = i + 3;
        while j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == i + 3 {
            return false;
        }
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        j < b.len() && b[j] == b'='
    })
}

/// 取到的内容 → 图片（太大、认不出的不要）。
fn image(bytes: Vec<u8>) -> Option<Asset> {
    if bytes.is_empty() || bytes.len() > MAX_ASSET {
        return None;
    }
    let mime = sniff(&bytes)?;
    Some(Asset { mime, bytes })
}

// ---------- 来源 ----------

/// 取图的一个来源（客户端、CommunityDragon；测试里换成假的）。
pub trait Origin: Send + Sync {
    /// 取一个已经过了 [`normalize`] 的路径。拿不到（没开、404、超时……）给 Err。
    fn fetch<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, String>>;
}

/// 客户端：本机接口，带令牌（[`Lcu::get_bytes`]）。
impl Origin for Lcu {
    fn fetch<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move { self.get_bytes(path).await.map_err(|e| e.to_string()) })
    }
}

/// 不连网（测试的地址写错了时用：谁都不连，免得测试悄悄连上真的 CommunityDragon）。
pub struct Offline;

impl Origin for Offline {
    fn fetch<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async { Err("不连网".to_string()) })
    }
}

/// CommunityDragon（只发公开的路径，不带别的）。
pub struct CommunityDragon {
    http: reqwest::Client,
    base: String,
}

impl Default for CommunityDragon {
    fn default() -> Self {
        Self::with_base(CDRAGON)
    }
}

impl CommunityDragon {
    /// 换地址（测试用本机的假服务器；本机地址不走代理）。
    pub fn with_base(base: &str) -> Self {
        let base = base.trim_end_matches('/').to_string();
        let mut builder = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("kotone");
        if loopback_base(&base).is_some() {
            builder = builder.no_proxy();
        }
        Self {
            http: builder.build().unwrap_or_default(),
            base,
        }
    }
}

impl Origin for CommunityDragon {
    fn fetch<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move {
            let url = cdragon_url(&self.base, path).ok_or_else(|| "不是素材路径".to_string())?;
            let response = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("连不上 CommunityDragon：{e}"))?;
            if !response.status().is_success() {
                return Err(format!("CommunityDragon 返回 {}", response.status()));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_ASSET as u64)
            {
                return Err("CommunityDragon 的这张图太大".into());
            }
            response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| format!("CommunityDragon 的图没读完：{e}"))
        })
    }
}

// ---------- 磁盘缓存 ----------

/// 缓存里有哪些文件、多大、多久没用（进程里第一次用时扫一遍目录）。
#[derive(Default)]
struct Index {
    /// 文件名 → (大小, 最后一次用的序号)。
    files: HashMap<String, (u64, u64)>,
    total: u64,
    next: u64,
}

impl Index {
    fn touch(&mut self, name: &str, size: u64) {
        self.next += 1;
        let used = self.next;
        if let Some((old, _)) = self.files.insert(name.to_string(), (size, used)) {
            self.total = self.total.saturating_sub(old);
        }
        self.total += size;
    }

    fn forget(&mut self, name: &str) {
        if let Some((size, _)) = self.files.remove(name) {
            self.total = self.total.saturating_sub(size);
        }
    }

    /// 超了上限：最久没用的先删（`keep` 是刚写的那个，不删）。返回要删的文件名。
    fn evict(&mut self, limit: u64, keep: &str) -> Vec<String> {
        let mut out = Vec::new();
        while self.total > limit {
            let Some(oldest) = self
                .files
                .iter()
                .filter(|(name, _)| name.as_str() != keep)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(name, _)| name.clone())
            else {
                break;
            };
            self.forget(&oldest);
            out.push(oldest);
        }
        out
    }
}

/// 素材层：磁盘缓存 + 客户端 + CommunityDragon。进程里一份（[`crate::LolPlugin::fetch_asset`]）。
pub struct Assets {
    dir: PathBuf,
    limit: u64,
    max_age: Duration,
    online: Box<dyn Origin>,
    index: Mutex<Option<Index>>,
    /// 同一个路径同时来几次：后来的等前一个取完，再读缓存。
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// CommunityDragon 取失败的路径和时间。
    failed: Mutex<HashMap<String, Instant>>,
    tmp: AtomicU64,
}

impl Assets {
    /// `dir`：缓存目录（用时才建）。在线来源是 CommunityDragon，上限 64 MB，14 天重新取。
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            limit: CACHE_LIMIT,
            max_age: MAX_AGE,
            online: Box::new(CommunityDragon::default()),
            index: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
            failed: Mutex::new(HashMap::new()),
            tmp: AtomicU64::new(0),
        }
    }

    /// 换掉在线来源（测试用）。
    pub fn with_online(mut self, online: impl Origin + 'static) -> Self {
        self.online = Box::new(online);
        self
    }

    /// 换掉缓存上限（测试用）。
    pub fn with_limit(mut self, bytes: u64) -> Self {
        self.limit = bytes;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 取一张图：缓存（新的）→ 客户端（`client`，没开时 None）→ CommunityDragon → 缓存里过期的 → NotFound。
    pub async fn fetch(
        &self,
        client: Option<&dyn Origin>,
        path: &str,
    ) -> Result<Asset, AssetError> {
        let path = normalize(path)?;
        let name = file_name(&path);
        let gate = self
            .inflight
            .lock()
            .unwrap()
            .entry(name.clone())
            .or_default()
            .clone();
        let result = {
            let _turn = gate.lock().await;
            self.fetch_in_turn(client, &path, &name).await
        };
        let mut inflight = self.inflight.lock().unwrap();
        // 表里一份 + 手上一份 = 没有人在等
        if Arc::strong_count(&gate) <= 2 {
            inflight.remove(&name);
        }
        result
    }

    async fn fetch_in_turn(
        &self,
        client: Option<&dyn Origin>,
        path: &str,
        name: &str,
    ) -> Result<Asset, AssetError> {
        let stale = match self.read(name) {
            Some((asset, true)) => return Ok(asset),
            Some((asset, false)) => Some(asset),
            None => None,
        };
        if let Some(client) = client {
            if let Some(asset) = client.fetch(path).await.ok().and_then(image) {
                self.write(name, &asset.bytes);
                return Ok(asset);
            }
        }
        let failed_recently = self
            .failed
            .lock()
            .unwrap()
            .get(name)
            .is_some_and(|at| at.elapsed() < RETRY_AFTER);
        if !failed_recently {
            match self.online.fetch(path).await.ok().and_then(image) {
                Some(asset) => {
                    self.failed.lock().unwrap().remove(name);
                    self.write(name, &asset.bytes);
                    return Ok(asset);
                }
                None => {
                    self.failed
                        .lock()
                        .unwrap()
                        .insert(name.to_string(), Instant::now());
                }
            }
        }
        stale.ok_or(AssetError::NotFound)
    }

    fn with_index<T>(&self, f: impl FnOnce(&mut Index) -> T) -> T {
        let mut guard = self.index.lock().unwrap();
        let index = guard.get_or_insert_with(|| scan(&self.dir));
        f(index)
    }

    /// 缓存里的这张图和它是不是新的（14 天内取的）。坏了的删掉。
    fn read(&self, name: &str) -> Option<(Asset, bool)> {
        let file = self.dir.join(name);
        let meta = std::fs::metadata(&file).ok()?;
        let Some(asset) = std::fs::read(&file).ok().and_then(image) else {
            let _ = std::fs::remove_file(&file);
            self.with_index(|index| index.forget(name));
            return None;
        };
        // 时钟往回拨过（修改时间在将来）也算新的
        let fresh = meta
            .modified()
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .map_or(true, |age| age < self.max_age);
        self.with_index(|index| index.touch(name, meta.len()));
        // 重启以后也知道谁最近用过（不行就算了）
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&file) {
            let _ = f.set_times(std::fs::FileTimes::new().set_accessed(SystemTime::now()));
        }
        Some((asset, fresh))
    }

    /// 写进缓存：先写临时文件再改名；超了上限删最久没用的。
    fn write(&self, name: &str, bytes: &[u8]) {
        let size = bytes.len() as u64;
        if size > self.limit || std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join(format!(
            "{name}.{}-{}.tmp",
            std::process::id(),
            self.tmp.fetch_add(1, Ordering::Relaxed)
        ));
        if std::fs::write(&tmp, bytes).is_err()
            || std::fs::rename(&tmp, self.dir.join(name)).is_err()
        {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        let victims = self.with_index(|index| {
            index.touch(name, size);
            index.evict(self.limit, name)
        });
        for victim in victims {
            let _ = std::fs::remove_file(self.dir.join(victim));
        }
    }
}

/// 缓存文件名：路径（小写）的 SHA-256 前 16 字节 + 扩展名。
fn file_name(path: &str) -> String {
    let hash: String = Sha256::digest(path.to_ascii_lowercase().as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{hash}.{}", extension(path).unwrap_or("bin"))
}

/// 扫一遍缓存目录：按最后访问（没有就按修改）时间排出谁最久没用；顺手删掉很久以前没写完的临时文件。
fn scan(dir: &Path) -> Index {
    let mut found: Vec<(String, u64, SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if name.ends_with(".tmp") {
            let old = SystemTime::now()
                .duration_since(modified)
                .is_ok_and(|age| age > STALE_TMP);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        let used = meta.accessed().unwrap_or(modified).max(modified);
        found.push((name, meta.len(), used));
    }
    found.sort_by_key(|(_, _, used)| *used);
    let mut index = Index::default();
    for (name, size, _) in found {
        index.touch(&name, size);
    }
    index
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// 程序生成的纯色 PNG（`w`×`h`，RGB）：不压缩的 deflate 块 + 手算的 CRC32、Adler-32。仓库里不放拳头的图。
    pub(crate) fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &byte in data {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
        }
        // 每行：过滤方式 0（不过滤）+ w 个像素
        let row: Vec<u8> = std::iter::once(0)
            .chain(rgb.iter().copied().cycle().take(3 * w as usize))
            .collect();
        let raw = row.repeat(h as usize);
        // zlib：头 + 不压缩的块（每块最多 65535 字节）+ Adler-32
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (i, block) in blocks.iter().enumerate() {
            zlib.push(u8::from(i + 1 == blocks.len()));
            let len = block.len() as u16;
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 位 RGB
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// 假来源：按路径回固定内容，记下被问了几次；可以慢一点（测并发）。
    #[derive(Clone, Default)]
    struct Fake {
        files: Arc<HashMap<String, Vec<u8>>>,
        calls: Arc<AtomicUsize>,
        asked: Arc<Mutex<Vec<String>>>,
        delay: Duration,
    }

    impl Fake {
        fn with(files: &[(&str, Vec<u8>)]) -> Self {
            Self {
                files: Arc::new(
                    files
                        .iter()
                        .map(|(p, b)| (p.to_string(), b.clone()))
                        .collect(),
                ),
                ..Self::default()
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Origin for Fake {
        fn fetch<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, String>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.asked.lock().unwrap().push(path.to_string());
                if !self.delay.is_zero() {
                    tokio::time::sleep(self.delay).await;
                }
                self.files
                    .get(path)
                    .cloned()
                    .ok_or_else(|| "404".to_string())
            })
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kotone-lol-assets-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const AHRI: &str = "/lol-game-data/assets/v1/champion-icons/103.png";
    const BOOTS: &str = "/lol-game-data/assets/ASSETS/Items/Icons2D/1001_Class_T1_BootsofSpeed.png";
    const GOLD: &str = "/fe/lol-static-assets/images/ranked-mini-crests/gold.svg";

    #[test]
    fn generated_png_is_a_png() {
        let p = png(2, 2, [200, 40, 90]);
        assert_eq!(sniff(&p), Some("image/png"));
        assert!(p.len() < 100);
    }

    #[test]
    fn only_three_prefixes_of_images_pass() {
        for ok in [
            AHRI,
            "lol-game-data/assets/v1/champion-icons/103.png",
            BOOTS,
            GOLD,
            "/fe/lol-shared-components/gold.png",
            "/lol-game-data/assets/v1/profile-icons/7335.jpg",
            "/lol-game-data/assets/v1/profile-icons/7335.JPEG",
            "/lol-game-data/assets/x.webp",
        ] {
            let path = normalize(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
            assert!(path.starts_with('/'), "{path}");
        }
        assert_eq!(normalize(&AHRI[1..]).unwrap(), AHRI);
        for bad in [
            "",
            "/",
            // 客户端的别的接口：能操作客户端的一律不转
            "/lol-matchmaking/v1/ready-check/accept",
            "/lol-champ-select/v1/session",
            "/lol-summoner/v1/current-summoner",
            "/lol-game-data/assets/v1/champion-summary.json",
            "/lol-game-data/assets/",
            "/fe/lol-static-assets/",
            "/fe/other/x.png",
            "/LOL-GAME-DATA/assets/v1/x.png",
            // 写法
            "/lol-game-data/assets/../../lol-summoner/v1/x.png",
            "/lol-game-data/assets/v1/../x.png",
            "/lol-game-data/assets/./x.png",
            "/lol-game-data/assets//x.png",
            "/lol-game-data/assets/%2e%2e/x.png",
            "/lol-game-data/assets/v1/x.png?y=1",
            "/lol-game-data/assets/v1/x.png#y",
            "/lol-game-data/assets\\v1\\x.png",
            "/lol-game-data/assets/v1/x y.png",
            "/lol-game-data/assets/v1/图.png",
            "//127.0.0.1/lol-game-data/assets/x.png",
            "https://raw.communitydragon.org/latest/x.png",
            "/lol-game-data/assets/v1/x.png.",
            "/lol-game-data/assets/v1/x.exe",
            "/lol-game-data/assets/v1/x",
        ] {
            assert!(normalize(bad).is_err(), "{bad}");
        }
        let long = format!("/lol-game-data/assets/{}.png", "a".repeat(300));
        assert!(normalize(&long).is_err());
    }

    #[test]
    fn community_dragon_paths_are_lowercase() {
        assert_eq!(
            cdragon_url(CDRAGON, BOOTS).as_deref(),
            Some("https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/assets/items/icons2d/1001_class_t1_bootsofspeed.png")
        );
        assert_eq!(
            cdragon_url("http://127.0.0.1:9/", AHRI).as_deref(),
            Some("http://127.0.0.1:9/plugins/rcp-be-lol-game-data/global/default/v1/champion-icons/103.png")
        );
        assert_eq!(
            cdragon_url(CDRAGON, GOLD).as_deref(),
            Some("https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-static-assets/global/default/images/ranked-mini-crests/gold.svg")
        );
        assert_eq!(
            cdragon_url(CDRAGON, "/fe/lol-shared-components/GOLD.png").as_deref(),
            Some("https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-shared-components/global/default/gold.png")
        );
        assert!(cdragon_url(CDRAGON, "/lol-summoner/v1/x.png").is_none());
    }

    #[test]
    fn content_decides_the_type_and_unsafe_svg_is_refused() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        let crest = br##"<svg width="17" height="13" viewBox="0 0 17 13" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M10.7 4.7Z" fill="#C89B3C"/></svg>"##;
        assert_eq!(sniff(crest), Some("image/svg+xml"));
        assert_eq!(
            sniff(b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"></svg>"),
            Some("image/svg+xml")
        );
        for bad in [
            &b"{\"message\":\"not found\"}"[..],
            b"<html><body>404</body></html>",
            b"<svg><script>alert(1)</script></svg>",
            b"<svg onload=\"alert(1)\"></svg>",
            b"<svg/onload=alert(1)></svg>",
            b"<svg><a href=\"javascript:alert(1)\">x</a></svg>",
            b"<svg><foreignObject><iframe/></foreignObject></svg>",
            b"",
        ] {
            assert_eq!(sniff(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn page_paths_pass_the_check() {
        for path in [
            champion_icon(103),
            profile_icon(0),
            profile_icon(7335),
            ranked_crest("GOLD"),
            ranked_crest(""),
        ] {
            assert!(normalize(&path).is_ok(), "{path}");
        }
        assert_eq!(champion_icon(0), "");
        assert_eq!(profile_icon(-1), "");
        assert_eq!(
            ranked_crest(" Diamond "),
            "/fe/lol-static-assets/images/ranked-mini-crests/diamond.svg"
        );
        assert_eq!(
            ranked_crest("../x"),
            "/fe/lol-static-assets/images/ranked-mini-crests/unranked.svg"
        );
    }

    #[tokio::test]
    async fn client_first_then_the_cache() {
        let dir = temp_dir("client");
        let red = png(2, 2, [255, 0, 0]);
        let client = Fake::with(&[(AHRI, red.clone())]);
        let online = Fake::with(&[]);
        let assets = Assets::new(dir.clone()).with_online(online.clone());
        let got = assets.fetch(Some(&client), &AHRI[1..]).await.unwrap();
        assert_eq!((got.mime, got.bytes.clone()), ("image/png", red.clone()));
        assert_eq!((client.calls(), online.calls()), (1, 0));
        // 缓存命中：两边都不问
        let again = assets.fetch(Some(&client), AHRI).await.unwrap();
        assert_eq!(again.bytes, red);
        assert_eq!((client.calls(), online.calls()), (1, 0));
        // 新开一份（重启以后）：磁盘上还在
        let restarted = Assets::new(dir.clone()).with_online(online.clone());
        assert_eq!(restarted.fetch(None, AHRI).await.unwrap().bytes, red);
        assert_eq!(online.calls(), 0);
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1, "没有留下临时文件");
        let name = files[0].file_name().to_string_lossy().to_string();
        assert!(name.ends_with(".png") && name.len() == 36, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_the_client_it_asks_community_dragon() {
        let dir = temp_dir("online");
        let blue = png(3, 1, [0, 0, 255]);
        let online = Fake::with(&[(BOOTS, blue.clone())]);
        let assets = Assets::new(dir.clone()).with_online(online.clone());
        assert_eq!(assets.fetch(None, BOOTS).await.unwrap().bytes, blue);
        // 客户端开着但没有这张图（国服独有的、或者回的不是图片）：也问 CommunityDragon
        let other = png(1, 1, [9, 9, 9]);
        let online2 = Fake::with(&[(AHRI, other.clone())]);
        let client = Fake::with(&[(AHRI, b"{\"message\":\"x\"}".to_vec())]);
        let assets2 = Assets::new(dir.clone()).with_online(online2.clone());
        assert_eq!(
            assets2.fetch(Some(&client), AHRI).await.unwrap().bytes,
            other
        );
        assert_eq!((client.calls(), online2.calls()), (1, 1));
        // 哪儿都没有：NotFound；10 分钟内不再问 CommunityDragon
        let missing = "/lol-game-data/assets/v1/champion-icons/999999.png";
        assert_eq!(
            assets2.fetch(None, missing).await,
            Err(AssetError::NotFound)
        );
        assert_eq!(
            assets2.fetch(None, missing).await,
            Err(AssetError::NotFound)
        );
        assert_eq!(online2.calls(), 2, "失败的那个只问了一次");
        // 不放行的路径谁都不问
        assert!(matches!(
            assets2
                .fetch(Some(&client), "/lol-champ-select/v1/session")
                .await,
            Err(AssetError::Refused(_))
        ));
        assert_eq!((client.calls(), online2.calls()), (1, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_same_path_at_once_is_fetched_once() {
        let dir = temp_dir("once");
        let green = png(2, 2, [0, 255, 0]);
        let mut online = Fake::with(&[(AHRI, green.clone())]);
        online.delay = Duration::from_millis(80);
        let assets = Assets::new(dir.clone()).with_online(online.clone());
        let (a, b, c) = tokio::join!(
            assets.fetch(None, AHRI),
            assets.fetch(None, AHRI),
            assets.fetch(None, &AHRI[1..])
        );
        assert_eq!(a.unwrap().bytes, green);
        assert_eq!(b.unwrap().bytes, green);
        assert_eq!(c.unwrap().bytes, green);
        assert_eq!(online.calls(), 1);
        assert!(assets.inflight.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn over_the_limit_the_least_recently_used_goes() {
        let dir = temp_dir("limit");
        let paths: Vec<String> = (1..=4)
            .map(|i| format!("/lol-game-data/assets/v1/champion-icons/{i}.png"))
            .collect();
        let images: Vec<Vec<u8>> = (1..=4u8).map(|i| png(4, 4, [i, i, i])).collect();
        let size = images[0].len() as u64;
        assert!(images.iter().all(|i| i.len() as u64 == size));
        let files: Vec<(&str, Vec<u8>)> = paths
            .iter()
            .map(String::as_str)
            .zip(images.iter().cloned())
            .collect();
        let online = Fake::with(&files);
        // 放得下 3 张
        let assets = Assets::new(dir.clone())
            .with_online(online.clone())
            .with_limit(size * 3 + size / 2);
        for p in &paths[..3] {
            assets.fetch(None, p).await.unwrap();
        }
        // 第 1 张刚用过，第 2 张最久没用
        assets.fetch(None, &paths[0]).await.unwrap();
        assert_eq!(online.calls(), 3);
        assets.fetch(None, &paths[3]).await.unwrap();
        let on_disk = |p: &str| dir.join(file_name(&normalize(p).unwrap())).exists();
        assert!(on_disk(&paths[0]) && on_disk(&paths[2]) && on_disk(&paths[3]));
        assert!(!on_disk(&paths[1]), "最久没用的删了");
        let total: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(total <= size * 3 + size / 2, "{total}");
        // 删掉的那张再要：重新取
        assets.fetch(None, &paths[1]).await.unwrap();
        assert_eq!(online.calls(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn after_fourteen_days_it_fetches_again_and_keeps_the_old_one_on_failure() {
        let dir = temp_dir("expire");
        let old = png(2, 2, [1, 2, 3]);
        let new = png(2, 2, [4, 5, 6]);
        let first = Fake::with(&[(AHRI, old.clone())]);
        let assets = Assets::new(dir.clone()).with_online(first.clone());
        assets.fetch(None, AHRI).await.unwrap();
        let file = dir.join(file_name(AHRI));
        let age = |days: u64| {
            let f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 3600))
                .unwrap();
        };
        // 13 天：还是新的
        age(13);
        assert_eq!(assets.fetch(None, AHRI).await.unwrap().bytes, old);
        assert_eq!(first.calls(), 1);
        // 15 天：重新取，换成新的
        age(15);
        let second = Fake::with(&[(AHRI, new.clone())]);
        let assets = Assets::new(dir.clone()).with_online(second.clone());
        assert_eq!(assets.fetch(None, AHRI).await.unwrap().bytes, new);
        assert_eq!(second.calls(), 1);
        assert_eq!(std::fs::read(&file).unwrap(), new);
        // 过期了、又取不到（没网）：用旧的那份
        age(15);
        let offline = Fake::with(&[]);
        let assets = Assets::new(dir.clone()).with_online(offline.clone());
        assert_eq!(assets.fetch(None, AHRI).await.unwrap().bytes, new);
        assert_eq!(offline.calls(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_broken_cache_file_is_replaced() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file_name(GOLD)), b"not an image").unwrap();
        // 很久以前没写完的临时文件：扫目录时删掉
        let leftover = dir.join("abc.png.1-1.tmp");
        std::fs::write(&leftover, b"half").unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&leftover)
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(2 * 3600))
            .unwrap();
        drop(f);
        let svg =
            br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h1v1z"/></svg>"#.to_vec();
        let online = Fake::with(&[(GOLD, svg.clone())]);
        let assets = Assets::new(dir.clone()).with_online(online.clone());
        let got = assets.fetch(None, GOLD).await.unwrap();
        assert_eq!((got.mime, got.bytes), ("image/svg+xml", svg));
        assert!(!leftover.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真的走一遍 HTTP：本机的假 CommunityDragon（只回一张图，别的 404），看换算出来的地址和取图的代码。
    #[tokio::test]
    async fn community_dragon_over_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = png(2, 2, [7, 7, 7]);
        let served = body.clone();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let reply = if path
                    == "/latest/plugins/rcp-be-lol-game-data/global/default/assets/items/icons2d/1001_class_t1_bootsofspeed.png"
                {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&served);
                    r
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                };
                let _ = socket.write_all(&reply).await;
                let _ = socket.shutdown().await;
            }
        });
        let dir = temp_dir("http");
        let online = CommunityDragon::with_base(&format!("http://127.0.0.1:{port}/latest/"));
        let assets = Assets::new(dir.clone()).with_online(online);
        assert_eq!(assets.fetch(None, BOOTS).await.unwrap().bytes, body);
        assert_eq!(assets.fetch(None, AHRI).await, Err(AssetError::NotFound));
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen[1].ends_with("/v1/champion-icons/103.png"), "{seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
