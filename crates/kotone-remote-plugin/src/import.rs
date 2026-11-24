//! 加服务器的两条路（MCP 方案 4.6）：**粘贴配置**，或者**从 Claude Desktop、VS Code 导入**。两条路都先变成
//! 一串候选（名字 + 原样的配置），界面给玩家看信任提示，玩家同意了才加进 `mcp.json`。
//!
//! 认的写法（都是事实标准，照抄不发明）：
//! - Claude Desktop（`%APPDATA%\Claude\claude_desktop_config.json`；Microsoft Store 版在
//!   `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude\`）、Claude Code、Cursor：`{"mcpServers": {名字: 服务器}}`；
//! - VS Code（`%APPDATA%\Code\User\mcp.json`，Insiders 在 `Code - Insiders`）：`{"servers": {名字: 服务器}, "inputs": […]}`，
//!   允许注释和多余的逗号；旧版写在 `settings.json` 的 `"mcp": {"servers": …}` 里；
//! - 只贴了一个服务器（`{"command": …}`）、或者 `"名字": {…}` 这一段：也认。
//!
//! Key：配置里明文写着的 Key（环境变量、请求头的名字像 key / token / secret / password / auth……）在加进来时
//! 搬进 Windows 凭据库，文件里换成 `${input:名字}`（[`split_secrets`]）。VS Code 的 `${input:…}` 本来就是这个写法，
//! 但它的值存在 VS Code 自己那里，Kotone 拿不到：要玩家在插件卡片里再填一次。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::{placeholder, placeholders, valid_key_name};

/// 能从哪里导入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ClaudeDesktop,
    VsCode,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::ClaudeDesktop, Source::VsCode];

    pub fn id(self) -> &'static str {
        match self {
            Source::ClaudeDesktop => "claude-desktop",
            Source::VsCode => "vscode",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Source::ClaudeDesktop => "Claude Desktop",
            Source::VsCode => "VS Code",
        }
    }

    /// 可能放配置文件的地方（`appdata` = `%APPDATA%`，`local` = `%LOCALAPPDATA%`），按先后。
    pub fn locations(self, appdata: Option<&Path>, local: Option<&Path>) -> Vec<PathBuf> {
        let mut out = Vec::new();
        match self {
            Source::ClaudeDesktop => {
                if let Some(appdata) = appdata {
                    out.push(appdata.join("Claude").join("claude_desktop_config.json"));
                }
                // Microsoft Store 版（MSIX）的 %APPDATA% 被重定向到包目录里
                if let Some(local) = local {
                    let packages = local.join("Packages");
                    if let Ok(entries) = std::fs::read_dir(&packages) {
                        let mut dirs: Vec<PathBuf> = entries
                            .filter_map(Result::ok)
                            .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
                            .map(|e| {
                                e.path()
                                    .join("LocalCache")
                                    .join("Roaming")
                                    .join("Claude")
                                    .join("claude_desktop_config.json")
                            })
                            .collect();
                        dirs.sort();
                        out.extend(dirs);
                    }
                }
            }
            Source::VsCode => {
                if let Some(appdata) = appdata {
                    for app in ["Code", "Code - Insiders"] {
                        let user = appdata.join(app).join("User");
                        out.push(user.join("mcp.json"));
                        out.push(user.join("settings.json"));
                    }
                }
            }
        }
        out
    }

    /// 这台电脑上找到的配置文件和里面的服务器（`settings.json` 里没写 MCP 的不算）。
    pub fn scan(self, appdata: Option<&Path>, local: Option<&Path>) -> Vec<Found> {
        self.locations(appdata, local)
            .into_iter()
            .filter(|p| p.is_file())
            .filter_map(|path| {
                let servers = std::fs::read_to_string(&path)
                    .map_err(|e| format!("读不了：{e}"))
                    .and_then(|text| parse_text(&text));
                let settings = path.file_name().is_some_and(|n| n == "settings.json");
                match servers {
                    // VS Code 的 settings.json 里多半没有 MCP：没有就不列
                    Err(_) if settings => None,
                    servers => Some(Found {
                        source: self,
                        path,
                        servers,
                    }),
                }
            })
            .collect()
    }
}

