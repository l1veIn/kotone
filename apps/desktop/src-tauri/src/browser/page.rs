//! 页面操作：打开、读页面、点击、输入、按键、等待、交给玩家、放歌。由后台任务的 `browser.*`
//! （经 `agent::browser_tools`，那里先做网址检查、站点授权）和主会话的 `music` 调用。
//! 后台任务只动她自己的标签页（第一次 `browser.open` 新开一个并切过去）；玩家的标签页她不读也不点。

use std::time::{Duration, Instant};

use kotone_agent::browser::{self as rules, SnapshotRef};
use kotone_core::cancel::CancellationToken;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::{agent_tab, cdp, create_tab, music_tab, runtime, update, Busy, Handover, Memo};

/// 打开网页最多等多久加载完。
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// 交给玩家最多等多久。
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(600);

/// 打开一个（已经检查过、玩家同意过的）网址，等它加载完。
pub async fn open(app: &AppHandle, url: &url::Url) -> Result<Value, String> {
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _ops = rt.ops.lock().await;
    let _busy = Busy::new(app);
    // 她的标签页：没有就新开一个直接打开这个网址（并切过去）；有就在里面打开（不切，玩家可能在看别的）。
    let (id, webview, before) = match agent_tab(app) {
        Some((id, webview)) => {
            set_memo(app, &id, None);
            let before = cdp::evaluate(&webview, "location.href")
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            super::go(app, &id, &webview, url).await?;
            (id, webview, before)
        }
        None => {
            let id = create_tab(app, true, Some(url.clone()), true)
                .await
                .map_err(|e| {
                    if e.starts_with("最多") {
                        format!(
                            "浏览器已经开着 {} 个标签页，满了，开不了你自己的标签页：请玩家关掉一个再试",
                            rules::MAX_TABS
                        )
                    } else {
                        e
                    }
                })?;
            let webview = super::webview_of(app, &id).ok_or("浏览器标签页刚建好就没了")?;
            (id, webview, "about:blank".to_string())
        }
    };
    let loaded = wait_loaded(&webview, url, &before).await;
    let info = cdp::evaluate(&webview, "({ title: document.title, url: location.href })")
        .await
        .unwrap_or(Value::Null);
    let mut out = json!({
        "url": info["url"].as_str().unwrap_or(url.as_str()),
        "title": info["title"].as_str().unwrap_or(""),
        "site": rules::site_of(url),
        "note": "用 browser.snapshot 读页面，再按编号操作",
    });
    if !loaded {
        out["warning"] = json!("页面还没完全加载完，读之前可以先 browser.wait 几秒");
    }
    // 右侧栏马上显示新页面（不等下一次轮询）。
    let (title, href) = (
        out["title"].as_str().unwrap_or("").to_string(),
        out["url"].as_str().unwrap_or("").to_string(),
    );
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(&id) {
            tab.info.title = Some(title).filter(|t| !t.is_empty());
            tab.info.site = Some(rules::site_of(url));
            tab.info.url = Some(href).filter(|u| !u.is_empty());
        }
    });
    Ok(out)
}

/// 她的标签页（没有就报错：先 browser.open）。
fn agent(app: &AppHandle) -> Result<(String, tauri::Webview), String> {
    agent_tab(app).ok_or_else(|| "还没打开网页：先 browser.open".to_string())
}

fn set_memo(app: &AppHandle, id: &str, memo: Option<Memo>) {
    if let Some(rt) = runtime(app) {
        if let Some(tab) = rt.inner.lock().unwrap().tab_mut(id) {
            tab.memo = memo;
        }
    }
}

/// 等导航完成并且 `document.readyState == complete`（最多 20 秒）。navigate 是异步的：
/// 先等地址变过来（或本来就是这个地址），免得把上一页的 complete 当成这一页的。
async fn wait_loaded(webview: &tauri::Webview, target: &url::Url, before: &str) -> bool {
    let started = Instant::now();
    while started.elapsed() < LOAD_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let Ok(value) = cdp::evaluate(
            webview,
            "({ state: document.readyState, href: location.href })",
        )
        .await
        else {
            continue;
        };
        let href = value["href"].as_str().unwrap_or("");
        let arrived = href != before || href == target.as_str();
        if arrived && href != "about:blank" && value["state"].as_str() == Some("complete") {
            return true;
        }
    }
    false
}

