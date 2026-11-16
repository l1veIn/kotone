//! 用真实的 ToolService（假桌面、假宿主）测工具清单、查找、权限与执行路由。
//! 不访问真实的 ~/.kotone、凭据库或任何 AI 接口。

use std::sync::{Arc, Mutex};

use kotone_ai::agent::{ToolOutcome, ToolPort};
use kotone_core::automation::library::{
    Automation, AutomationBody, CreatedBy, ParamKind, ParamSpec, ProgramBody,
};
use kotone_core::automation::SpeakAction;
use kotone_core::cancel::CancellationToken;
use kotone_core::reminders::DeliveryKind;
use kotone_tools::testing::{FakeAppHost, FakeDesktop, ManualClock};
use kotone_tools::{ServiceDeps, ServiceLimits, ToolService};
use serde_json::{json, Value};

use crate::catalog::{saved_entry, Catalog, Source, CORE_TOOLS};
use crate::permissions::{default_decision, Decision, Grants, Mode, Policy, Rule};
use crate::prompt::{system_prompt, Entry, PromptContext};
use crate::{Answer, Asker, BuiltinHost, KotonePort, PermissionRequest, Session};

// ---------- 假实现 ----------

fn service() -> (Arc<ToolService>, Arc<FakeAppHost>) {
    let desktop = Arc::new(FakeDesktop::default());
    let host = FakeAppHost::new();
    let service = ToolService::new(ServiceDeps {
        instance_id: "inst-test".into(),
        app_version: "0.0.0-test".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: ManualClock::at(0),
        ocr_backends: vec![],
        host: host.clone(),
    });
    (service, host)
}

#[derive(Default, Clone)]
struct FakeBuiltin {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl BuiltinHost for FakeBuiltin {
    async fn say(&self, text: String, delivery: Vec<DeliveryKind>) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push(("say".into(), json!({"text": text, "n": delivery.len()})));
        Ok(json!({"delivered": ["speech"]}))
    }

    async fn remind(
        &self,
        delay_seconds: u64,
        text: String,
        _delivery: Vec<DeliveryKind>,
    ) -> Result<Value, String> {
        self.calls.lock().unwrap().push((
            "remind".into(),
            json!({"delay": delay_seconds, "text": text}),
        ));
        Ok(json!({"id": "run-1", "status": "scheduled"}))
    }

    async fn send_to_game_chat(&self, text: String, send: Option<bool>) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push(("chat".into(), json!({"text": text, "send": send})));
        Ok(json!({"id": "run-2", "sent": true}))
    }
}

struct FakeAsker {
    answer: Answer,
    asked: Arc<Mutex<Vec<PermissionRequest>>>,
}

impl FakeAsker {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            asked: Arc::default(),
        }
    }
}

impl Asker for FakeAsker {
    async fn ask(&self, request: PermissionRequest, _cancel: &CancellationToken) -> Answer {
        self.asked.lock().unwrap().push(request);
        self.answer
    }
}

fn saved(id: &str, params: Vec<ParamSpec>) -> Automation {
    Automation {
        id: id.into(),
        name: "连按Q".into(),
        description: "每隔一段时间按一次 Q".into(),
        params,
        action: AutomationBody::Program(ProgramBody {
            source: "key('Q')".into(),
        }),
        created_by: CreatedBy::User,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

fn param(name: &str, kind: ParamKind, default: Option<Value>) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        label: String::new(),
        description: String::new(),
        kind,
        default,
    }
}

struct Fixture {
    port: KotonePort<FakeBuiltin, FakeAsker>,
    host: Arc<FakeAppHost>,
    builtin: FakeBuiltin,
    asked: Arc<Mutex<Vec<PermissionRequest>>>,
}

