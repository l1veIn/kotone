//! 播放器（ADR 034 第三节）：一个播放列表，一次放一首，放完自动下一首；她说话时压低（D7）。
//!
//! - 选歌（D6）：要 VIP / 付费 / 没版权的、链接只有试听片段的都跳过，往后找能完整放的；跳过的连同
//!   原因交给调用的人（她说“原唱要 VIP，先放了 xx 的版本”）。一次最多试 [`MAX_TRIES`] 首，免得一直请求。
//! - 链接 20 分钟过期：刚开始就下载失败（HTTP 错误）时重新要一次链接再放，只重试一次。
//! - 状态通过 `on_state` 交出去：每次变化一次，放的时候每秒一次。

mod download;
mod output;
mod playback;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{MusicError, MusicProvider, Playable, Quality, StreamInfo, Track};
use output::Control;
use playback::{Event, Playback};

/// 一次选歌最多要几首的播放链接。
pub const MAX_TRIES: usize = 8;
/// 她说话时音乐压到原来的多少（D7）。
pub const DUCK_GAIN: f32 = 0.25;
/// “上一首”：放过这么久就从头放这一首。
const RESTART_AFTER_MS: u64 = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Error,
}

/// 播放器现在的样子（界面、她的 `music` 工具都看这个）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub status: Status,
    pub track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: u8,
    pub index: usize,
    /// 前端合同里叫 `queueLength`。
    #[serde(rename = "queueLength")]
    pub queue_len: usize,
    pub liked: bool,
    /// 放的是什么：“每日推荐”、歌单名、搜的词。
    pub source: Option<String>,
    pub error: Option<String>,
}

/// 选歌的结果：开始放的那首，和跳过的（带原因）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayOutcome {
    pub started: Option<Track>,
    pub skipped: Vec<(Track, String)>,
}

struct Data {
    queue: Vec<Track>,
    index: usize,
    status: Status,
    source: Option<String>,
    error: Option<String>,
    volume: u8,
    quality: Quality,
    liked: bool,
    /// 私人 FM：放完这一批自动要下一批。
    fm: bool,
    duration_ms: u64,
    /// 这一首已经因为链接过期重试过了。
    retried: bool,
}

struct Inner {
    provider: Arc<dyn MusicProvider>,
    on_state: Arc<dyn Fn(PlayerState) + Send + Sync>,
    rt: tokio::runtime::Handle,
    _own_runtime: Option<tokio::runtime::Runtime>,
    http: reqwest::Client,
    data: Mutex<Data>,
    playback: Mutex<Option<Playback>>,
    generation: AtomicU64,
    ducked: AtomicBool,
    op: tokio::sync::Mutex<()>,
}

/// 播放器。`on_state` 在任意线程上调用，要快。
pub struct Player {
    inner: Arc<Inner>,
}

