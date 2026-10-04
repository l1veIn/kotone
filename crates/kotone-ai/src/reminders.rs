//! AI produces a bounded reminder plan. Only core may commit that plan after
//! checking cancellation/session ownership; no provider response executes code.

use std::sync::{Arc, RwLock};

use kotone_core::assistant::{AssistantFuture, AssistantHandler, AssistantPlan, AssistantRequest};
use kotone_core::cancel::CancellationToken;
use kotone_core::connection::ConnectionResolver;
use kotone_core::reminders::{DeliveryKind, ReminderIntent};
use kotone_core::settings::Settings;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct ReminderAssistant {
    client: reqwest::Client,
    connections: Arc<dyn ConnectionResolver>,
    settings: Arc<RwLock<Settings>>,
}

impl ReminderAssistant {
    pub fn new(connections: Arc<dyn ConnectionResolver>, settings: Arc<RwLock<Settings>>) -> Self {
        Self {
            client: reqwest::Client::new(),
            connections,
            settings,
        }
    }
}

impl AssistantHandler for ReminderAssistant {
    fn handle<'a>(
        &'a self,
        request: AssistantRequest,
        cancel: CancellationToken,
    ) -> AssistantFuture<'a> {
        Box::pin(async move {
            let config = self.settings.read().unwrap().assistant.clone();
            if config.connection_id.trim().is_empty() {
                return Err("请先在「提醒助手」选择支持工具调用的 AI 连接".into());
            }
            if request.text.trim().is_empty() || request.text.chars().count() > 2_000 {
                return Err("提醒指令为空或过长，请说一句简短的提醒".into());
            }
            let connection = self.connections.resolve(&config.connection_id)?;
            if connection
                .api_key
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                return Err("提醒助手的 AI 连接尚未保存密钥".into());
            }
            let defaults = config
                .default_delivery
                .iter()
                .map(|kind| match kind {
                    DeliveryKind::Voice => "voice_reminder",
                    DeliveryKind::Danmaku => "danmaku_reminder",
                })
                .collect::<Vec<_>>()
                .join(", ");
            let prompt = format!(
                // Persona source: .repochan/persona/current.json. The persona
                // changes delivery wording, never timing, game facts or scope.
                "你是琴音（Kotone），屏幕前用户的个人助手，表达温暖、轻快、简短。只处理用户明确提出的相对时间提醒。\n\
                 提醒的听众和接收者始终是正在使用 Kotone 的用户本人，不是队友或其他第三人；不要称呼用户为'队友'。\n\
                 可调用 voice_reminder（语音播报）和 danmaku_reminder（全屏弹幕）两个工具。\n\
                 将秒/分钟/小时换算为 delaySeconds，范围 1 到 604800；text 是到时提醒的简短内容，最多300字。\n\
                 用户未指定提醒方式时默认调用：{defaults}；指定方式时遵从用户。\n\
                 同一个提醒的两种方式必须使用完全相同的 delaySeconds 和 text。一次最多8个不同提醒。\n\
                 提醒text要适合到时直接对用户本人念出，只说简短正文，温暖通过自然措辞体现，不套称呼。\n\
                 保留用户给出的英雄、技能、对象、数量、否定和行动事实，不改动时间，不自行补充战术或倒计时。\n\
                 不要替用户向别人喊话或号召队伍；不要添加用户没有请求的'准备集合'、'提前占位'等战术指导，也不要承诺代替用户操作。\n\
                 不要用'提醒你'、'提醒一下'、'队友'、'主人'等机械前缀开场，不自报姓名。\n\
                 不要加入表情符号、Markdown、舞台动作；不要为了卖萌添加语气词或口头禅。\n\
                 用户要求原文时原样保留，仍作为念给用户本人的内容；其他情况也不额外添加开场白，不能改写事实。\n\
                 例如5分钟后提醒我打小龙：delaySeconds=300，text=该打小龙了。\n\
                 例如5分钟后提醒我打野有闪：delaySeconds=300，text=打野有闪了。\n\
                 例如用户说'30秒后原样提醒我：不要进河道'：delaySeconds=30，text=\"不要进河道\"。\n\
                 时间不明确、绝对时刻、循环提醒或非提醒请求，请用简短中文解释/询问，不调用工具。\n\
                 不猜测游戏状态，不自行添加提醒，不声称已经设置成功；只有应用提交后才算创建。"
            );
            let mut body = json!({
                "model": connection.model,
                "messages": [
                    {"role":"system", "content":prompt},
                    {"role":"user", "content":request.text}
                ],
                "tools": tools_schema(),
                "tool_choice":"auto",
                "temperature":0.0,
                "max_tokens":2048
            });
            if connection.provider.starts_with("dashscope") {
                body["enable_thinking"] = json!(false);
            }
            let response = crate::transport::chat_json(
                &self.client,
                &connection.base_url,
                connection.api_key.as_deref(),
                body,
                &cancel,
            )
            .await
            .map_err(|e| e.to_string())?;
            if cancel.is_cancelled() {
                return Err("提醒指令已取消".into());
            }
            parse_plan(&response)
        })
    }
}