fn fixture(policy: Policy, answer: Answer) -> Fixture {
    let (service, host) = service();
    let specs = service.capabilities()["tools"].as_array().cloned().unwrap();
    let interval = param(
        "interval",
        ParamKind::Number {
            min: Some(0.1),
            max: Some(10.0),
            integer: false,
        },
        Some(json!(1)),
    );
    let catalog = Catalog::build(&specs, &[saved("auto-1", vec![interval])]);
    let builtin = FakeBuiltin::default();
    let asker = FakeAsker::new(answer);
    let asked = asker.asked.clone();
    Fixture {
        port: KotonePort::new(
            service,
            catalog,
            policy,
            Arc::new(Session::default()),
            builtin.clone(),
            asker,
        ),
        host,
        builtin,
        asked,
    }
}

async fn call(port: &KotonePort<FakeBuiltin, FakeAsker>, name: &str, args: Value) -> ToolOutcome {
    port.call("call-1", name, args, &CancellationToken::default())
        .await
}

fn names(port: &KotonePort<FakeBuiltin, FakeAsker>) -> Vec<String> {
    port.definitions().into_iter().map(|d| d.name).collect()
}

// ---------- 清单 ----------

#[test]
fn catalog_offers_core_tools_and_hides_external_channels() {
    let f = fixture(Policy::default(), Answer::Deny);
    let all: Vec<&str> = f
        .port
        .catalog()
        .entries()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    for hidden in [
        "agent.reply",
        "event.read",
        "assistant.submit",
        "companion.ask",
        "companion.type",
        "system.capabilities",
    ] {
        assert!(!all.contains(&hidden), "{hidden} must not reach the agent");
    }
    assert!(all.contains(&"settings.update"));
    let offered = names(&f.port);
    for core in CORE_TOOLS {
        assert!(
            offered.contains(&core.to_string()),
            "{core} should be offered"
        );
    }
    assert!(offered.contains(&"saved.auto-1".to_string()));
    assert!(!offered.contains(&"settings.update".to_string()));
    // 每个工具的参数顶层都是 object。
    for def in f.port.definitions() {
        assert_eq!(def.parameters["type"], "object", "{}", def.name);
    }
}

#[tokio::test]
async fn tools_find_lists_groups_then_enables_matches() {
    let f = fixture(Policy::default(), Answer::Deny);
    let groups: Value =
        serde_json::from_str(&call(&f.port, "tools.find", json!({})).await.content).unwrap();
    assert!(groups["groups"].as_array().unwrap().len() >= 5);
    assert!(!names(&f.port).contains(&"hotkeys.status".to_string()));

    let found = call(&f.port, "tools.find", json!({"query": "热键"})).await;
    assert!(!found.is_error);
    let offered = names(&f.port);
    assert!(
        offered.contains(&"hotkeys.status".to_string()),
        "{offered:?}"
    );
    assert!(!offered.contains(&"models.download".to_string()));

    call(&f.port, "tools.find", json!({"group": "models"})).await;
    assert!(names(&f.port).contains(&"models.download".to_string()));
}

#[test]
fn saved_automation_becomes_a_typed_tool() {
    let region = param("area", ParamKind::Region, None);
    let entry = saved_entry(&saved("auto-2", vec![region])).unwrap();
    assert_eq!(entry.name, "saved.auto-2");
    assert_eq!(
        entry.source,
        Source::Saved {
            id: "auto-2".into()
        }
    );
    assert_eq!(entry.parameters["required"], json!(["area"]));
    assert_eq!(entry.parameters["properties"]["area"]["type"], "object");
    assert!(entry.parameters["properties"]["delaySeconds"].is_object());
    // 模型给不出图片：没有缺省模板的自动化不交给模型。
    let image = param("template", ParamKind::Image, None);
    assert!(saved_entry(&saved("auto-3", vec![image])).is_none());
    let speak = Automation {
        action: AutomationBody::Speak(SpeakAction {
            text: "喝水".into(),
            delivery: vec![DeliveryKind::Voice],
        }),
        ..saved("auto-4", Vec::new())
    };
    assert!(saved_entry(&speak).unwrap().description.contains("播报"));
}

// ---------- 权限规则 ----------

