//! 联网搜索与读网页（ADR 026）：DeepSeek「网页搜索与抓取」接口（`/v1/tools/search`、`/v1/tools/fetch`）。
//! DeepSeek 指南：Agent 或自己的程序要搜索、抓取，用这两个接口（每 1000 次成功请求 5 元，失败、无结果不计费）。
//! 抓取在 DeepSeek 的服务器上做，访问不到玩家的本机和局域网（实测 `127.0.0.1` → 403）。
//!
//! 这里只发请求、截短、把错误换成给玩家看的话；“只读这段对话里出现过的网址”由调用方（桌面）检查。

use std::time::Duration;

use kotone_core::connection::ResolvedConnection;
use serde::Serialize;
use serde_json::{json, Value};

/// 一次最多返回几条结果。
pub const MAX_RESULTS: u64 = 10;
/// 每条结果的摘要最多几个字（接口的 `summary` 约 600–800 字，写给模型看）。
const SUMMARY_CHARS: usize = 900;
/// 读一页最多给模型几个字（太长的页面截断，免得一页吃掉半段上下文）。
pub const PAGE_CHARS: usize = 8000;
/// 搜索最慢等多久；抓取慢一些（实测 3–7 秒）。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
const FETCH_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    /// 发布日期（`YYYY-MM-DD`；接口没给时为空）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub url: String,
    pub title: String,
    /// Markdown 正文（截到 [`PAGE_CHARS`]）。
    pub content: String,
    pub truncated: bool,
}

fn endpoint(connection: &ResolvedConnection, path: &str) -> String {
    let base = connection.base_url.trim().trim_end_matches('/');
    let base = if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    };
    format!("{base}/tools/{path}")
}

fn client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// 发一次请求；429 / 5xx / 连接失败时等一秒再试一次（接口是只读的，官方说可以重试）。
async fn post(
    connection: &ResolvedConnection,
    path: &str,
    body: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let key = connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if key.is_empty() {
        return Err("DeepSeek 连接还没有保存密钥".into());
    }
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let response = client()
            .post(endpoint(connection, path))
            .bearer_auth(&key)
            .timeout(timeout)
            .json(&body)
            .send()
            .await;
        let response = match response {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err("联网请求超时".into()),
            Err(_) => {
                last = "连不上 DeepSeek，请检查网络".into();
                continue;
            }
        };
        let status = response.status().as_u16();
        match status {
            200 => {
                return response
                    .json::<Value>()
                    .await
                    .map_err(|_| "联网结果格式不正确".into())
            }
            429 | 500..=599 => {
                last = format!("DeepSeek 联网服务暂时不可用（HTTP {status}）");
                continue;
            }
            401 => return Err("DeepSeek 密钥无效（HTTP 401）".into()),
            402 => return Err("DeepSeek 账户余额不足，请先充值".into()),
            403 => return Err("这个网址被 DeepSeek 的安全策略拒绝了".into()),
            404 => return Err("这个网页读不到内容（打不开、被网站拦了，或不是文字网页）".into()),
            _ => return Err(format!("联网请求失败（HTTP {status}）")),
        }
    }
    Err(last)
}

fn clip(text: &str, max: usize) -> (String, bool) {
    let text = text.trim();
    if text.chars().count() <= max {
        (text.to_string(), false)
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        (out, true)
    }
}