fn tools_schema() -> Value {
    json!([
        tool(
            "voice_reminder",
            "在指定秒数后用语音提醒正在使用 Kotone 的用户本人"
        ),
        tool(
            "danmaku_reminder",
            "在指定秒数后用全屏弹幕提醒正在使用 Kotone 的用户本人"
        )
    ])
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "type":"function",
        "function": {
            "name":name, "description":description,
            "parameters": {
                "type":"object", "additionalProperties":false,
                "properties": {
                    "delaySeconds":{"type":"integer","minimum":1,"maximum":604800},
                    "text":{"type":"string","minLength":1,"maxLength":300}
                },
                "required":["delaySeconds","text"]
            }
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arguments {
    delay_seconds: u64,
    text: String,
}

fn parse_plan(response: &Value) -> Result<AssistantPlan, String> {
    let choice = response.pointer("/choices/0").ok_or("AI 没有返回结果")?;
    if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
        return Err("AI 回答被截断，请把提醒拆成更短的指令".into());
    }
    let message = choice.get("message").ok_or("AI 没有返回消息")?;
    let calls = match message.get("tool_calls") {
        Some(Value::Array(calls)) => calls.as_slice(),
        None | Some(Value::Null) => &[],
        _ => return Err("AI 工具调用格式不正确".into()),
    };
    if calls.len() > 16 {
        return Err("一次最多设置8个提醒".into());
    }
    let mut reminders: Vec<ReminderIntent> = Vec::new();
    for call in calls {
        if call.get("type").and_then(Value::as_str) != Some("function") {
            return Err("AI 返回了不支持的工具调用".into());
        }
        let function = call.get("function").ok_or("AI 工具调用缺少函数")?;
        let delivery = match function.get("name").and_then(Value::as_str) {
            Some("voice_reminder") => DeliveryKind::Voice,
            Some("danmaku_reminder") => DeliveryKind::Danmaku,
            _ => return Err("AI 返回了未授权的提醒工具".into()),
        };
        let arguments = function
            .get("arguments")
            .and_then(Value::as_str)
            .ok_or("提醒参数格式不正确")?;
        let mut args: Arguments =
            serde_json::from_str(arguments).map_err(|_| "提醒参数无效，请重新描述时间与内容")?;
        args.text = args.text.trim().to_string();
        if !(1..=604_800).contains(&args.delay_seconds)
            || args.text.is_empty()
            || args.text.chars().count() > 300
        {
            return Err("提醒必须在1秒到7天内，内容不超过300字".into());
        }
        if let Some(existing) = reminders
            .iter_mut()
            .find(|r| r.delay_seconds == args.delay_seconds && r.text == args.text)
        {
            if !existing.delivery.contains(&delivery) {
                existing.delivery.push(delivery);
            }
        } else {
            reminders.push(ReminderIntent {
                delay_seconds: args.delay_seconds,
                text: args.text,
                delivery: vec![delivery],
            });
        }
    }
    if reminders.len() > 8 {
        return Err("一次最多设置8个提醒".into());
    }
    let message = if reminders.is_empty() {
        message
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("没有创建提醒，请说明多久之后提醒什么；所选模型需要支持工具调用")
            .chars()
            .take(300)
            .collect()
    } else {
        format!("已设置 {} 个提醒", reminders.len())
    };
    Ok(AssistantPlan { message, reminders })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(name: &str, delay: Value, text: &str) -> Value {
        json!({"type":"function","function":{"name":name,"arguments":json!({"delaySeconds":delay,"text":text}).to_string()}})
    }
    fn response(calls: Vec<Value>) -> Value {
        json!({"choices":[{"message":{"tool_calls":calls}}]})
    }

    #[test]
    fn two_tools_merge_into_one_reminder_without_duplicate_delivery() {
        let plan = parse_plan(&response(vec![
            call("voice_reminder", json!(300), "打野有闪"),
            call("danmaku_reminder", json!(300), "打野有闪"),
            call("voice_reminder", json!(300), "打野有闪"),
        ]))
        .unwrap();
        assert_eq!(plan.reminders.len(), 1);
        assert_eq!(plan.reminders[0].delay_seconds, 300);
        assert_eq!(plan.reminders[0].delivery.len(), 2);
    }

    #[test]
    fn reject_unknown_tool_or_invalid_delay_atomically() {
        for bad in [
            call("execute_shell", json!(120), "小龙"),
            call("voice_reminder", json!(-1), "小龙"),
            call("voice_reminder", json!(0), "小龙"),
            call("voice_reminder", json!(1.5), "小龙"),
            call("voice_reminder", json!(604801), "小龙"),
        ] {
            assert!(parse_plan(&response(vec![
                call("voice_reminder", json!(120), "小龙"),
                bad
            ]))
            .is_err());
        }
    }

    #[test]
    fn clarification_never_creates_a_timer() {
        let plan =
            parse_plan(&json!({"choices":[{"message":{"content":"请问多久之后提醒？"}}]})).unwrap();
        assert!(plan.reminders.is_empty());
        assert_eq!(plan.message, "请问多久之后提醒？");
    }

    #[test]
    fn reminder_delivery_preserves_self_directed_wording_and_exact_quotes() {
        for (delay, text) in [
            (300, "该打小龙了。"),
            (300, "打野有闪了。"),
            (30, "不要进河道"),
            (120, "该喝水了。"),
        ] {
            let plan = parse_plan(&response(vec![
                call("voice_reminder", json!(delay), text),
                call("danmaku_reminder", json!(delay), text),
            ]))
            .unwrap();
            assert_eq!(plan.reminders.len(), 1);
            assert_eq!(plan.reminders[0].delay_seconds, delay);
            assert_eq!(plan.reminders[0].text, text);
            assert_eq!(
                plan.reminders[0].delivery,
                vec![DeliveryKind::Voice, DeliveryKind::Danmaku]
            );
        }
    }

    struct TestConnections {
        base_url: String,
    }

    impl ConnectionResolver for TestConnections {
        fn resolve(&self, id: &str) -> Result<kotone_core::connection::ResolvedConnection, String> {
            assert_eq!(id, "test-connection");
            Ok(kotone_core::connection::ResolvedConnection {
                id: id.into(),
                display_name: "test".into(),
                kind: kotone_core::connection::ConnectionKind::Remote,
                provider: "dashscope".into(),
                base_url: self.base_url.clone(),
                model: "test-planner".into(),
                api_key: Some("test-key".into()),
            })
        }
    }

    fn handler(base_url: String) -> ReminderAssistant {
        let mut settings = Settings::default();
        settings.assistant.connection_id = "test-connection".into();
        ReminderAssistant::new(
            Arc::new(TestConnections { base_url }),
            Arc::new(RwLock::new(settings)),
        )
    }

    fn request() -> AssistantRequest {
        AssistantRequest {
            session_id: "test-session".into(),
            text: "五分钟后提醒我打野有闪".into(),
            received_at_ms: 123_000,
        }
    }

    #[tokio::test]
    async fn handler_sends_only_bounded_tool_schema_and_returns_a_merged_plan() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response(vec![
                call("voice_reminder", json!(300), "打野有闪了。"),
                call("danmaku_reminder", json!(300), "打野有闪了。"),
            ])))
            .expect(1)
            .mount(&server)
            .await;
        let plan = handler(format!("{}/v1/", server.uri()))
            .handle(request(), CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(plan.reminders.len(), 1);
        assert_eq!(plan.reminders[0].delay_seconds, 300);
        assert_eq!(plan.reminders[0].text, "打野有闪了。");
        assert_eq!(
            plan.reminders[0].delivery,
            vec![DeliveryKind::Voice, DeliveryKind::Danmaku]
        );
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "test-planner");
        let instructions = body["messages"][0]["content"].as_str().unwrap();
        assert!(instructions.contains("屏幕前用户的个人助手"));
        assert!(instructions.contains("不要替用户向别人喊话或号召队伍"));
        assert!(instructions.contains("不自行补充战术"));
        assert!(instructions.contains("用户要求原文时原样保留"));
        assert!(instructions.contains("text=该打小龙了。"));
        assert!(instructions.contains("text=打野有闪了。"));
        assert!(instructions.contains("不额外添加开场白"));
        assert!(instructions.contains("不要用'提醒你'、'提醒一下'、'队友'、'主人'等机械前缀开场"));
        assert_eq!(body["messages"][1]["content"], request().text);
        assert_eq!(body["tools"].as_array().unwrap().len(), 2);
        assert_eq!(body["tools"][0]["function"]["name"], "voice_reminder");
        assert_eq!(body["tools"][1]["function"]["name"], "danmaku_reminder");
        assert_eq!(body["enable_thinking"], false);
    }

    #[tokio::test]
    async fn cancelled_in_flight_handler_does_not_return_a_late_reminder_plan() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(2))
                    .set_body_json(response(vec![call(
                        "voice_reminder",
                        json!(300),
                        "打野有闪",
                    )])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let handler = handler(server.uri());
        let cancel = CancellationToken::default();
        let task_token = cancel.clone();
        let task = tokio::spawn(async move { handler.handle(request(), task_token).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().contains("取消"));
    }
}
