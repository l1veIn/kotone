//! `kotone-cli plugin check`：插件作者发布前自己跑的检查（插件标准 0.1 第七节，ADR 047 D4 第 7 条；
//! 照 Cortico 的 `pnpm check:extension`、MCP Inspector）。
//!
//! 输入和 `mcp.json` 里的一条一样：`--command <程序> [-- 参数…]`（子进程）、`--url <网址>`（Streamable HTTP），或者
//! `--config <文件> [--id <名字>]` 读 `mcpServers` 里的一条。`--id` 是插件 id（事件全名的前缀，缺省 `plugin`）。
//! 检查本身在 `kotone_remote_plugin::check`（和宿主同一份代码），这里只管参数和输出。
//!
//! - 不碰凭据库、不读 `~/.kotone`：配置里的 `${input:名字}` 只从环境变量 `KOTONE_INPUT_<名字>` 读
//!   （名字转大写，字母数字以外的换成 `_`），读不到就报错说明。
//! - 输出：给人看的 ✓ / ! / ✗ 清单；`--json` 给机器。
//! - 退出码：有 ✗ 是 1；只有 ! 是 0（`--strict` 时 ! 也算 1）；参数、配置不对是 2。

use std::path::PathBuf;

use clap::{Args, Subcommand};
use kotone_core::profile::GameProfile;
use kotone_remote_plugin::check::{self, Level, Report};
use kotone_remote_plugin::{check_id, McpFile, ServerSpec};
use serde_json::{json, Map, Value};

/// 不写 `--id` 时事件全名的前缀（占位）。
const DEFAULT_ID: &str = "plugin";

#[derive(Subcommand, Debug)]
pub enum PluginCommand {
    /// 发布前检查一个第三方插件（MCP 服务器）：连上、读 Kotone 清单、看工具和页面、每个观察项真调一次
    Check(CheckArgs),
}

#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("server").required(true).args(["command", "url", "config"])))]
pub struct CheckArgs {
    /// 子进程：要运行的程序（它的参数写在 -- 后面，如 `--command node -- server.js`）
    #[arg(long)]
    command: Option<String>,
    /// 子进程的参数（写在 -- 后面）
    #[arg(last = true)]
    args: Vec<String>,
    /// 子进程的环境变量 NAME=值（可写多次）
    #[arg(long = "env", value_name = "NAME=值")]
    env: Vec<String>,
    /// 子进程的工作目录
    #[arg(long)]
    cwd: Option<String>,
    /// Streamable HTTP：服务器的网址
    #[arg(long)]
    url: Option<String>,
    /// HTTP 请求头 NAME=值（可写多次）
    #[arg(long = "header", value_name = "NAME=值")]
    header: Vec<String>,
    /// 读一个 mcp.json 格式的文件（{"mcpServers": {…}}），用 --id 那一条（只有一条时可以不写）
    #[arg(long)]
    config: Option<PathBuf>,
    /// 插件 id：玩家在 mcp.json 里给的短名，事件全名是 <id>.<名字>（缺省 plugin，只是占位）
    #[arg(long)]
    id: Option<String>,
    /// 输出 JSON（给脚本、CI）
    #[arg(long)]
    json: bool,
    /// ! 也算没过（退出码 1）
    #[arg(long)]
    strict: bool,
    /// 不再用 2026-07-28（server/discover）连一次
    #[arg(long)]
    no_modern: bool,
}

pub async fn run(command: PluginCommand) -> i32 {
    match command {
        PluginCommand::Check(args) => cmd_check(args).await,
    }
}

/// `${input:名字}` 从哪个环境变量读。
pub fn input_env(name: &str) -> String {
    let tail: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("KOTONE_INPUT_{tail}")
}

fn pairs(items: &[String], what: &str) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for item in items {
        let (name, value) = item
            .split_once('=')
            .filter(|(n, _)| !n.trim().is_empty())
            .ok_or_else(|| format!("{what} 要写成 NAME=值：{item}"))?;
        out.insert(name.trim().to_string(), json!(value));
    }
    Ok(out)
}

