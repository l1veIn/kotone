//! 调试协议桥：经 WebView2 的 `CallDevToolsProtocolMethod` 在**程序内**调用浏览器调试协议
//! （不开调试端口，别的程序控制不了这个浏览器；ADR 031，照 DSH 桌面版“自动化必须走只碰单个页面的中间层”）。
//!
//! 调用在主线程上发出（`with_webview`），结果由 WebView2 在主线程回调，经 oneshot 交回 tokio。
//! **回调里只发 channel，不 emit、不阻塞**：在 `with_webview` 回调里发事件会让 tauri-runtime-wry 自锁
//! （2026-11-17 的教训）。

use std::time::Duration;

use serde_json::{json, Value};
use tauri::Webview;

/// 一次调用最多等多久（整页快照在大页面上要一两秒）。
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// 把每次调用记进日志（`KOTONE_BROWSER_TRACE`，排查用）。
pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 调用一个调试协议方法，返回它的结果对象。
pub async fn call(webview: &Webview, method: &str, params: Value) -> Result<Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let name = method.to_string();
    let params = params.to_string();
    webview
        .with_webview(move |platform| {
            #[cfg(windows)]
            unsafe {
                use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
                use windows::core::HSTRING;
                // 调用没发出去时处理器被丢掉，tx 跟着丢掉，等待的一方收到“没有回应”。
                if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
                    kotone_core::log::log(&format!("browser: cdp {name} →"));
                }
                let traced = name.clone();
                let sent = (|| -> windows::core::Result<()> {
                    let core = platform.controller().CoreWebView2()?;
                    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                        move |result, json| {
                            if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
                                kotone_core::log::log(&format!(
                                    "browser: cdp {traced} ← {} {}",
                                    if result.is_ok() { "ok" } else { "err" },
                                    json.chars().take(120).collect::<String>()
                                ));
                            }
                            let _ = tx.send(match result {
                                Ok(()) => Ok(json),
                                Err(e) => Err(if json.trim().is_empty() {
                                    e.message().to_string()
                                } else {
                                    json
                                }),
                            });
                            Ok(())
                        },
                    ));
                    core.CallDevToolsProtocolMethod(
                        &HSTRING::from(name.as_str()),
                        &HSTRING::from(params.as_str()),
                        &handler,
                    )
                })();
                if let Err(e) = sent {
                    kotone_core::log::log(&format!("browser: 调试协议调用 {name} 没发出去：{e}"));
                }
            }
            #[cfg(not(windows))]
            {
                let _ = (platform, name, params, tx);
            }
        })
        .map_err(|e| format!("浏览器不可用：{e}"))?;
    match tokio::time::timeout(CALL_TIMEOUT, rx).await {
        Ok(Ok(Ok(json))) => {
            serde_json::from_str(&json).map_err(|e| format!("浏览器的回复看不懂：{e}"))
        }
        Ok(Ok(Err(e))) => Err(format!("浏览器报错（{method}）：{}", short(&e))),
        Ok(Err(_)) => Err(format!("浏览器没有回应（{method}）")),
        Err(_) => Err(format!(
            "浏览器 {} 秒没有回应（{method}）",
            CALL_TIMEOUT.as_secs()
        )),
    }
}

/// 页面现在有没有在出声（WebView2 `IsDocumentPlayingAudio`，不管播放器怎么写的都准）。
pub async fn playing_audio(webview: &Webview) -> Option<bool> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<bool>>();
    webview
        .with_webview(move |platform| {
            #[cfg(windows)]
            unsafe {
                use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
                use windows::core::Interface;
                let value = (|| -> windows::core::Result<bool> {
                    let core = platform
                        .controller()
                        .CoreWebView2()?
                        .cast::<ICoreWebView2_8>()?;
                    let mut playing = windows::core::BOOL::default();
                    core.IsDocumentPlayingAudio(&mut playing)?;
                    Ok(playing.as_bool())
                })();
                let _ = tx.send(value.ok());
            }
            #[cfg(not(windows))]
            {
                let _ = (platform, tx.send(None));
            }
        })
        .ok()?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .ok()?
        .ok()?
}

/// 这个标签页是不是被静音了（WebView2 `IsMuted`）。
pub async fn muted(webview: &Webview) -> Option<bool> {
    mute_call(webview, None).await
}

/// 静音 / 取消静音这个标签页；返回设好之后的状态。
pub async fn set_muted(webview: &Webview, muted: bool) -> Option<bool> {
    mute_call(webview, Some(muted)).await
}