/// 读页面的结果：交给模型的 JSON，以及要不要交给玩家（验证码）。
pub struct SnapshotResult {
    pub value: Value,
    pub captcha: Option<String>,
}

/// 读页面：整页（含 iframe）带编号；`more` 接着上次没读完的地方。
pub async fn snapshot(app: &AppHandle, more: bool) -> Result<SnapshotResult, String> {
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _ops = rt.ops.lock().await;
    let _busy = Busy::new(app);
    let (id, webview) = agent(app)?;
    if more {
        let mut inner = rt.inner.lock().unwrap();
        let Some(memo) = inner.tab_mut(&id).and_then(|t| t.memo.as_mut()) else {
            return Err("没有读到一半的页面：直接 browser.snapshot".into());
        };
        let Some(start) = memo.next else {
            return Err("这一页已经读完了".into());
        };
        let (chunk, next) = rules::page(&memo.text, start, rules::SNAPSHOT_PAGE_CHARS);
        memo.next = next;
        return Ok(SnapshotResult {
            value: json!({
                "url": memo.url,
                "content": rules::wrap_untrusted(&chunk, &rules::nonce()),
                "more": next.is_some(),
            }),
            captcha: None,
        });
    }
    let captured = cdp::call(
        &webview,
        "DOMSnapshot.captureSnapshot",
        json!({"computedStyles": [], "includeDOMRects": false, "includePaintOrder": false}),
    )
    .await?;
    let snap = rules::render_snapshot(&captured);
    let title = cdp::evaluate(&webview, "document.title")
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let probe = cdp::evaluate(&webview, rules::CAPTCHA_PROBE)
        .await
        .unwrap_or(Value::Null);
    let captcha = if probe["detected"].as_bool() == Some(true) || rules::captcha_hint(&snap.text) {
        Some(probe["kind"].as_str().unwrap_or("captcha").to_string())
    } else {
        None
    };
    let (chunk, next) = rules::page(&snap.text, 0, rules::SNAPSHOT_PAGE_CHARS);
    let changes = {
        let inner = rt.inner.lock().unwrap();
        inner
            .tab(&id)
            .and_then(|t| t.memo.as_ref())
            .filter(|m| m.url == snap.url)
            .and_then(|m| rules::diff_summary(&m.text, &snap.text))
    };
    let mut value = json!({
        "url": snap.url,
        "title": title,
        "content": rules::wrap_untrusted(&chunk, &rules::nonce()),
        "more": next.is_some(),
        "refs": snap.refs.len(),
    });
    if let Some(changes) = changes {
        value["changes"] = json!(changes);
    }
    if snap.has_password_field {
        value["note"] = json!("页面上有密码框：要登录就用 browser.handover 交给玩家（网易云这类可以扫码），不要自己填");
    }
    if snap.text.trim().is_empty() {
        value["note"] = json!("页面是空的或还没加载出来：browser.wait 几秒再读");
    }
    set_memo(
        app,
        &id,
        Some(Memo {
            url: snap.url,
            text: snap.text,
            refs: snap.refs,
            next,
        }),
    );
    Ok(SnapshotResult { value, captcha })
}

/// 最近一次读页面里的这个编号（导航后或没读过时拒绝：旧编号不用）。
fn element(app: &AppHandle, id: &str, reference: u64) -> Result<SnapshotRef, String> {
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let inner = rt.inner.lock().unwrap();
    let Some(memo) = inner.tab(id).and_then(|t| t.memo.as_ref()) else {
        return Err("页面变过了（或还没读）：先 browser.snapshot，再用新的编号".into());
    };
    memo.refs
        .iter()
        .find(|r| r.backend_node_id == reference)
        .cloned()
        .ok_or_else(|| {
            format!("编号 {reference} 不在最近一次读到的页面里：先 browser.snapshot，再用新的编号")
        })
}

