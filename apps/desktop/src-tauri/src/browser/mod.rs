//! 内置浏览器（ADR 031）：聊天页右侧栏里的完整浏览器（多标签页、地址栏、前进后退），
//! 后台任务经调试协议操作“她的标签页”。
//!
//! - 每个标签页是主窗口（`main`，关闭时只是隐藏）里的一个 Tauri 子网页视图（`add_child`，label `browser-N`）。
//!   所有标签页共用 `<KOTONE_HOME>/browser/` 一份数据（登录共享；独立的 WebView2 环境，和 Kotone 自己的界面
//!   分开）。只有当前标签页摆在右侧栏报来的区域里，其余隐藏、不销毁：页面、登录、正在放的歌都保留（照 DSH 桌面版
//!   侧栏浏览器 `keepMounted` 与按工作区共享分区，`.agents/notes/implemented/feature/2026-09-16-sidebar-browser.md`、
//!   `2026-09-20-desktop-browser-webview.md`，MIT；标签页、地址栏、前进后退照 Codex 内置浏览器）。
//! - 访客策略（照同一篇 DSH 记录）：只许不带账号密码的 http(s)，拦本机、内网、Kotone 自己的地址
//!   （网址检查照 xylt369 url-guard）；拒绝权限请求、下载、HTTP 认证弹窗；网页里的新窗口进新标签页；
//!   网页不能抢焦点、弹对话框。网页调用不了 Kotone 的命令：Tauri 2.11 对非本地来源一律拒绝自定义命令
//!   （`webview/mod.rs` 的 remote-origin 守卫），我们的 capabilities 也没有 `remote`（见测试）。
//! - 被全屏游戏挡住时收不到输入（wqty123/dsh-browser `docs/limitations.md`）：浏览器环境加
//!   `--disable-features=CalculateNativeWinOcclusion` 等启动参数；点击、输入走页面内的 `click()` /
//!   原生 setter，不走输入管线。
//! - 她（后台任务）只用自己的标签页（`agent`）：第一次 `browser.open` 新开一个并切过去，玩家的标签页她不动。

mod cdp;
mod extensions;
mod page;
mod view;

pub use extensions::{
    add as extension_add, list as extensions, remove as extension_remove,
    toggle as extension_toggle, ExtensionInfo,
};
pub use page::*;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use kotone_core::log;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Webview};

/// 标签页视图的 label 前缀（`browser-1`、`browser-2`……）。capabilities 不能匹配它们（见测试）。
pub const LABEL_PREFIX: &str = "browser-";
pub const EVENT: &str = "kotone://browser";

/// 一个标签页（交给界面）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub id: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub site: Option<String>,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// 这个标签页在出声。
    pub audible: bool,
    /// 她（后台任务）在用的标签页。
    pub agent: bool,
    /// 页面里有元素要全屏（例如视频的全屏按钮）。
    pub fullscreen_element: bool,
}

/// 交给界面的浏览器状态（`kotone://browser`）。`url / title / site / loading` 是当前标签页的；
/// `media` 是正在出声的标签页的（没有就当前标签页），卡片上的播放控制对它。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub open: bool,
    pub url: Option<String>,
    pub title: Option<String>,
    pub site: Option<String>,
    pub loading: bool,
    pub handover: Option<Handover>,
    pub media: Option<Media>,
    pub busy: bool,
    /// 浏览器没能打开时给玩家看的原因（右侧栏网页卡片上显示）。
    pub error: Option<String>,
    /// 一句临时提示（例如网页想下载文件被拦下），几秒后自己消失。
    pub notice: Option<String>,
    pub tabs: Vec<BrowserTab>,
    pub active_tab: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handover {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub playing: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// 右侧栏里浏览器区域的位置（逻辑像素，相对窗口内容区左上角）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

/// 最近一次读页面（编号只认这一次的；导航后清空，旧编号就会被拒绝）。
#[derive(Default)]
struct Memo {
    url: String,
    text: String,
    refs: Vec<kotone_agent::browser::SnapshotRef>,
    next: Option<usize>,
}

/// 一个标签页。`V` 是网页视图（测试里用 `()`）。
struct Tab<V = Webview> {
    info: BrowserTab,
    webview: V,
    memo: Option<Memo>,
    media: Option<Media>,
}

struct Inner<V = Webview> {
    tabs: Vec<Tab<V>>,
    active: Option<String>,
    /// 她的标签页。
    agent: Option<String>,
    handover: Option<Handover>,
    handover_tx: Option<tokio::sync::oneshot::Sender<()>>,
    busy: usize,
    error: Option<String>,
    notice: Option<String>,
    bounds: Bounds,
    next_id: u64,
    /// 这次运行里扩展和 WebView2 对过一遍没有（第一个标签页建好时对一次）。
    extensions_synced: bool,
    /// 上一次发出去的状态（有变化才发事件）。
    last: BrowserState,
}

impl<V> Default for Inner<V> {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            agent: None,
            handover: None,
            handover_tx: None,
            busy: 0,
            error: None,
            notice: None,
            bounds: Bounds::default(),
            next_id: 0,
            extensions_synced: false,
            last: BrowserState::default(),
        }
    }
}

