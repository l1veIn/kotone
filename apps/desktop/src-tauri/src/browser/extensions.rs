//! 浏览器插件（ADR 031）：玩家加的、已解压的 Chrome / Edge 插件文件夹（WebView2 只支持这种，没有应用商店）。
//!
//! - 列表记在设置的 `browser.extensions`（新字段，旧配置为空），重启后照样在。
//! - 装、删、开关走 WebView2 Profile 的插件接口（`ICoreWebView2Profile7`，装进浏览器自己的数据目录）。
//!   不用 wry 的 `extensions_path`：它在建视图时把目录里的子文件夹逐个装，任何一个出错整个视图就建不出来。
//! - 每次运行第一个标签页建好时对一遍：设置里有、浏览器里没有的装上（清除登录后数据目录没了也能装回来），
//!   浏览器没开时玩家删掉的卸掉，开关照设置。别的插件不碰（WebView2 自带 Edge PDF Viewer 这类内置插件，
//!   也在列表里，卸不掉也不该卸：2026-11-21 真机）。
//! - WebView2 回调里只发 channel，不 emit、不阻塞（同 `cdp`）。

use std::time::Duration;

use kotone_core::log;
use kotone_core::settings::{BrowserConfig, BrowserExtensionConfig};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, Webview};

/// 交给界面的一个插件。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub path: String,
    /// 已经装进浏览器（没开过浏览器时为 false，下次打开时装）。
    pub installed: bool,
}

/// 一次插件操作最多等多久。
const TIMEOUT: Duration = Duration::from_secs(20);

fn configured(app: &AppHandle) -> BrowserConfig {
    app.try_state::<crate::SharedState>()
        .map(|s| s.settings.read().unwrap().browser.clone())
        .unwrap_or_default()
}

fn save(app: &AppHandle, config: &BrowserConfig) -> Result<(), String> {
    crate::settings_cmd::update_settings_from_backend(
        app,
        json!({"browser": {
            "extensions": config.extensions,
            "pendingUninstall": config.pending_uninstall,
        }}),
    )
    .map(|_| ())
}

fn info(entry: &BrowserExtensionConfig) -> ExtensionInfo {
    ExtensionInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        enabled: entry.enabled,
        path: entry.path.clone(),
        installed: entry.installed_id.is_some(),
    }
}

/// 插件文件夹路径的编号（FNV-1a，跨版本稳定；路径不分大小写、斜杠方向）。
fn id_of(path: &str) -> String {
    let normal = path
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normal.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 读插件文件夹里的 manifest.json：要有 `manifest_version` 和 `name`；名字是 `__MSG_x__` 时从
/// `_locales/<default_locale>/messages.json` 里查。返回（规范化的路径，插件名）。
fn read_manifest(path: &str) -> Result<(String, String), String> {
    let dir = std::path::Path::new(path.trim());
    if !dir.is_dir() {
        return Err(format!("找不到这个文件夹：{path}"));
    }
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("打不开这个文件夹：{e}"))?;
    let shown = dir.to_string_lossy();
    // canonicalize 在 Windows 上带 `\\?\` 前缀，WebView2 不认。
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
    let manifest = std::fs::read_to_string(dir.join("manifest.json")).map_err(|_| {
        "这个文件夹里没有 manifest.json：要选已解压的 Chrome / Edge 插件文件夹（不是 .crx 文件）".to_string()
    })?;
    let manifest: serde_json::Value = serde_json::from_str(manifest.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("manifest.json 看不懂：{e}"))?;
    if manifest["manifest_version"].as_u64().is_none() {
        return Err("manifest.json 里没有 manifest_version，不像浏览器插件".into());
    }
    let raw = manifest["name"].as_str().unwrap_or("").trim().to_string();
    let name = localized(&dir, &manifest, &raw).unwrap_or(raw);
    let name = if name.is_empty() {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "插件".into())
    } else {
        name
    };
    Ok((shown, name))
}

fn localized(dir: &std::path::Path, manifest: &serde_json::Value, raw: &str) -> Option<String> {
    let key = raw
        .strip_prefix("__MSG_")?
        .strip_suffix("__")?
        .to_lowercase();
    let locale = manifest["default_locale"].as_str()?;
    let text =
        std::fs::read_to_string(dir.join("_locales").join(locale).join("messages.json")).ok()?;
    let messages: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    messages
        .as_object()?
        .iter()
        .find(|(k, _)| k.to_lowercase() == key)
        .and_then(|(_, v)| v["message"].as_str().map(str::to_string))
}

