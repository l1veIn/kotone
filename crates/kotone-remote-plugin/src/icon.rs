//! 第三方插件的头像（ADR 049 D7 第 2 条）：服务器在 `serverInfo.icons` 里给的是 `data:image/…` 就照旧直接用；
//! 只给了 `https://` 的，**加的时候下载一次**（[`kotone_mcp::fetch_icon`]：只认 https、≤200 KB、不跳到别的主机、
//! 只收 PNG / JPEG / WebP / SVG）存到插件自己的目录（`<KOTONE_HOME>/plugins/<id>/icon.<扩展名>`，记下是哪个地址的
//! 在 `icon.json`），以后只读本机文件，不再联网取；界面拿到的是 `data:` 地址（主窗口的 CSP 本来就放行 `data:`）。
//! 都没有、下不来就是首字块（界面 `PluginLogo`）。

use std::path::Path;

use base64::Engine as _;
use kotone_mcp::{sniff_icon, Icon, ICON_MAX_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 记下头像是从哪个地址下的（地址变了就重新下）。
pub const ICON_RECORD: &str = "icon.json";

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    src: String,
    file: String,
}

/// 头像文件的扩展名。
fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "svg",
    }
}

/// 声明的类型认不认（没声明的先下下来看内容）。
fn declared_ok(icon: &Value) -> bool {
    match icon.get("mimeType").and_then(Value::as_str) {
        None => true,
        Some(mime) => matches!(
            mime.trim().to_ascii_lowercase().as_str(),
            "image/png" | "image/jpeg" | "image/jpg" | "image/webp" | "image/svg+xml"
        ),
    }
}

/// 要下载哪一张：服务器给了 `data:image/` 的就不用下（None）；否则第一张 `https://` 的（亮色主题的、没写主题的优先；
/// 声明了类型的只要 PNG / JPEG / WebP / SVG）。
pub fn wanted(icons: &[Value]) -> Option<String> {
    fn src(icon: &Value) -> Option<&str> {
        icon.get("src").and_then(Value::as_str).map(str::trim)
    }
    if icons
        .iter()
        .any(|i| src(i).is_some_and(|s| s.starts_with("data:image/")))
    {
        return None;
    }
    let candidates: Vec<&Value> = icons
        .iter()
        .filter(|i| src(i).is_some_and(|s| s.starts_with("https://") && s.len() <= 2048))
        .filter(|i| declared_ok(i))
        .collect();
    let dark = |i: &Value| i.get("theme").and_then(Value::as_str) == Some("dark");
    candidates
        .iter()
        .find(|i| !dark(i))
        .or_else(|| candidates.first())
        .and_then(|i| src(i))
        .map(str::to_string)
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 本机存着的这个地址的头像（`data:` 地址）；没存过、地址变了、文件坏了是 None。
pub fn load(dir: &Path, src: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(ICON_RECORD)).ok()?;
    let record: Record = serde_json::from_str(&text).ok()?;
    // 只认自己写的文件名（`icon.<扩展名>`），不顺着记录去读别处的文件
    if record.src != src || !record.file.starts_with("icon.") || record.file.contains(['/', '\\']) {
        return None;
    }
    let bytes = std::fs::read(dir.join(&record.file)).ok()?;
    if bytes.len() > ICON_MAX_BYTES {
        return None;
    }
    let mime = sniff_icon(&bytes)?;
    Some(data_uri(mime, &bytes))
}

/// 存下刚下载的头像（换了类型时删掉旧的那个文件），回给界面用的 `data:` 地址。
pub fn save(dir: &Path, src: &str, icon: &Icon) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let file = format!("icon.{}", extension(icon.mime));
    std::fs::write(dir.join(&file), &icon.bytes).map_err(|e| e.to_string())?;
    for other in ["png", "jpg", "webp", "svg"]
        .map(|ext| format!("icon.{ext}"))
        .into_iter()
        .filter(|name| *name != file)
    {
        let _ = std::fs::remove_file(dir.join(other));
    }
    let record = serde_json::to_string(&Record {
        src: src.to_string(),
        file,
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(dir.join(ICON_RECORD), record).map_err(|e| e.to_string())?;
    Ok(data_uri(icon.mime, &icon.bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn only_https_icons_are_downloaded_and_data_icons_need_none() {
        assert_eq!(wanted(&[]), None);
        assert_eq!(
            wanted(&[
                json!({"src": "https://x.example.com/a.png"}),
                json!({"src": "data:image/png;base64,AAAA"})
            ]),
            None,
            "已经有 data: 的就不用下"
        );
        assert_eq!(
            wanted(&[
                json!({"src": "http://x.example.com/plain.png"}),
                json!({"src": "https://x.example.com/a.gif", "mimeType": "image/gif"}),
                json!({"src": "https://x.example.com/dark.png", "theme": "dark"}),
                json!({"src": "https://x.example.com/light.png", "theme": "light"}),
            ]),
            Some("https://x.example.com/light.png".into())
        );
        assert_eq!(
            wanted(&[
                json!({"src": "https://x.example.com/dark.svg", "theme": "dark", "mimeType": "image/svg+xml"})
            ]),
            Some("https://x.example.com/dark.svg".into()),
            "只有暗色的也用"
        );
        assert_eq!(wanted(&[json!({"src": "javascript:alert(1)"})]), None);
    }

    #[test]
    fn saved_icons_are_read_back_as_data_uris() {
        let dir = tempfile::tempdir().unwrap();
        let src = "https://x.example.com/a.png";
        assert_eq!(load(dir.path(), src), None);
        let data = save(
            dir.path(),
            src,
            &Icon {
                bytes: PNG.to_vec(),
                mime: "image/png",
            },
        )
        .unwrap();
        assert!(data.starts_with("data:image/png;base64,"));
        assert!(dir.path().join("icon.png").is_file());
        assert_eq!(load(dir.path(), src), Some(data));
        assert_eq!(
            load(dir.path(), "https://x.example.com/other.png"),
            None,
            "地址变了要重新下"
        );

        // 换成 SVG：旧的 PNG 删掉
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        save(
            dir.path(),
            src,
            &Icon {
                bytes: svg.to_vec(),
                mime: "image/svg+xml",
            },
        )
        .unwrap();
        assert!(!dir.path().join("icon.png").exists());
        assert!(load(dir.path(), src)
            .unwrap()
            .starts_with("data:image/svg+xml;base64,"));

        // 文件被换成别的东西（不是图片）、记录指向别处：都不认
        std::fs::write(dir.path().join("icon.svg"), b"<html></html>").unwrap();
        assert_eq!(load(dir.path(), src), None);
        std::fs::write(
            dir.path().join(ICON_RECORD),
            json!({"src": src, "file": "../secret.png"}).to_string(),
        )
        .unwrap();
        assert_eq!(load(dir.path(), src), None);
    }
}
