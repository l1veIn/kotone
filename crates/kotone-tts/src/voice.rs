//! 声音编号：`提供方/声音`（docs/product-evolution-game-tools.md §11.1）。
//!
//! 例如 `system/<Windows 语音 token>`、`kokoro/zf_001`、`openai/alloy`、`volcano/BV700_streaming`。
//! 旧配置里没有前缀的写法（Windows 语音 token、`kokoro:zf_001`）照常接受。选中的声音属于
//! 另一个提供方或找不到时直接报错，**不偷偷换成别的声音**。

pub const SYSTEM: &str = "system";
pub const KOKORO: &str = "kokoro";
pub const OPENAI: &str = "openai";
pub const VOLCANO: &str = "volcano";
pub const DEEPSEEK: &str = "deepseek";

const PROVIDERS: &[&str] = &[SYSTEM, KOKORO, OPENAI, VOLCANO, DEEPSEEK];

/// 加上提供方前缀（已带本提供方前缀的原样返回；旧的 `kokoro:` 写法改成 `kokoro/`）。
pub fn qualify(provider: &str, id: &str) -> String {
    let id = id.trim();
    if id.is_empty() {
        return String::new();
    }
    match native(provider, id) {
        Ok(native) => format!("{provider}/{native}"),
        Err(_) => id.to_string(),
    }
}

/// 去掉提供方前缀，得到引擎自己的声音名。属于其他提供方时报错。
pub fn native<'a>(provider: &str, id: &'a str) -> Result<&'a str, String> {
    let id = id.trim();
    if let Some((prefix, rest)) = id.split_once('/') {
        if prefix == provider {
            return Ok(rest);
        }
        if PROVIDERS.contains(&prefix) {
            return Err(format!(
                "所选声音 {id} 属于 {prefix}，与当前语音引擎（{provider}）不符，请重新选择声音"
            ));
        }
    }
    if provider == KOKORO {
        if let Some(rest) = id.strip_prefix("kokoro:") {
            return Ok(rest);
        }
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_prefixes_round_trip_and_never_cross_providers() {
        assert_eq!(qualify(KOKORO, "kokoro:zf_001"), "kokoro/zf_001");
        assert_eq!(qualify(KOKORO, "kokoro/zf_001"), "kokoro/zf_001");
        assert_eq!(qualify(OPENAI, "alloy"), "openai/alloy");
        assert_eq!(qualify(SYSTEM, ""), "");
        let token =
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Speech\Voices\Tokens\TTS_MS_ZH-CN_HUIHUI_11.0";
        assert_eq!(native(SYSTEM, &qualify(SYSTEM, token)).unwrap(), token);
        assert_eq!(native(SYSTEM, token).unwrap(), token);
        assert!(native(OPENAI, "kokoro/zf_001").is_err());
        assert!(native(VOLCANO, "openai/alloy")
            .unwrap_err()
            .contains("不符"));
    }
}
