//! 哪条 API 连接是哪一家的声音（ADR 048）：同一家的 Key 对话和声音共用，“自动”跟着她用的模型走。
//!
//! 只认按量付费的 Key 能直接调语音合成的几家：DeepSeek、MiniMax（国内 / 国际）、阿里云百炼（北京 / 新加坡）。
//! 百炼 Token Plan 这类订阅 Key 不能按量调用语音，不算（地址是 `token-plan.…`，认不出来）。

use crate::assistant::TtsEngine;
use crate::connection::Connection;
use crate::settings::Settings;

/// 国内站还是国际站（MiniMax、百炼两边的账号和 Key 不通用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Cn,
    Intl,
}

impl Region {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cn => "cn",
            Self::Intl => "intl",
        }
    }

    /// `modelConfigs[…].region` 里存的值；认不出来是 None。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "cn" => Some(Self::Cn),
            "intl" => Some(Self::Intl),
            _ => None,
        }
    }
}

fn host_of(base_url: &str) -> String {
    let rest = base_url
        .trim()
        .split_once("://")
        .map_or(base_url.trim(), |(_, rest)| rest);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// 这条连接是哪一家的声音、哪个站。先看地址（自己改过地址的也认得），认不出再看预设。
pub fn voice_of_connection(connection: &Connection) -> Option<(TtsEngine, Region)> {
    let host = host_of(&connection.base_url);
    if host_is(&host, "deepseek.com") {
        return Some((TtsEngine::Deepseek, Region::Cn));
    }
    if host_is(&host, "minimax.io") {
        return Some((TtsEngine::Minimax, Region::Intl));
    }
    if [
        "minimaxi.com",
        "minimax.cn",
        "minimax.chat",
        "minimaxi.chat",
    ]
    .iter()
    .any(|domain| host_is(&host, domain))
    {
        return Some((TtsEngine::Minimax, Region::Cn));
    }
    if host == "dashscope.aliyuncs.com" {
        return Some((TtsEngine::Dashscope, Region::Cn));
    }
    if host == "dashscope-intl.aliyuncs.com" {
        return Some((TtsEngine::Dashscope, Region::Intl));
    }
    // 硅基流动没有聊天预设（用“自定义”连接接它）：只为了共用 Key，“自动”不跟它（见 [`AUTO_ENGINES`]）
    if host == "api.siliconflow.cn" {
        return Some((TtsEngine::Siliconflow, Region::Cn));
    }
    // 地址认不出（比如自己架的代理）：照预设算，和以前 DeepSeek 语音的认法一样
    match connection.provider.as_str() {
        "deepseek" => Some((TtsEngine::Deepseek, Region::Cn)),
        "minimax-cn" => Some((TtsEngine::Minimax, Region::Cn)),
        "minimax" => Some((TtsEngine::Minimax, Region::Intl)),
        "dashscope" => Some((TtsEngine::Dashscope, Region::Cn)),
        "dashscope-intl" => Some((TtsEngine::Dashscope, Region::Intl)),
        _ => None,
    }
}

/// 她正在用的连接 ID（0.1.x 旧提醒助手还没升级时是旧字段里的那条）。
pub fn her_connection_id(settings: &Settings) -> &str {
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        settings.assistant.connection_id.trim()
    } else {
        settings.companion_connection_id().trim()
    }
}

/// “自动”会跟的几家（ADR 048）：她用这几家的模型、连接里有 Key，就用这家的声音。
pub const AUTO_ENGINES: &[TtsEngine] = &[
    TtsEngine::Deepseek,
    TtsEngine::Minimax,
    TtsEngine::Dashscope,
];

/// 她用的模型那一家有没有声音：(引擎, 连接 ID, 站)。“自动”用它（还要另查这条连接有没有 Key）。
pub fn her_voice(settings: &Settings) -> Option<(TtsEngine, String, Region)> {
    let id = her_connection_id(settings);
    if id.is_empty() {
        return None;
    }
    let connection = settings.connections.iter().find(|c| c.id == id)?;
    let (engine, region) = voice_of_connection(connection)?;
    AUTO_ENGINES
        .contains(&engine)
        .then(|| (engine, connection.id.clone(), region))
}