fn entry<'a>(f: &'a Fixture, name: &str) -> &'a crate::catalog::ToolEntry {
    f.port.catalog().get(name).unwrap()
}

#[test]
fn default_rules_follow_side_effects() {
    let f = fixture(Policy::default(), Answer::Deny);
    let allow = [
        ("automation.list", json!({})),
        ("automation.run", json!({"id": "auto-1"})),
        ("automation.stop", json!({})),
        ("automation.save", json!({})),
        ("companion.read_chat", json!({})),
        ("reply.say", json!({})),
        ("reminder.create", json!({})),
        ("game_chat.send", json!({})),
        ("saved.auto-1", json!({})),
        ("settings.get", json!({})),
    ];
    for (name, args) in allow {
        assert_eq!(
            default_decision(entry(&f, name), &args),
            Decision::Allow,
            "{name}"
        );
    }
    let ask = [
        ("settings.update", json!({})),
        ("automation.delete", json!({})),
        ("automation.run", json!({"source": "key('Q')"})),
        ("models.download", json!({})),
        ("app.quit", json!({})),
        ("elevation.restart", json!({})),
        ("autostart.set", json!({})),
        ("history.clear", json!({})),
    ];
    for (name, args) in ask {
        assert!(
            matches!(
                default_decision(entry(&f, name), &args),
                Decision::Ask { .. }
            ),
            "{name}"
        );
    }
    // 不认识的副作用：询问。
    let mut odd = entry(&f, "automation.list").clone();
    odd.side_effects = vec!["launches-rockets".into()];
    assert!(matches!(
        default_decision(&odd, &json!({})),
        Decision::Ask { .. }
    ));
}

#[test]
fn modes_and_user_rules() {
    let f = fixture(Policy::default(), Answer::Deny);
    let update = entry(&f, "settings.update");
    let chat = entry(&f, "game_chat.send");
    let read = entry(&f, "companion.read_chat");

    let all = Policy {
        mode: Mode::AllowAll,
        ..Default::default()
    };
    assert_eq!(all.decide(update, &json!({})), Decision::Allow);
    let mut denied = all.clone();
    denied.rules.insert("settings.update".into(), Rule::Deny);
    assert!(matches!(
        denied.decide(update, &json!({})),
        Decision::Deny { .. }
    ));

    let ro = Policy {
        mode: Mode::ReadOnly,
        ..Default::default()
    };
    assert!(matches!(ro.decide(chat, &json!({})), Decision::Deny { .. }));
    assert_eq!(ro.decide(read, &json!({})), Decision::Allow);

    let mut ask_chat = Policy::default();
    ask_chat.rules.insert("game_chat.send".into(), Rule::Ask);
    assert!(matches!(
        ask_chat.decide(chat, &json!({})),
        Decision::Ask { .. }
    ));
}

#[test]
fn conversation_grants_for_new_programs_are_per_source() {
    let f = fixture(Policy::default(), Answer::Deny);
    let run = entry(&f, "automation.run");
    let policy = Policy::default();
    let grants = Grants::default();
    let a = json!({"source": "key('Q')"});
    let b = json!({"source": "key('E')"});
    crate::permissions::remember(&grants, run, &a);
    assert_eq!(policy.decide_with(run, &a, &grants), Decision::Allow);
    assert!(matches!(
        policy.decide_with(run, &b, &grants),
        Decision::Ask { .. }
    ));
}

// ---------- 执行路由 ----------

#[tokio::test]
async fn read_tools_run_through_the_tool_service_without_asking() {
    let f = fixture(Policy::default(), Answer::Deny);
    let out = call(&f.port, "automation.list", json!({})).await;
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(f.host.invocations()[0].0, "automation.list");
    assert!(f.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn risky_tools_ask_and_respect_the_answer() {
    let f = fixture(Policy::default(), Answer::Deny);
    let out = call(&f.port, "settings.update", json!({"patch": {}})).await;
    assert!(out.is_error && out.content.contains("拒绝"));
    assert!(f.host.invocations().is_empty(), "denied call must not run");
    let asked = f.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].call_id, "call-1");
    assert_eq!(asked[0].reason, "会修改设置");

    let f = fixture(Policy::default(), Answer::AllowForConversation);
    call(&f.port, "settings.update", json!({"patch": {}})).await;
    call(&f.port, "settings.update", json!({"patch": {}})).await;
    assert_eq!(
        f.asked.lock().unwrap().len(),
        1,
        "asked once per conversation"
    );
    assert_eq!(f.host.invocations().len(), 2);
}