/// 点击一个元素：页面内 `click()` 为主（窗口隐藏、被游戏挡住也有效），出错时退到调试协议的鼠标事件。
pub async fn click(app: &AppHandle, reference: u64) -> Result<Value, String> {
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _ops = rt.ops.lock().await;
    let _busy = Busy::new(app);
    let (id, webview) = agent(app)?;
    let target = element(app, &id, reference)?;
    let how = match cdp::call_on_node(&webview, reference, rules::CLICK_FUNCTION, &[]).await {
        Ok(v) => v.as_str().unwrap_or("clicked").to_string(),
        Err(e) if e.contains("不在页面上") => return Err(e),
        Err(_) => {
            cdp::mouse_click(&webview, reference).await?;
            "mouse".to_string()
        }
    };
    tokio::time::sleep(Duration::from_millis(600)).await;
    let url = cdp::evaluate(&webview, "location.href")
        .await
        .unwrap_or(Value::Null);
    Ok(json!({
        "clicked": format!("{} \"{}\"", target.role, target.name),
        "how": how,
        "url": url,
        "note": "页面可能变了：要看结果就再 browser.snapshot",
    }))
}

/// 在输入框里输入（先清空）。密码、卡号这类不让输入。
pub async fn type_text(
    app: &AppHandle,
    reference: u64,
    text: &str,
    submit: bool,
) -> Result<Value, String> {
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _ops = rt.ops.lock().await;
    let _busy = Busy::new(app);
    let (id, webview) = agent(app)?;
    let target = element(app, &id, reference)?;
    if target.sensitive {
        return Err("这是密码 / 卡号这类输入框，不能替玩家填：用 browser.handover 交给玩家".into());
    }
    let result = cdp::call_on_node(
        &webview,
        reference,
        rules::TYPE_FUNCTION,
        &[json!(text), json!(submit)],
    )
    .await?;
    if result.as_str() == Some("not-editable") {
        return Err(format!("编号 {reference}（{}）不是输入框", target.role));
    }
    if submit {
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    Ok(json!({"typed": true, "submitted": submit, "note": "要看结果就再 browser.snapshot"}))
}

/// 按一个键（发给页面）。
pub async fn press(app: &AppHandle, key: &str) -> Result<Value, String> {
    let script = rules::press_script(key)?;
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _ops = rt.ops.lock().await;
    let _busy = Busy::new(app);
    let (_, webview) = agent(app)?;
    cdp::evaluate(&webview, &script).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(json!({"pressed": key}))
}

/// 等某段文字出现（最多 30 秒），或者等几秒。
pub async fn wait(
    app: &AppHandle,
    text: Option<&str>,
    seconds: Option<f64>,
) -> Result<Value, String> {
    let limit = Duration::from_secs_f64(
        seconds
            .unwrap_or(if text.is_some() { 15.0 } else { 3.0 })
            .clamp(0.0, 30.0),
    );
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        tokio::time::sleep(limit).await;
        return Ok(json!({"waited": limit.as_secs_f64()}));
    };
    let _busy = Busy::new(app);
    let (_, webview) = agent(app)?;
    let script = rules::find_text_script(text);
    let started = Instant::now();
    loop {
        if cdp::evaluate(&webview, &script)
            .await
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            return Ok(json!({"found": true, "afterSeconds": started.elapsed().as_secs_f64()}));
        }
        if started.elapsed() >= limit {
            return Ok(
                json!({"found": false, "note": format!("{} 秒内没等到“{text}”", limit.as_secs())}),
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 页面上还有没有登录框 / 扫码（交给玩家登录时判断“登好了”用）。
const LOGIN_PROBE: &str = r#"(() => {
  const docs = [];
  const collect = (doc) => { try { docs.push(doc); for (const f of doc.querySelectorAll('iframe')) { try { if (f.contentDocument) collect(f.contentDocument); } catch (e) {} } } catch (e) {} };
  collect(document);
  const visible = (el) => !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
  for (const doc of docs) {
    if ([...doc.querySelectorAll('input[type=password]')].some(visible)) return true;
    const text = doc.body ? doc.body.innerText || '' : '';
    if (/扫码登录|二维码登录|扫描二维码|请登录后|密码登录|短信登录/.test(text)) return true;
  }
  return false;
})()"#;

/// 交给玩家（登录、验证码、付款）：切到她的标签页、右侧栏展开、提示玩家；等玩家点“好了”、
/// 页面上的登录框 / 验证码消失、或 10 分钟。
pub async fn handover(
    app: &AppHandle,
    reason: &str,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let (id, webview) = agent(app)?;
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    update(app, |inner| {
        inner.handover_tx = Some(tx);
        inner.handover = Some(Handover {
            reason: reason.to_string(),
        });
        // 玩家要在她的标签页上登录：切过去。
        inner.active = Some(id.clone());
    });
    super::apply_layout(app);
    kotone_core::log::log(&format!("browser: 交给玩家：{reason}"));
    let blocked = |v: Value| v.as_bool().unwrap_or(false);
    let at_start_login = blocked(
        cdp::evaluate(&webview, LOGIN_PROBE)
            .await
            .unwrap_or(Value::Null),
    );
    let at_start_captcha = blocked(
        cdp::evaluate(&webview, rules::CAPTCHA_PROBE)
            .await
            .map(|v| v["detected"].clone())
            .unwrap_or(Value::Null),
    );
    let started = Instant::now();
    let outcome = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Err("已停止".to_string()),
            done = &mut rx => {
                break Ok(if done.is_ok() { "玩家说弄好了" } else { "浏览器关掉了" });
            }
            _ = tokio::time::sleep(Duration::from_secs(3)) => {
                if started.elapsed() >= HANDOVER_TIMEOUT {
                    break Ok("等了 10 分钟玩家还没弄好");
                }
                // 跟着她现在的标签页看（登录弹出的新窗口会成为她新的标签页）。
                let Some((_, webview)) = agent_tab(app) else { break Ok("她的标签页关掉了") };
                let login = blocked(cdp::evaluate(&webview, LOGIN_PROBE).await.unwrap_or(Value::Null));
                let captcha = blocked(
                    cdp::evaluate(&webview, rules::CAPTCHA_PROBE)
                        .await
                        .map(|v| v["detected"].clone())
                        .unwrap_or(Value::Null),
                );
                if (at_start_login || at_start_captcha) && !login && !captcha {
                    break Ok("页面上的登录 / 验证已经没有了，看起来弄好了");
                }
            }
        }
    };
    if let Some((now, _)) = agent_tab(app) {
        set_memo(app, &now, None);
    }
    update(app, |inner| {
        inner.handover = None;
        inner.handover_tx = None;
    });
    let outcome = outcome?;
    Ok(json!({"result": outcome, "note": "先 browser.snapshot 看看现在的页面"}))
}