/// 回调里只读写属性、经 channel 交回，不 emit（`with_webview` 里发事件会自锁）。
async fn mute_call(webview: &Webview, set: Option<bool>) -> Option<bool> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<bool>>();
    webview
        .with_webview(move |platform| {
            #[cfg(windows)]
            unsafe {
                use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
                use windows::core::Interface;
                let value = (|| -> windows::core::Result<bool> {
                    let core = platform
                        .controller()
                        .CoreWebView2()?
                        .cast::<ICoreWebView2_8>()?;
                    if let Some(muted) = set {
                        core.SetIsMuted(muted)?;
                    }
                    let mut muted = windows::core::BOOL::default();
                    core.IsMuted(&mut muted)?;
                    Ok(muted.as_bool())
                })();
                let _ = tx.send(value.ok());
            }
            #[cfg(not(windows))]
            {
                let _ = (platform, set, tx.send(None));
            }
        })
        .ok()?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .ok()?
        .ok()?
}

fn short(text: &str) -> String {
    text.chars().take(300).collect()
}

/// 在页面主框架里求值一个表达式（`returnByValue`、等 Promise），返回值本身。
pub async fn evaluate(webview: &Webview, expression: &str) -> Result<Value, String> {
    let result = call(
        webview,
        "Runtime.evaluate",
        json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
    )
    .await?;
    if let Some(details) = result.get("exceptionDetails") {
        let text = details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or("页面脚本出错");
        return Err(format!("页面脚本出错：{}", short(text)));
    }
    Ok(result["result"]["value"].clone())
}

/// 对某个元素（`backendNodeId`）调用一个函数（this = 那个元素），返回函数的返回值。
pub async fn call_on_node(
    webview: &Webview,
    backend_node_id: u64,
    function: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let resolved = call(
        webview,
        "DOM.resolveNode",
        json!({"backendNodeId": backend_node_id}),
    )
    .await
    .map_err(|_| "这个元素已经不在页面上了：先 browser.snapshot 重新读页面".to_string())?;
    let Some(object_id) = resolved["object"]["objectId"].as_str().map(str::to_string) else {
        return Err("这个元素已经不在页面上了：先 browser.snapshot 重新读页面".into());
    };
    let args: Vec<Value> = arguments.iter().map(|v| json!({"value": v})).collect();
    let result = call(
        webview,
        "Runtime.callFunctionOn",
        json!({
            "objectId": object_id,
            "functionDeclaration": function,
            "arguments": args,
            "returnByValue": true,
            "awaitPromise": true,
            // 当成玩家亲手点的（点击、输入）：否则 target=_blank 的链接被弹窗拦截挡掉、播放按钮可能没反应。
            // 只给对元素的操作；轮询用的 evaluate 不带，免得网页一直有“用户刚操作过”的权限去弹窗。
            "userGesture": true,
        }),
    )
    .await;
    let _ = call(
        webview,
        "Runtime.releaseObject",
        json!({"objectId": object_id}),
    )
    .await;
    let result = result?;
    if let Some(details) = result.get("exceptionDetails") {
        let text = details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or("页面脚本出错");
        return Err(format!("页面脚本出错：{}", short(text)));
    }
    Ok(result["result"]["value"].clone())
}

/// 备选的点击：调试协议的鼠标事件（移动 → 按下 → 松开），发完检查点中的是不是这个元素
/// （照 ego / bcdp：`DOM.getNodeForLocation`）。浏览器视图看不见时 Chromium 会悄悄丢掉这类输入，
/// 所以只在页面内 `click()` 出错时才用。
pub async fn mouse_click(webview: &Webview, backend_node_id: u64) -> Result<(), String> {
    let model = call(
        webview,
        "DOM.getBoxModel",
        json!({"backendNodeId": backend_node_id}),
    )
    .await?;
    let quad = model["model"]["content"]
        .as_array()
        .ok_or("这个元素没有显示出来，点不到")?;
    let xs: Vec<f64> = quad.iter().step_by(2).filter_map(Value::as_f64).collect();
    let ys: Vec<f64> = quad
        .iter()
        .skip(1)
        .step_by(2)
        .filter_map(Value::as_f64)
        .collect();
    if xs.len() != 4 || ys.len() != 4 {
        return Err("这个元素没有显示出来，点不到".into());
    }
    let x = xs.iter().sum::<f64>() / 4.0;
    let y = ys.iter().sum::<f64>() / 4.0;
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let mut params = json!({"type": kind, "x": x, "y": y});
        if kind != "mouseMoved" {
            params["button"] = json!("left");
            params["clickCount"] = json!(1);
        }
        call(webview, "Input.dispatchMouseEvent", params).await?;
    }
    let hit = call(
        webview,
        "DOM.getNodeForLocation",
        json!({"x": x.round() as i64, "y": y.round() as i64}),
    )
    .await
    .ok()
    .and_then(|v| v["backendNodeId"].as_u64());
    match hit {
        Some(id) if id != backend_node_id => {
            Err("点的位置被别的东西挡住了：先 browser.snapshot 看看".into())
        }
        _ => Ok(()),
    }
}
