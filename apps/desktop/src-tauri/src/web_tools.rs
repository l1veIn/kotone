//! 琴音的 `web.search` / `web.fetch`（ADR 026）。用「设置 → 模型 → 联网搜索」里选中的后端
//! （目前只有 DeepSeek 搜索与抓取接口；和语音识别、文字识别、语音合成并列）。
//!
//! - `web.fetch` 只读这段对话里出现过的网址：玩家发的消息，或工具结果（搜索结果等）里的。
//!   模型自己写的话不算——它不能自己拼一个网址，把数据藏在网址里带出去。
//! - 成功一次记一笔用量（“联网搜索”，0.005 元）。

use kotone_ai::agent::Message;
use kotone_core::connection::ResolvedConnection;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 选中的搜索后端对应的连接（含密钥）；关掉了或没配好时说明怎么办。
fn backend(app: &AppHandle) -> Result<ResolvedConnection, String> {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    if kotone_ai::web::resolve_active(&settings) == kotone_ai::web::OFF_ID {
        return Err(
            "联网搜索没有开：需要一条 DeepSeek 连接，并在「设置 → 模型 → 联网搜索」里选“DeepSeek 联网搜索”"
                .into(),
        );
    }
    let id = kotone_ai::web::deepseek_connection_id(&settings)
        .ok_or("联网搜索需要一条 DeepSeek 连接：在「设置 → API 连接」里添加")?;
    let connection = state.connections.resolve(&id)?;
    if connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("联网搜索用的 DeepSeek 连接还没有保存 API key".into());
    }
    Ok(connection)
}

/// 搜索 / 读网页的费用记到发起的对话（主会话这一轮，或子会话自己）。
fn owner(caller: &str) -> Option<kotone_core::usage::Owner> {
    if caller.is_empty() {
        return None;
    }
    Some(
        kotone_core::usage::current_turn()
            .filter(|t| t.conversation == caller)
            .unwrap_or(kotone_core::usage::Owner {
                conversation: caller.to_string(),
                turn: None,
            }),
    )
}

pub async fn search(app: &AppHandle, caller: &str, arguments: &Value) -> Result<Value, String> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let count = arguments.get("count").and_then(Value::as_u64).unwrap_or(5);
    let connection = backend(app)?;
    let hits = kotone_ai::web::search(&connection, &query, count).await?;
    kotone_core::usage::record_search_for(&connection.provider, "search", owner(caller));
    kotone_core::log::log(&format!("web: search ({} results)", hits.len()));
    if hits.is_empty() {
        return Ok(json!({"results": [], "note": "没有搜到结果，换个说法再搜"}));
    }
    Ok(json!({"results": hits}))
}

pub async fn fetch(app: &AppHandle, caller: &str, arguments: &Value) -> Result<Value, String> {
    let url = arguments
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !kotone_ai::web::valid_url(&url) {
        return Err("只能读 http / https 网址".into());
    }
    let store = crate::agent::conversation_store(app)?;
    let conversation = store
        .load(caller)
        .map_err(|e| e.to_string())?
        .ok_or("找不到这段对话")?;
    if !seen_in(&conversation.messages, &url) {
        return Err(
            "只能读这段对话里出现过的网址（web.search 的结果或玩家发来的链接），不能自己写网址"
                .into(),
        );
    }
    let connection = backend(app)?;
    let page = kotone_ai::web::fetch(&connection, &url).await?;
    kotone_core::usage::record_search_for(&connection.provider, "fetch", owner(caller));
    kotone_core::log::log(&format!(
        "web: fetched a page ({} chars)",
        page.content.chars().count()
    ));
    Ok(json!(page))
}

/// 网址是不是出现在玩家的消息或工具结果里（不看模型自己写的话）。末尾的 `/` 不计较。
fn seen_in(messages: &[Message], url: &str) -> bool {
    let bare = url.trim_end_matches('/');
    messages.iter().any(|m| match m {
        Message::User { text, .. } => text.contains(bare),
        Message::Tool { content, .. } => content.contains(bare),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_urls_from_the_player_or_tool_results_can_be_fetched() {
        let messages = vec![
            Message::user("看看 https://example.com/guide 这篇"),
            Message::Assistant {
                text: "我去读 https://evil.example/?q=secret".into(),
                tool_calls: Vec::new(),
                reasoning: String::new(),
                usage: None,
                at_ms: 0,
            },
            Message::Tool {
                call_id: "c1".into(),
                name: "web.search".into(),
                content: r#"{"results":[{"url":"https://www.wanplus.cn/lol/aram/"}]}"#.into(),
                is_error: false,
                untrusted: true,
                images: Vec::new(),
                at_ms: 0,
            },
        ];
        assert!(seen_in(&messages, "https://example.com/guide"));
        assert!(seen_in(&messages, "https://www.wanplus.cn/lol/aram"));
        assert!(!seen_in(&messages, "https://evil.example/?q=secret"));
        assert!(!seen_in(&messages, "https://example.com/other"));
    }
}
