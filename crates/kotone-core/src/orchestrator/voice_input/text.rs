use crate::orchestrator::*;
use crate::postprocess::{PostProcessingConfig, ProcessingContext};

/// 已完成全部文本生产步骤、可安全进入预览或注入的文本。
/// `source_text` 保留识别原文，`text` 承载后处理完成的可交付文本。
#[derive(Clone)]
pub(crate) struct ReadyText {
    pub(super) source_text: String,
    pub(super) text: String,
}

impl ReadyText {
    pub(crate) fn unchanged(text: String) -> Self {
        Self {
            source_text: text.clone(),
            text,
        }
    }
}

/// STT 已完成、但尚未完成后处理的文本与本轮不可变 pipeline/context 快照。
#[derive(Clone)]
pub(crate) struct RecognizedText {
    pub(crate) source_text: String,
    pub(super) pipeline: PostProcessingConfig,
    pub(super) context: ProcessingContext,
}

/// 状态机中待用户/系统继续处置的类型化文本，替代语义过载的 `preview_text`。
/// Recognized 只能重试处理，预览与注入只接受 Ready。
#[derive(Clone)]
pub(crate) enum PendingText {
    Recognized(RecognizedText),
    Ready(ReadyText),
    /// 助手预览保留识别完成时间，只能提交 AI，不能转换为可注入文本。
    Assistant(AssistantRequest),
}

impl PendingText {
    /// 出错但保留文字：Error 态可重试发送（不再经过文字处理）。
    pub(crate) fn retryable(text: String) -> Self {
        Self::Ready(ReadyText::unchanged(text))
    }

    pub(super) fn ready(&self) -> Option<&ReadyText> {
        match self {
            Self::Recognized(_) | Self::Assistant(_) => None,
            Self::Ready(ready) => Some(ready),
        }
    }

    pub(crate) fn recognized(&self) -> Option<&RecognizedText> {
        match self {
            Self::Recognized(recognized) => Some(recognized),
            Self::Ready(_) | Self::Assistant(_) => None,
        }
    }
}