/// 参数 → 插件 id 和 `mcpServers` 里的一条（读法和宿主一样：[`ServerSpec::parse`]）。
fn spec_of(args: &CheckArgs) -> Result<(String, ServerSpec), String> {
    let (id, entry) = if let Some(path) = &args.config {
        let file = McpFile::load(path)?;
        let ids = file.ids();
        let id = match &args.id {
            Some(id) => id.clone(),
            None if ids.len() == 1 => ids[0].clone(),
            None => {
                return Err(format!(
                    "{} 里有 {} 个服务器，用 --id 选一个：{}",
                    path.display(),
                    ids.len(),
                    ids.join("、")
                ))
            }
        };
        let entry = file.entry(&id).cloned().ok_or_else(|| {
            format!(
                "{} 的 mcpServers 里没有 {id}（有：{}）",
                path.display(),
                ids.join("、")
            )
        })?;
        (id, entry)
    } else if let Some(command) = &args.command {
        let mut entry = json!({"command": command, "args": args.args});
        let env = pairs(&args.env, "--env")?;
        if !env.is_empty() {
            entry["env"] = Value::Object(env);
        }
        if let Some(cwd) = &args.cwd {
            entry["cwd"] = json!(cwd);
        }
        (args.id.clone().unwrap_or_else(|| DEFAULT_ID.into()), entry)
    } else {
        let url = args.url.clone().unwrap_or_default();
        let mut entry = json!({"type": "http", "url": url});
        let headers = pairs(&args.header, "--header")?;
        if !headers.is_empty() {
            entry["headers"] = Value::Object(headers);
        }
        (args.id.clone().unwrap_or_else(|| DEFAULT_ID.into()), entry)
    };
    // 缺省的 plugin 只是占位（它是 Kotone 自己用的名字，玩家起不了）：只查玩家会真的起的名字
    if id != DEFAULT_ID || args.id.is_some() || args.config.is_some() {
        check_id(&id)?;
    }
    let spec = ServerSpec::parse(&entry)?;
    Ok((id, spec))
}

/// 这一版 Kotone 带的游戏（游戏 id；只看内置的，不读 `~/.kotone/profiles`）。
fn known_games() -> Vec<String> {
    [
        GameProfile::builtin_lol(),
        GameProfile::builtin_dota2(),
        GameProfile::builtin_cs2(),
        GameProfile::builtin_genshin(),
        GameProfile::builtin_zzz(),
        GameProfile::builtin_generic(),
    ]
    .into_iter()
    .map(|p| p.id)
    .collect()
}

async fn cmd_check(args: CheckArgs) -> i32 {
    let (id, spec) = match spec_of(&args) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let config = match spec.resolve(&|name: &str| std::env::var(input_env(name)).ok()) {
        Ok(config) => config.client("kotone", env!("CARGO_PKG_VERSION")),
        Err(missing) => {
            let vars: Vec<String> = missing.iter().map(|n| input_env(n)).collect();
            eprintln!(
                "配置里要填 Key：{}。检查命令不读 Kotone 的凭据库，先设环境变量再跑：{}",
                missing.join("、"),
                vars.join("、")
            );
            return 2;
        }
    };
    // 填进去的 Key 不出现在输出里（连不上时的错误可能带着网址、参数）：输出常被贴进 CI 日志、issue
    let secrets: Vec<String> = spec
        .keys()
        .iter()
        .filter_map(|n| std::env::var(input_env(n)).ok())
        .filter(|v| v.trim().chars().count() >= 4)
        .collect();
    let mut report = check::run(&check::Options {
        id,
        spec,
        config,
        known_games: Some(known_games()),
        modern: !args.no_modern,
    })
    .await;
    for line in report.sections.iter_mut().flat_map(|s| s.lines.iter_mut()) {
        for secret in &secrets {
            line.text = line.text.replace(secret.as_str(), "***");
        }
    }
    if args.json {
        let mut value = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
        value["counts"] = json!({
            "ok": report.count(Level::Ok),
            "warn": report.count(Level::Warn),
            "fail": report.count(Level::Fail),
        });
        value["exitCode"] = json!(report.exit_code(args.strict));
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        print!("{}", human(&report, args.strict));
    }
    report.exit_code(args.strict)
}