/// 一个找到的配置文件。
#[derive(Debug, Clone)]
pub struct Found {
    pub source: Source,
    pub path: PathBuf,
    pub servers: Result<Vec<Candidate>, String>,
}

/// 一个候选的服务器：名字（原来的写法，不一定能当 id）和原样的配置。
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub entry: Value,
    /// 加进来以后可能不像玩家想的那样工作的地方（Kotone 不认的 VS Code 变量、`envFile`……）。
    pub warnings: Vec<String>,
}

fn looks_like_server(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.contains_key("command") || o.contains_key("url"))
}

/// 认出一段配置里的服务器（见模块说明里的几种写法）。只贴了一个服务器、没有名字时名字是空的（由
/// [`suggest_name`] 猜一个）。
pub fn parse_text(text: &str) -> Result<Vec<Candidate>, String> {
    let text = text.trim().trim_start_matches('\u{FEFF}');
    let cleaned = strip_jsonc(text);
    let value: Value = match serde_json::from_str(&cleaned) {
        Ok(v) => v,
        // 只贴了 `"名字": {…}` 这一段：补上外面的括号再试
        Err(e) if !cleaned.trim_start().starts_with('{') => {
            serde_json::from_str(&format!("{{{}}}", cleaned.trim().trim_end_matches(',')))
                .map_err(|_| format!("不是合法的 JSON：{e}"))?
        }
        Err(e) => return Err(format!("不是合法的 JSON：{e}")),
    };
    let root = value
        .as_object()
        .ok_or("最外层要是一个对象（{\"mcpServers\": {…}}）")?;
    let servers: &Map<String, Value> = if let Some(m) = root.get("mcpServers") {
        m.as_object().ok_or("mcpServers 要是一个对象")?
    } else if let Some(m) = root.get("servers") {
        m.as_object().ok_or("servers 要是一个对象")?
    } else if let Some(m) = root.get("mcp").and_then(|m| m.get("servers")) {
        m.as_object().ok_or("mcp.servers 要是一个对象")?
    } else if looks_like_server(&value) {
        return Ok(vec![candidate(String::new(), &value)]);
    } else if !root.is_empty() && root.values().all(looks_like_server) {
        root
    } else {
        return Err(
            "没认出 MCP 服务器的配置：要有 mcpServers（Claude Desktop、Claude Code、Cursor 的写法）或 servers（VS Code 的写法）"
                .into(),
        );
    };
    Ok(servers
        .iter()
        .map(|(name, entry)| candidate(name.clone(), entry))
        .collect())
}

fn candidate(name: String, entry: &Value) -> Candidate {
    let mut warnings = Vec::new();
    if let Some(object) = entry.as_object() {
        if object.contains_key("envFile") {
            warnings.push(
                "这个服务器写了 envFile：Kotone 不读它，要用的环境变量请写进 env".to_string(),
            );
        }
        let text = entry.to_string();
        let foreign: Vec<&str> = [
            "${workspaceFolder",
            "${env:",
            "${userHome",
            "${config:",
            "${command:",
        ]
        .into_iter()
        .filter(|v| text.contains(v))
        .collect();
        if !foreign.is_empty() {
            warnings.push(format!(
                "里面有 VS Code 的变量（{}…}}），Kotone 不认识，会原样交给程序",
                foreign.join("…}、")
            ));
        }
    }
    Candidate {
        name,
        entry: entry.clone(),
        warnings,
    }
}

/// 一个服务器没有名字时猜一个：`npx -y @modelcontextprotocol/server-everything` → `everything`，
/// `https://mcp.example.com/mcp` → `example`。
pub fn suggest_name(entry: &Value) -> String {
    let Some(spec) = crate::ServerSpec::parse(entry).ok() else {
        return "mcp-server".into();
    };
    let raw = match &spec.launch {
        crate::Launch::Stdio { command, .. } => spec.package().unwrap_or_else(|| {
            Path::new(command)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        }),
        crate::Launch::Http { url, .. } => {
            let host = url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split(['/', ':', '?'])
                .next()
                .unwrap_or("");
            // 本机的、只写了 IP 的：名字里看不出是什么，叫 local
            if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok() {
                return "local".into();
            }
            host.split('.')
                .find(|label| !matches!(*label, "www" | "mcp" | "api" | "mcp-api"))
                .unwrap_or(host)
                .to_string()
        }
    };
    // 包名：去掉作用域、版本、常见的前后缀
    let base = raw.rsplit('/').next().unwrap_or(&raw);
    let base = base.split('@').find(|s| !s.is_empty()).unwrap_or(base);
    let base = base
        .trim_start_matches("server-")
        .trim_start_matches("mcp-server-")
        .trim_start_matches("mcp-")
        .trim_end_matches("-mcp-server")
        .trim_end_matches("-mcp")
        .trim_end_matches("-server");
    if base.is_empty() {
        "mcp-server".into()
    } else {
        base.to_string()
    }
}

