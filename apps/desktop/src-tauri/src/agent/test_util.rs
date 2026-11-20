//! 测试用的消息构造。

use super::*;
use kotone_ai::agent::ToolCall;

pub(super) fn user(source: &str) -> Message {
    Message::User {
        text: "嗨".into(),
        attachments: Vec::new(),
        source: source.into(),
        at_ms: 0,
        context: String::new(),
    }
}

pub(super) fn assistant(text: &str, calls: &[&str]) -> Message {
    Message::Assistant {
        text: text.into(),
        tool_calls: calls
            .iter()
            .map(|name| ToolCall {
                id: "c".into(),
                name: (*name).into(),
                arguments: "{}".into(),
            })
            .collect(),
        reasoning: String::new(),
        usage: None,
        at_ms: 0,
    }
}

pub(super) fn heard(text: &str) -> Message {
    Message::User {
        text: "（听到）".into(),
        attachments: vec![Attachment::Data {
            label: "听到的话".into(),
            text: text.into(),
        }],
        source: "heard".into(),
        at_ms: 0,
        context: String::new(),
    }
}