/// 标签页的账（纯逻辑，不碰网页视图）。
impl<V> Inner<V> {
    fn tab(&self, id: &str) -> Option<&Tab<V>> {
        self.tabs.iter().find(|t| t.info.id == id)
    }

    fn tab_mut(&mut self, id: &str) -> Option<&mut Tab<V>> {
        self.tabs.iter_mut().find(|t| t.info.id == id)
    }

    /// 给要建的标签页取个编号；满了（[`MAX_TABS`](kotone_agent::browser::MAX_TABS)）报错。
    fn reserve(&mut self) -> Result<String, String> {
        if self.tabs.len() >= kotone_agent::browser::MAX_TABS {
            return Err(format!(
                "最多开 {} 个标签页：先关掉一个",
                kotone_agent::browser::MAX_TABS
            ));
        }
        self.next_id += 1;
        Ok(self.next_id.to_string())
    }

    /// 放进一个建好的标签页。`agent`：她的（换掉原来那个的身份，原来的留给玩家）；`select`：切过去
    /// （还没有当前标签页时也切过去）。
    fn insert(&mut self, id: &str, webview: V, agent: bool, select: bool) {
        self.tabs.push(Tab {
            info: BrowserTab {
                id: id.to_string(),
                ..Default::default()
            },
            webview,
            memo: None,
            media: None,
        });
        self.error = None;
        if agent {
            self.agent = Some(id.to_string());
        }
        if select || self.active.is_none() {
            self.active = Some(id.to_string());
        }
    }

    fn select(&mut self, id: &str) -> Result<(), String> {
        if self.tab(id).is_none() {
            return Err("没有这个标签页".into());
        }
        self.active = Some(id.to_string());
        Ok(())
    }

    /// 拿掉一个标签页（交回它的视图去关）：关的是当前的就切到右边那个，没有就左边那个；
    /// 关的是她的，她下次再开新的，正在等玩家登录的交接就此结束（丢掉发送端 = “关掉了”）。
    fn remove(&mut self, id: &str) -> Result<V, String> {
        let Some(index) = self.tabs.iter().position(|t| t.info.id == id) else {
            return Err("没有这个标签页".into());
        };
        let tab = self.tabs.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|i| self.tabs.get(i)))
                .map(|t| t.info.id.clone());
        }
        if self.agent.as_deref() == Some(id) {
            self.agent = None;
            self.handover_tx = None;
            self.handover = None;
        }
        Ok(tab.webview)
    }

    /// 交给界面的状态：`url / title / site / loading` 是当前标签页的；`media` 是正在出声的标签页的
    /// （没有就当前标签页）。
    fn compose(&self) -> BrowserState {
        let active = self.active.as_deref().and_then(|id| self.tab(id));
        let playing = self.tabs.iter().find(|t| t.info.audible).or(active);
        BrowserState {
            open: !self.tabs.is_empty(),
            url: active.and_then(|t| t.info.url.clone()),
            title: active.and_then(|t| t.info.title.clone()),
            site: active.and_then(|t| t.info.site.clone()),
            loading: active.is_some_and(|t| t.info.loading),
            handover: self.handover.clone(),
            media: playing.and_then(|t| t.media.clone()),
            busy: self.busy > 0,
            error: self.error.clone(),
            notice: self.notice.clone(),
            tabs: self
                .tabs
                .iter()
                .map(|t| BrowserTab {
                    agent: self.agent.as_deref() == Some(t.info.id.as_str()),
                    ..t.info.clone()
                })
                .collect(),
            active_tab: self.active.clone(),
        }
    }
}

#[derive(Default)]
pub struct BrowserRuntime {
    inner: Mutex<Inner>,
    /// 创建标签页串行（同时来两个也按顺序建，数量上限才准）。
    create: tokio::sync::Mutex<()>,
    /// 页面操作串行（同一时间只有一件后台的事在动她的标签页）。
    ops: tokio::sync::Mutex<()>,
    /// 去抖：只应用最新一次的位置。
    bounds_gen: AtomicU64,
    /// 轮询歌和标题的任务代数（全部关掉时让旧任务退出）。
    poll_gen: AtomicU64,
    /// 临时提示的代数（只清掉自己那一条）。
    notice_gen: AtomicU64,
}