impl Player {
    /// 在 tokio 运行时里建（拿当前运行时）；不在的话自己开一个。
    pub fn new(
        provider: Arc<dyn MusicProvider>,
        on_state: Arc<dyn Fn(PlayerState) + Send + Sync>,
    ) -> Self {
        let (rt, own) = match tokio::runtime::Handle::try_current() {
            Ok(handle) => (handle, None),
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .expect("music runtime");
                (runtime.handle().clone(), Some(runtime))
            }
        };
        let inner = Arc::new(Inner {
            provider,
            on_state,
            rt,
            _own_runtime: own,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            data: Mutex::new(Data {
                queue: Vec::new(),
                index: 0,
                status: Status::Idle,
                source: None,
                error: None,
                volume: 70,
                quality: Quality::default(),
                liked: false,
                fm: false,
                duration_ms: 0,
                retried: false,
            }),
            playback: Mutex::new(None),
            generation: AtomicU64::new(0),
            ducked: AtomicBool::new(false),
            op: tokio::sync::Mutex::new(()),
        });
        // 放的时候每秒报一次进度。
        let weak = Arc::downgrade(&inner);
        inner.rt.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(inner) = weak.upgrade() else { break };
                if inner.data.lock().unwrap().status == Status::Playing {
                    inner.emit();
                }
            }
        });
        Self { inner }
    }

    pub fn set_quality(&self, quality: Quality) {
        self.inner.data.lock().unwrap().quality = quality;
    }

    /// 音量 0–100。
    pub fn set_volume(&self, volume: u8) {
        self.inner.data.lock().unwrap().volume = volume.min(100);
        self.inner.apply_gain();
        self.inner.emit();
    }

    /// 她开口 / 说完（D7）。
    pub fn duck(&self, on: bool) {
        self.inner.ducked.store(on, Ordering::SeqCst);
        self.inner.apply_gain();
    }

    /// 从 `tracks[index]` 开始放这一列（不能放的往后跳）。
    pub async fn play(
        &self,
        tracks: Vec<Track>,
        index: usize,
        source: Option<String>,
    ) -> Result<PlayOutcome, MusicError> {
        let _op = self.inner.op.lock().await;
        self.inner.play_list(tracks, index, source, false).await
    }

    /// 搜歌直接放：最匹配的放不了就放第一首能完整放的，跳过的写进结果（D6）。
    pub async fn play_query(&self, query: &str) -> Result<PlayOutcome, MusicError> {
        let (tracks, _) = self.inner.provider.search(query, 0, 20).await?;
        let _op = self.inner.op.lock().await;
        self.inner
            .play_list(tracks, 0, Some(query.to_string()), false)
            .await
    }

    /// 私人 FM：放完这一批自动要下一批。
    pub async fn play_fm(&self, source: Option<String>) -> Result<PlayOutcome, MusicError> {
        let tracks = self.inner.provider.fm().await?;
        let _op = self.inner.op.lock().await;
        self.inner.play_list(tracks, 0, source, true).await
    }

    pub fn pause(&self) {
        self.inner.set_paused(true);
    }

    pub fn resume(&self) {
        self.inner.set_paused(false);
    }

    pub fn toggle(&self) {
        let paused = self.inner.data.lock().unwrap().status == Status::Paused;
        self.inner.set_paused(!paused);
    }

    pub fn stop(&self) {
        self.inner.halt(Status::Idle);
        self.inner.emit();
    }

    pub async fn next(&self) -> Result<PlayOutcome, MusicError> {
        let _op = self.inner.op.lock().await;
        self.inner.advance().await
    }

    pub async fn previous(&self) -> Result<PlayOutcome, MusicError> {
        let _op = self.inner.op.lock().await;
        let position = self.inner.position_ms();
        let (queue, index, current) = {
            let data = self.inner.data.lock().unwrap();
            (
                data.queue.clone(),
                data.index,
                data.queue.get(data.index).cloned(),
            )
        };
        if position > RESTART_AFTER_MS || index == 0 {
            self.seek(0);
            return Ok(PlayOutcome {
                started: current,
                skipped: Vec::new(),
            });
        }
        let order: Vec<usize> = (0..index).rev().collect();
        self.inner.start_first(&queue, order).await
    }

    pub fn seek(&self, ms: u64) {
        if let Some(playback) = self.inner.playback.lock().unwrap().as_ref() {
            playback.control.request_seek(ms);
        }
    }

    /// 红心 / 取消红心正在放的这首。
    pub async fn like(&self, like: bool) -> Result<(), MusicError> {
        let id = self
            .inner
            .current()
            .map(|t| t.id)
            .ok_or_else(|| MusicError::Api {
                code: 0,
                message: "现在没有在放歌".into(),
            })?;
        self.inner.provider.like(&id, like).await?;
        self.inner.data.lock().unwrap().liked = like;
        self.inner.emit();
        Ok(())
    }

    pub fn state(&self) -> PlayerState {
        self.inner.state()
    }

    pub fn queue(&self) -> (Vec<Track>, usize) {
        let data = self.inner.data.lock().unwrap();
        (data.queue.clone(), data.index)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.inner.halt(Status::Idle);
    }
}

/// 从 `order` 里按顺序找第一首能完整放的：先看曲目信息（不用请求），再要链接看是不是试听。
/// 遇到要登录 / 风控马上返回错误（不再往下试）。
pub(crate) async fn pick(
    provider: &dyn MusicProvider,
    quality: Quality,
    tracks: &[Track],
    order: impl IntoIterator<Item = usize>,
) -> Result<(Option<(usize, StreamInfo)>, Vec<(Track, String)>), MusicError> {
    let mut skipped = Vec::new();
    let mut tries = 0;
    for i in order {
        let Some(track) = tracks.get(i) else { continue };
        if track.playable != Playable::Full {
            let reason = track.reason.clone().unwrap_or_else(|| "放不了".into());
            skipped.push((track.clone(), reason));
            continue;
        }
        if tries >= MAX_TRIES {
            break;
        }
        tries += 1;
        match provider.stream(&track.id, quality).await {
            Ok(Some(stream)) if !stream.trial => return Ok((Some((i, stream)), skipped)),
            Ok(Some(_)) => skipped.push((track.clone(), "只有试听片段（要网易云音乐 VIP）".into())),
            Ok(None) => skipped.push((track.clone(), "拿不到播放链接（可能没有版权）".into())),
            Err(e @ (MusicError::NeedLogin | MusicError::RateLimited(_))) => return Err(e),
            Err(e) => skipped.push((track.clone(), e.to_string())),
        }
    }
    Ok((None, skipped))
}

