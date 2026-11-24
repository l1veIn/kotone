//! MCP Apps 的资源和素材（ADR 039 D5、D6；MCP 方案 4.5、第 3 和第 5 期）。
//!
//! **清单和资源怎么对应**（一种登记方式，原则 15：页面只在清单里登记一次）：
//! - 清单 `pages` 里的页面 `ui://kotone/<id>/<页面>` 可以由插件代码做成 MCP App：[`crate::Plugin::resource`]
//!   给出这个地址的 HTML（`text/html;profile=mcp-app`，[`APP_MIME`]）。Kotone（回环代理里的沙盒 iframe）和
//!   外部宿主（`kotone.exe mcp` 的 `resources/read`）显示的是同一份。
//! - 没做成 App 的页面（`resource` 给 `None`）照旧是宿主里的 Svelte 页面：不列进 `resources/list`，
//!   工具的 `_meta.ui.resourceUri` 对外（MCP）也不带它，免得外部宿主去读一个读不到的资源。
//! - 覆盖层（`overlays`）不是资源：本体的覆盖层显示它们（ADR 038 D5），宿主不会拿覆盖层的地址来问。
//! - 资源跟着页面的数据开关走（`needs`）：插件关着或要的开关关着时，宿主不列、不给读。
//! - 页面要连的外部地址写在 `_meta.ui.csp`（照 MCP Apps 规范 2026-01-26）：宿主从 `default-src 'none'`
//!   开始，只放行这里声明的；没声明的一律挡。[`crate::Manifest::check_resource`] 检查写法。
//!
//! **素材**（[`crate::Plugin::asset`]）：页面在 Kotone 里经回环代理的 `/assets/<插件 id>/<路径>` 取图片这类
//! 文件；路径由宿主先过 [`valid_asset_path`]。图片不进安装包（ADR 039 D6），从哪取、怎么缓存由插件自己管。

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// MCP App 的 HTML 资源类型（MCP Apps 规范：`text/html;profile=mcp-app`）。
pub const APP_MIME: &str = "text/html;profile=mcp-app";

/// 页面要连的外部地址（MCP Apps `McpUiResourceCsp`）。每一项是一个源（`https://raw.communitydragon.org`）
/// 或主机（`cdn.example.com`、`https://*.example.com`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Csp {
    /// fetch / XHR / WebSocket（CSP `connect-src`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect_domains: Vec<String>,
    /// 图片、脚本、样式、字体、音视频（`img-src`、`script-src`、`style-src`、`font-src`、`media-src`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_domains: Vec<String>,
    /// 嵌套的 iframe（`frame-src`；不写 = 不许）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_domains: Vec<String>,
    /// `<base>`（`base-uri`；不写 = 只能是自己）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_uri_domains: Vec<String>,
}

impl Csp {
    pub fn is_empty(&self) -> bool {
        self.all().next().is_none()
    }

    /// 全部声明（检查写法用）。
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.connect_domains
            .iter()
            .chain(&self.resource_domains)
            .chain(&self.frame_domains)
            .chain(&self.base_uri_domains)
    }
}

/// 资源的 `_meta.ui`（MCP Apps `McpUiResourceMeta`；`permissions`、`domain` 这次不支持，宿主不给）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUi {
    #[serde(default, skip_serializing_if = "Csp::is_empty")]
    pub csp: Csp,
    /// 要不要宿主给边框和底色（不写由宿主定）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefers_border: Option<bool>,
}

/// 一个 MCP App 资源：一个页面的完整 HTML。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// `ui://kotone/<插件 id>/<页面>`，清单 `pages` 里的地址。
    pub uri: String,
    /// [`APP_MIME`]。
    pub mime_type: String,
    /// 完整的 HTML5 文档（脚本、样式都内联，例如 Vite + `vite-plugin-singlefile` 打出来的单个文件）。
    pub text: String,
    pub ui: ResourceUi,
}

impl Resource {
    /// 一个 MCP App 页面（[`APP_MIME`]，没有 CSP 声明 = 不连任何外部地址）。
    pub fn app(uri: &str, html: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            mime_type: APP_MIME.into(),
            text: html.into(),
            ui: ResourceUi::default(),
        }
    }

    pub fn csp(mut self, csp: Csp) -> Self {
        self.ui.csp = csp;
        self
    }

    pub fn prefers_border(mut self, border: bool) -> Self {
        self.ui.prefers_border = Some(border);
        self
    }

    fn meta(&self) -> Option<Value> {
        let ui = serde_json::to_value(&self.ui).ok()?;
        (ui.as_object().is_some_and(|o| !o.is_empty())).then(|| json!({ "ui": ui }))
    }

    /// `resources/list` 的一项（MCP `Resource`）：`name` 是页面标题。
    pub fn listing(&self, name: &str) -> Value {
        let mut out = Map::new();
        out.insert("uri".into(), json!(self.uri));
        out.insert("name".into(), json!(name));
        out.insert("mimeType".into(), json!(self.mime_type));
        if let Some(meta) = self.meta() {
            out.insert("_meta".into(), meta);
        }
        Value::Object(out)
    }

    /// `resources/read` 结果里 `contents` 的一项（MCP `TextResourceContents` + `_meta.ui`）。
    pub fn content(&self) -> Value {
        let mut out = Map::new();
        out.insert("uri".into(), json!(self.uri));
        out.insert("mimeType".into(), json!(self.mime_type));
        out.insert("text".into(), json!(self.text));
        if let Some(meta) = self.meta() {
            out.insert("_meta".into(), meta);
        }
        Value::Object(out)
    }
}

