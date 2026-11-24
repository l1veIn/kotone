//! `KOTONE_HOME/mcp.json`：玩家加的第三方 MCP 服务器（MCP 方案 4.6）。
//!
//! **格式照事实标准**（原则 11：新的配置格式是单向门，不自己发明）：就是 Claude Desktop / Claude Code / Cursor 的
//! `mcpServers`，键是服务器的名字（在 Kotone 里就是插件 id），值是
//! `{"command", "args", "env", "cwd"}`（在你电脑上运行一个程序）或 `{"type": "http", "url", "headers"}`
//! （连一个网址，Streamable HTTP）。`type` 照 VS Code / Claude Code 的写法可写可不写。
//!
//! - **Key 不写进这个文件**：要 Key 的地方写成 `${input:名字}`（VS Code `mcp.json` 的写法），Key 存在 Windows
//!   凭据库（`<插件 id>:<名字>`，和内置插件一样），连接时才填进去。
//! - **旧版本也能读新文件**（原则 19）：认不出的字段原样留着（读的时候不管，写回去时不丢）；一个服务器写错了
//!   只影响它自己，别的照常。
//! - 开关、信任这些 Kotone 自己的事不写进这个文件，在 `config.json` 的 `plugins.<id>`（`enabled`、
//!   `settings.trusted`），这个文件拿去别的宿主也能用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// 文件名（在 `KOTONE_HOME` 下）。
pub const FILE_NAME: &str = "mcp.json";
/// 顶层的键（Claude Desktop 的写法）。
pub const SERVERS: &str = "mcpServers";

/// 不能当第三方插件 id 的名字：本体自己的地址和工具前缀（`ui://kotone/…`、`settings.*`、`tools.find`……）、
/// 凭据库里本体用的名字（`connection:<id>`）、已保存自动化的工具名前缀（`saved.<自动化 id>`，琴音的清单里
/// 现算出来的，宿主查不到）。内置插件的 id 和本体工具的前缀由宿主再查一遍（桌面端知道全部）。
pub const RESERVED_IDS: &[&str] = &[
    "kotone",
    "connection",
    "settings",
    "plugins",
    "plugin",
    "tools",
    "system",
    "ui",
    "agent",
    "app",
    "mcp",
    "saved",
];

/// 插件 id 的写法和保留名。
pub fn check_id(id: &str) -> Result<(), String> {
    if !kotone_plugin_api::valid_id(id) {
        return Err(format!(
            "名字“{id}”不能当插件 id：只能用小写字母、数字和连字符（-），1 到 32 个字，不以连字符开头或结尾"
        ));
    }
    if RESERVED_IDS.contains(&id) {
        return Err(format!("名字“{id}”是 Kotone 自己用的，换一个"));
    }
    Ok(())
}

/// 怎么启动 / 连接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// 在你电脑上运行一个程序，经标准输入输出说话。
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    /// 连一个网址（Streamable HTTP）。
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// 一个服务器的配置（`mcpServers` 里的一项，读出来的样子）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    pub launch: Launch,
}

/// 字符串、数字、布尔都当成字符串（有的配置把端口写成数字）；对象、数组不行。
fn scalar(value: &Value, what: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(format!("{what} 要是一个字符串")),
    }
}

fn string_map(value: Option<&Value>, what: &str) -> Result<BTreeMap<String, String>, String> {
    match value {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| Ok((k.clone(), scalar(v, &format!("{what} 里的 {k}"))?)))
            .collect(),
        Some(_) => Err(format!("{what} 要是一个对象（名字 → 值）")),
    }
}

