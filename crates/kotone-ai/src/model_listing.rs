//! 读服务商的模型列表（`GET {baseUrl}/models`），同时当作“测试连接”。
//!
//! 照 DeepSeek Harness 的 `dsh-llm-pi-ai/discovery.ts`（MIT）：列表接口不花 token，一次请求就能确认
//! 地址连得上、密钥有效，并拿到服务商真实提供的模型（给模型菜单用）。OpenAI 兼容接口用 Bearer 鉴权；
//! Anthropic 用 `x-api-key` + `anthropic-version`，在 `{根}/v1/models`。回复同时接受标准的 `data` 数组
//! 和部分网关 / Gemini 用的 `models` 数组。不支持列表接口（404 / 405 / 501）时报告“不支持”，由调用方
//! 改用一次很短的对话请求确认连通。错误信息不回显服务商正文与带鉴权的地址。

use kotone_core::cancel::CancellationToken;
use kotone_core::connection::ResolvedConnection;
use serde_json::Value;

/// 回复超过这个大小就拒收（地址是用户填的）。
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    /// 服务商列出的模型 id（按列表顺序、去重）。
    Models(Vec<String>),
    /// 这个服务商没有模型列表接口。
    Unsupported,
}

fn is_anthropic(connection: &ResolvedConnection) -> bool {
    connection.provider == "anthropic" || connection.base_url.contains("api.anthropic.com")
}

/// 列表地址：OpenAI 兼容是 `{base}/models`；Anthropic 是 `{根}/v1/models`（根 = 去掉末尾的 `/v1`）。
pub fn listing_url(connection: &ResolvedConnection) -> String {
    let base = connection.base_url.trim().trim_end_matches('/');
    if is_anthropic(connection) {
        let root = base.strip_suffix("/v1").unwrap_or(base);
        format!("{root}/v1/models?limit=1000")
    } else {
        format!("{base}/models")
    }
}

/// 从回复里取模型 id：`data[].id`，或 `models[].id / name`（Gemini 的 `models/xxx` 去掉前缀）。
pub fn parse_models(body: &Value) -> Vec<String> {
    let entries = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array));
    let mut out: Vec<String> = Vec::new();
    for entry in entries.into_iter().flatten() {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| entry.get("name").and_then(Value::as_str))
            .or_else(|| entry.as_str())
            .map(|id| id.trim().trim_start_matches("models/").to_string());
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 读一次模型列表。连不上、密钥无效、服务商出错时返回 Err（给用户看的一句话）。
pub async fn list_models(
    connection: &ResolvedConnection,
    cancel: &CancellationToken,
) -> Result<Listing, String> {
    let key = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or("这条连接还没有保存 API key")?;
    let mut builder = client().get(listing_url(connection));
    builder = if is_anthropic(connection) {
        builder
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
    } else {
        builder.bearer_auth(key)
    };
    let request = async {
        let mut response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                "连接超时，请检查接口地址与网络".to_string()
            } else {
                "连不上这个接口地址，请检查地址与网络".to_string()
            }
        })?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            404 | 405 | 501 => return Ok(Listing::Unsupported),
            401 | 403 => return Err(format!("API key 无效或没有权限（HTTP {status}）")),
            429 => return Err("服务商限流或余额不足（HTTP 429），稍后再试或检查账户".into()),
            _ => return Err(format!("服务商返回 HTTP {status}，请检查连接与服务商状态")),
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err("模型列表太大，读取中止".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "读取模型列表失败".to_string())?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err("模型列表太大，读取中止".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        // 2xx 但不是模型列表（例如网关把未知路径转到网页）：当作不支持，改用对话请求确认。
        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(Listing::Unsupported);
        };
        if body.get("data").is_none() && body.get("models").is_none() {
            return Ok(Listing::Unsupported);
        }
        Ok(Listing::Models(parse_models(&body)))
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("请求已取消".into()),
        result = tokio::time::timeout(TIMEOUT, request) =>
            result.map_err(|_| "连接超时，请检查接口地址与网络".to_string())?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn connection(base_url: String, provider: &str) -> ResolvedConnection {
        ResolvedConnection {
            id: "c".into(),
            display_name: "c".into(),
            kind: Default::default(),
            provider: provider.into(),
            base_url,
            model: "m".into(),
            api_key: Some("sk-test".into()),
        }
    }

    #[test]
    fn urls_follow_each_protocol() {
        let c = connection("https://api.deepseek.com/v1/".into(), "deepseek");
        assert_eq!(listing_url(&c), "https://api.deepseek.com/v1/models");
        let a = connection("https://api.anthropic.com/v1".into(), "anthropic");
        assert_eq!(
            listing_url(&a),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
    }

    #[test]
    fn parses_data_and_models_shapes() {
        let openai = json!({"object": "list", "data": [{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}, {"id": "deepseek-chat"}]});
        assert_eq!(
            parse_models(&openai),
            ["deepseek-chat", "deepseek-reasoner"]
        );
        let gemini = json!({"models": [{"name": "models/gemini-2.5-pro"}]});
        assert_eq!(parse_models(&gemini), ["gemini-2.5-pro"]);
    }

    #[tokio::test]
    async fn lists_models_with_bearer_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "m"}]})))
            .expect(1)
            .mount(&server)
            .await;
        let c = connection(format!("{}/v1", server.uri()), "deepseek");
        let listing = list_models(&c, &CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(listing, Listing::Models(vec!["m".into()]));
    }

    #[tokio::test]
    async fn reports_bad_keys_and_missing_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bad/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("secret body sk-test"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/none/models"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let bad = list_models(
            &connection(format!("{}/bad", server.uri()), "x"),
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
        assert!(bad.contains("API key") && !bad.contains("secret"), "{bad}");
        let none = list_models(
            &connection(format!("{}/none", server.uri()), "x"),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
        assert_eq!(none, Listing::Unsupported);
        let down = list_models(
            &connection("http://127.0.0.1:9".into(), "x"),
            &CancellationToken::default(),
        )
        .await;
        assert!(down.is_err());
    }
}