/// 名字 → 能当插件 id 的写法（小写、`_` 和空格换成 `-`、去掉别的字符、最多 32 个字），和已有的撞了就加 `-2`、`-3`。
/// `taken`：这个 id 已经被用了（别的服务器、内置插件、本体的工具前缀、保留名）。
pub fn suggest_id(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut id = String::new();
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            id.push(c);
        } else if (c == '-' || c == '_' || c == ' ' || c == '.') && !id.ends_with('-') {
            id.push('-');
        }
    }
    let mut id: String = id.trim_matches('-').chars().take(28).collect();
    id = id.trim_end_matches('-').to_string();
    if id.is_empty() {
        id = "mcp-server".into();
    }
    let usable = |candidate: &str| crate::config::check_id(candidate).is_ok() && !taken(candidate);
    if usable(&id) {
        return id;
    }
    (2..)
        .map(|n| format!("{id}-{n}"))
        .find(|candidate| usable(candidate))
        .expect("总能找到一个没用过的")
}

/// 名字看起来是 Key（环境变量名、请求头名，不分大小写）。
pub fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "AUTH",
        "CREDENTIAL",
        "COOKIE",
        "BEARER",
    ]
    .iter()
    .any(|word| upper.contains(word))
        || upper == "PAT"
        || upper.ends_with("_PAT")
}

/// 参数、网址查询串里的名字看起来是 Key：[`secret_name`]，但不是指向 Key 的东西（`--key-file`、`--token-path`、
/// `--auth-type`、`--api-key-env` 这类的值是路径、类型、变量名，不是 Key 本身）。
fn secret_flag(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    secret_name(name)
        && ![
            "FILE", "PATH", "DIR", "URL", "URI", "TYPE", "MODE", "HOST", "PORT", "ENV", "NAME",
        ]
        .iter()
        .any(|word| upper.contains(word))
}

/// 一个参数的值像不像 Key：8 个字以上，没有空白，不是路径、网址、包名、选项。
fn secret_value(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    chars.len() >= 8
        && !value.chars().any(char::is_whitespace)
        && !value.starts_with(['-', '/', '.', '~', '@', '$'])
        && !value.contains('\\')
        && !value.contains("://")
        && chars.get(1) != Some(&':')
        && placeholders(value).is_empty()
}

/// 拿出来的 Key 叫什么（`${input:名字}`）：原来的名字里不合规矩的字换成 `_`，撞了加 `_2`。
fn key_name(name: &str, used: &mut Vec<String>) -> String {
    let base: String = name
        .trim_start_matches('-')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let base = if valid_key_name(&base) {
        base
    } else {
        "KEY".into()
    };
    let key = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}_{n}")))
        .find(|k| !used.contains(k))
        .expect("总能找到一个没用过的");
    used.push(key.clone());
    key
}

