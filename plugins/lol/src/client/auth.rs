//! 从 `LeagueClientUx.exe` 的命令行读端口和令牌（照 LeagueAkari 的 `ux-command-line-parser.ts`）。
//! 国服不写 lockfile（实测 0 字节、停在 2024 年），所以只读命令行。

use std::fmt;

/// 连客户端要的东西。令牌是凭据：`Debug` 不打印它，也不进日志。
#[derive(Clone, PartialEq, Eq)]
pub struct LcuAuth {
    pub port: u16,
    pub token: String,
    /// `rso_platform_id`，例如 `TJ100`、`HN1`。
    pub platform: String,
    /// `region`，国服是 `TENCENT`。
    pub region: String,
}

impl fmt::Debug for LcuAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LcuAuth")
            .field("port", &self.port)
            .field("token", &"***")
            .field("platform", &self.platform)
            .field("region", &self.region)
            .finish()
    }
}

impl LcuAuth {
    /// 是不是腾讯的服务器（国服）。
    pub fn is_tencent(&self) -> bool {
        self.region.eq_ignore_ascii_case("TENCENT")
    }
}

/// 取 `--name=value`（值可能被引号包着）。
fn arg(cmd: &str, name: &str) -> Option<String> {
    let key = format!("--{name}=");
    let start = cmd.find(&key)? + key.len();
    let value: String = cmd[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"')
        .collect();
    (!value.is_empty()).then_some(value)
}

/// 命令行 → 端口和令牌；不是客户端的命令行时 None。
pub fn parse_command_line(cmd: &str) -> Option<LcuAuth> {
    let port = arg(cmd, "app-port")?.parse().ok()?;
    let token = arg(cmd, "remoting-auth-token")?;
    Some(LcuAuth {
        port,
        token,
        platform: arg(cmd, "rso_platform_id").unwrap_or_default(),
        region: arg(cmd, "region").unwrap_or_default(),
    })
}

/// 测试用的 `KOTONE_LOL_LCU=<端口>:<令牌>` → 连 127.0.0.1 的这个端口（不读命令行，连录好数据的假客户端）。
pub fn parse_port_token(value: &str) -> Option<LcuAuth> {
    let (port, token) = value.trim().split_once(':')?;
    let port = port.trim().parse().ok().filter(|p: &u16| *p > 0)?;
    let token = token.trim();
    (!token.is_empty()).then(|| LcuAuth {
        port,
        token: token.to_string(),
        platform: String::new(),
        region: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_token_override() {
        let auth = parse_port_token(" 52260:AbC_d-1 ").unwrap();
        assert_eq!((auth.port, auth.token.as_str()), (52260, "AbC_d-1"));
        for bad in [
            "",
            "52260",
            "52260:",
            ":tok",
            "0:tok",
            "abc:tok",
            "70000:tok",
        ] {
            assert!(parse_port_token(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn parses_plain_and_quoted_arguments() {
        let cmd = r#""C:/WeGameApps/英雄联盟/LeagueClient/LeagueClientUx.exe" "--riotclient-auth-token=zzz" "--riotclient-app-port=52129" "--remoting-auth-token=AbC_d-1" --app-port=52260 "--region=TENCENT" "--rso_platform_id=TJ100" "--locale=zh_CN""#;
        let auth = parse_command_line(cmd).unwrap();
        assert_eq!(auth.port, 52260);
        assert_eq!(auth.token, "AbC_d-1");
        assert_eq!(auth.platform, "TJ100");
        assert!(auth.is_tencent());
        // riotclient-app-port 不能被当成 app-port
        assert_ne!(auth.port, 52129);
    }

    #[test]
    fn rejects_other_command_lines() {
        assert!(parse_command_line("notepad.exe foo.txt").is_none());
        assert!(parse_command_line("x.exe --app-port=abc --remoting-auth-token=t").is_none());
        assert!(parse_command_line("x.exe --app-port=1").is_none());
    }

    #[test]
    fn debug_hides_the_token() {
        let auth = parse_command_line("x --app-port=1 --remoting-auth-token=secret").unwrap();
        assert!(!format!("{auth:?}").contains("secret"));
    }
}