fn runtime(app: &AppHandle) -> Option<tauri::State<'_, BrowserRuntime>> {
    app.try_state::<BrowserRuntime>()
}

/// 玩家在设置里开着内置浏览器没有（设置 → 浏览器，ADR 038；默认开）。
fn enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::SharedState>()
        .map_or(true, |s| s.settings.read().unwrap().browser.enabled)
}

/// 设置里刚关掉内置浏览器：关掉所有标签页（右侧栏的“网页”跟着设置收起来）。动网页视图不能在同步命令
/// （主线程的 IPC 回调）里做，交给后台。
pub fn switched_off(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        close(&app);
        log::log("browser: 设置里关了内置浏览器，标签页都关掉了");
    });
}

pub fn data_dir() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("browser")
}

pub fn sites_path() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("browser-sites.json")
}

pub fn state(app: &AppHandle) -> BrowserState {
    runtime(app)
        .map(|rt| rt.inner.lock().unwrap().compose())
        .unwrap_or_default()
}

/// 改状态，有变化才发事件（不在 with_webview 回调里调用）。
fn update(app: &AppHandle, f: impl FnOnce(&mut Inner)) {
    let Some(rt) = runtime(app) else { return };
    let changed = {
        let mut inner = rt.inner.lock().unwrap();
        f(&mut inner);
        let next = inner.compose();
        (next != inner.last).then(|| {
            inner.last = next.clone();
            next
        })
    };
    if let Some(state) = changed {
        let _ = app.emit(EVENT, state);
    }
}

/// 一句临时提示，8 秒后自己消失。
fn notice(app: &AppHandle, text: String) {
    let Some(rt) = runtime(app) else { return };
    let generation = rt.notice_gen.fetch_add(1, Ordering::SeqCst) + 1;
    log::log(&format!("browser: {text}"));
    update(app, |inner| inner.notice = Some(text));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        let current =
            runtime(&app).is_some_and(|rt| rt.notice_gen.load(Ordering::SeqCst) == generation);
        if current {
            update(&app, |inner| inner.notice = None);
        }
    });
}

fn site_of_str(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .filter(|u| u.has_host())
        .map(|u| kotone_agent::browser::site_of(&u))
}

fn webview_of(app: &AppHandle, id: &str) -> Option<Webview> {
    runtime(app)?
        .inner
        .lock()
        .unwrap()
        .tab(id)
        .map(|t| t.webview.clone())
}

/// 当前标签页。
fn active_tab(app: &AppHandle) -> Option<(String, Webview)> {
    let rt = runtime(app)?;
    let inner = rt.inner.lock().unwrap();
    let id = inner.active.clone()?;
    let webview = inner.tab(&id)?.webview.clone();
    Some((id, webview))
}

/// 她的标签页。
pub(crate) fn agent_tab(app: &AppHandle) -> Option<(String, Webview)> {
    let rt = runtime(app)?;
    let inner = rt.inner.lock().unwrap();
    let id = inner.agent.clone()?;
    let webview = inner.tab(&id)?.webview.clone();
    Some((id, webview))
}

/// 后台正在操作页面（界面显示“她在操作”）。
pub(crate) struct Busy(AppHandle);

