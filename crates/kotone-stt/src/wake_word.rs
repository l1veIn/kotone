//! 自定义唤醒词：中文 → 唤醒模型（sherpa-onnx KWS，WenetSpeech zipformer）认的拼音 token。
//!
//! 照搬 sherpa-onnx 的 `text2token`（`tokens_type="ppinyin"`，Apache-2.0，
//! `sherpa_onnx/utils.py` @ 4488718d）：每个字取带声调的拼音，拆成“声母 + 带调韵母”
//! （y、w 也当声母；零声母音节整个算一个 token；j/q/x/y 后的 ü 写作 u），每个 token 都要在
//! 模型的 `tokens.txt` 里，否则整行无效。拼音用 `pinyin` crate（MIT，数据与 text2token 用的
//! pypinyin 同源；逐字取音，不按词消歧）——多音字让用户自己选读音（存进角色）。
//!
//! 关键词行格式照 sherpa-onnx `utils.cc` 的 `EncodeBase`：`<token…> [:boost] [#threshold] @label`；
//! 阈值一定带小数点（这个模型的 tokens.txt 里有 `#0`…`#29`，`#1` 会被当成 token）。

use pinyin::{ToPinyin, ToPinyinMulti};

/// 唤醒模型 `sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01` 的 `tokens.txt`
/// （SHA-256 72316508…cf2e，与下载校验的一致）。内置一份，没下载模型时也能检查唤醒词。
const TOKENS_TXT: &str = include_str!("wake_tokens.txt");

/// 声母（`strict=False`：y、w 也算），长的在前。
const INITIALS: &[&str] = &[
    "zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r",
    "z", "c", "s", "y", "w",
];

/// 最少几个字（再短很容易喊不中，见 sherpa-onnx issue #3809）；少于推荐长度给提示。
pub const MIN_CHARS: usize = 3;
pub const RECOMMENDED_CHARS: usize = 4;
pub const MAX_CHARS: usize = 8;

fn vocabulary() -> &'static std::collections::HashSet<&'static str> {
    static VOCAB: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    VOCAB.get_or_init(|| {
        TOKENS_TXT
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|t| !t.starts_with('<') && !t.starts_with('#'))
            .collect()
    })
}

/// 一个字的检查结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyllableCheck {
    pub character: String,
    /// 用的读音（带声调）。
    pub reading: String,
    /// 这个字所有的读音（多于一个时界面给下拉框）。
    pub choices: Vec<String>,
    pub tokens: Vec<String>,
    /// 模型不认的 token（有就整个唤醒词不能用）。
    pub missing: Vec<String>,
}

/// 唤醒词的检查结果。`ok` 为 true 时 `tokens` 可以直接用。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeWordCheck {
    pub text: String,
    pub syllables: Vec<SyllableCheck>,
    pub tokens: Vec<String>,
    pub ok: bool,
    /// 不能用的原因。
    pub problems: Vec<String>,
    /// 能用、但可能不好使的提醒。
    pub warnings: Vec<String>,
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// 一个带调音节拆成 token（`text2token` 的规则）。
pub fn split_syllable(syllable: &str) -> Vec<String> {
    match INITIALS.iter().find(|i| syllable.starts_with(*i)) {
        Some(initial) => {
            let rest = &syllable[initial.len()..];
            if rest.is_empty() {
                vec![(*initial).to_string()]
            } else {
                vec![(*initial).to_string(), rest.to_string()]
            }
        }
        None => vec![syllable.to_string()],
    }
}

