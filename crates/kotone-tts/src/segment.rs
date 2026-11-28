//! 流式分句（ADR 029 第六节“边生成边念”）：模型的文字一段段到，凑够一句就交给语音合成，
//! 不等整段回答写完。
//!
//! 规则照 xiaozhi-esp32-server（MIT）`core/providers/tts/base.py`：
//! - **第一段**可以在逗号、顿号处切（`，、,~`），让第一声尽快出来；之后只在句末标点
//!   （`。？！；：?!;:`）和换行处切；
//! - 不在引号、括号里切（`_quote_safe_cut`）；切点后面紧跟的标点和右引号一起带走；
//! - 送去合成前去掉 Markdown 记号和表情（`MarkdownCleaner`、`get_string_no_punctuation_or_emoji`）。
//!
//! 补充：
//! - 中文按字计长度（LiveKit Agents，Apache-2.0，`tokenize/blingfire.py`）：有效字数不足
//!   [`MIN_CHARS`] 的片段（“好，”）并到下一句，避免一个字一次请求；
//! - 半角标点 `?!;:,` 只有后面跟着空白或非 ASCII 字符时才算句读，`12:30`、`3,000`、
//!   `https://…` 不会被切开；还看不到后一个字时先等（流式输入下一段再判断）。CJK 句末标点
//!   本身无歧义，直接切（Pipecat，BSD-2，`utils/string.py`）。

/// 一句可以合成的话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// 模型原文（含标点与 Markdown）：打断时按它截取“已经念出来的部分”记进对话。
    pub raw: String,
    /// 送去合成的文字：去掉 Markdown 记号和表情、首尾空白。
    pub speak: String,
}

impl Sentence {
    /// 有效字数（不算标点、空白），按它估计这句的时长。
    pub fn weight(&self) -> usize {
        spoken_chars(&self.speak)
    }
}

/// 有效字数不足这么多的片段并到下一句。
pub const MIN_CHARS: usize = 2;

const FIRST_EXTRA: &[char] = &['，', '、', ',', '~', '～'];
const ENDERS: &[char] = &['。', '？', '！', '；', '：', '?', '!', ';', ':', '…'];
/// 只在后面跟着空白或非 ASCII 字符时才算句读的半角标点。
const ASCII_AMBIGUOUS: &[char] = &['?', '!', ';', ':', ','];
/// 成对的引号 / 括号：(开, 合)。前后同符的 `"` 按奇偶计。
const PAIRS: &[(char, char)] = &[
    ('“', '”'),
    ('‘', '’'),
    ('「', '」'),
    ('『', '』'),
    ('《', '》'),
    ('（', '）'),
    ('(', ')'),
    ('【', '】'),
    ('[', ']'),
    ('〈', '〉'),
];
const CLOSERS: &[char] = &['”', '’', '」', '』', '》', '）', ')', '】', ']', '〉', '"'];

/// 喂入流式文字、吐出整句。一次回答用一个。
#[derive(Debug, Default)]
pub struct Segmenter {
    pending: String,
    emitted_any: bool,
}