impl Busy {
    pub fn new(app: &AppHandle) -> Self {
        update(app, |inner| inner.busy += 1);
        Self(app.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        update(&self.0, |inner| inner.busy = inner.busy.saturating_sub(1));
    }
}

// ---------- 标签页 ----------

/// 新开一个标签页（`agent`：她的；`select`：切过去），可选马上打开一个网址。返回标签页 id。
async fn create_tab(
    app: &AppHandle,
    agent: bool,
    url: Option<url::Url>,
    select: bool,
) -> Result<String, String> {
    // 设置里关着（ADR 038）：不建网页视图（玩家、她、网页里的新窗口都一样）
    const OFF: &str = "内置浏览器关着（设置 → 浏览器）";
    if !enabled(app) {
        return Err(OFF.into());
    }
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _guard = rt.create.lock().await;
    let id = rt.inner.lock().unwrap().reserve()?;
    let webview = view::create(app, &id).await?;
    // 建的时候玩家刚好关了：不留下
    if !enabled(app) {
        let _ = webview.close();
        return Err(OFF.into());
    }
    let mut first = false;
    update(app, |inner| {
        inner.insert(&id, webview.clone(), agent, select);
        first = !inner.extensions_synced;
        inner.extensions_synced = true;
    });
    log::log(&format!(
        "browser: 新开标签页 {id}{}",
        if agent { "（她的）" } else { "" }
    ));
    if first {
        let (app, webview) = (app.clone(), webview.clone());
        tauri::async_runtime::spawn(async move { extensions::sync(&app, &webview).await });
    }
    apply_layout(app);
    start_polling(app);
    if let Some(url) = url {
        go(app, &id, &webview, &url).await?;
    }
    Ok(id)
}

/// 交给 tokio 去开新窗口的标签页。显式擦掉类型：建视图 → 新窗口回调 → 建视图是个环，
/// 编译器推不出 `Send`。
fn open_from_page_later(app: AppHandle, opener: String, url: url::Url) {
    let task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(async move { open_from_page(&app, &opener, url).await });
    tauri::async_runtime::spawn(task);
}

/// 网页里的新窗口（`window.open`、`target=_blank`）：开新标签页；从她的标签页开出来的也是她的；
/// 标签页满了就在原来的标签页里打开。
async fn open_from_page(app: &AppHandle, opener: &str, url: url::Url) {
    let from_agent =
        runtime(app).is_some_and(|rt| rt.inner.lock().unwrap().agent.as_deref() == Some(opener));
    if let Err(e) = create_tab(app, from_agent, Some(url.clone()), true).await {
        log::log(&format!(
            "browser: 新窗口没开成新标签页（{e}），在原标签页打开"
        ));
        if let Some(webview) = webview_of(app, opener) {
            let _ = webview.navigate(url);
        }
    }
}

/// 只把当前标签页摆到右侧栏的区域里，其余隐藏（不销毁，声音照常）。
fn apply_layout(app: &AppHandle) {
    let Some(rt) = runtime(app) else { return };
    let (bounds, active, others) = {
        let inner = rt.inner.lock().unwrap();
        let active = inner
            .active
            .as_deref()
            .and_then(|id| inner.tab(id))
            .map(|t| t.webview.clone());
        let others: Vec<Webview> = inner
            .tabs
            .iter()
            .filter(|t| Some(t.info.id.as_str()) != inner.active.as_deref())
            .map(|t| t.webview.clone())
            .collect();
        (inner.bounds, active, others)
    };
    if let Some(webview) = active {
        let (position, size) = view::placement(bounds);
        let _ = webview.set_position(position);
        let _ = webview.set_size(size);
        let _ = if bounds.visible && bounds.width >= 1.0 && bounds.height >= 1.0 {
            webview.show()
        } else {
            webview.hide()
        };
    }
    for webview in others {
        let _ = webview.hide();
    }
}

/// 界面报来浏览器区域的位置（去抖 40 ms，只应用最新一次）。
pub async fn set_bounds(app: &AppHandle, bounds: Bounds) {
    let Some(rt) = runtime(app) else { return };
    rt.inner.lock().unwrap().bounds = bounds;
    let generation = rt.bounds_gen.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let Some(rt) = runtime(app) else { return };
    if rt.bounds_gen.load(Ordering::SeqCst) != generation {
        return;
    }
    apply_layout(app);
}

// ---------- 页面事件（view 的 WebView2 事件和 on_page_load 交过来，已经在 tokio 上） ----------

fn tab_loaded(app: &AppHandle, id: &str, started: bool, url: &str) {
    update(app, |inner| {
        let Some(tab) = inner.tab_mut(id) else { return };
        tab.info.loading = started;
        if started {
            // 导航了：上一次读页面的编号作废。
            tab.memo = None;
        }
        if url != "about:blank" {
            tab.info.url = Some(url.to_string());
            tab.info.site = site_of_str(url);
        }
    });
}

fn tab_title(app: &AppHandle, id: &str, title: String) {
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(id) {
            // 空白页的标题是 “about:blank”，不显示。
            tab.info.title = Some(title).filter(|t| !t.trim().is_empty() && t != "about:blank");
        }
    });
}

fn tab_source(app: &AppHandle, id: &str, url: &str) {
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(id) {
            if tab.info.url.as_deref() != Some(url) {
                tab.memo = None;
            }
            // 空白页（新标签页）没有网址。
            let blank = url == "about:blank" || url.is_empty();
            tab.info.url = (!blank).then(|| url.to_string());
            tab.info.site = if blank { None } else { site_of_str(url) };
            if blank {
                tab.info.title = None;
            }
        }
    });
}

fn tab_history(app: &AppHandle, id: &str, back: bool, forward: bool) {
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(id) {
            tab.info.can_go_back = back;
            tab.info.can_go_forward = forward;
        }
    });
}