/// 把配置里明文写着的 Key 拿出来：返回换成 `${input:名字}` 的配置和拿出来的 `(名字, Key)`。名字撞了加 `_2`。看这几处：
/// - `env`、`headers` 里名字像 Key、值不是空的也不已经是 `${input:…}` 的项；
/// - `url` 的查询串里名字像 Key 的参数（`?token=…`、`?api_key=…`，原样搬走，填回去还是原来的写法）；
/// - `args` 里的 `--api-key=…`、`TOKEN=…`，和 `--token <值>`（后面那个值像 Key 才算：8 个字以上、不是路径、
///   网址、包名、选项）。猜不准的不搬：留在 `mcp.json` 里照样能用，只是没进安全存储。
pub fn split_secrets(entry: &Value) -> (Value, Vec<(String, String)>) {
    let mut entry = entry.clone();
    let mut secrets: Vec<(String, String)> = Vec::new();
    let mut used: Vec<String> = placeholders(&entry.to_string());
    for field in ["env", "headers"] {
        let Some(map) = entry.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for (name, value) in map.iter_mut() {
            let Some(text) = value.as_str() else { continue };
            if text.trim().is_empty() || !placeholders(text).is_empty() || !secret_name(name) {
                continue;
            }
            let key = key_name(name, &mut used);
            secrets.push((key.clone(), text.to_string()));
            *value = Value::String(placeholder(&key));
        }
    }
    // 网址的查询串：`https://x/mcp?token=…&region=eu` → `…?token=${input:token}&region=eu`
    if let Some(Value::String(url)) = entry.get_mut("url") {
        if let Some((base, rest)) = url.clone().split_once('?') {
            let (query, fragment) = match rest.split_once('#') {
                Some((q, f)) => (q, Some(f)),
                None => (rest, None),
            };
            let pairs: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((name, value))
                        if secret_flag(name)
                            && value.chars().count() >= 4
                            && placeholders(value).is_empty() =>
                    {
                        let key = key_name(name, &mut used);
                        secrets.push((key.clone(), value.to_string()));
                        format!("{name}={}", placeholder(&key))
                    }
                    _ => pair.to_string(),
                })
                .collect();
            *url = format!(
                "{base}?{}{}",
                pairs.join("&"),
                fragment.map(|f| format!("#{f}")).unwrap_or_default()
            );
        }
    }
    // 参数：`--api-key=…`、`API_TOKEN=…`、`--token <值>`
    if let Some(args) = entry.get_mut("args").and_then(Value::as_array_mut) {
        let mut i = 0;
        while i < args.len() {
            let Some(arg) = args[i].as_str().map(str::to_string) else {
                i += 1;
                continue;
            };
            if let Some((name, value)) = arg.split_once('=') {
                let env_style = !name.starts_with('-')
                    && !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if (name.starts_with('-') || env_style) && secret_flag(name) && secret_value(value)
                {
                    let key = key_name(name, &mut used);
                    secrets.push((key.clone(), value.to_string()));
                    args[i] = Value::String(format!("{name}={}", placeholder(&key)));
                }
            } else if arg.starts_with('-') && secret_flag(&arg) {
                if let Some(value) = args
                    .get(i + 1)
                    .and_then(Value::as_str)
                    .filter(|v| secret_value(v))
                    .map(str::to_string)
                {
                    let key = key_name(&arg, &mut used);
                    secrets.push((key.clone(), value));
                    args[i + 1] = Value::String(placeholder(&key));
                    i += 1;
                }
            }
            i += 1;
        }
    }
    (entry, secrets)
}