impl ServerSpec {
    /// 读 `mcpServers` 里的一项。
    pub fn parse(entry: &Value) -> Result<Self, String> {
        let object = entry
            .as_object()
            .ok_or("每个服务器要是一个对象（{\"command\": …} 或 {\"url\": …}）")?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .map(|t| t.trim().to_ascii_lowercase());
        match kind.as_deref() {
            None | Some("stdio") | Some("http") | Some("streamable-http") | Some("streamablehttp") => {}
            Some("sse") => {
                return Err(
                    "旧的 HTTP+SSE 连接方式 Kotone 不支持；看看这个服务器有没有 Streamable HTTP 的地址（很多是把地址最后的 /sse 换成 /mcp）"
                        .into(),
                )
            }
            Some(other) => return Err(format!("不认识的连接方式 type = “{other}”（认 stdio、http）")),
        }
        let url = object.get("url").and_then(Value::as_str).map(str::trim);
        let command = object.get("command").and_then(Value::as_str).map(str::trim);
        let http = matches!(
            kind.as_deref(),
            Some("http") | Some("streamable-http") | Some("streamablehttp")
        ) || (url.is_some() && command.is_none());
        if http {
            let url = url
                .filter(|u| !u.is_empty())
                .ok_or("连网址的服务器要写 url")?;
            let lower = url.to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(format!("url 要以 http:// 或 https:// 开头：{url}"));
            }
            return Ok(Self {
                launch: Launch::Http {
                    url: url.to_string(),
                    headers: string_map(object.get("headers"), "headers")?,
                },
            });
        }
        let command = command
            .filter(|c| !c.is_empty())
            .ok_or("要写 command（在你电脑上运行的程序）或 url（服务器的网址）")?;
        let args = match object.get("args") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, v)| scalar(v, &format!("args 第 {} 项", i + 1)))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("args 要是一个数组".into()),
        };
        let cwd = match object.get("cwd") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(Value::String(s)) => Some(s.trim().to_string()),
            Some(_) => return Err("cwd 要是一个字符串（文件夹路径）".into()),
        };
        Ok(Self {
            launch: Launch::Stdio {
                command: command.to_string(),
                args,
                env: string_map(object.get("env"), "env")?,
                cwd,
            },
        })
    }

    /// 写回 `mcpServers` 的样子（只有标准的字段；认不出的字段由 [`McpFile`] 另外留着）。
    pub fn to_entry(&self) -> Value {
        match &self.launch {
            Launch::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut out = Map::new();
                out.insert("command".into(), json!(command));
                if !args.is_empty() {
                    out.insert("args".into(), json!(args));
                }
                if !env.is_empty() {
                    out.insert("env".into(), json!(env));
                }
                if let Some(cwd) = cwd {
                    out.insert("cwd".into(), json!(cwd));
                }
                Value::Object(out)
            }
            Launch::Http { url, headers } => {
                let mut out = Map::new();
                out.insert("type".into(), json!("http"));
                out.insert("url".into(), json!(url));
                if !headers.is_empty() {
                    out.insert("headers".into(), json!(headers));
                }
                Value::Object(out)
            }
        }
    }

    pub fn is_stdio(&self) -> bool {
        matches!(self.launch, Launch::Stdio { .. })
    }

    /// 所有要填进去的地方（参数、环境变量、网址、请求头）。
    fn fields(&self) -> Vec<&str> {
        match &self.launch {
            Launch::Stdio {
                command,
                args,
                env,
                cwd,
            } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .chain(env.values().map(String::as_str))
                .chain(cwd.as_deref())
                .collect(),
            Launch::Http { url, headers } => std::iter::once(url.as_str())
                .chain(headers.values().map(String::as_str))
                .collect(),
        }
    }

    /// 要玩家填的 Key（`${input:名字}`），按名字排好、不重复。
    pub fn keys(&self) -> Vec<String> {
        let mut names: Vec<String> = self.fields().into_iter().flat_map(placeholders).collect();
        names.sort();
        names.dedup();
        names
    }

    /// 配置的指纹：信任记的是“信任的是哪一份配置”，程序、参数、网址、写在文件里的值变了就要重新信任
    /// （照 VS Code：配置改了，信任跟着失效）。Key 本身不在里面（只有 `${input:名字}`）。
    pub fn fingerprint(&self) -> String {
        let canonical = serde_json::to_string(&self.to_entry()).unwrap_or_default();
        let digest = Sha256::digest(canonical.as_bytes());
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    /// 一行话：运行什么 / 连哪里（插件卡片、信任提示用）。
    pub fn summary(&self) -> String {
        match &self.launch {
            Launch::Stdio { command, args, .. } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            Launch::Http { url, .. } => url.clone(),
        }
    }

    /// 它会运行谁写的程序：`npx` / `uvx` / `bunx` / `pnpm dlx` / `docker run` 后面的包名或镜像（猜不出就是 None）。
    pub fn package(&self) -> Option<String> {
        let Launch::Stdio { command, args, .. } = &self.launch else {
            return None;
        };
        let program = Path::new(command)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let rest: Vec<&str> = match program.as_str() {
            "npx" | "uvx" | "bunx" | "pipx" => args.iter().map(String::as_str).collect(),
            "pnpm" | "yarn" | "bun" | "npm" => args
                .iter()
                .map(String::as_str)
                .skip_while(|a| !matches!(*a, "dlx" | "exec" | "x"))
                .skip(1)
                .collect(),
            "docker" | "podman" => args
                .iter()
                .map(String::as_str)
                .skip_while(|a| *a != "run")
                .skip(1)
                .collect(),
            _ => return None,
        };
        // 跳过选项（`-y`、`--from x` 这类带值的猜不全：取第一个不以 - 开头、像包名的）
        rest.into_iter()
            .filter(|a| !a.starts_with('-'))
            .find(|a| !a.contains('=') && !a.is_empty())
            .map(str::to_string)
    }

    /// 信任提示要给玩家看的事实（照 VS Code、Claude Desktop：运行什么程序 / 连哪个网址、要哪些 Key）。
    /// 环境变量和请求头只给名字，不给值（值里可能是 Key）。
    pub fn trust_facts(&self) -> Value {
        match &self.launch {
            Launch::Stdio {
                command,
                args,
                env,
                cwd,
            } => json!({
                "kind": "stdio",
                "command": command,
                "args": args,
                "cwd": cwd,
                "package": self.package(),
                "envNames": env.keys().collect::<Vec<_>>(),
                "keys": self.keys(),
                "summary": self.summary(),
            }),
            Launch::Http { url, headers } => json!({
                "kind": "http",
                "url": url,
                "headerNames": headers.keys().collect::<Vec<_>>(),
                "keys": self.keys(),
                "summary": self.summary(),
            }),
        }
    }

    /// 连接用的配置：`${input:名字}` 换成凭据库里的 Key。缺的 Key 一起报出来（名字列表）。
    pub fn resolve(
        &self,
        secret: &dyn Fn(&str) -> Option<String>,
    ) -> Result<kotone_mcp::ServerConfig, Vec<String>> {
        let mut missing = Vec::new();
        let mut fill = |text: &str| -> String {
            substitute(text, secret).unwrap_or_else(|names| {
                missing.extend(names);
                String::new()
            })
        };
        let config = match &self.launch {
            Launch::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut config =
                    kotone_mcp::ServerConfig::stdio(fill(command), args.iter().map(|a| fill(a)));
                for (name, value) in env {
                    config = config.env(name, fill(value));
                }
                if let Some(cwd) = cwd {
                    config = config.cwd(fill(cwd));
                }
                config
            }
            Launch::Http { url, headers } => {
                let mut config = kotone_mcp::ServerConfig::http(fill(url));
                for (name, value) in headers {
                    config = config.header(name, fill(value));
                }
                config
            }
        };
        if missing.is_empty() {
            Ok(config)
        } else {
            missing.sort();
            missing.dedup();
            Err(missing)
        }
    }
}