/// 给人看的清单。
pub fn human(report: &Report, strict: bool) -> String {
    let mut out = format!("检查插件 {}（{}）\n", report.plugin, report.target);
    for section in &report.sections {
        out.push_str(&format!("\n{}\n", section.title));
        for line in &section.lines {
            out.push_str(&format!("  {} {}\n", line.level.mark(), line.text));
        }
    }
    let (ok, warn, fail) = (
        report.count(Level::Ok),
        report.count(Level::Warn),
        report.count(Level::Fail),
    );
    let verdict = if fail > 0 {
        "有 ✗：先改好再发布"
    } else if warn > 0 && strict {
        "没有 ✗，但 --strict 下 ! 也算没过"
    } else if warn > 0 {
        "没有 ✗，可以发布；! 是提醒，看一眼"
    } else {
        "都没问题，可以发布"
    };
    out.push_str(&format!("\n✓ {ok}  ! {warn}  ✗ {fail}：{verdict}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: PluginCommand,
    }

    fn parse(args: &[&str]) -> CheckArgs {
        let mut all = vec!["x", "check"];
        all.extend_from_slice(args);
        match Cli::try_parse_from(all).unwrap().command {
            PluginCommand::Check(args) => args,
        }
    }

    #[test]
    fn inputs_come_from_kotone_input_env_vars() {
        assert_eq!(input_env("token"), "KOTONE_INPUT_TOKEN");
        assert_eq!(input_env("api.key-2"), "KOTONE_INPUT_API_KEY_2");
    }

    #[test]
    fn command_url_and_config_become_one_mcp_servers_entry() {
        let (id, spec) = spec_of(&parse(&[
            "--command",
            "node",
            "--env",
            "A=1",
            "--",
            "server.js",
            "--port",
            "3",
        ]))
        .unwrap();
        assert_eq!(id, "plugin", "缺省的占位 id 不查保留名");
        assert_eq!(
            spec.to_entry(),
            json!({"command": "node", "args": ["server.js", "--port", "3"], "env": {"A": "1"}})
        );
        let (id, spec) = spec_of(&parse(&[
            "--url",
            "http://127.0.0.1:3000/mcp",
            "--id",
            "farm",
            "--header",
            "Authorization=Bearer ${input:token}",
        ]))
        .unwrap();
        assert_eq!(id, "farm");
        assert!(spec.is_local());
        assert_eq!(spec.keys(), ["token"]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"steam": {"command": "steam.exe"}, "farm": {"url": "https://x.example.com/mcp"}}}"#,
        )
        .unwrap();
        let p = path.to_string_lossy().into_owned();
        assert!(spec_of(&parse(&["--config", &p]))
            .unwrap_err()
            .contains("--id"));
        let (id, spec) = spec_of(&parse(&["--config", &p, "--id", "steam"])).unwrap();
        assert_eq!((id.as_str(), spec.summary()), ("steam", "steam.exe".into()));
        assert!(spec_of(&parse(&["--config", &p, "--id", "nope"])).is_err());
        assert!(
            spec_of(&parse(&["--command", "x", "--id", "Bad Id"])).is_err(),
            "插件 id 照宿主的规矩"
        );
        assert!(spec_of(&parse(&["--command", "x", "--id", "plugin"])).is_err());
        assert!(
            Cli::try_parse_from(["x", "check"]).is_err(),
            "要给一个服务器"
        );
    }

    #[test]
    fn human_output_ends_with_a_verdict() {
        let report: Report = Report {
            plugin: "farm".into(),
            target: "farm.exe".into(),
            sections: Vec::new(),
            manifest: None,
        };
        assert!(human(&report, false).contains("✓ 0  ! 0  ✗ 0：都没问题"));
    }
}