fn tab_fullscreen(app: &AppHandle, id: &str, on: bool) {
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(id) {
            tab.info.fullscreen_element = on;
        }
    });
}

/// 每 2 秒看一次每个标签页出不出声；出声的和当前的再读一次歌名，当前的顺便核对标题和网址。全部关掉后退出。
fn start_polling(app: &AppHandle) {
    let Some(rt) = runtime(app) else { return };
    if rt.inner.lock().unwrap().tabs.len() > 1 {
        return; // 已经在轮询了（第一个标签页建好时开的）
    }
    let generation = rt.poll_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(rt) = runtime(&app) else { return };
            if rt.poll_gen.load(Ordering::SeqCst) != generation {
                return;
            }
            let tabs: Vec<(String, Webview, bool)> = {
                let inner = rt.inner.lock().unwrap();
                inner
                    .tabs
                    .iter()
                    .map(|t| {
                        (
                            t.info.id.clone(),
                            t.webview.clone(),
                            inner.active.as_deref() == Some(t.info.id.as_str()),
                        )
                    })
                    .collect()
            };
            if tabs.is_empty() {
                return;
            }
            let mut seen = Vec::new();
            for (id, webview, active) in tabs {
                let audible = cdp::playing_audio(&webview).await.unwrap_or(false);
                let (media, page) = if audible || active {
                    let script = format!(
                        "(() => {{ const m = {}; return {{ title: document.title, url: location.href, media: m }}; }})()",
                        kotone_agent::browser::MEDIA_NOW_SCRIPT
                    );
                    match cdp::evaluate(&webview, &script).await {
                        Ok(value) => (
                            with_audio(media_of(&value["media"]), Some(audible)),
                            Some(value),
                        ),
                        Err(_) => (None, None),
                    }
                } else {
                    (None, None)
                };
                seen.push((id, audible, media, page));
            }
            update(&app, |inner| {
                for (id, audible, media, page) in seen {
                    let Some(tab) = inner.tab_mut(&id) else {
                        continue;
                    };
                    tab.info.audible = audible;
                    tab.media = media;
                    if let Some(page) = page {
                        if let Some(title) = page["title"].as_str().filter(|t| !t.is_empty()) {
                            tab.info.title = Some(title.to_string());
                        }
                        if let Some(url) = page["url"].as_str().filter(|u| *u != "about:blank") {
                            tab.info.site = site_of_str(url);
                            tab.info.url = Some(url.to_string());
                        }
                    }
                }
            });
        }
    });
}

/// 媒体脚本的结果 → 界面上的“正在放”。页面里没有音视频时为 None。
/// 页面在出声就算“在放”（播放器用不挂在页面上的 Audio、或者在跨域 iframe 里，脚本找不到时也准）。
fn with_audio(media: Option<Media>, audible: Option<bool>) -> Option<Media> {
    match (media, audible) {
        (Some(mut m), Some(true)) => {
            m.playing = true;
            Some(m)
        }
        (None, Some(true)) => Some(Media {
            playing: true,
            title: None,
            artist: None,
        }),
        (media, _) => media,
    }
}

fn media_of(value: &Value) -> Option<Media> {
    if value["count"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let text = |k: &str| {
        value[k]
            .as_str()
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty())
    };
    Some(Media {
        playing: value["playing"].as_bool().unwrap_or(false),
        title: text("title"),
        artist: text("artist"),
    })
}

// ---------- 玩家在界面上的操作 ----------

/// 玩家自己打开的网站算已同意（记进站点授权，她以后打开也不用再问）。
fn approve_site(path: &std::path::Path, url: &url::Url) {
    let mut sites = kotone_agent::browser::SiteApprovals::load(path);
    let site = kotone_agent::browser::site_of(url);
    if !site.is_empty() && !sites.covers(&site) {
        sites.approve(&site);
        if let Err(e) = sites.save(path) {
            log::log(&format!("browser: 记不住同意过的网站：{e}"));
        }
    }
}

/// 地址栏回车：网址就打开，搜索词用必应搜；对当前标签页（没有就新开一个）。
pub async fn navigate(app: &AppHandle, input: &str) -> Result<Value, String> {
    let url = kotone_agent::browser::resolve_address(input)?;
    approve_site(&sites_path(), &url);
    let id = match active_tab(app) {
        Some((id, webview)) => {
            go(app, &id, &webview, &url).await?;
            id
        }
        None => create_tab(app, false, Some(url.clone()), true).await?,
    };
    Ok(json!({"tab": id, "url": url.as_str()}))
}

