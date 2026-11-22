//! 后台任务的浏览器工具（ADR 031）：网址检查、第一次打开某个网站经主会话问玩家、交给玩家时告诉玩家，
//! 页面操作本身在 `crate::browser`。主会话没有这些工具（清单里就没给），这里再挡一遍。

use super::*;

use kotone_agent::browser as rules;

/// 玩家把浏览器工具设成了“每次询问”时问的话。
fn describe(tool: &str, arguments: &Value) -> String {
    match tool {
        rules::TOOL_OPEN => format!("打开 {}", str_arg(arguments, "url")),
        rules::TOOL_CLICK => "在网页上点一下".into(),
        rules::TOOL_TYPE => "在网页上输入文字".into(),
        rules::TOOL_PRESS => "在网页上按一个键".into(),
        _ => "读网页".into(),
    }
}

/// 后台任务调用 `browser.*`。`confirm`：玩家把这个工具设成了“每次询问”。
pub(super) async fn browser_tool(
    app: &AppHandle,
    caller: &str,
    tool: &str,
    arguments: &Value,
    confirm: bool,
    cancel: &CancellationToken,
) -> ToolOutcome {
    let Some(spec) = child_spec(app, caller) else {
        return ToolOutcome::error("网页只在后台操作：用 agent.start 交给后台");
    };
    if confirm && tool != rules::TOOL_HANDOVER {
        let question = format!("要我{}吗？", describe(tool, arguments));
        if subagent_ask(app, &spec, caller, question, cancel).await == Answer::Deny {
            return ToolOutcome::error("没有执行：玩家没同意");
        }
    }
    let result = match tool {
        rules::TOOL_OPEN => open(app, &spec, caller, arguments, cancel).await,
        rules::TOOL_SNAPSHOT => {
            let more = arguments
                .get("more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match crate::browser::snapshot(app, more).await {
                Ok(snap) => match snap.captcha {
                    // 验证码：交给玩家，弄好了再读一次（照 ego 的 ego_wait_for_human）。
                    Some(kind) => {
                        let reason = "网页要做人机验证".to_string();
                        log::log(&format!("agent: {} 遇到验证码（{kind}）", spec.name));
                        match hand_over(app, &spec, &reason, cancel).await {
                            Ok(done) => crate::browser::snapshot(app, false).await.map(|again| {
                                let mut value = again.value;
                                value["handover"] = done;
                                value
                            }),
                            Err(e) => Err(e),
                        }
                    }
                    None => Ok(snap.value),
                },
                Err(e) => Err(e),
            }
        }
        rules::TOOL_CLICK => match arguments.get("ref").and_then(Value::as_u64) {
            Some(reference) => crate::browser::click(app, reference).await,
            None => Err("ref 要写最近一次 browser.snapshot 里的编号".into()),
        },
        rules::TOOL_TYPE => match arguments.get("ref").and_then(Value::as_u64) {
            Some(reference) => {
                let submit = arguments
                    .get("submit")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                crate::browser::type_text(app, reference, str_arg(arguments, "text"), submit).await
            }
            None => Err("ref 要写最近一次 browser.snapshot 里的编号".into()),
        },
        rules::TOOL_PRESS => crate::browser::press(app, str_arg(arguments, "key")).await,
        rules::TOOL_WAIT => {
            let text = arguments.get("text").and_then(Value::as_str);
            let seconds = arguments.get("seconds").and_then(Value::as_f64);
            crate::browser::wait(app, text, seconds).await
        }
        rules::TOOL_HANDOVER => {
            let reason = str_arg(arguments, "reason").trim();
            let reason = if reason.is_empty() {
                "网页要你来弄一下"
            } else {
                reason
            };
            hand_over(app, &spec, reason, cancel).await
        }
        other => Err(format!("没有名为 {other} 的浏览器工具")),
    };
    match result {
        Ok(value) => ToolOutcome::ok(value.to_string()),
        Err(e) => ToolOutcome::error(e),
    }
}

/// 打开网址：网址检查（含域名解析后的地址）、第一次打开某个网站先经主会话问玩家（记在
/// `<KOTONE_HOME>/browser-sites.json`）。
async fn open(
    app: &AppHandle,
    spec: &ChildSpec,
    caller: &str,
    arguments: &Value,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let url = rules::check_url(str_arg(arguments, "url"))?;
    resolves_publicly(&url).await?;
    let site = rules::site_of(&url);
    let path = crate::browser::sites_path();
    let mut sites = rules::SiteApprovals::load(&path);
    if !sites.covers(&site) {
        let question = format!("要我打开 {site} 吗？");
        log::log(&format!("agent: {} asks before opening {site}", spec.name));
        if subagent_ask(app, spec, caller, question, cancel).await == Answer::Deny {
            return Err(format!("没有打开：玩家没同意打开 {site}"));
        }
        sites.approve(&site);
        if let Err(e) = sites.save(&path) {
            log::log(&format!("browser: 记不住同意过的网站：{e}"));
        }
    }
    crate::browser::open(app, &url).await
}

/// 域名解析后的地址也不能是本机 / 内网（照 url-guard 的“解析后再查一遍”）。解析失败交给浏览器自己报错。
async fn resolves_publicly(url: &url::Url) -> Result<(), String> {
    let Some(url::Host::Domain(host)) = url.host() else {
        return Ok(());
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let Ok(Ok(addresses)) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    else {
        return Ok(());
    };
    for address in addresses {
        if rules::is_private_ip(address.ip()) {
            return Err(format!("{host} 指向本机或内网的地址，不能打开"));
        }
    }
    Ok(())
}

/// 交给玩家：语音入口时开口说一句（不把 Kotone 拉到前台，玩家可能在游戏里），界面上右侧栏展开、
/// 显示横幅；等玩家弄好。
async fn hand_over(
    app: &AppHandle,
    spec: &ChildSpec,
    reason: &str,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    if last_entry(app, &spec.parent) == Some(Entry::Call) {
        let line = format!("{reason}，你方便时打开 Kotone，在右边的浏览器里弄一下");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::assistant::deliver_text(&app, line, vec![DeliveryKind::Voice]).await;
        });
    }
    crate::browser::handover(app, reason, cancel).await
}