/// 检查一个唤醒词。`readings` 是用户为每个字选的读音（可以缺、可以空，缺的用第一个读音）。
pub fn check(text: &str, readings: &[String]) -> WakeWordCheck {
    let text: String = text.trim().chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = WakeWordCheck {
        text: text.clone(),
        syllables: Vec::new(),
        tokens: Vec::new(),
        ok: false,
        problems: Vec::new(),
        warnings: Vec::new(),
    };
    let count = text.chars().count();
    if count == 0 {
        out.problems.push("唤醒词是空的".into());
        return out;
    }
    if let Some(bad) = text.chars().find(|c| !is_cjk(*c)) {
        out.problems.push(format!(
            "唤醒词只能用汉字（“{bad}”不行；唤醒模型只认中文拼音）"
        ));
        return out;
    }
    if count > MAX_CHARS {
        out.problems.push(format!("唤醒词最多 {MAX_CHARS} 个字"));
        return out;
    }
    if count < MIN_CHARS {
        out.problems.push(format!(
            "唤醒词至少 {MIN_CHARS} 个字（太短很难喊中；两个字的名字可以重复两遍，如“小雨小雨”）"
        ));
    } else if count < RECOMMENDED_CHARS {
        out.warnings
            .push("3 个字的唤醒词喊中率可能偏低，推荐 4 个字（如名字重复两遍）".into());
    }
    let vocab = vocabulary();
    for (i, c) in text.chars().enumerate() {
        let choices: Vec<String> = c
            .to_pinyin_multi()
            .map(|multi| {
                multi
                    .into_iter()
                    .map(|p| p.with_tone().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut uniq: Vec<String> = Vec::new();
        for choice in choices {
            if !uniq.contains(&choice) {
                uniq.push(choice);
            }
        }
        let default = c
            .to_pinyin()
            .map(|p| p.with_tone().to_string())
            .or_else(|| uniq.first().cloned())
            .unwrap_or_default();
        let reading = readings
            .get(i)
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty() && uniq.contains(r))
            .unwrap_or(default);
        let tokens = if reading.is_empty() {
            Vec::new()
        } else {
            split_syllable(&reading)
        };
        let missing: Vec<String> = if tokens.is_empty() {
            vec![c.to_string()]
        } else {
            tokens
                .iter()
                .filter(|t| !vocab.contains(t.as_str()))
                .cloned()
                .collect()
        };
        if !missing.is_empty() {
            out.problems.push(format!(
                "“{c}”（{}）唤醒模型不认识，换一个字或换个读音",
                if reading.is_empty() {
                    "没有拼音"
                } else {
                    &reading
                }
            ));
        }
        out.tokens.extend(tokens.iter().cloned());
        out.syllables.push(SyllableCheck {
            character: c.to_string(),
            reading,
            choices: uniq,
            tokens,
            missing,
        });
    }
    let mut seen: Vec<&str> = Vec::new();
    for s in &out.syllables {
        if s.choices.len() > 1 && !seen.contains(&s.character.as_str()) {
            seen.push(&s.character);
        }
    }
    if !seen.is_empty() {
        out.warnings.push(format!(
            "{} 是多音字，确认一下读音",
            seen.iter()
                .map(|c| format!("“{c}”"))
                .collect::<Vec<_>>()
                .join("")
        ));
    }
    if ["你好", "好的", "可以", "我们", "开始"].contains(&text.as_str()) {
        out.problems
            .push("这是日常常说的话，容易误唤醒，换一个".into());
    }
    out.ok = out.problems.is_empty();
    out
}

/// 关键词行（给 KWS 的 `keywords_buf`，一行一个）。标签不能有空白和 `/`。
pub fn keyword_line(tokens: &[String], label: &str) -> String {
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_whitespace() || c == '/' {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{} @{label}", tokens.join(" "))
}

/// 角色名对应的默认唤醒词：两个字的名字重复两遍（“琴音琴音”），三个字以上直接用名字，
/// 一个字的加“小”再重复。名字里有非汉字时返回 None（得自己填）。
pub fn default_for_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_whitespace()).collect();
    if name.is_empty() || !name.chars().all(is_cjk) {
        return None;
    }
    Some(match name.chars().count() {
        1 => format!("小{name}小{name}"),
        2 => format!("{name}{name}"),
        _ => name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> String {
        let c = check(text, &[]);
        assert!(c.ok, "{text}: {:?}", c.problems);
        c.tokens.join(" ")
    }

    #[test]
    fn matches_sherpa_onnx_text2token_reference_outputs() {
        // 参考输出：sherpa-onnx text2token(ppinyin) + pypinyin 0.55.0（研究记录 target/research）。
        assert_eq!(tokens("琴音琴音"), "q ín y īn q ín y īn");
        assert_eq!(tokens("你好琴音"), "n ǐ h ǎo q ín y īn");
        assert_eq!(tokens("小爱同学"), "x iǎo ài t óng x ué");
        assert_eq!(tokens("你好问问"), "n ǐ h ǎo w èn w èn");
        assert_eq!(tokens("小月小月"), "x iǎo y uè x iǎo y uè");
        assert_eq!(tokens("你好小雪"), "n ǐ h ǎo x iǎo x uě");
        assert_eq!(tokens("乌云乌云"), "w ū y ún w ū y ún");
        assert_eq!(split_syllable("lǜ"), ["l", "ǜ"]);
        assert_eq!(split_syllable("lüè"), ["l", "üè"]);
        assert_eq!(split_syllable("ān"), ["ān"]);
        assert_eq!(split_syllable("ér"), ["ér"]);
        // 预置唤醒词与这里算的一致
        for (_, label, expected) in crate::wake::PHRASES {
            assert_eq!(tokens(label), *expected);
        }
    }

    #[test]
    fn polyphones_offer_choices_and_take_the_users_reading() {
        let default = check("银行银行", &[]);
        assert!(default.ok);
        assert!(default.syllables[1].choices.contains(&"háng".to_string()));
        assert!(default.warnings.iter().any(|w| w.contains("多音字")));
        let chosen = check(
            "银行银行",
            &["yín".into(), "háng".into(), "".into(), "háng".into()],
        );
        assert_eq!(chosen.syllables[1].reading, "háng");
        assert_eq!(chosen.syllables[3].reading, "háng");
        // 不是这个字的读音：不采用
        let bogus = check("琴音琴音", &["zhāng".into()]);
        assert_eq!(bogus.syllables[0].reading, "qín");
    }

    #[test]
    fn rejects_what_the_model_cannot_spot() {
        assert!(!check("", &[]).ok);
        assert!(!check("琴音2号", &[]).ok);
        assert!(!check("Kotone", &[]).ok);
        assert!(!check("琴音", &[]).ok, "两个字太短");
        assert!(check("小琴音", &[]).ok);
        assert!(check("小琴音", &[])
            .warnings
            .iter()
            .any(|w| w.contains("3 个字")));
        // 模型不认的读音（轻声的 -ing 等不在 tokens.txt 里）
        let neutral = check(
            "东西东西",
            &["dōng".into(), "xi".into(), "dōng".into(), "xi".into()],
        );
        assert!(neutral.ok, "轻声 i 在词表里：{:?}", neutral.problems);
        assert!(vocabulary().contains("ín") && !vocabulary().contains("ing"));
        assert!(!check("一二三四五六七八九", &[]).ok, "太长");
    }

    #[test]
    fn keyword_lines_and_default_names() {
        assert_eq!(
            keyword_line(&["x".into(), "iǎo".into()], "小 雨/x"),
            "x iǎo @小_雨_x"
        );
        assert_eq!(default_for_name("琴音").as_deref(), Some("琴音琴音"));
        assert_eq!(default_for_name("小雨").as_deref(), Some("小雨小雨"));
        assert_eq!(default_for_name("林").as_deref(), Some("小林小林"));
        assert_eq!(default_for_name("星野花").as_deref(), Some("星野花"));
        assert_eq!(default_for_name("Mia"), None);
    }
}