/// 让一个标签页打开网址。空白的新标签页用 `location.replace`：不在历史里留一条空白页
/// （不然“后退”会退回空白页）。
async fn go(app: &AppHandle, id: &str, webview: &Webview, url: &url::Url) -> Result<(), String> {
    let blank = runtime(app)
        .and_then(|rt| {
            rt.inner
                .lock()
                .unwrap()
                .tab(id)
                .map(|t| t.info.url.is_none())
        })
        .unwrap_or(false);
    let replaced = blank
        && cdp::evaluate(
            webview,
            &format!(
                "location.href === 'about:blank' && (location.replace({}), true)",
                serde_json::to_string(url.as_str()).unwrap_or_default()
            ),
        )
        .await
        .ok()
        .and_then(|v| v.as_bool())
            == Some(true);
    if !replaced {
        webview
            .navigate(url.clone())
            .map_err(|e| format!("打开失败：{e}"))?;
    }
    tab_source(app, id, url.as_str());
    Ok(())
}

/// 新标签页（可选马上打开地址栏里的内容）。
pub async fn tab_new(app: &AppHandle, input: Option<&str>) -> Result<Value, String> {
    let url = match input.map(str::trim).filter(|s| !s.is_empty()) {
        Some(input) => {
            let url = kotone_agent::browser::resolve_address(input)?;
            approve_site(&sites_path(), &url);
            Some(url)
        }
        None => None,
    };
    let id = create_tab(app, false, url, true).await?;
    Ok(json!({"tab": id}))
}

/// 切到某个标签页。
pub fn tab_select(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut result = Ok(());
    update(app, |inner| result = inner.select(id));
    result?;
    apply_layout(app);
    Ok(())
}

/// 关掉一个标签页（切到哪个、她的标签页怎么办见 [`Inner::remove`]）。
pub fn tab_close(app: &AppHandle, id: &str) -> Result<(), String> {
    let Some(rt) = runtime(app) else {
        return Err("浏览器还没准备好".into());
    };
    let mut removed = Err(String::new());
    let mut empty = false;
    update(app, |inner| {
        removed = inner.remove(id);
        empty = inner.tabs.is_empty();
    });
    let webview = removed?;
    let _ = webview.close();
    log::log(&format!("browser: 关掉标签页 {id}"));
    if empty {
        rt.poll_gen.fetch_add(1, Ordering::SeqCst);
    }
    apply_layout(app);
    Ok(())
}

/// 界面上的按钮：刷新、后退、前进、停止、关掉当前标签页、播放、暂停、下一首。
pub async fn control(app: &AppHandle, action: &str) -> Result<Value, String> {
    match action {
        "reload" => {
            let webview = match active_tab(app) {
                Some((_, webview)) => webview,
                // 上次没建起来（卡片上显示了原因）时，“重试”就是重新建一个标签页。
                None if state(app).error.is_some() => {
                    create_tab(app, false, None, true).await?;
                    return Ok(json!({"ok": true}));
                }
                None => return Err("浏览器还没打开".into()),
            };
            webview.reload().map_err(|e| e.to_string())?;
            Ok(json!({"ok": true}))
        }
        "back" | "forward" | "stop" => {
            let (_, webview) = active_tab(app).ok_or("浏览器还没打开")?;
            view::history(&webview, action)?;
            Ok(json!({"ok": true}))
        }
        "close" => {
            let (id, _) = active_tab(app).ok_or("浏览器还没打开")?;
            tab_close(app, &id)?;
            Ok(json!({"ok": true}))
        }
        other => Err(format!("不认识的操作：{other}")),
    }
}

/// 关掉所有标签页（清除登录时）。
pub fn close(app: &AppHandle) {
    let webviews: Vec<Webview> = runtime(app)
        .map(|rt| {
            rt.poll_gen.fetch_add(1, Ordering::SeqCst);
            let mut inner = rt.inner.lock().unwrap();
            inner.handover_tx = None;
            inner.tabs.drain(..).map(|t| t.webview).collect()
        })
        .unwrap_or_default();
    let count = webviews.len();
    for webview in webviews {
        let _ = webview.close();
    }
    if count > 0 {
        log::log("browser: 所有标签页已关闭");
    }
    update(app, |inner| {
        inner.active = None;
        inner.agent = None;
        inner.handover = None;
        inner.busy = 0;
        inner.error = None;
        inner.notice = None;
        inner.extensions_synced = false;
    });
}

/// 清除浏览器登录：关掉所有标签页，删掉数据目录（浏览器进程退出要一会儿，重试几次）。
pub async fn clear_data(app: &AppHandle) -> Result<(), String> {
    close(app);
    let dir = data_dir();
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                log::log("browser: 已清除浏览器登录");
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("没能删掉浏览器数据（{last}），重启 Kotone 后再试"))
}