const OPEN: &str = "${input:";

/// Key 的名字：字母、数字和 `_.-`，1 到 64 个字。
pub fn valid_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `${input:名字}` 的写法。
pub fn placeholder(name: &str) -> String {
    format!("{OPEN}{name}}}")
}

/// 一段字里的 `${input:名字}`。
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        match after.find('}') {
            Some(end) if valid_key_name(&after[..end]) => {
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out
}

/// 把 `${input:名字}` 换成 Key；缺的名字一起报。
fn substitute(text: &str, secret: &dyn Fn(&str) -> Option<String>) -> Result<String, Vec<String>> {
    let mut out = String::new();
    let mut missing = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find('}') {
            Some(end) if valid_key_name(&after[..end]) => {
                match secret(&after[..end]) {
                    Some(value) => out.push_str(&value),
                    None => missing.push(after[..end].to_string()),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(OPEN);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

/// `KOTONE_HOME/mcp.json` 整个文件。认不出的键（顶层的、每个服务器里的）都原样留着。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct McpFile {
    root: Map<String, Value>,
}

impl McpFile {
    /// 文件在哪（`KOTONE_HOME` 下）。
    pub fn path(home: &Path) -> PathBuf {
        home.join(FILE_NAME)
    }

    /// 读文件的内容。允许注释和多余的逗号（玩家可能照 VS Code 的写法手改；写回去时注释会丢）。
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim_start_matches('\u{FEFF}');
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let value: Value = serde_json::from_str(&crate::import::strip_jsonc(text))
            .map_err(|e| format!("mcp.json 不是合法的 JSON：{e}"))?;
        let Value::Object(root) = value else {
            return Err("mcp.json 的最外层要是一个对象（{\"mcpServers\": {…}}）".into());
        };
        match root.get(SERVERS) {
            None | Some(Value::Object(_)) => Ok(Self { root }),
            Some(_) => Err("mcp.json 里的 mcpServers 要是一个对象（名字 → 服务器）".into()),
        }
    }

    /// 读文件；没有文件 = 空的。文件坏了报错（不拿空的覆盖它）。
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("读不了 {}：{e}", path.display())),
        }
    }

    /// 写文件（先写临时文件再换名，写一半断电也不会坏）。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建不了 {}：{e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_text())
            .map_err(|e| format!("写不了 {}：{e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("写不了 {}：{e}", path.display()))
    }

    pub fn to_text(&self) -> String {
        let mut text =
            serde_json::to_string_pretty(&Value::Object(self.root.clone())).unwrap_or_default();
        text.push('\n');
        text
    }

    fn servers_map(&self) -> Option<&Map<String, Value>> {
        self.root.get(SERVERS).and_then(Value::as_object)
    }

    /// 全部服务器的名字（= 插件 id）。
    pub fn ids(&self) -> Vec<String> {
        self.servers_map()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 某个服务器的原样内容。
    pub fn entry(&self, id: &str) -> Option<&Value> {
        self.servers_map()?.get(id)
    }

    /// 全部服务器读出来的样子：名字不能当 id、内容写错的给错误（只影响它自己）。
    pub fn servers(&self) -> Vec<(String, Result<ServerSpec, String>)> {
        self.servers_map()
            .map(|m| {
                m.iter()
                    .map(|(id, entry)| {
                        let spec = check_id(id).and_then(|()| ServerSpec::parse(entry));
                        (id.clone(), spec)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 加一个或换掉一个。换掉时原来那一项里认不出的字段留着（标准字段以新的为准）。
    pub fn set(&mut self, id: &str, spec: &ServerSpec) {
        let mut entry = match spec.to_entry() {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let servers = self
            .root
            .entry(SERVERS)
            .or_insert_with(|| Value::Object(Map::new()));
        if !servers.is_object() {
            *servers = Value::Object(Map::new());
        }
        let servers = servers.as_object_mut().expect("刚放进去的是对象");
        if let Some(Value::Object(old)) = servers.get(id) {
            for (key, value) in old {
                if !STANDARD.contains(&key.as_str()) {
                    entry.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        servers.insert(id.to_string(), Value::Object(entry));
    }

    /// 删掉一个；本来就没有返回 false。
    pub fn remove(&mut self, id: &str) -> bool {
        self.root
            .get_mut(SERVERS)
            .and_then(Value::as_object_mut)
            .is_some_and(|m| m.remove(id).is_some())
    }
}

/// Kotone 认的标准字段（换掉一项时以新的为准；别的字段原样留着）。
const STANDARD: &[&str] = &["type", "command", "args", "env", "cwd", "url", "headers"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_and_http_entries() {
        let stdio = ServerSpec::parse(&json!({
            "command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything", 3001],
            "env": {"GITHUB_TOKEN": "${input:GITHUB_TOKEN}", "PORT": 8080, "EMPTY": null}
        }))
        .unwrap();
        let Launch::Stdio { args, env, .. } = &stdio.launch else {
            panic!()
        };
        assert_eq!(args[2], "3001", "数字当成字符串");
        assert_eq!(env.get("PORT").map(String::as_str), Some("8080"));
        assert!(!env.contains_key("EMPTY"));
        assert_eq!(stdio.keys(), vec!["GITHUB_TOKEN"]);
        assert_eq!(
            stdio.package().as_deref(),
            Some("@modelcontextprotocol/server-everything")
        );
        assert_eq!(
            stdio.summary(),
            "npx -y @modelcontextprotocol/server-everything 3001"
        );

        // VS Code / Claude Code 的写法：type http；没写 type、只有 url 也算
        for entry in [
            json!({"type": "http", "url": "https://mcp-api.op.gg/mcp"}),
            json!({"url": "https://mcp-api.op.gg/mcp", "headers": {"Authorization": "Bearer ${input:token}"}}),
            json!({"type": "streamableHttp", "url": "http://127.0.0.1:3001/mcp"}),
        ] {
            let spec = ServerSpec::parse(&entry).unwrap();
            assert!(!spec.is_stdio(), "{entry}");
        }
        let with_key = ServerSpec::parse(
            &json!({"url": "https://x.example.com/mcp", "headers": {"Authorization": "Bearer ${input:token}"}}),
        )
        .unwrap();
        assert_eq!(with_key.keys(), vec!["token"]);
        assert_eq!(with_key.to_entry()["type"], "http");
    }

    #[test]
    fn mistakes_are_explained() {
        for (entry, words) in [
            (json!({"type": "sse", "url": "https://x/sse"}), "HTTP+SSE"),
            (json!({"type": "websocket", "url": "ws://x"}), "不认识"),
            (json!({"url": "ftp://x"}), "http://"),
            (json!({"args": ["x"]}), "command"),
            (json!({"command": "x", "args": "x"}), "数组"),
            (json!({"command": "x", "env": ["A"]}), "对象"),
            (json!({"command": "x", "env": {"A": {"b": 1}}}), "字符串"),
            (json!("npx"), "对象"),
        ] {
            let error = ServerSpec::parse(&entry).unwrap_err();
            assert!(error.contains(words), "{entry} → {error}");
        }
    }

    #[test]
    fn ids_follow_the_plugin_rules_and_reserved_names() {
        assert!(check_id("everything").is_ok());
        assert!(check_id("op-gg").is_ok());
        for bad in [
            "Everything",
            "my_server",
            "-x",
            "",
            "kotone",
            "connection",
            "settings",
            "saved",
        ] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    /// Key 不进文件：`${input:名字}` 连接时才从凭据库换进去；缺的一起报出来。
    #[test]
    fn keys_are_filled_in_when_connecting() {
        let spec = ServerSpec::parse(&json!({
            "command": "server.exe",
            "args": ["--token=${input:token}", "--x"],
            "env": {"A": "${input:a}", "PLAIN": "1", "BROKEN": "${input:bad name}"}
        }))
        .unwrap();
        assert_eq!(spec.keys(), vec!["a", "token"]);
        let missing = spec.resolve(&|_| None).unwrap_err();
        assert_eq!(missing, vec!["a", "token"]);
        let config = spec.resolve(&|name| Some(format!("<{name}>"))).unwrap();
        let kotone_mcp::Transport::Stdio { args, env, .. } = config.transport else {
            panic!()
        };
        assert_eq!(args, vec!["--token=<token>", "--x"]);
        assert_eq!(env["A"], "<a>");
        assert_eq!(env["PLAIN"], "1");
        assert_eq!(env["BROKEN"], "${input:bad name}", "名字不合法的原样留着");
        assert_eq!(
            placeholders("${input:x}${input:y} ${input:"),
            vec!["x", "y"]
        );
    }

    /// 信任跟着配置走：程序、参数、网址变了，指纹就变；Key 的值不在里面。
    #[test]
    fn fingerprints_follow_the_config() {
        let a = ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "a"]})).unwrap();
        let same =
            ServerSpec::parse(&json!({"args": ["-y", "a"], "command": "npx", "extra": 1})).unwrap();
        let other = ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "b"]})).unwrap();
        assert_eq!(a.fingerprint(), same.fingerprint());
        assert_ne!(a.fingerprint(), other.fingerprint());
        assert_eq!(a.fingerprint().len(), 16);
    }

    /// 原则 19：认不出的字段（顶层的、服务器里的）原样留着；一个服务器写错了不影响别的；文件坏了报错、不当成空的。
    #[test]
    fn the_file_keeps_what_it_does_not_know() {
        let text = r#"{
          // 玩家照 VS Code 的写法加的注释
          "mcpServers": {
            "everything": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"], "disabled": false, "timeout": 30},
            "Bad_Name": {"command": "x"},
            "broken": {"args": []},
          },
          "inputs": [{"id": "token", "type": "promptString"}],
          "futureKotoneField": {"x": 1}
        }"#;
        let mut file = McpFile::parse(text).unwrap();
        let servers = file.servers();
        assert_eq!(servers.len(), 3);
        let by_id: BTreeMap<_, _> = servers.into_iter().collect();
        assert!(by_id["everything"].is_ok());
        assert!(by_id["Bad_Name"].as_ref().unwrap_err().contains("插件 id"));
        assert!(by_id["broken"].as_ref().unwrap_err().contains("command"));

        // 换掉 everything 的参数：它的 disabled / timeout 留着
        let spec = ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "x"]})).unwrap();
        file.set("everything", &spec);
        file.set(
            "new-one",
            &ServerSpec::parse(&json!({"url": "http://127.0.0.1:1/mcp"})).unwrap(),
        );
        assert!(file.remove("broken"));
        assert!(!file.remove("nope"));
        let back = McpFile::parse(&file.to_text()).unwrap();
        let everything = back.entry("everything").unwrap();
        assert_eq!(everything["args"], json!(["-y", "x"]));
        assert_eq!(everything["disabled"], false);
        assert_eq!(everything["timeout"], 30);
        assert_eq!(back.root["futureKotoneField"], json!({"x": 1}));
        assert_eq!(back.root["inputs"][0]["id"], "token");
        let mut ids = back.ids();
        ids.sort();
        assert_eq!(ids, vec!["Bad_Name", "everything", "new-one"]);

        assert!(McpFile::parse("[1]").is_err());
        assert!(McpFile::parse("{\"mcpServers\": []}").is_err());
        assert!(McpFile::parse("{not json").is_err());
        assert_eq!(McpFile::parse("").unwrap(), McpFile::default());
        assert_eq!(
            McpFile::parse("\u{FEFF}{}").unwrap().ids(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn saving_and_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = McpFile::path(dir.path());
        assert_eq!(
            McpFile::load(&path).unwrap(),
            McpFile::default(),
            "没有文件 = 空的"
        );
        let mut file = McpFile::default();
        file.set(
            "everything",
            &ServerSpec::parse(&json!({"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"]}))
                .unwrap(),
        );
        file.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"mcpServers\""), "{text}");
        assert_eq!(McpFile::load(&path).unwrap(), file);
        std::fs::write(&path, "{broken").unwrap();
        assert!(McpFile::load(&path).is_err());
    }
}