/// 随便一个开着的标签页（插件装在整个浏览器上，用哪个标签页都一样）。
fn any_tab(app: &AppHandle) -> Option<Webview> {
    super::runtime(app)?
        .inner
        .lock()
        .unwrap()
        .tabs
        .first()
        .map(|t| t.webview.clone())
}

/// 插件列表。
pub fn list(app: &AppHandle) -> Vec<ExtensionInfo> {
    configured(app).extensions.iter().map(info).collect()
}

/// 加一个插件文件夹。浏览器开着就马上装（装不上就不加，如实报错）；没开着就先记下，下次打开时装。
pub async fn add(app: &AppHandle, path: &str) -> Result<ExtensionInfo, String> {
    let (path, name) = read_manifest(path)?;
    let id = id_of(&path);
    let mut config = configured(app);
    if let Some(existing) = config.extensions.iter().find(|e| e.id == id) {
        return Ok(info(existing));
    }
    let mut entry = BrowserExtensionConfig {
        id,
        path: path.clone(),
        name,
        enabled: true,
        installed_id: None,
    };
    if let Some(webview) = any_tab(app) {
        let installed = install(&webview, &path).await?;
        if !installed.name.is_empty() {
            entry.name = installed.name;
        }
        config.pending_uninstall.retain(|id| *id != installed.id);
        entry.installed_id = Some(installed.id);
        // 新装的插件只对之后打开的页面生效：刷新一下各标签页。
        reload_tabs(app).await;
    }
    log::log(&format!(
        "browser: 加了插件 {}（{}）",
        entry.name, entry.path
    ));
    config.extensions.push(entry.clone());
    save(app, &config)?;
    Ok(info(&entry))
}

/// 删掉一个插件（浏览器开着就马上卸，没开着就下次打开时卸）。
pub async fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut config = configured(app);
    let Some(index) = config.extensions.iter().position(|e| e.id == id) else {
        return Err("没有这个插件".into());
    };
    let entry = config.extensions.remove(index);
    if let Some(installed) = entry.installed_id.clone() {
        let done = match any_tab(app) {
            Some(webview) => match uninstall(&webview, &installed).await {
                Ok(()) => {
                    reload_tabs(app).await;
                    true
                }
                Err(e) => {
                    log::log(&format!("browser: 插件 {} 没卸掉：{e}", entry.name));
                    false
                }
            },
            None => false,
        };
        if !done && !config.pending_uninstall.contains(&installed) {
            // 浏览器没开（或这次没卸掉）：下次打开时卸。
            config.pending_uninstall.push(installed);
        }
    }
    log::log(&format!("browser: 删了插件 {}", entry.name));
    save(app, &config)
}

/// 开 / 关一个插件。
pub async fn toggle(app: &AppHandle, id: &str, enabled: bool) -> Result<ExtensionInfo, String> {
    let mut config = configured(app);
    let Some(entry) = config.extensions.iter_mut().find(|e| e.id == id) else {
        return Err("没有这个插件".into());
    };
    entry.enabled = enabled;
    let entry = entry.clone();
    if let (Some(webview), Some(installed)) = (any_tab(app), entry.installed_id.as_deref()) {
        enable(&webview, installed, enabled).await?;
        reload_tabs(app).await;
    }
    save(app, &config)?;
    Ok(info(&entry))
}

/// 插件变了以后重新打开各标签页的网页（插件只对之后打开的页面生效）。用 `location.replace` 换成标签页的网址，
/// 不用刷新、也不用再导航一次：页面还在加载时刷新会刷回空白页，再导航一次会在历史里多留一条
/// （“后退”退回空白页或同一页；2026-11-21 真机）。
async fn reload_tabs(app: &AppHandle) {
    let pages: Vec<(Webview, String)> = super::runtime(app)
        .map(|rt| {
            rt.inner
                .lock()
                .unwrap()
                .tabs
                .iter()
                .filter_map(|t| Some((t.webview.clone(), t.info.url.clone()?)))
                .collect()
        })
        .unwrap_or_default();
    for (webview, url) in pages {
        let script = format!(
            "location.replace({})",
            serde_json::to_string(&url).unwrap_or_default()
        );
        if super::cdp::evaluate(&webview, &script).await.is_err() {
            let _ = webview.reload();
        }
    }
}

