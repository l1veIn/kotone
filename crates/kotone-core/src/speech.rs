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
