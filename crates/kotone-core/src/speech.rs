//! 本机语音播报端口。与识别引擎和文本注入分离；实现不得把文本解释为 XML。

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechVoice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SpeechRequest {
    pub text: String,
    /// None / 空串使用所选实现的默认语音。
    pub voice_id: Option<String>,
    pub volume: u16,
    pub rate: i32,
}

impl SpeechRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("播报文本不能为空".into());
        }
        if self.text.contains('\0') {
            return Err("播报文本不能包含空字符".into());
        }
        if self.text.chars().count() > 300 {
            return Err("播报文本不能超过 300 字".into());
        }
        if self.volume > 100 {
            return Err("播报音量应为 0 到 100".into());
        }
        if !(-10..=10).contains(&self.rate) {
            return Err("播报语速应为 -10 到 10".into());
        }
        Ok(())
    }
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

pub trait SpeechOutput: Send + Sync {
    fn voices(&self) -> Result<Vec<SpeechVoice>, String>;
    /// 在最终麦克风状态检查之前领取票据，不能在延迟执行的工作线程里重新领取。
    fn generation(&self) -> u64;
    /// 有界排队，返回时本条已经播完、失败或取消。请从阻塞工作线程调用。
    fn speak(&self, request: SpeechRequest) -> Result<(), String> {
        self.speak_if_current(request, self.generation())
    }
    /// 只有票据仍有效才入队；cancel 必须使已领取但尚未入队的票据一起失效。
    fn speak_if_current(
        &self,
        request: SpeechRequest,
        expected_generation: u64,
    ) -> Result<(), String>;
    /// 取消当前及已经排队的播报。识别开始前调用，避免把播报录回麦克风。
    fn cancel(&self) -> Result<(), String>;
    /// 先在后台合成并缓存、不播放（例如唤醒应答“嗯？/我在”，用时立刻出声）。
    /// 不等结果；不缓存的引擎（系统语音本来就即时出声）什么也不做。
    fn prefetch(&self, _request: SpeechRequest) {}
    /// 只合成、不播放，返回声音数据（单声道 f32，采样率）。用于把一句话送进游戏语音
    /// （ADR 020）；合成结果进缓存，紧接着 `speak` 同一句会立刻出声。系统语音直接出声、
    /// 拿不到数据，返回错误。阻塞，从工作线程调用。
    fn synthesize(&self, _request: SpeechRequest) -> Result<(Vec<f32>, u32), String> {
        Err("当前的语音引擎（系统语音）不能送进游戏语音：请在「琴音 → 交互 → 声音与弹幕」换成在线的声音或 Kokoro".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_audio_parameters_and_embedded_nul() {
        let mut request = SpeechRequest {
            text: "提醒".into(),
            voice_id: None,
            volume: 80,
            rate: 0,
        };
        assert!(request.validate().is_ok());
        request.volume = 101;
        assert!(request.validate().is_err());
        request.volume = 80;
        request.rate = 11;
        assert!(request.validate().is_err());
        request.rate = 0;
        request.text = "一\0二".into();
        assert!(request.validate().is_err());
    }
}