/// 每次运行第一个标签页建好时：浏览器里的插件和设置对一遍（只动 Kotone 自己装的）。
pub(super) async fn sync(app: &AppHandle, webview: &Webview) {
    let mut config = configured(app);
    if config.extensions.is_empty() && config.pending_uninstall.is_empty() {
        return;
    }
    let present = match installed(webview).await {
        Ok(present) => present,
        Err(e) => {
            log::log(&format!("browser: 读不到浏览器里的插件：{e}"));
            return;
        }
    };
    let plan = plan_sync(&config, &present);
    let mut touched = false;
    for id in &plan.uninstall {
        match uninstall(webview, id).await {
            Ok(()) => touched = true,
            Err(e) => log::log(&format!("browser: 删掉的插件 {id} 没卸掉：{e}")),
        }
    }
    config.pending_uninstall.clear();
    for entry in config.extensions.iter_mut() {
        if plan.install.contains(&entry.id) {
            match install(webview, &entry.path).await {
                Ok(new) => {
                    log::log(&format!("browser: 装上了插件 {}", entry.name));
                    touched = true;
                    let id = new.id.clone();
                    entry.installed_id = Some(new.id);
                    if new.enabled != entry.enabled {
                        if let Err(e) = enable(webview, &id, entry.enabled).await {
                            log::log(&format!("browser: 插件 {} 开关没切过来：{e}", entry.name));
                        }
                    }
                }
                Err(e) => {
                    log::log(&format!("browser: 插件 {} 装不上：{e}", entry.name));
                    entry.installed_id = None;
                }
            }
        } else if let Some(id) = entry
            .installed_id
            .as_deref()
            .filter(|id| plan.toggle.contains(&id.to_string()))
        {
            match enable(webview, id, entry.enabled).await {
                Ok(()) => touched = true,
                Err(e) => log::log(&format!("browser: 插件 {} 开关没切过来：{e}", entry.name)),
            }
        }
    }
    if config != configured(app) {
        if let Err(e) = save(app, &config) {
            log::log(&format!("browser: 插件列表没存上：{e}"));
        }
    }
    if touched {
        reload_tabs(app).await;
    }
}

/// 对一遍要做的事（纯逻辑）：卸哪些（玩家删掉的、还在浏览器里的）、装哪些（设置里有、浏览器里没有的，
/// 按 Kotone 的编号）、开关哪些（按 WebView2 的 id）。别的插件不碰。
#[derive(Debug, Default, PartialEq)]
struct SyncPlan {
    uninstall: Vec<String>,
    install: Vec<String>,
    toggle: Vec<String>,
}

fn plan_sync(config: &BrowserConfig, present: &[Installed]) -> SyncPlan {
    let mut plan = SyncPlan::default();
    for id in &config.pending_uninstall {
        let still_wanted = config
            .extensions
            .iter()
            .any(|e| e.installed_id.as_deref() == Some(id.as_str()));
        if !still_wanted && present.iter().any(|p| p.id == *id) {
            plan.uninstall.push(id.clone());
        }
    }
    for entry in &config.extensions {
        let found = entry
            .installed_id
            .as_deref()
            .and_then(|id| present.iter().find(|p| p.id == id));
        match found {
            None => plan.install.push(entry.id.clone()),
            Some(found) if found.enabled != entry.enabled => plan.toggle.push(found.id.clone()),
            Some(_) => {}
        }
    }
    plan
}

// ---------- WebView2 Profile 插件接口 ----------

#[derive(Debug, Clone)]
struct Installed {
    id: String,
    name: String,
    enabled: bool,
}

type Reply = tokio::sync::oneshot::Sender<Result<Vec<Installed>, String>>;

/// 在主线程上拿到 Profile 做一件事，等 WebView2 回调交回结果。
async fn profile_op(
    webview: &Webview,
    what: &'static str,
    op: impl FnOnce(&Profile, Reply) -> Result<(), String> + Send + 'static,
) -> Result<Vec<Installed>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |platform| {
            #[cfg(windows)]
            {
                use windows::core::Interface;
                let profile = (|| -> windows::core::Result<Profile> {
                    let core = unsafe { platform.controller().CoreWebView2()? };
                    let core13 = core
                        .cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_13>(
                    )?;
                    unsafe { core13.Profile()?.cast() }
                })();
                match profile {
                    Ok(profile) => {
                        // 出错时 tx 已经交给 op：op 自己负责回复或丢掉（丢掉 = “没有回应”）。
                        if let Err(e) = op(&profile, tx) {
                            log::log(&format!("browser: 插件操作（{what}）没发出去：{e}"));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("这个 WebView2 版本不支持插件（{e}）")));
                    }
                }
            }
            #[cfg(not(windows))]
            {
                let _ = (platform, op, what);
                let _ = tx.send(Err("只有 Windows 支持浏览器插件".into()));
            }
        })
        .map_err(|e| format!("浏览器不可用：{e}"))?;
    match tokio::time::timeout(TIMEOUT, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(format!("浏览器没有回应（{what}）")),
        Err(_) => Err(format!("浏览器 {} 秒没有回应（{what}）", TIMEOUT.as_secs())),
    }
}