/// 调试版 `browser_probe` 看的标签页：当前标签页（真机脚本切标签页后看得到别的页面）。
fn probe_tab(app: &AppHandle) -> Result<(String, tauri::Webview), String> {
    super::active_tab(app).ok_or_else(|| "还没打开网页".to_string())
}

/// 只给调试版的 `browser_probe` 用：在页面里执行一段脚本（真机脚本排查各音乐网站的播放器）。
pub async fn probe_eval(app: &AppHandle, expression: &str) -> Result<Value, String> {
    let (_, webview) = probe_tab(app)?;
    cdp::evaluate(&webview, expression).await
}

/// 只给调试版的 `browser_probe` 用：在她的标签页上经浏览器的输入管线点一下（像玩家亲手点的，
/// 真机脚本测页面全屏这类要“用户操作”才给的功能；不经过系统输入）。
pub async fn probe_mouse(app: &AppHandle, reference: u64) -> Result<Value, String> {
    let (_, webview) = agent(app)?;
    cdp::mouse_click(&webview, reference).await?;
    Ok(json!({"clicked": reference}))
}

/// 只给调试版的 `browser_probe` 用：把焦点给她的标签页（像玩家点进了网页）。
pub fn probe_focus(app: &AppHandle) -> Result<Value, String> {
    let (_, webview) = agent(app)?;
    webview.set_focus().map_err(|e| e.to_string())?;
    Ok(json!({"focused": true}))
}