/// 链接里看格式（帮解码器认）。
fn extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    (ext.len() <= 4 && ext.chars().all(|c| c.is_ascii_alphanumeric())).then_some(ext)
}

/// 音量曲线：平方（听感上更均匀）。
fn gain_of(volume: u8, ducked: bool) -> f32 {
    let v = volume.min(100) as f32 / 100.0;
    v * v * if ducked { DUCK_GAIN } else { 1.0 }
}

impl Inner {
    fn current(&self) -> Option<Track> {
        let data = self.data.lock().unwrap();
        data.queue.get(data.index).cloned()
    }

    fn position_ms(&self) -> u64 {
        self.playback
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.control.position_ms())
            .unwrap_or(0)
    }

    fn state(&self) -> PlayerState {
        let position_ms = self.position_ms();
        let data = self.data.lock().unwrap();
        let track = if data.status == Status::Idle && data.queue.is_empty() {
            None
        } else {
            data.queue.get(data.index).cloned()
        };
        let duration_ms = if data.duration_ms > 0 {
            data.duration_ms
        } else {
            track.as_ref().map(|t| t.duration_ms).unwrap_or(0)
        };
        PlayerState {
            status: data.status,
            track,
            position_ms: position_ms.min(duration_ms.max(position_ms)),
            duration_ms,
            volume: data.volume,
            index: data.index,
            queue_len: data.queue.len(),
            liked: data.liked,
            source: data.source.clone(),
            error: data.error.clone(),
        }
    }

    fn emit(&self) {
        (self.on_state)(self.state());
    }

    fn apply_gain(&self) {
        let gain = gain_of(
            self.data.lock().unwrap().volume,
            self.ducked.load(Ordering::SeqCst),
        );
        if let Some(playback) = self.playback.lock().unwrap().as_ref() {
            playback.control.set_gain(gain);
        }
    }

    fn set_paused(&self, paused: bool) {
        let has = self.playback.lock().unwrap().as_ref().map(|p| {
            p.control.paused.store(paused, Ordering::SeqCst);
        });
        if has.is_some() {
            let mut data = self.data.lock().unwrap();
            if matches!(
                data.status,
                Status::Playing | Status::Paused | Status::Loading
            ) {
                data.status = if paused {
                    Status::Paused
                } else {
                    Status::Playing
                };
            }
        }
        self.emit();
    }

    /// 停掉正在放的（不报事件），状态改成 `status`。
    fn halt(&self, status: Status) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(playback) = self.playback.lock().unwrap().take() {
            playback.stop();
        }
        self.data.lock().unwrap().status = status;
    }

    async fn play_list(
        self: &Arc<Self>,
        tracks: Vec<Track>,
        index: usize,
        source: Option<String>,
        fm: bool,
    ) -> Result<PlayOutcome, MusicError> {
        let quality = self.data.lock().unwrap().quality;
        let (found, skipped) = pick(&*self.provider, quality, &tracks, index..tracks.len()).await?;
        let Some((i, stream)) = found else {
            // 一首都放不了：正在放的不动。
            return Ok(PlayOutcome {
                started: None,
                skipped,
            });
        };
        let started = tracks[i].clone();
        {
            let mut data = self.data.lock().unwrap();
            data.queue = tracks;
            data.source = source;
            data.fm = fm;
        }
        self.start(i, stream);
        Ok(PlayOutcome {
            started: Some(started),
            skipped,
        })
    }

    async fn start_first(
        self: &Arc<Self>,
        queue: &[Track],
        order: Vec<usize>,
    ) -> Result<PlayOutcome, MusicError> {
        let quality = self.data.lock().unwrap().quality;
        let (found, skipped) = pick(&*self.provider, quality, queue, order).await?;
        let Some((i, stream)) = found else {
            return Ok(PlayOutcome {
                started: None,
                skipped,
            });
        };
        self.start(i, stream);
        Ok(PlayOutcome {
            started: queue.get(i).cloned(),
            skipped,
        })
    }

    /// 下一首；放到头了：私人 FM 要下一批，否则停。
    async fn advance(self: &Arc<Self>) -> Result<PlayOutcome, MusicError> {
        let (queue, index, fm) = {
            let data = self.data.lock().unwrap();
            (data.queue.clone(), data.index, data.fm)
        };
        let outcome = self
            .start_first(&queue, (index + 1..queue.len()).collect())
            .await?;
        if outcome.started.is_some() || !fm {
            if outcome.started.is_none() {
                self.halt(Status::Idle);
                self.emit();
            }
            return Ok(outcome);
        }
        let more = self.provider.fm().await?;
        let start = {
            let mut data = self.data.lock().unwrap();
            let start = data.queue.len();
            data.queue.extend(more);
            start
        };
        let queue = self.data.lock().unwrap().queue.clone();
        let mut next = self
            .start_first(&queue, (start..queue.len()).collect())
            .await?;
        next.skipped.splice(0..0, outcome.skipped);
        if next.started.is_none() {
            self.halt(Status::Idle);
            self.emit();
        }
        Ok(next)
    }

    /// 开始放队列里第 `index` 首（链接已经拿到了）。
    fn start(self: &Arc<Self>, index: usize, stream: StreamInfo) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(old) = self.playback.lock().unwrap().take() {
            old.stop();
        }
        let (gain, track_id) = {
            let mut data = self.data.lock().unwrap();
            data.index = index;
            data.status = Status::Loading;
            data.error = None;
            data.liked = false;
            data.duration_ms = 0;
            data.retried = false;
            (
                gain_of(data.volume, self.ducked.load(Ordering::SeqCst)),
                data.queue.get(index).map(|t| t.id.clone()),
            )
        };
        self.spawn_playback(generation, &stream, gain);
        self.emit();
        // 红心状态（要请求，放在后台）
        if let Some(id) = track_id {
            let weak = Arc::downgrade(self);
            self.rt.spawn(async move {
                let Some(inner) = weak.upgrade() else { return };
                if let Ok(liked) = inner.provider.is_liked(&id).await {
                    if inner.generation.load(Ordering::SeqCst) == generation {
                        inner.data.lock().unwrap().liked = liked;
                        inner.emit();
                    }
                }
            });
        }
    }

    fn spawn_playback(self: &Arc<Self>, generation: u64, stream: &StreamInfo, gain: f32) {
        let download = download::Download::new();
        self.rt.spawn(download::fetch(
            self.http.clone(),
            stream.url.clone(),
            download.clone(),
        ));
        let control = Control::new(gain);
        let weak: Weak<Inner> = Arc::downgrade(self);
        let playback = Playback::start(
            download,
            extension(&stream.url),
            control,
            Box::new(move |event| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                inner.on_event(generation, event);
            }),
        );
        *self.playback.lock().unwrap() = Some(playback);
    }

    fn on_event(self: &Arc<Self>, generation: u64, event: Event) {
        match event {
            Event::Started { duration_ms } => {
                {
                    let mut data = self.data.lock().unwrap();
                    if let Some(ms) = duration_ms {
                        data.duration_ms = ms;
                    }
                    if data.status == Status::Loading {
                        data.status = Status::Playing;
                    }
                }
                self.emit();
            }
            Event::Ended => {
                let inner = self.clone();
                self.rt.spawn(async move {
                    let _op = inner.op.lock().await;
                    if inner.generation.load(Ordering::SeqCst) == generation {
                        if let Err(e) = inner.advance().await {
                            inner.fail(e.to_string());
                        }
                    }
                });
            }
            Event::Failed(message) => {
                kotone_core::log::log(&format!("music: playback failed: {message}"));
                let inner = self.clone();
                self.rt.spawn(async move {
                    let _op = inner.op.lock().await;
                    if inner.generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    inner.recover(generation, message).await;
                });
            }
        }
    }

    /// 放到一半出错：刚开始就 HTTP 出错（多半链接过期）先重新要一次链接；设备坏了就停；
    /// 别的跳到下一首。
    async fn recover(self: &Arc<Self>, generation: u64, message: String) {
        let device = message.contains("输出设备") || message.contains("放不出声音");
        if device {
            self.fail(message);
            return;
        }
        let (retry, track, quality) = {
            let data = self.data.lock().unwrap();
            (
                !data.retried && message.starts_with("HTTP "),
                data.queue.get(data.index).cloned(),
                data.quality,
            )
        };
        if retry {
            if let Some(track) = track {
                if let Ok(Some(stream)) = self.provider.stream(&track.id, quality).await {
                    if self.generation.load(Ordering::SeqCst) == generation && !stream.trial {
                        let gain = gain_of(
                            self.data.lock().unwrap().volume,
                            self.ducked.load(Ordering::SeqCst),
                        );
                        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                        if let Some(old) = self.playback.lock().unwrap().take() {
                            old.stop();
                        }
                        self.data.lock().unwrap().retried = true;
                        self.spawn_playback(generation, &stream, gain);
                        return;
                    }
                }
            }
        }
        self.data.lock().unwrap().error = Some(message.clone());
        match self.advance().await {
            Ok(outcome) if outcome.started.is_some() => {}
            Ok(_) => self.fail(message),
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn fail(&self, message: String) {
        self.halt(Status::Error);
        self.data.lock().unwrap().error = Some(message);
        self.emit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Account, Playlist, QrPoll, QrStart};
    use async_trait::async_trait;
    use std::collections::HashMap;

    /// 假来源：按 ID 给链接（`trial` 表示试听），记下要了几次链接。
    struct Fake {
        streams: HashMap<String, Option<StreamInfo>>,
        calls: Mutex<Vec<String>>,
        error: Option<MusicError>,
    }

    fn track(id: &str, playable: Playable) -> Track {
        Track {
            id: id.into(),
            name: id.into(),
            artists: vec!["a".into()],
            album: String::new(),
            cover_url: None,
            duration_ms: 1000,
            playable,
            reason: (playable != Playable::Full).then(|| "要网易云音乐 VIP".into()),
        }
    }

    fn stream(trial: bool) -> Option<StreamInfo> {
        Some(StreamInfo {
            url: "http://m8.music.126.net/a.mp3?x=1".into(),
            bitrate: 320_000,
            trial,
        })
    }

    #[async_trait]
    impl MusicProvider for Fake {
        fn id(&self) -> &'static str {
            "ne"
        }
        async fn account(&self) -> Result<Option<Account>, MusicError> {
            Ok(None)
        }
        async fn login_qr_start(&self) -> Result<QrStart, MusicError> {
            unimplemented!()
        }
        async fn login_qr_poll(&self, _: &str) -> Result<QrPoll, MusicError> {
            unimplemented!()
        }
        async fn logout(&self) -> Result<(), MusicError> {
            Ok(())
        }
        async fn search(&self, _: &str, _: u32, _: u32) -> Result<(Vec<Track>, bool), MusicError> {
            Ok((Vec::new(), false))
        }
        async fn stream(&self, id: &str, _: Quality) -> Result<Option<StreamInfo>, MusicError> {
            self.calls.lock().unwrap().push(id.into());
            if let Some(e) = &self.error {
                return Err(e.clone());
            }
            Ok(self.streams.get(id).cloned().flatten())
        }
        async fn daily(&self) -> Result<Vec<Track>, MusicError> {
            Ok(Vec::new())
        }
        async fn playlists(&self) -> Result<Vec<Playlist>, MusicError> {
            Ok(Vec::new())
        }
        async fn playlist(&self, _: &str) -> Result<(Playlist, Vec<Track>), MusicError> {
            Err(MusicError::NeedLogin)
        }
        async fn liked(&self) -> Result<Vec<Track>, MusicError> {
            Ok(Vec::new())
        }
        async fn fm(&self) -> Result<Vec<Track>, MusicError> {
            Ok(Vec::new())
        }
        async fn like(&self, _: &str, _: bool) -> Result<(), MusicError> {
            Ok(())
        }
        async fn is_liked(&self, _: &str) -> Result<bool, MusicError> {
            Ok(false)
        }
    }

    fn fake(streams: &[(&str, Option<StreamInfo>)]) -> Fake {
        Fake {
            streams: streams
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            calls: Mutex::new(Vec::new()),
            error: None,
        }
    }

    #[tokio::test]
    async fn pick_skips_vip_trials_and_missing_links_and_says_why() {
        let tracks = vec![
            track("ne:1", Playable::Vip),
            track("ne:2", Playable::Full),
            track("ne:3", Playable::Full),
            track("ne:4", Playable::Full),
        ];
        let provider = fake(&[
            ("ne:2", stream(true)),
            ("ne:3", None),
            ("ne:4", stream(false)),
        ]);
        let (found, skipped) = pick(&provider, Quality::Exhigh, &tracks, 0..4)
            .await
            .unwrap();
        assert_eq!(found.map(|(i, _)| i), Some(3));
        let reasons: Vec<&str> = skipped.iter().map(|(_, r)| r.as_str()).collect();
        assert_eq!(reasons.len(), 3);
        assert!(
            reasons[0].contains("VIP")
                && reasons[1].contains("试听")
                && reasons[2].contains("链接")
        );
        // VIP 的那首不用要链接
        assert_eq!(
            *provider.calls.lock().unwrap(),
            vec!["ne:2", "ne:3", "ne:4"]
        );
    }

    #[tokio::test]
    async fn pick_gives_up_after_max_tries_and_passes_login_errors_up() {
        let tracks: Vec<Track> = (0..20)
            .map(|i| track(&format!("ne:{i}"), Playable::Full))
            .collect();
        let provider = fake(&[]);
        let (found, skipped) = pick(&provider, Quality::Exhigh, &tracks, 0..20)
            .await
            .unwrap();
        assert!(found.is_none());
        assert_eq!(provider.calls.lock().unwrap().len(), MAX_TRIES);
        assert_eq!(skipped.len(), MAX_TRIES);
        let mut provider = fake(&[]);
        provider.error = Some(MusicError::NeedLogin);
        assert_eq!(
            pick(&provider, Quality::Exhigh, &tracks, 0..20)
                .await
                .unwrap_err(),
            MusicError::NeedLogin
        );
        // 倒着找（上一首）
        let provider = fake(&[("ne:1", stream(false)), ("ne:0", stream(false))]);
        let (found, _) = pick(&provider, Quality::Exhigh, &tracks, vec![1, 0])
            .await
            .unwrap();
        assert_eq!(found.map(|(i, _)| i), Some(1));
    }

    #[tokio::test]
    async fn nothing_playable_leaves_the_player_idle_and_reports_reasons() {
        let states = Arc::new(Mutex::new(Vec::<PlayerState>::new()));
        let sink = states.clone();
        let player = Player::new(
            Arc::new(fake(&[("ne:2", stream(true))])),
            Arc::new(move |s| sink.lock().unwrap().push(s)),
        );
        let outcome = player
            .play(
                vec![track("ne:1", Playable::Vip), track("ne:2", Playable::Full)],
                0,
                Some("x".into()),
            )
            .await
            .unwrap();
        assert!(outcome.started.is_none());
        assert_eq!(outcome.skipped.len(), 2);
        let state = player.state();
        assert_eq!(state.status, Status::Idle);
        assert!(
            state.track.is_none() && state.queue_len == 0,
            "queue untouched"
        );
        player.set_volume(150);
        assert_eq!(player.state().volume, 100);
        assert!(!states.lock().unwrap().is_empty(), "volume change emitted");
        player.duck(true);
        player.pause();
        assert_eq!(
            player.state().status,
            Status::Idle,
            "pausing nothing changes nothing"
        );
        assert!(player.like(true).await.is_err(), "nothing playing");
    }

    #[test]
    fn helpers() {
        assert_eq!(
            extension("http://m8.music.126.net/2026/a/b.mp3?vuutv=x").as_deref(),
            Some("mp3")
        );
        assert_eq!(extension("http://x/y.flac").as_deref(), Some("flac"));
        assert_eq!(extension("http://x/y"), None);
        assert_eq!(gain_of(100, false), 1.0);
        assert_eq!(gain_of(50, false), 0.25);
        assert_eq!(gain_of(100, true), DUCK_GAIN);
        assert_eq!(gain_of(0, false), 0.0);
        let state = PlayerState {
            status: Status::Playing,
            track: None,
            position_ms: 1,
            duration_ms: 2,
            volume: 70,
            index: 0,
            queue_len: 0,
            liked: false,
            source: None,
            error: None,
        };
        let v = serde_json::to_value(&state).unwrap();
        assert_eq!(v["status"], "playing");
        assert_eq!(v["positionMs"], 1);
        assert_eq!(v["queueLength"], 0);
    }
}