#[cfg(windows)]
type Profile = webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Profile7;
#[cfg(not(windows))]
type Profile = ();

#[cfg(windows)]
fn describe(
    extension: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2BrowserExtension,
) -> Installed {
    unsafe {
        let mut id = windows::core::PWSTR::null();
        let mut name = windows::core::PWSTR::null();
        let mut enabled = windows::core::BOOL::default();
        let id = if extension.Id(&mut id).is_ok() {
            webview2_com::take_pwstr(id)
        } else {
            String::new()
        };
        let name = if extension.Name(&mut name).is_ok() {
            webview2_com::take_pwstr(name)
        } else {
            String::new()
        };
        let _ = extension.IsEnabled(&mut enabled);
        Installed {
            id,
            name,
            enabled: enabled.as_bool(),
        }
    }
}

/// 浏览器里现在装着的插件。
async fn installed(webview: &Webview) -> Result<Vec<Installed>, String> {
    profile_op(webview, "读插件", |profile, tx| {
        #[cfg(windows)]
        unsafe {
            let handler = webview2_com::ProfileGetBrowserExtensionsCompletedHandler::create(
                Box::new(move |result, list| {
                    let _ = tx.send(result.map_err(|e| e.message().to_string()).map(|()| {
                        let mut out = Vec::new();
                        if let Some(list) = list {
                            let mut count = 0;
                            let _ = list.Count(&mut count);
                            for i in 0..count {
                                if let Ok(extension) = list.GetValueAtIndex(i) {
                                    out.push(describe(&extension));
                                }
                            }
                        }
                        out
                    }));
                    Ok(())
                }),
            );
            profile
                .GetBrowserExtensions(&handler)
                .map_err(|e| e.message().to_string())
        }
        #[cfg(not(windows))]
        {
            let _ = (profile, tx);
            Ok(())
        }
    })
    .await
}

/// 装一个插件文件夹。
async fn install(webview: &Webview, path: &str) -> Result<Installed, String> {
    let path = path.to_string();
    let added = profile_op(webview, "装插件", move |profile, tx| {
        #[cfg(windows)]
        unsafe {
            let handler = webview2_com::ProfileAddBrowserExtensionCompletedHandler::create(
                Box::new(move |result, extension| {
                    let _ = tx.send(match (result, extension) {
                        (Ok(()), Some(extension)) => Ok(vec![describe(&extension)]),
                        (Ok(()), None) => Err("浏览器没有交回插件".into()),
                        (Err(e), _) => Err(e.message().to_string()),
                    });
                    Ok(())
                }),
            );
            profile
                .AddBrowserExtension(&windows::core::HSTRING::from(path.as_str()), &handler)
                .map_err(|e| e.message().to_string())
        }
        #[cfg(not(windows))]
        {
            let _ = (profile, tx, path);
            Ok(())
        }
    })
    .await
    .map_err(|e| {
        format!("插件装不上（{e}）：WebView2 只支持已解压的 Chrome / Edge 插件，部分插件接口不支持")
    })?;
    added
        .into_iter()
        .next()
        .ok_or_else(|| "浏览器没有交回插件".to_string())
}

/// 对浏览器里某个插件做一件事（卸掉 / 开关）：先按 id 找到它。
async fn with_extension(
    webview: &Webview,
    what: &'static str,
    id: &str,
    enabled: Option<bool>,
) -> Result<(), String> {
    let id = id.to_string();
    profile_op(webview, what, move |profile, tx| {
        #[cfg(windows)]
        unsafe {
            let handler = webview2_com::ProfileGetBrowserExtensionsCompletedHandler::create(
                Box::new(move |result, list| {
                    let found = result.ok().and(list).and_then(|list| {
                        let mut count = 0;
                        let _ = list.Count(&mut count);
                        (0..count)
                            .filter_map(|i| list.GetValueAtIndex(i).ok())
                            .find(|e| describe(e).id == id)
                    });
                    let Some(extension) = found else {
                        // 本来就不在：卸掉算成功，开关报错。
                        let _ = tx.send(if enabled.is_none() {
                            Ok(Vec::new())
                        } else {
                            Err("浏览器里没有这个插件".into())
                        });
                        return Ok(());
                    };
                    let sent = match enabled {
                        None => {
                            let done = webview2_com::BrowserExtensionRemoveCompletedHandler::create(
                                Box::new(move |result| {
                                    let _ = tx.send(
                                        result
                                            .map(|()| Vec::new())
                                            .map_err(|e| e.message().to_string()),
                                    );
                                    Ok(())
                                }),
                            );
                            extension.Remove(&done)
                        }
                        Some(on) => {
                            let done = webview2_com::BrowserExtensionEnableCompletedHandler::create(
                                Box::new(move |result| {
                                    let _ = tx.send(
                                        result
                                            .map(|()| Vec::new())
                                            .map_err(|e| e.message().to_string()),
                                    );
                                    Ok(())
                                }),
                            );
                            extension.Enable(on, &done)
                        }
                    };
                    if let Err(e) = sent {
                        log::log(&format!("browser: 插件操作没发出去：{e}"));
                    }
                    Ok(())
                }),
            );
            profile
                .GetBrowserExtensions(&handler)
                .map_err(|e| e.message().to_string())
        }
        #[cfg(not(windows))]
        {
            let _ = (profile, tx, id, enabled);
            Ok(())
        }
    })
    .await
    .map(|_| ())
}