/// 只给调试版的 `browser_probe` 用：WebView2 说页面现在有没有在出声（静音时也照实回答）。
pub async fn probe_audio(app: &AppHandle) -> Result<Value, String> {
    let (_, webview) = probe_tab(app)?;
    Ok(json!({"documentPlayingAudio": cdp::playing_audio(&webview).await}))
}

/// 主会话的 `music`：控制 Kotone 浏览器里在放的歌。
pub async fn music(app: &AppHandle, action: &str) -> Result<Value, String> {
    if !matches!(action, "play" | "pause" | "next" | "now") {
        return Err("action 只能是 play / pause / next / now".into());
    }
    // 正在出声的标签页；没有就她的标签页，再没有就当前的。
    let Some((id, webview, site)) = music_tab(app) else {
        return Err(
            "现在 Kotone 里没有在放歌（还没打开音乐网站）。玩家想听歌就交给后台去音乐网站上放"
                .into(),
        );
    };
    let site = site.unwrap_or_default();
    let script = rules::media_script(action, rules::next_selectors(&site));
    let value = cdp::evaluate(&webview, &script).await?;
    let audible = cdp::playing_audio(&webview).await;
    let media = super::with_audio(super::media_of(&value), audible);
    set_media(app, &id, media.clone());
    let count = value["count"].as_u64().unwrap_or(0);
    if count == 0 && audible != Some(true) {
        return Err("Kotone 的浏览器里没有在放的歌（页面上没有播放器）".into());
    }
    let done = value["done"].as_bool().unwrap_or(false);
    let mut out = json!({
        "site": site,
        "playing": media.as_ref().is_some_and(|m| m.playing),
        "title": value["title"],
        "artist": value["artist"],
    });
    match action {
        "next" if !done => {
            out["note"] = json!("这个网站找不到“下一首”按钮：要换歌就交给后台在页面上点");
        }
        // 页面脚本找不到播放器、可这个标签页还在出声（rc1 之后实测：QQ 音乐试听）：退一步把这个
        // 标签页静音，先让声音停下，并如实说明；说“继续”时取消静音。
        "pause" if !done && audible == Some(true) => {
            if cdp::set_muted(&webview, true).await == Some(true) {
                out["playing"] = json!(false);
                out["muted"] = json!(true);
                out["note"] = json!(
                    "这个网站的播放器暂停不了，先把这个标签页静音了（歌其实还在走）；玩家说“继续”就取消静音"
                );
            } else {
                out["note"] = json!("这个网站的播放器暂停不了，静音也没成功");
            }
        }
        "play" if !done && cdp::muted(&webview).await == Some(true) => {
            // 真机脚本半夜测放歌时整个浏览器是静音的，那时不取消。
            if std::env::var_os("KOTONE_BROWSER_MUTE").is_none() {
                cdp::set_muted(&webview, false).await;
            }
            out["muted"] = json!(false);
            out["playing"] = json!(audible == Some(true));
            out["note"] = json!("取消静音了");
        }
        "play" if !done => {
            out["note"] = json!("没有暂停着的歌可以继续");
        }
        "pause" if !done => {
            out["note"] = json!("本来就没在放");
        }
        _ => {}
    }
    if action == "next" && done {
        // 换歌要一会儿：等新歌名出来再回答。
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if let Ok(after) = cdp::evaluate(&webview, &rules::media_script("now", &[])).await {
            out["playing"] = after["playing"].clone();
            out["title"] = after["title"].clone();
            out["artist"] = after["artist"].clone();
            set_media(app, &id, super::media_of(&after));
        }
    }
    Ok(out)
}

fn set_media(app: &AppHandle, id: &str, media: Option<super::Media>) {
    update(app, |inner| {
        if let Some(tab) = inner.tab_mut(id) {
            tab.media = media;
        }
    });
}