/// 玩家在横幅上点了“好了”。
pub fn handover_done(app: &AppHandle) {
    if let Some(tx) = runtime(app).and_then(|rt| rt.inner.lock().unwrap().handover_tx.take()) {
        let _ = tx.send(());
    }
}

/// Tauri / wry 用 `log` 库报自己的警告和错误（例如子网页视图在主线程上没建出来：调用方照样拿到句柄，
/// 之后发给它的消息都被悄悄丢掉），Kotone 原来没有接，这里转进 Kotone 的日志。
pub fn forward_runtime_log() {
    struct Forward;
    impl ::log::Log for Forward {
        fn enabled(&self, metadata: &::log::Metadata) -> bool {
            metadata.level() <= ::log::Level::Warn
        }
        fn log(&self, record: &::log::Record) {
            if self.enabled(record.metadata()) {
                kotone_core::log::log(&format!(
                    "{} {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                ));
            }
        }
        fn flush(&self) {}
    }
    static FORWARD: Forward = Forward;
    if ::log::set_logger(&FORWARD).is_ok() {
        ::log::set_max_level(::log::LevelFilter::Warn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_tab(inner: &mut Inner<()>, agent: bool, select: bool) -> String {
        let id = inner.reserve().unwrap();
        inner.insert(&id, (), agent, select);
        id
    }

    #[test]
    fn tabs_open_select_close_and_stop_at_the_limit() {
        let mut inner = Inner::<()>::default();
        assert!(!inner.compose().open);
        let a = open_tab(&mut inner, false, true);
        let b = open_tab(&mut inner, false, true);
        let c = open_tab(&mut inner, false, false);
        let state = inner.compose();
        assert!(state.open);
        assert_eq!(state.active_tab.as_deref(), Some(b.as_str()));
        assert_eq!(state.tabs.len(), 3);
        inner.select(&a).unwrap();
        assert_eq!(inner.active.as_deref(), Some(a.as_str()));
        assert!(inner.select("nope").is_err());
        // 关掉当前的：切到右边那个；最右边的被关：切到左边那个
        inner.remove(&a).unwrap();
        assert_eq!(inner.active.as_deref(), Some(b.as_str()));
        inner.select(&c).unwrap();
        inner.remove(&c).unwrap();
        assert_eq!(inner.active.as_deref(), Some(b.as_str()));
        // 关掉不是当前的：当前的不变
        let d = open_tab(&mut inner, false, false);
        inner.remove(&d).unwrap();
        assert_eq!(inner.active.as_deref(), Some(b.as_str()));
        assert!(inner.remove(&d).is_err());
        inner.remove(&b).unwrap();
        assert_eq!(inner.active, None);
        assert!(!inner.compose().open);
        // 上限 8 个；关掉一个又能开；编号不重复
        let ids: Vec<String> = (0..kotone_agent::browser::MAX_TABS)
            .map(|_| open_tab(&mut inner, false, true))
            .collect();
        assert!(inner.reserve().unwrap_err().contains('8'));
        inner.remove(&ids[0]).unwrap();
        let again = open_tab(&mut inner, false, true);
        assert!(!ids.contains(&again) && again != a && again != b);
    }

    #[test]
    fn her_tab_is_separate_from_the_players_tabs() {
        let mut inner = Inner::<()>::default();
        let mine = open_tab(&mut inner, false, true);
        inner.tab_mut(&mine).unwrap().info.url = Some("https://www.bilibili.com/".into());
        let hers = open_tab(&mut inner, true, true);
        let state = inner.compose();
        assert_eq!(
            state.tabs.iter().map(|t| t.agent).collect::<Vec<_>>(),
            vec![false, true]
        );
        // 当前标签页的字段是她的（刚切过去）；切回玩家的，顶上的字段跟着变
        assert_eq!(state.active_tab.as_deref(), Some(hers.as_str()));
        assert_eq!(state.url, None);
        inner.select(&mine).unwrap();
        assert_eq!(
            inner.compose().url.as_deref(),
            Some("https://www.bilibili.com/")
        );
        // 顶上的“在出声”跟着出声的标签页（放歌不在浏览器里了，ADR 034；只读）
        inner.tab_mut(&mine).unwrap().info.audible = true;
        inner.tab_mut(&mine).unwrap().media = Some(Media {
            playing: true,
            title: Some("晴天".into()),
            artist: None,
        });
        inner.select(&hers).unwrap();
        assert_eq!(
            inner.compose().media.and_then(|m| m.title).as_deref(),
            Some("晴天")
        );
        // 关掉她的标签页：交接结束（等着的一方收到“关掉了”），她下次再开新的
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        inner.handover_tx = Some(tx);
        inner.handover = Some(Handover {
            reason: "扫码登录".into(),
        });
        inner.remove(&hers).unwrap();
        assert_eq!(inner.agent, None);
        assert_eq!(inner.compose().handover, None);
        assert!(rx.try_recv().is_err());
        assert_eq!(inner.active.as_deref(), Some(mine.as_str()));
        // 她新开的标签页（例如从她的页面点开的新窗口）接替“她的”身份，原来那个留给玩家
        let first = open_tab(&mut inner, true, true);
        let second = open_tab(&mut inner, true, true);
        assert_eq!(inner.agent.as_deref(), Some(second.as_str()));
        assert!(!inner
            .compose()
            .tabs
            .iter()
            .any(|t| t.id == first && t.agent));
    }

    #[test]
    fn sites_the_player_opens_are_approved_for_her() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser-sites.json");
        let url = kotone_agent::browser::resolve_address("www.bilibili.com/video").unwrap();
        approve_site(&path, &url);
        let sites = kotone_agent::browser::SiteApprovals::load(&path);
        assert!(sites.covers("bilibili.com"));
        assert!(!sites.covers("music.163.com"));
        // 搜索词打开的是必应：记的是必应，不是搜的词
        let search = kotone_agent::browser::resolve_address("晴天 周杰伦").unwrap();
        approve_site(&path, &search);
        assert!(kotone_agent::browser::SiteApprovals::load(&path).covers("cn.bing.com"));
    }

    #[test]
    fn navigation_only_reaches_public_http_pages() {
        let ok = |s: &str| view::allowed_navigation(&url::Url::parse(s).unwrap());
        assert!(ok("about:blank"));
        assert!(ok("https://music.163.com/#/discover"));
        assert!(!ok("http://tauri.localhost/index.html"));
        assert!(!ok("http://127.0.0.1:1430/"));
        assert!(!ok("file:///C:/Users/"));
        assert!(!ok("kotone://anything"));
    }

    #[test]
    fn hidden_or_unknown_bounds_park_the_view_outside_the_window() {
        let (position, _) = view::placement(Bounds::default());
        assert!(position.x < 0.0 && position.y < 0.0);
        let (position, size) = view::placement(Bounds {
            x: 800.0,
            y: 40.0,
            width: 400.0,
            height: 600.0,
            visible: true,
        });
        assert_eq!(
            (position.x, position.y, size.width, size.height),
            (800.0, 40.0, 400.0, 600.0)
        );
    }

    #[test]
    fn audible_pages_count_as_playing() {
        assert_eq!(
            with_audio(None, Some(true)),
            Some(Media {
                playing: true,
                title: None,
                artist: None
            })
        );
        assert_eq!(with_audio(None, Some(false)), None);
        let paused = Media {
            playing: false,
            title: Some("晴天".into()),
            artist: None,
        };
        assert!(
            with_audio(Some(paused.clone()), Some(true))
                .unwrap()
                .playing
        );
        assert_eq!(with_audio(Some(paused.clone()), None), Some(paused));
    }

    #[test]
    fn media_reads_playing_title_and_artist() {
        assert_eq!(media_of(&serde_json::json!({"count": 0})), None);
        assert_eq!(
            media_of(
                &serde_json::json!({"count": 1, "playing": true, "title": "晴天", "artist": "周杰伦"})
            ),
            Some(Media {
                playing: true,
                title: Some("晴天".into()),
                artist: Some("周杰伦".into())
            })
        );
    }

    /// 网页调用不了 Kotone 的命令：Tauri 2.11 对非本地来源拒绝自定义命令，除非某个 capability 写了 `remote`。
    /// 这里确认我们的 capabilities 一个都没写，也没有哪个按名字匹配到浏览器的标签页视图（`browser-N`）。
    #[test]
    fn no_capability_grants_remote_origins() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let json: Value = serde_json::from_str(&text).unwrap();
            assert!(
                json.get("remote").is_none(),
                "{} 不能给远程网页权限",
                path.display()
            );
            let windows = json["windows"].as_array().cloned().unwrap_or_default();
            assert!(
                !windows.iter().any(|w| {
                    let w = w.as_str().unwrap_or("");
                    w == "*" || w.starts_with("browser")
                }),
                "{} 不能匹配浏览器视图",
                path.display()
            );
            assert!(json.get("webviews").is_none(), "{}", path.display());
        }
    }
}