/// 一个素材文件（[`crate::Plugin::asset`] 给的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// `image/png`、`image/webp`、`image/svg+xml`……
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// 素材路径的写法：1–256 个字符，用 `/` 分段，每段非空、不是 `.` / `..`，只有字母、数字和 `._-`。
/// 宿主（回环代理）先检查，插件拿到的都是这样的路径。
pub fn valid_asset_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// CSP 里的一个外部地址（[`Csp`] 的一项）规范成源；写法不对（`*`、引号、分号、空白、`@`、别的协议……）给 `None`。
///
/// 改写自 goose 的 `crates/goose/src/acp/mcp_app_proxy.rs`（`normalize_csp_source`，Apache-2.0，
/// 见 `THIRD_PARTY_NOTICES.md`）：`https://cdn.example.com/x.js` → `https://cdn.example.com`；
/// 主机名要有点（`localhost` 和 IP 除外），子域通配只能写在最前面（`*.example.com`）。
pub fn normalize_csp_source(source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty()
        || source
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, ';' | ',' | '"' | '\''))
    {
        return None;
    }
    if let Some((scheme, rest)) = source.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https" | "ws" | "wss") {
            return None;
        }
        let authority = rest.split(['/', '?', '#']).next()?;
        return valid_host_source(authority)
            .then(|| format!("{scheme}://{}", authority.to_ascii_lowercase()));
    }
    valid_host_source(source).then(|| source.to_ascii_lowercase())
}

fn valid_host_source(source: &str) -> bool {
    if source.is_empty() || source == "*" || source.contains('@') {
        return false;
    }
    let (host, port) = match source.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        _ => (source, None),
    };
    if port.is_some_and(|p| p.parse::<u16>().is_err()) {
        return false;
    }
    let host = host.strip_prefix("*.").unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_serialize_like_mcp_apps() {
        let r = Resource::app("ui://kotone/lol/tier", "<!doctype html><html></html>")
            .csp(Csp {
                resource_domains: vec!["https://raw.communitydragon.org".into()],
                ..Csp::default()
            })
            .prefers_border(false);
        assert_eq!(
            r.listing("强度榜"),
            json!({
                "uri": "ui://kotone/lol/tier",
                "name": "强度榜",
                "mimeType": "text/html;profile=mcp-app",
                "_meta": {"ui": {"csp": {"resourceDomains": ["https://raw.communitydragon.org"]}, "prefersBorder": false}},
            })
        );
        let content = r.content();
        assert_eq!(content["text"], "<!doctype html><html></html>");
        assert_eq!(content["mimeType"], APP_MIME);
        assert_eq!(
            content["_meta"]["ui"]["csp"]["resourceDomains"][0],
            "https://raw.communitydragon.org"
        );
        // 什么都没声明：不带 _meta
        let plain = Resource::app("ui://kotone/x/y", "<html></html>");
        assert!(plain.content().get("_meta").is_none());
        assert!(plain.listing("y").get("_meta").is_none());
        // 读回来一样（宿主转发 `_meta.ui` 时）
        let back: ResourceUi = serde_json::from_value(content["_meta"]["ui"].clone()).unwrap();
        assert_eq!(back, r.ui);
    }

    #[test]
    fn csp_sources_are_normalized_or_dropped() {
        for (input, expected) in [
            (
                "https://raw.communitydragon.org/latest/x.png",
                Some("https://raw.communitydragon.org"),
            ),
            (
                "wss://api.example.com/socket",
                Some("wss://api.example.com"),
            ),
            ("https://*.cloudflare.com", Some("https://*.cloudflare.com")),
            ("cdn.example.com", Some("cdn.example.com")),
            ("localhost:3000", Some("localhost:3000")),
            ("HTTPS://CDN.Example.com", Some("https://cdn.example.com")),
            ("*", None),
            ("'unsafe-inline'", None),
            ("javascript:alert(1)", None),
            ("https://example.com;", None),
            ("https://user@example.com", None),
            ("https://a.com https://b.com", None),
            ("data:", None),
            ("https://*", None),
            ("https://example.com:99999", None),
            ("intranet", None),
        ] {
            assert_eq!(normalize_csp_source(input).as_deref(), expected, "{input}");
        }
    }

    #[test]
    fn asset_paths() {
        for ok in [
            "lol-game-data/assets/v1/champion-icons/1.png",
            "a",
            "ASSETS/Items/Icons2D/1001_Class_T1_BootsofSpeed.png",
        ] {
            assert!(valid_asset_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/abs",
            "a//b",
            "../secret",
            "a/../b",
            "a/./b",
            "a\\b",
            "a b",
            "a?x=1",
            "%2e%2e/x",
            "c:/windows",
            &"a".repeat(257),
        ] {
            assert!(!valid_asset_path(bad), "{bad}");
        }
    }
}