/// 搜索。`count` 1–[`MAX_RESULTS`]。
pub async fn search(
    connection: &ResolvedConnection,
    query: &str,
    count: u64,
) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 128 {
        return Err("搜索词为空或超过 128 字".into());
    }
    let body = json!({"query": query, "count": count.clamp(1, MAX_RESULTS)});
    let value = post(connection, "search", body, SEARCH_TIMEOUT).await?;
    let s = |r: &Value, k: &str| {
        r.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Ok(value
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter(|r| !s(r, "url").is_empty())
                .map(|r| {
                    let summary = if s(r, "summary").is_empty() {
                        s(r, "snippet")
                    } else {
                        s(r, "summary")
                    };
                    SearchHit {
                        title: s(r, "title"),
                        url: s(r, "url"),
                        site: s(r, "site_name"),
                        date: s(r, "date").chars().take(10).collect(),
                        summary: clip(&summary, SUMMARY_CHARS).0,
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

/// 读一页（只接受 http / https、带主机名、不带用户名密码的网址）。
pub async fn fetch(connection: &ResolvedConnection, url: &str) -> Result<Page, String> {
    let url = url.trim();
    if !valid_url(url) {
        return Err("只能读 http / https 网址".into());
    }
    let value = post(connection, "fetch", json!({"url": url}), FETCH_TIMEOUT).await?;
    let text = |k: &str| {
        value
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let (content, truncated) = clip(&text("content"), PAGE_CHARS);
    Ok(Page {
        url: url.to_string(),
        title: text("title"),
        content,
        truncated,
    })
}

/// 接口接受的网址：http(s)、有主机名、不带账号密码、不超过 8192 字。
pub fn valid_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    url.len() <= 8192
        && !host.is_empty()
        && !host.contains('@')
        && !url.contains(char::is_whitespace)
}

/// 每次成功的搜索 / 抓取的费用（人民币）：5 元 / 1000 次。
pub const COST_PER_CALL: f64 = 0.005;

// ---------- 统一模型清单的“联网搜索”类别（和语音识别、文字识别、语音合成并列） ----------

use kotone_core::model_catalog::{
    schema_of, CatalogBackend, CatalogModel, ModelCategory, StaticConfigField,
};
use kotone_core::settings::Settings;

/// DeepSeek 联网搜索（搜索与抓取接口）。
pub const DEEPSEEK_SEARCH_ID: &str = "deepseek-search";
/// 不联网（明确关掉）。
pub const OFF_ID: &str = "off";

const DEEPSEEK_SCHEMA: &[StaticConfigField] = &[StaticConfigField {
    key: "connection",
    label: "API 连接（DeepSeek；留空 = 用琴音正在用的 DeepSeek 连接）",
    kind: "connection",
    default: "",
    options: &[],
    required: false,
}];

pub fn is_known(id: &str) -> bool {
    id == DEEPSEEK_SEARCH_ID || id == OFF_ID
}

/// DeepSeek 联网搜索用哪条连接：清单里配置的 → 琴音正在用的（若是 DeepSeek）→ 第一条 DeepSeek 连接。
pub fn deepseek_connection_id(settings: &Settings) -> Option<String> {
    settings
        .model_configs
        .get(DEEPSEEK_SEARCH_ID)
        .and_then(|cfg| cfg.get("connection"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| settings.deepseek_connection_id())
}

/// 当前用的搜索：显式选择优先；空 = 自动（有 DeepSeek 连接就用 DeepSeek，否则不联网）。
pub fn resolve_active(settings: &Settings) -> &'static str {
    match settings.search.active_model_id.trim() {
        DEEPSEEK_SEARCH_ID => DEEPSEEK_SEARCH_ID,
        OFF_ID => OFF_ID,
        _ if deepseek_connection_id(settings).is_some() => DEEPSEEK_SEARCH_ID,
        _ => OFF_ID,
    }
}

/// 列出“联网搜索”类别。`deepseek_has_key`：所用连接是否已保存密钥（调用方查凭据库）。
pub fn entries(settings: &Settings, deepseek_has_key: Option<bool>) -> Vec<CatalogModel> {
    let active = resolve_active(settings);
    let (ready, state, reason) = match (deepseek_connection_id(settings), deepseek_has_key) {
        (None, _) => (
            false,
            "not-configured",
            Some("需要一条 DeepSeek API 连接".to_string()),
        ),
        (Some(_), Some(false)) => (
            false,
            "not-configured",
            Some("所选连接尚未保存 API key".to_string()),
        ),
        (Some(_), _) => (true, "ready", None),
    };
    let model = |id: &str, name: &str, description: &str, backend, ready: bool| CatalogModel {
        category: ModelCategory::Search,
        id: id.into(),
        engine_id: id.into(),
        display_name: name.into(),
        description: description.into(),
        size_bytes: 0,
        backend,
        downloaded: true,
        ready,
        state: if ready { "ready".into() } else { state.into() },
        reason: if ready { None } else { reason.clone() },
        active: active == id,
        recommended: id == DEEPSEEK_SEARCH_ID,
        languages: vec!["auto".into()],
        data_egress: if id == OFF_ID { "none" } else { "text" }.into(),
        config_schema: if id == OFF_ID {
            Vec::new()
        } else {
            schema_of(DEEPSEEK_SCHEMA)
        },
        api_key_url: (id == DEEPSEEK_SEARCH_ID)
            .then(|| "https://platform.deepseek.com/api_keys".into()),
        license: if id == OFF_ID {
            ""
        } else {
            "DeepSeek API terms"
        }
        .into(),
        files: Vec::new(),
    };
    vec![
        model(
            DEEPSEEK_SEARCH_ID,
            "DeepSeek 联网搜索（在线）",
            "琴音查游戏版本、出装、海克斯强化、打法时用；搜索词和要读的网址会发给 DeepSeek，网页由 DeepSeek 的服务器去读。每次成功的搜索或读网页 0.005 元",
            CatalogBackend::Remote,
            ready,
        ),
        model(
            OFF_ID,
            "不联网",
            "琴音只凭自己的知识回答（游戏版本相关的内容可能过时）",
            CatalogBackend::Local,
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::connection::ConnectionKind;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn connection(base: &str) -> ResolvedConnection {
        ResolvedConnection {
            id: "ds".into(),
            display_name: "DeepSeek".into(),
            kind: ConnectionKind::Remote,
            provider: "deepseek".into(),
            base_url: format!("{base}/v1"),
            model: "deepseek-flash".into(),
            api_key: Some("sk-test".into()),
        }
    }

    #[tokio::test]
    async fn search_returns_trimmed_hits() {
        let server = MockServer::start().await;
        let long = "很长".repeat(1000);
        Mock::given(method("POST"))
            .and(path("/v1/tools/search"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({"query": "海克斯大乱斗 强化", "count": 5})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [
                {"title": "海克斯强化推荐", "url": "https://example.com/a", "site_name": "九游", "date": "2026-11-14T08:00:00Z",
                 "snippet": "短", "summary": long},
                {"title": "无网址", "url": "", "summary": "x"},
                {"title": "只有片段", "url": "https://example.com/b", "snippet": "片段"}
            ]})))
            .mount(&server)
            .await;
        let hits = search(&connection(&server.uri()), "海克斯大乱斗 强化", 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].site, "九游");
        assert_eq!(hits[0].date, "2026-11-14");
        assert!(hits[0].summary.chars().count() <= SUMMARY_CHARS + 1);
        assert_eq!(hits[1].summary, "片段");
    }

    #[tokio::test]
    async fn fetch_clips_long_pages_and_explains_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/fetch"))
            .and(body_partial_json(
                json!({"url": "https://example.com/long"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"title": "长页", "content": "字".repeat(PAGE_CHARS + 50)}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/fetch"))
            .and(body_partial_json(json!({"url": "https://example.com/403"})))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let c = connection(&server.uri());
        let page = fetch(&c, "https://example.com/long").await.unwrap();
        assert!(page.truncated && page.content.chars().count() == PAGE_CHARS + 1);
        assert!(fetch(&c, "https://example.com/403")
            .await
            .unwrap_err()
            .contains("安全策略"));
        assert!(fetch(&c, "file:///C:/Windows").await.is_err());
        assert!(fetch(&c, "https://user:pw@example.com/").await.is_err());
    }

    #[test]
    fn endpoint_accepts_base_urls_with_or_without_v1() {
        let mut c = connection("https://api.deepseek.com");
        assert_eq!(
            endpoint(&c, "search"),
            "https://api.deepseek.com/v1/tools/search"
        );
        c.base_url = "https://api.deepseek.com/".into();
        assert_eq!(
            endpoint(&c, "fetch"),
            "https://api.deepseek.com/v1/tools/fetch"
        );
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use kotone_core::connection::Connection;

    fn with_deepseek() -> Settings {
        let mut settings = Settings::default();
        let mut c: Connection = serde_json::from_value(serde_json::json!({
            "id": "ds", "displayName": "DeepSeek", "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com/v1", "model": "deepseek-flash"
        }))
        .unwrap();
        c.id = "ds".into();
        settings.connections.push(c);
        settings
    }

    #[test]
    fn auto_uses_deepseek_when_there_is_a_deepseek_connection() {
        assert_eq!(resolve_active(&Settings::default()), OFF_ID);
        let mut settings = with_deepseek();
        assert_eq!(resolve_active(&settings), DEEPSEEK_SEARCH_ID);
        assert_eq!(deepseek_connection_id(&settings).as_deref(), Some("ds"));
        settings.search.active_model_id = OFF_ID.into();
        assert_eq!(resolve_active(&settings), OFF_ID);
        let list = entries(&settings, Some(true));
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|m| m.category == ModelCategory::Search));
        assert!(list.iter().find(|m| m.id == OFF_ID).unwrap().active);
        let ds = list.iter().find(|m| m.id == DEEPSEEK_SEARCH_ID).unwrap();
        assert!(ds.ready && ds.recommended && !ds.active);
        assert!(!entries(&settings, Some(false))[0].ready);
    }
}