async fn uninstall(webview: &Webview, id: &str) -> Result<(), String> {
    with_extension(webview, "卸插件", id, None).await
}

async fn enable(webview: &Webview, id: &str, enabled: bool) -> Result<(), String> {
    with_extension(webview, "开关插件", id, Some(enabled)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(id: &str, enabled: bool) -> Installed {
        Installed {
            id: id.into(),
            name: id.into(),
            enabled,
        }
    }

    fn entry(id: &str, installed: Option<&str>, enabled: bool) -> BrowserExtensionConfig {
        BrowserExtensionConfig {
            id: id.into(),
            path: format!(r"C:\ext\{id}"),
            name: id.into(),
            enabled,
            installed_id: installed.map(str::to_string),
        }
    }

    #[test]
    fn sync_only_touches_kotones_own_extensions() {
        let config = BrowserConfig {
            extensions: vec![
                entry("new", None, true),
                entry("gone", Some("edge-gone"), true),
                entry("off", Some("edge-off"), false),
                entry("same", Some("edge-same"), true),
            ],
            pending_uninstall: vec!["edge-removed".into(), "edge-never-installed".into()],
        };
        let browser = [
            present("edge-pdf-viewer", true), // WebView2 自带的：不碰
            present("edge-off", true),
            present("edge-same", true),
            present("edge-removed", true),
        ];
        assert_eq!(
            plan_sync(&config, &browser),
            SyncPlan {
                uninstall: vec!["edge-removed".into()],
                install: vec!["new".into(), "gone".into()],
                toggle: vec!["edge-off".into()],
            }
        );
        // 删了又加回同一个文件夹（还没打开过浏览器）：不卸
        let readded = BrowserConfig {
            extensions: vec![entry("same", Some("edge-same"), true)],
            pending_uninstall: vec!["edge-same".into()],
        };
        assert_eq!(plan_sync(&readded, &browser), SyncPlan::default());
    }

    #[test]
    fn extension_ids_are_stable_per_folder() {
        assert_eq!(id_of(r"C:\ext\adblock"), id_of("c:/ext/adblock/"));
        assert_ne!(id_of(r"C:\ext\adblock"), id_of(r"C:\ext\other"));
        assert_eq!(id_of(r"C:\ext\adblock").len(), 16);
    }

    #[test]
    fn manifests_must_be_unpacked_extensions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_manifest(&dir.path().join("missing").to_string_lossy()).is_err());
        assert!(read_manifest(&dir.path().to_string_lossy())
            .unwrap_err()
            .contains("manifest.json"));
        std::fs::write(dir.path().join("manifest.json"), r#"{"name": "x"}"#).unwrap();
        assert!(read_manifest(&dir.path().to_string_lossy()).is_err());
        std::fs::write(
            dir.path().join("manifest.json"),
            "\u{feff}{\"manifest_version\": 3, \"name\": \"__MSG_appName__\", \"default_locale\": \"zh_CN\"}",
        )
        .unwrap();
        let locale = dir.path().join("_locales").join("zh_CN");
        std::fs::create_dir_all(&locale).unwrap();
        std::fs::write(
            locale.join("messages.json"),
            r#"{"appname": {"message": "去广告"}}"#,
        )
        .unwrap();
        let (path, name) = read_manifest(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(name, "去广告");
        assert!(!path.starts_with(r"\\?\"));
    }

    #[test]
    fn the_test_extension_fixture_is_valid() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../scripts/manual-e2e/fixtures/browser-extension-title");
        let (_, name) = read_manifest(&fixture.to_string_lossy()).unwrap();
        assert_eq!(name, "Kotone test: title marker");
    }
}