/// 去掉 JSONC 的注释（`//`、`/* */`）和对象、数组最后多余的逗号（VS Code 的配置允许这些）。字符串里的不动。
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            ',' => {
                // 后面（跳过空白和注释）紧跟 } 或 ] 的逗号去掉
                let mut j = i + 1;
                loop {
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'/') {
                        while j < chars.len() && chars[j] != '\n' {
                            j += 1;
                        }
                    } else if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'*') {
                        j += 2;
                        while j < chars.len()
                            && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                        {
                            j += 1;
                        }
                        j += 2;
                    } else {
                        break;
                    }
                }
                if !matches!(chars.get(j), Some('}') | Some(']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let text = r#"{
          // 注释
          "servers": { /* 块注释 */
            "a": {"url": "https://x.example.com/mcp", "note": "// 不是注释, /* 也不是 */",},
          },
          "inputs": [1, 2, ],
        }"#;
        let value: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value["servers"]["a"]["note"], "// 不是注释, /* 也不是 */");
        assert_eq!(value["inputs"], json!([1, 2]));
        assert_eq!(strip_jsonc(r#"{"a": "x\"//y"}"#), r#"{"a": "x\"//y"}"#);
    }

    #[test]
    fn the_shapes_people_paste() {
        let claude = r#"{"mcpServers": {"filesystem": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\me"]}}, "preferences": {}}"#;
        let found = parse_text(claude).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "filesystem");
        assert_eq!(found[0].entry["args"][2], "C:\\Users\\me");

        let vscode = r#"{
          "inputs": [{"type": "promptString", "id": "token", "password": true}],
          "servers": {
            "github": {"type": "http", "url": "https://api.githubcopilot.com/mcp/", "headers": {"Authorization": "Bearer ${input:token}"}},
            "local": {"type": "stdio", "command": "uvx", "args": ["mcp-server-time"], "envFile": "${workspaceFolder}/.env"},
          }
        }"#;
        let found = parse_text(vscode).unwrap();
        assert_eq!(
            found.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["github", "local"]
        );
        let local = &found[1];
        assert!(
            local.warnings.iter().any(|w| w.contains("envFile")),
            "{:?}",
            local.warnings
        );
        assert!(
            local
                .warnings
                .iter()
                .any(|w| w.contains("${workspaceFolder")),
            "{:?}",
            local.warnings
        );

        let settings = r#"{"editor.fontSize": 14, "mcp": {"servers": {"x": {"command": "x"}}}}"#;
        assert_eq!(parse_text(settings).unwrap()[0].name, "x");

        let single =
            r#"{"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"]}"#;
        let found = parse_text(single).unwrap();
        assert_eq!(found[0].name, "");
        assert_eq!(suggest_name(&found[0].entry), "everything");

        let fragment = r#""everything": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"]},"#;
        assert_eq!(parse_text(fragment).unwrap()[0].name, "everything");

        let bare_map = r#"{"a": {"url": "http://127.0.0.1:3001/mcp"}, "b": {"command": "b.exe"}}"#;
        assert_eq!(parse_text(bare_map).unwrap().len(), 2);

        for bad in ["[1]", "{\"x\": 1}", "not json", "{\"mcpServers\": []}"] {
            assert!(parse_text(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_become_ids() {
        let none = |_: &str| false;
        assert_eq!(suggest_id("Everything", &none), "everything");
        assert_eq!(suggest_id("my_server v2", &none), "my-server-v2");
        assert_eq!(suggest_id("__", &none), "mcp-server");
        assert_eq!(suggest_id("测试", &none), "mcp-server");
        assert_eq!(suggest_id("kotone", &none), "kotone-2", "保留名");
        let taken = |id: &str| id == "lol" || id == "lol-2";
        assert_eq!(suggest_id("LoL", &taken), "lol-3");
        let long = suggest_id(&"a".repeat(60), &none);
        assert!(long.len() <= 32 && crate::config::check_id(&long).is_ok());

        assert_eq!(
            suggest_name(&json!({"url": "https://mcp-api.op.gg/mcp"})),
            "op"
        );
        assert_eq!(
            suggest_name(&json!({"url": "http://127.0.0.1:3001/mcp"})),
            "local"
        );
        assert_eq!(
            suggest_name(&json!({"url": "http://localhost:3001/mcp"})),
            "local"
        );
        assert_eq!(
            suggest_name(
                &json!({"command": "uvx", "args": ["mcp-server-time", "--local-timezone=Asia/Shanghai"]})
            ),
            "time"
        );
        assert_eq!(
            suggest_name(&json!({"command": "C:\\tools\\weather-mcp.exe"})),
            "weather"
        );
    }

    /// 明文写在配置里的 Key 拿出来进凭据库，文件里只留 `${input:名字}`；不像 Key 的、已经是占位的不动。
    #[test]
    fn plain_keys_move_out_of_the_file() {
        let entry = json!({
            "command": "npx",
            "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_secret", "LOG_LEVEL": "debug",
                    "API_KEY": "${input:API_KEY}", "EMPTY_KEY": ""},
        });
        let (clean, secrets) = split_secrets(&entry);
        assert_eq!(
            secrets,
            vec![(
                "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
                "ghp_secret".to_string()
            )]
        );
        assert_eq!(
            clean["env"]["GITHUB_PERSONAL_ACCESS_TOKEN"],
            "${input:GITHUB_PERSONAL_ACCESS_TOKEN}"
        );
        assert_eq!(clean["env"]["LOG_LEVEL"], "debug");
        assert_eq!(clean["env"]["API_KEY"], "${input:API_KEY}");
        assert!(!clean.to_string().contains("ghp_secret"));

        let http = json!({"url": "https://x.example.com/mcp", "headers": {"Authorization": "Bearer abc", "X-Client": "kotone"}});
        let (clean, secrets) = split_secrets(&http);
        assert_eq!(
            secrets,
            vec![("Authorization".to_string(), "Bearer abc".to_string())]
        );
        assert_eq!(clean["headers"]["Authorization"], "${input:Authorization}");
        assert_eq!(clean["headers"]["X-Client"], "kotone");
    }

    /// 网址查询串和参数里明文写着的 Key 也搬走（审查后加的，2026-11-25）：连不上时的错误、信任提示、
    /// 卡片上的摘要里就看不到了；填回去和原来一字不差。路径、类型、变量名这类不是 Key 的不动。
    #[test]
    fn keys_in_urls_and_args_move_out_too() {
        let http = json!({"url": "https://mcp.example.com/mcp?token=abc123def456&region=eu#x"});
        let (clean, secrets) = split_secrets(&http);
        assert_eq!(
            clean["url"],
            "https://mcp.example.com/mcp?token=${input:token}&region=eu#x"
        );
        assert_eq!(
            secrets,
            vec![("token".to_string(), "abc123def456".to_string())]
        );
        let spec = crate::ServerSpec::parse(&clean).unwrap();
        assert!(!spec.summary().contains("abc123"));
        let kotone_mcp::Transport::Http { url, .. } = spec
            .resolve(&|name| {
                secrets
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            })
            .unwrap()
            .transport
        else {
            panic!()
        };
        assert_eq!(
            url,
            "https://mcp.example.com/mcp?token=abc123def456&region=eu#x"
        );

        let stdio = json!({"command": "npx", "args": [
            "-y", "@acme/mcp-server", "--api-key=sk-1234567890", "--token", "ghp_abcdefgh12345",
            "--key-file", "C:\\keys\\k.pem", "--auth-type", "bearer-token-mode", "-e", "API_TOKEN=xyz12345678",
            "--secret", "--verbose", "--token", "short"
        ]});
        let (clean, secrets) = split_secrets(&stdio);
        let args: Vec<&str> = clean["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "-y",
                "@acme/mcp-server",
                "--api-key=${input:api-key}",
                "--token",
                "${input:token}",
                "--key-file",
                "C:\\keys\\k.pem",
                "--auth-type",
                "bearer-token-mode",
                "-e",
                "API_TOKEN=${input:API_TOKEN}",
                "--secret",
                "--verbose",
                "--token",
                "short"
            ]
        );
        assert_eq!(
            secrets,
            vec![
                ("api-key".to_string(), "sk-1234567890".to_string()),
                ("token".to_string(), "ghp_abcdefgh12345".to_string()),
                ("API_TOKEN".to_string(), "xyz12345678".to_string()),
            ]
        );
        let spec = crate::ServerSpec::parse(&clean).unwrap();
        assert_eq!(spec.package().as_deref(), Some("@acme/mcp-server"));
        assert!(!spec.trust_facts().to_string().contains("sk-1234567890"));
    }

    #[test]
    fn where_the_configs_live() {
        let appdata = Path::new("C:\\Users\\me\\AppData\\Roaming");
        let claude = Source::ClaudeDesktop.locations(Some(appdata), None);
        assert_eq!(
            claude[0],
            appdata.join("Claude").join("claude_desktop_config.json")
        );
        let code = Source::VsCode.locations(Some(appdata), None);
        assert!(code.contains(&appdata.join("Code").join("User").join("mcp.json")));
        assert!(code.contains(
            &appdata
                .join("Code - Insiders")
                .join("User")
                .join("mcp.json")
        ));
        assert_eq!(Source::from_id("vscode"), Some(Source::VsCode));

        // 扫一个假的 %APPDATA%：Claude 有配置；VS Code 只有一个不含 MCP 的 settings.json（不列）
        let dir = tempfile::tempdir().unwrap();
        let claude_dir = dir.path().join("Claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("claude_desktop_config.json"),
            r#"{"mcpServers": {"time": {"command": "uvx", "args": ["mcp-server-time"]}}}"#,
        )
        .unwrap();
        let user = dir.path().join("Code").join("User");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("settings.json"), r#"{"editor.fontSize": 14}"#).unwrap();
        let found = Source::ClaudeDesktop.scan(Some(dir.path()), None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].servers.as_ref().unwrap()[0].name, "time");
        assert!(Source::VsCode.scan(Some(dir.path()), None).is_empty());
    }
}