#[tokio::test]
async fn read_only_mode_hides_and_refuses_writes() {
    let f = fixture(
        Policy {
            mode: Mode::ReadOnly,
            ..Default::default()
        },
        Answer::AllowOnce,
    );
    assert!(!names(&f.port).contains(&"game_chat.send".to_string()));
    assert!(names(&f.port).contains(&"companion.read_chat".to_string()));
    let out = call(&f.port, "game_chat.send", json!({"text": "gg"})).await;
    assert!(out.is_error && out.content.contains("只读"));
    assert!(f.builtin.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn saved_automations_run_through_automation_run() {
    let f = fixture(Policy::default(), Answer::Deny);
    let out = call(
        &f.port,
        "saved.auto-1",
        json!({"interval": 0.5, "delaySeconds": 30}),
    )
    .await;
    assert!(!out.is_error, "{}", out.content);
    let (tool, args) = f.host.invocations()[0].clone();
    assert_eq!(tool, "automation.run");
    assert_eq!(args["id"], "auto-1");
    assert_eq!(args["args"], json!({"interval": 0.5}));
    assert_eq!(args["delaySeconds"], 30);
}

#[tokio::test]
async fn builtins_validate_then_reach_the_host() {
    let f = fixture(Policy::default(), Answer::Deny);
    let out = call(&f.port, "game_chat.send", json!({"text": "gg\nwp"})).await;
    assert!(out.is_error);
    let out = call(
        &f.port,
        "game_chat.send",
        json!({"text": "我去下路", "send": false}),
    )
    .await;
    assert!(!out.is_error);
    let out = call(
        &f.port,
        "reminder.create",
        json!({"delaySeconds": 0, "text": "买装备"}),
    )
    .await;
    assert!(out.is_error);
    let out = call(
        &f.port,
        "reminder.create",
        json!({"delaySeconds": 300, "text": "买装备", "delivery": ["voice"]}),
    )
    .await;
    assert!(!out.is_error);
    let out = call(
        &f.port,
        "reply.say",
        json!({"text": "好", "delivery": ["shout"]}),
    )
    .await;
    assert!(out.is_error);
    let calls = f.builtin.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0],
        ("chat".into(), json!({"text": "我去下路", "send": false}))
    );
    assert_eq!(calls[1].1["delay"], 300);
}

#[tokio::test]
async fn screen_text_results_are_marked_untrusted() {
    let f = fixture(Policy::default(), Answer::Deny);
    let out = call(&f.port, "companion.read_chat", json!({})).await;
    assert!(out.untrusted);
    let out = call(&f.port, "automation.list", json!({})).await;
    assert!(!out.untrusted);
}

// ---------- 系统提示 ----------

#[test]
fn prompt_mentions_entry_game_notes_and_saved_automations() {
    let automations = [saved("auto-1", Vec::new())];
    let game = system_prompt(&PromptContext {
        entry: Entry::Game,
        profile_name: "英雄联盟",
        profile_notes: "聊天在左下角",
        automations: &automations,
    });
    assert!(game.contains("游戏内"));
    assert!(game.contains("聊天在左下角"));
    assert!(game.contains("saved.auto-1"));
    let chat = system_prompt(&PromptContext {
        entry: Entry::Chat,
        profile_name: "",
        profile_notes: "",
        automations: &[],
    });
    assert!(chat.contains("聊天窗口"));
    assert!(!chat.contains("游戏说明"));
}