impl Segmenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一段模型输出，返回这次凑够的句子（可能为空）。
    pub fn push(&mut self, delta: &str) -> Vec<Sentence> {
        self.pending.push_str(delta);
        let mut out = Vec::new();
        while let Some(end) = self.find_cut(false) {
            let rest = self.pending.split_off(end);
            let raw = std::mem::replace(&mut self.pending, rest);
            out.push(make_sentence(raw));
            self.emitted_any = true;
        }
        out
    }

    /// 回答写完了：吐出剩下的（不足 [`MIN_CHARS`] 也吐，只要有字）。
    pub fn finish(&mut self) -> Option<Sentence> {
        let raw = std::mem::take(&mut self.pending);
        self.emitted_any = false;
        let sentence = make_sentence(raw);
        (sentence.weight() > 0).then_some(sentence)
    }

    /// 还没吐出去的原文。
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// 找下一个切点（字节偏移，切在它之前）。
    fn find_cut(&self, at_end: bool) -> Option<usize> {
        let chars: Vec<(usize, char)> = self.pending.char_indices().collect();
        let first = !self.emitted_any;
        let mut depth = [0i32; 16];
        let mut ascii_quote_open = false;
        let mut i = 0;
        while i < chars.len() {
            let (_, c) = chars[i];
            if c == '"' {
                ascii_quote_open = !ascii_quote_open;
            } else if let Some(k) = PAIRS.iter().position(|(open, _)| *open == c) {
                depth[k] += 1;
            } else if let Some(k) = PAIRS.iter().position(|(_, close)| *close == c) {
                depth[k] = (depth[k] - 1).max(0);
            }
            let quoted = ascii_quote_open || depth.iter().any(|d| *d > 0);
            let is_newline = c == '\n';
            let is_cut_char = ENDERS.contains(&c) || (first && FIRST_EXTRA.contains(&c));
            if !quoted && (is_newline || is_cut_char) {
                if ASCII_AMBIGUOUS.contains(&c) {
                    match chars.get(i + 1) {
                        // 还看不到后一个字：等下一段再判断（finish 时由 finish() 兜底）
                        None if !at_end => return None,
                        Some((_, next)) if !(next.is_whitespace() || !next.is_ascii()) => {
                            i += 1;
                            continue;
                        }
                        _ => {}
                    }
                }
                // 切点后面紧跟的标点、右引号一起带走（“好吗？！”、“好！”）
                let mut end = i + 1;
                while let Some((_, next)) = chars.get(end) {
                    if ENDERS.contains(next)
                        || CLOSERS.contains(next)
                        || (first && FIRST_EXTRA.contains(next))
                        || *next == '\n'
                    {
                        end += 1;
                    } else {
                        break;
                    }
                }
                // “……”、“？！”这类连着的标点可能还没写完：后面还看不到字时先等
                if end == chars.len() && !at_end && !is_newline && c == '…' {
                    return None;
                }
                let byte_end = chars.get(end).map_or(self.pending.len(), |(b, _)| *b);
                if spoken_chars(&clean(&self.pending[..byte_end])) >= MIN_CHARS {
                    return Some(byte_end);
                }
                // 太短（“好，”）：并到下一句，接着往后找
                i = end;
                continue;
            }
            i += 1;
        }
        None
    }
}

fn make_sentence(raw: String) -> Sentence {
    let speak = clean(&raw);
    Sentence { raw, speak }
}

/// 有效字数：去掉标点、空白和符号后的字符数。
pub fn spoken_chars(text: &str) -> usize {
    text.chars()
        .filter(|c| c.is_alphanumeric() && !is_emoji(*c))
        .count()
}

/// 送去合成前的清理：Markdown 记号、表情、多余空白（照 xiaozhi `MarkdownCleaner`），
/// 挨着中文的空格去掉（[`tighten`]）。
/// 句末标点保留（合成要靠它停顿），开头的标点去掉。
pub fn clean(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.split('\n') {
        let mut line = line.trim_start();
        // 标题、引用、列表记号
        line = line.trim_start_matches('#').trim_start();
        line = line.trim_start_matches('>').trim_start();
        for marker in ["- ", "* ", "+ ", "• "] {
            if let Some(rest) = line.strip_prefix(marker) {
                line = rest;
            }
        }
        let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && line[digits..].starts_with(". ") {
            line = &line[digits + 2..];
        }
        lines.push(strip_inline(line));
    }
    let joined = lines.join(" ");
    let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    tighten(&collapsed)
        .trim_start_matches(|c: char| !c.is_alphanumeric() && !is_open_quote(c))
        .trim()
        .to_string()
}

/// 念之前去掉挨着中文的空格：屏幕上的话按界面用语在数字、英文两边留半角空格
/// （“卡牌大师到 6 级了”“10 分 30 秒刷新”），合成会在空格处停顿，念成“到六……级了”。
/// 空白（连续的算一处）只要有一边是中日文字、全角标点或全角字符就去掉；两边都是
/// 英文、数字时保留（“Faker is back”照常念，“3 4”不会并成“34”）。重复调用结果不变。
pub fn tighten(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_whitespace() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let before = start.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i).copied();
        if !(before.is_some_and(is_cjk) || after.is_some_and(is_cjk)) {
            out.extend(&chars[start..i]);
        }
    }
    out
}