/// 这一家的声音用哪条连接的 Key：她正在用的那条是这一家就用它，否则用第一条这一家的连接。
/// 把 [`Settings::deepseek_connection_id`] 推广到每一家。
pub fn vendor_connection(settings: &Settings, engine: TtsEngine) -> Option<(String, Region)> {
    let her = her_connection_id(settings);
    let matches = |c: &&Connection| voice_of_connection(c).is_some_and(|(e, _)| e == engine);
    let connection = settings
        .connections
        .iter()
        .filter(matches)
        .find(|c| c.id == her)
        .or_else(|| settings.connections.iter().find(matches))?;
    let (_, region) = voice_of_connection(connection)?;
    Some((connection.id.clone(), region))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionKind;

    fn connection(id: &str, provider: &str, base_url: &str) -> Connection {
        Connection {
            id: id.into(),
            display_name: id.into(),
            kind: ConnectionKind::Remote,
            provider: provider.into(),
            base_url: base_url.into(),
            model: "m".into(),
        }
    }

    #[test]
    fn connections_are_matched_by_address_then_preset() {
        let cases = [
            (
                "deepseek",
                "https://api.deepseek.com/v1",
                Some((TtsEngine::Deepseek, Region::Cn)),
            ),
            (
                "minimax-cn",
                "https://api.minimaxi.com/v1",
                Some((TtsEngine::Minimax, Region::Cn)),
            ),
            (
                "minimax",
                "https://api.minimax.io/v1",
                Some((TtsEngine::Minimax, Region::Intl)),
            ),
            // 预设是国内、地址改成了国际站：跟着地址走
            (
                "minimax-cn",
                "https://api.minimax.io/v1",
                Some((TtsEngine::Minimax, Region::Intl)),
            ),
            (
                "custom",
                "https://api.minimax.cn/v1",
                Some((TtsEngine::Minimax, Region::Cn)),
            ),
            (
                "dashscope",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                Some((TtsEngine::Dashscope, Region::Cn)),
            ),
            (
                "dashscope-intl",
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                Some((TtsEngine::Dashscope, Region::Intl)),
            ),
            // 订阅 Key（Token Plan）不能按量调语音
            (
                "qwen-token-plan-cn",
                "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                None,
            ),
            ("moonshot-cn", "https://api.moonshot.cn/v1", None),
            (
                "custom",
                "https://api.siliconflow.cn/v1",
                Some((TtsEngine::Siliconflow, Region::Cn)),
            ),
            ("custom", "https://my-proxy.example/v1", None),
            // 长得像的别家域名不算
            ("custom", "https://notdeepseek.com.evil.example/v1", None),
        ];
        for (provider, url, expected) in cases {
            assert_eq!(
                voice_of_connection(&connection("c", provider, url)),
                expected,
                "{provider} {url}"
            );
        }
    }

    #[test]
    fn her_voice_follows_her_connection_and_vendor_lookup_prefers_it() {
        let mut s = Settings {
            connections: vec![
                connection("mm-1", "minimax-cn", "https://api.minimaxi.com/v1"),
                connection("ds", "deepseek", "https://api.deepseek.com"),
                connection("mm-2", "minimax", "https://api.minimax.io/v1"),
            ],
            ..Settings::default()
        };
        s.assistant.connection_id = "mm-2".into();
        assert_eq!(
            her_voice(&s),
            Some((TtsEngine::Minimax, "mm-2".into(), Region::Intl))
        );
        assert_eq!(
            vendor_connection(&s, TtsEngine::Minimax),
            Some(("mm-2".into(), Region::Intl))
        );
        assert_eq!(
            vendor_connection(&s, TtsEngine::Deepseek),
            Some(("ds".into(), Region::Cn))
        );
        assert_eq!(vendor_connection(&s, TtsEngine::Dashscope), None);
        s.assistant.connection_id = "nope".into();
        assert_eq!(her_voice(&s), None);
        // 她用硅基流动聊天：Key 能共用，“自动”不跟
        s.connections
            .push(connection("sf", "custom", "https://api.siliconflow.cn/v1"));
        s.assistant.connection_id = "sf".into();
        assert_eq!(her_voice(&s), None);
        assert_eq!(
            vendor_connection(&s, TtsEngine::Siliconflow),
            Some(("sf".into(), Region::Cn))
        );
        assert_eq!(
            vendor_connection(&s, TtsEngine::Minimax),
            Some(("mm-1".into(), Region::Cn))
        );
    }
}