/// 中日文字、CJK 标点、全角字符（含全角标点）。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F     // CJK 标点（。、「」《》）和全角空格
        | 0x3040..=0x30FF   // 平假名、片假名
        | 0x3400..=0x4DBF   // 扩展 A
        | 0x4E00..=0x9FFF   // 基本汉字
        | 0xF900..=0xFAFF   // 兼容汉字
        | 0xFF00..=0xFFEF   // 全角 ASCII、全角标点（，！？：）
        | 0x20000..=0x3FFFF // 扩展 B 及以后
    )
}

fn is_open_quote(c: char) -> bool {
    PAIRS.iter().any(|(open, _)| *open == c) || c == '"'
}

/// 行内 Markdown：链接 / 图片留文字，去掉强调、行内代码、删除线、表格竖线；去掉表情。
fn strip_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // ![alt](url) / [text](url) → alt / text
        if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
            let start = if c == '!' { i + 2 } else { i + 1 };
            if let Some(close) = chars[start..].iter().position(|x| *x == ']') {
                let close = start + close;
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close + 2..].iter().position(|x| *x == ')') {
                        out.extend(&chars[start..close]);
                        i = close + 2 + end + 1;
                        continue;
                    }
                }
            }
        }
        match c {
            '*' | '`' | '|' => {}
            '~' if chars.get(i + 1) == Some(&'~') || (i > 0 && chars[i - 1] == '~') => {}
            '_' if chars.get(i + 1) == Some(&'_') || (i > 0 && chars[i - 1] == '_') => {}
            c if is_emoji(c) => {}
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// 表情与它们的修饰符（不念）。
fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // 各类表情、符号、扑克、旗帜、区域指示符
        | 0x2600..=0x27BF   // 杂项符号、装饰符号
        | 0x2B00..=0x2BFF   // 箭头与星形
        | 0xFE0F | 0xFE0E   // 变体选择符
        | 0x200D            // 零宽连接符
        | 0xE0020..=0xE007F // 标签字符
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(deltas: &[&str]) -> Vec<String> {
        let mut s = Segmenter::new();
        let mut out: Vec<String> = deltas
            .iter()
            .flat_map(|d| s.push(d))
            .map(|x| x.speak)
            .collect();
        if let Some(last) = s.finish() {
            out.push(last.speak);
        }
        out
    }

    #[test]
    fn first_segment_cuts_at_a_comma_later_ones_only_at_sentence_ends() {
        assert_eq!(
            feed(&["能听到，很清楚，", "有什么要我帮忙的？", "我在呢"]),
            vec!["能听到，", "很清楚，有什么要我帮忙的？", "我在呢"]
        );
    }

    #[test]
    fn streamed_one_character_at_a_time_gives_the_same_sentences() {
        let text = "好的，我来看看。队友说先别打龙！你要我回一句吗？";
        let deltas: Vec<String> = text.chars().map(|c| c.to_string()).collect();
        let refs: Vec<&str> = deltas.iter().map(String::as_str).collect();
        assert_eq!(
            feed(&refs),
            vec![
                "好的，",
                "我来看看。",
                "队友说先别打龙！",
                "你要我回一句吗？"
            ]
        );
    }

    #[test]
    fn too_short_pieces_merge_into_the_next_sentence() {
        assert_eq!(feed(&["好，我看看。"]), vec!["好，我看看。"]);
        assert_eq!(feed(&["嗯。", "这把能赢。"]), vec!["嗯。这把能赢。"]);
        // 只有一个字的回答也要念
        assert_eq!(feed(&["好"]), vec!["好"]);
    }

    #[test]
    fn never_cuts_inside_quotes_or_brackets() {
        assert_eq!(
            feed(&["他说：“先别打！我马上到。”然后就走了。"]),
            vec!["他说：", "“先别打！我马上到。”然后就走了。"]
        );
        assert_eq!(
            feed(&["打野（刚才说：先拿龙！）在下路。下一句。"]),
            vec!["打野（刚才说：先拿龙！）在下路。", "下一句。"]
        );
        assert_eq!(
            feed(&["书名《你好！世界》很好看。"]),
            vec!["书名《你好！世界》很好看。"]
        );
    }

    #[test]
    fn trailing_punctuation_and_closers_travel_with_the_sentence() {
        assert_eq!(
            feed(&["真的吗？！不会吧！"]),
            vec!["真的吗？！", "不会吧！"]
        );
        assert_eq!(
            feed(&["他说“好！”", "我们走吧。"]),
            vec!["他说“好！”我们走吧。"]
        );
    }

    #[test]
    fn ascii_punctuation_needs_a_following_space_or_cjk() {
        // 时间、千分位、网址里的半角标点不切
        assert_eq!(
            feed(&["12:30 开团，3,000 金币够了。看 https://op.gg/champions 就行。"]),
            vec![
                "12:30开团，",
                "3,000金币够了。",
                "看https://op.gg/champions就行。"
            ]
        );
        // 半角句读后跟着空格才切；还看不到后一个字时先等
        let mut s = Segmenter::new();
        assert!(s.push("Ready?").is_empty());
        assert_eq!(s.push(" Go!")[0].speak, "Ready?");
        assert_eq!(s.finish().unwrap().speak, "Go!");
    }

    #[test]
    fn newlines_end_a_sentence_and_markdown_and_emoji_are_not_spoken() {
        assert_eq!(
            feed(&["## 出装\n- **饮魔刀**：先做\n- 再出`三相`😀\n"]),
            vec!["出装", "饮魔刀：", "先做", "再出三相"]
        );
        assert_eq!(
            feed(&["看[这个攻略](https://example.com/a?b=1)吧！👍"]),
            vec!["看这个攻略吧！"]
        );
    }

    #[test]
    fn ellipsis_waits_for_what_follows() {
        let mut s = Segmenter::new();
        assert!(s.push("让我想想…").is_empty());
        assert_eq!(s.push("…好了，")[0].speak, "让我想想……");
        assert_eq!(s.finish().unwrap().speak, "好了，");
    }

    #[test]
    fn raw_text_keeps_the_original_for_truncation() {
        let mut s = Segmenter::new();
        let got = s.push("**好的**，我来。");
        assert_eq!(got[0].raw, "**好的**，");
        assert_eq!(got[0].speak, "好的，");
        assert_eq!(got[0].weight(), 2);
        let all: String = got
            .iter()
            .map(|x| x.raw.clone())
            .chain(s.finish().map(|x| x.raw))
            .collect();
        assert_eq!(all, "**好的**，我来。");
    }

    #[test]
    fn only_punctuation_or_emoji_produces_nothing() {
        assert!(feed(&["。。。"]).is_empty());
        assert!(feed(&["👍🎉"]).is_empty());
        assert!(feed(&[""]).is_empty());
    }

    #[test]
    fn spaces_next_to_chinese_are_not_spoken() {
        // 英雄联盟插件的推送原话（屏幕上留空格，念的时候不留）
        assert_eq!(tighten("卡牌大师到 6 级了"), "卡牌大师到6级了");
        assert_eq!(tighten("对面卡萨丁 16 级了"), "对面卡萨丁16级了");
        assert_eq!(tighten("大龙 10 分 30 秒刷新"), "大龙10分30秒刷新");
        assert_eq!(tighten("对面最快 12 秒复活"), "对面最快12秒复活");
        assert_eq!(tighten("对面 EZ 6 级了，小心"), "对面EZ 6级了，小心");
        // 全角标点、CJK 标点两边的空格也去掉
        assert_eq!(tighten("好了 ， 走吧 。"), "好了，走吧。");
        assert_eq!(tighten("「Faker」 回来了"), "「Faker」回来了");
        // 两边都是英文、数字：保留
        assert_eq!(tighten("Faker is back"), "Faker is back");
        assert_eq!(tighten("3 4"), "3 4");
        assert_eq!(tighten("12:30 3,000"), "12:30 3,000");
        // 连续空白算一处；重复调用不变
        assert_eq!(tighten("到  	 6 级"), "到6级");
        assert_eq!(tighten(&tighten("到 6 级")), "到6级");
        // 走 clean 也一样
        assert_eq!(clean("卡牌大师到 6 级了"), "卡牌大师到6级了");
    }

    #[test]
    fn mixed_english_and_chinese() {
        assert_eq!(
            feed(&["OK，这把 ADC 是 Jinx。她 late game 很强！"]),
            vec!["OK，", "这把ADC是Jinx。", "她late game很强！"]
        );
    }
}
