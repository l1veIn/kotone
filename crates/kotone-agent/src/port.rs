//! 内置 Agent 的工具端口：[`kotone_ai::agent::ToolPort`] 的 Kotone 实现。
//!
//! 一次调用：查清单 → 权限（允许 / 询问 / 禁止）→ 按来源执行（工具服务 / 内置动作 /
//! 已保存的自动化 / `tools.find`）→ 结果文字（屏幕文字类标为不可信）。

use std::future::Future;
use std::sync::{Arc, Mutex};

use kotone_ai::agent::{Attachment, ToolDef, ToolOutcome, ToolPort};
use kotone_core::cancel::CancellationToken;
use kotone_core::reminders::DeliveryKind;
use kotone_tools::protocol::ToolCallRequest;
use kotone_tools::ToolService;
use serde_json::{json, Value};

use crate::catalog::{
    Catalog, Source, ToolEntry, GROUPS, HOST_BUILTINS, TOOL_ASK, TOOL_DANMAKU, TOOL_GAME_CHAT,
    TOOL_MEMORY, TOOL_REMIND, TOOL_SKILL_MANAGE, TOOL_SKILL_VIEW, TOOL_SOUL_WRITE, TOOL_SPEAK,
};
use crate::permissions::{
    self, Answer, Asker, Decide, Decision, Grants, PermissionRequest, Policy,
};
use crate::workspace::{MemoryOp, MemoryTarget, Workspace};

/// `tools.find` 一次最多列出几个工具。
const MAX_FIND: usize = 12;

/// 内置动作的桌面实现。
pub trait BuiltinHost: Send + Sync {
    /// 对玩家说一句（`speak`，ADR 029 第八节）：按回复方式念出来 / 显示在气泡、弹幕，说完（或被打断）才返回。
    /// `call_id`：这次调用（边生成边念时已经开始念的那一路靠它找到）。
    fn speak(
        &self,
        call_id: &str,
        text: String,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// 在屏幕上显示一句弹幕（`danmaku`，后台用）。
    fn danmaku(&self, text: String) -> impl Future<Output = Result<Value, String>> + Send;

    /// 用语音问玩家一句，再免唤醒收音 `wait_seconds` 秒，返回 `{answered, answer}`。
    fn ask(
        &self,
        question: String,
        wait_seconds: u64,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// 延迟播报（进自动化调度器，ADR 013“延迟 + 播报”）。
    fn remind(
        &self,
        delay_seconds: u64,
        text: String,
        delivery: Vec<DeliveryKind>,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// 打进当前游戏的聊天框，并写一条执行记录。`send` 为 None = 跟随用户设置。
    fn send_to_game_chat(
        &self,
        text: String,
        send: Option<bool>,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// 需要知道是哪个会话调用的内置工具（[`HOST_BUILTINS`]：`session.*`、`voice.listen.*`）。
    /// 桌面端按调用它的会话执行；没有桌面（测试、外部宿主）时不可用。
    fn builtin(
        &self,
        tool: &str,
        _arguments: &Value,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        let message = format!("{tool} 在这里不可用");
        async move { Err(message) }
    }

    /// 执行命令、读写文件（ADR 030，[`Source::Exec`]）。桌面端用 kotone-exec 执行：先按它的规则判断
    /// 要不要经主会话问玩家（读过外面的东西、删除类命令、第一次改游戏目录……），再执行，并记下
    /// 这次有没有读到外面的东西。`confirm`：玩家把这个工具设成了“每次询问”，无论规则怎么判都先问。
    /// 只在后台任务里可用；没有桌面（测试、外部宿主）时不可用。
    fn exec(
        &self,
        _call_id: &str,
        tool: &str,
        _arguments: &Value,
        _confirm: bool,
        _cancel: &CancellationToken,
    ) -> impl Future<Output = ToolOutcome> + Send {
        let message = format!("{tool} 在这里不可用");
        async move { ToolOutcome::error(message) }
    }

    /// 插件的工具（ADR 038，[`Source::Plugin`]）：桌面端交给插件执行，只有 `content` 给她，
    /// `structured_content` / `ui` 给聊天卡片。没有桌面（测试、外部宿主）时不可用。
    fn plugin(
        &self,
        tool: &str,
        _arguments: &Value,
    ) -> impl Future<Output = Result<kotone_plugin_api::ToolResult, String>> + Send {
        let message = format!("{tool} 在这里不可用");
        async move { Err(message) }
    }

    /// 玩家已经在别处一直允许了这次调用（例如游戏配置里“让琴音操作”设为一直允许）：
    /// 权限规则是“询问”时不再问。用户设为禁止、只读模式时仍然不执行。
    fn preapproved(&self, _tool: &str, _arguments: &Value) -> bool {
        false
    }
}

/// 工具服务调用的期限（毫秒）：多数用服务的默认值，一边看一边操作的要更久。
fn service_timeout_ms(tool: &str) -> Option<u64> {
    match tool {
        "game.look" => Some(20_000),
        "game.act" => Some(30_000),
        _ => None,
    }
}

/// 从工具结果里取出图片（`image: {mimeType, base64}`，`game.act` 的在 `look.image`），
/// 作为图片交给模型，结果文字里只留尺寸。
pub fn take_images(result: &mut Value) -> Vec<Attachment> {
    let mut out = Vec::new();
    let mut take = |holder: Option<&mut Value>, label: &str| {
        let Some(image) = holder.and_then(|h| h.get_mut("image")) else {
            return;
        };
        let mime = image
            .get("mimeType")
            .and_then(Value::as_str)
            .map(String::from);
        let data = image
            .get("base64")
            .and_then(Value::as_str)
            .map(String::from);
        if let (Some(mime), Some(data)) = (mime, data) {
            if let Some(object) = image.as_object_mut() {
                object.remove("base64");
                object.insert("attached".into(), json!(true));
            }
            out.push(Attachment::Image {
                label: label.to_string(),
                mime,
                data,
            });
        }
    };
    take(Some(result), "游戏截图");
    take(result.get_mut("look"), "操作后的游戏截图");
    out
}

/// 一段对话里跨轮保留的状态：“本段对话都允许”的授权，以及会话开始时冻结的东西。
///
/// 照 DeepSeek Harness：系统提示与工具清单在一段对话里**一个字都不变**，会变的信息跟着
/// 每条用户消息走（[`crate::prompt::turn_context`]），这样 DeepSeek 的前缀缓存每次都能命中。
#[derive(Default)]
pub struct Session {
    pub grants: Grants,
    /// 会话开始时冻结的工作区快照（照 Hermes：会话中途写入的记忆下一段会话才生效，前缀不变）。
    pub snapshot: std::sync::OnceLock<crate::workspace::Snapshot>,
    /// 会话开始时冻结的系统提示。
    pub system: std::sync::OnceLock<String>,
    /// 会话开始时冻结的工具清单（全部工具，一开始就给；不再经 `tools.find` 中途启用）。
    pub definitions: std::sync::OnceLock<Vec<ToolDef>>,
    /// 上一轮附注里的游戏（变了才再附上游戏说明）。
    pub last_game: Mutex<Option<String>>,
}

/// 这一轮从哪个入口来（两个入口共用一段对话、同一份工具清单）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    /// 聊天窗口。
    #[default]
    Full,
    /// 语音入口：`reply.ask`（用语音追问玩家）只在这里能用。
    Game,
}

pub struct KotonePort<H, A> {
    scope: Scope,
    /// 琴音的工作区（记忆、Skill、人格）；没有时这些工具报不可用。
    workspace: Option<Workspace>,
    /// 子会话：只能用这些工具（ADR 022）；None = 不限（主会话）。
    tool_filter: Option<Vec<String>>,
    service: Arc<ToolService>,
    catalog: Catalog,
    policy: Policy,
    session: Arc<Session>,
    host: H,
    asker: A,
}

impl<H: BuiltinHost, A: Asker> KotonePort<H, A> {
    pub fn new(
        service: Arc<ToolService>,
        catalog: Catalog,
        policy: Policy,
        session: Arc<Session>,
        host: H,
        asker: A,
    ) -> Self {
        Self {
            scope: Scope::Full,
            workspace: None,
            tool_filter: None,
            service,
            catalog,
            policy,
            session,
            host,
            asker,
        }
    }

    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// 子会话的工具范围：只提供、只允许调用这些工具（ADR 022）。
    pub fn with_tool_filter(mut self, tools: Vec<String>) -> Self {
        self.tool_filter = Some(tools);
        self
    }

    /// 主会话：全部工具；子会话：只有交给它的那几个。
    fn offered(&self, entry: &ToolEntry) -> bool {
        match &self.tool_filter {
            Some(filter) => filter.iter().any(|t| t == &entry.name),
            None => true,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// 按关键词或分组查工具（只是查，工具清单一开始就是全的）。
    fn find(&self, arguments: &Value) -> ToolOutcome {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty());
        let group = arguments.get("group").and_then(Value::as_str);
        if query.is_none() && group.is_none() {
            let groups: Vec<Value> = GROUPS
                .iter()
                .map(|(id, label, _)| {
                    let count = self.catalog.find(None, Some(id)).len();
                    json!({"group": id, "label": label, "tools": count})
                })
                .collect();
            return ToolOutcome::ok(
                json!({"groups": groups, "hint": "用 query 或 group 查找"}).to_string(),
            );
        }
        let found: Vec<&ToolEntry> = self
            .catalog
            .find(query, group)
            .into_iter()
            .filter(|e| self.offered(e))
            .collect();
        let total = found.len();
        let limit = if query.is_none() {
            usize::MAX
        } else {
            MAX_FIND
        };
        let tools: Vec<Value> = found
            .into_iter()
            .take(limit)
            .map(|e| {
                let short: String = e.description.chars().take(160).collect();
                json!({"name": e.name, "group": e.group, "description": short})
            })
            .collect();
        ToolOutcome::ok(
            json!({
                "tools": tools,
                "matched": total,
                "note": if total > limit { "结果太多，只列了前几个；换更具体的关键词" } else { "这些工具都可以直接调用" },
            })
            .to_string(),
        )
    }

    async fn call_service(&self, tool: &str, arguments: Value) -> ToolOutcome {
        let envelope = self
            .service
            .call(ToolCallRequest {
                request_id: None,
                tool: tool.to_string(),
                arguments,
                timeout_ms: service_timeout_ms(tool),
            })
            .await;
        if envelope.ok {
            let mut result = envelope.result.unwrap_or_else(|| json!({}));
            let images = take_images(&mut result);
            let mut outcome = ToolOutcome::ok(result.to_string());
            outcome.images = images;
            outcome
        } else {
            let error = envelope
                .error
                .map(|e| format!("{}：{}", e.code, e.message))
                .unwrap_or_else(|| "工具执行失败".into());
            ToolOutcome::error(error)
        }
    }

    async fn call_builtin(&self, call_id: &str, name: &str, arguments: &Value) -> ToolOutcome {
        let text = arguments
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let delivery = match parse_delivery(arguments) {
            Ok(d) => d,
            Err(e) => return ToolOutcome::error(e),
        };
        let result = match name {
            TOOL_SPEAK | TOOL_DANMAKU if text.is_empty() || text.chars().count() > 300 => {
                return ToolOutcome::error("text 为空或超过 300 字");
            }
            // 玩家在打字：不念（ADR 029 第八节）
            TOOL_SPEAK if self.scope != Scope::Game => Ok(json!({
                "spoken": false,
                "note": "这一轮是玩家打字的，不念出来；直接写在回复里"
            })),
            TOOL_SPEAK => self.host.speak(call_id, text).await,
            TOOL_DANMAKU => self.host.danmaku(text).await,
            TOOL_ASK if self.scope != Scope::Game => {
                return ToolOutcome::error("reply.ask 只在通话里用；在聊天窗口里直接在回复里问玩家")
            }
            TOOL_ASK => {
                let question = arguments
                    .get("question")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string();
                if question.is_empty() || question.chars().count() > 120 {
                    return ToolOutcome::error("question 为空或超过 120 字");
                }
                let wait = arguments
                    .get("waitSeconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(8)
                    .clamp(3, 15);
                self.host.ask(question, wait).await
            }
            TOOL_REMIND => {
                let delay = arguments.get("delaySeconds").and_then(Value::as_u64);
                match delay {
                    Some(d) if (1..=604_800).contains(&d) => {}
                    _ => return ToolOutcome::error("delaySeconds 必须在 1 秒到 7 天之间"),
                }
                if text.is_empty() || text.chars().count() > 300 {
                    return ToolOutcome::error("text 为空或超过 300 字");
                }
                self.host.remind(delay.unwrap_or(1), text, delivery).await
            }
            TOOL_GAME_CHAT => {
                if text.is_empty() || text.chars().count() > 200 || text.contains(['\r', '\n']) {
                    return ToolOutcome::error("要打的字为空、超过 200 字或含有换行");
                }
                let send = arguments.get("send").and_then(Value::as_bool);
                self.host.send_to_game_chat(text, send).await
            }
            TOOL_MEMORY | TOOL_SKILL_VIEW | TOOL_SKILL_MANAGE | TOOL_SOUL_WRITE => {
                self.call_workspace(name, arguments)
            }
            name if HOST_BUILTINS.contains(&name) => self.host.builtin(name, arguments).await,
            _ => return ToolOutcome::error(format!("没有名为 {name} 的内置动作")),
        };
        match result {
            Ok(value) => ToolOutcome::ok(value.to_string()),
            Err(message) => ToolOutcome::error(message),
        }
    }

    /// 工作区工具（记忆、Skill、人格）：纯文件操作，照 Hermes 的 memory / skill_view / skill_manage。
    fn call_workspace(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        let ws = self.workspace.as_ref().ok_or("这里没有琴音的工作区")?;
        let s = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match name {
            TOOL_MEMORY => {
                let target = match s("target").as_str() {
                    "user" => MemoryTarget::User,
                    "memory" => MemoryTarget::Memory,
                    "game" => MemoryTarget::Game(s("game")),
                    _ => return Err("target 只能是 user、memory 或 game".into()),
                };
                let ops = arguments
                    .get("operations")
                    .and_then(Value::as_array)
                    .ok_or("缺少 operations")?
                    .iter()
                    .map(|op| {
                        let f =
                            |k: &str| op.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        match op.get("op").and_then(Value::as_str) {
                            Some("add") => Ok(MemoryOp::Add(f("text"))),
                            Some("patch") if !f("old").is_empty() => Ok(MemoryOp::Patch {
                                old: f("old"),
                                new: f("new"),
                            }),
                            Some("remove") if !f("old").is_empty() => {
                                Ok(MemoryOp::Remove(f("old")))
                            }
                            _ => Err(
                                "每个操作要有 op：add {text} / patch {old, new} / remove {old}"
                                    .to_string(),
                            ),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let (entries, chars) = ws.apply_memory(&target, &ops)?;
                Ok(
                    json!({"saved": true, "entries": entries, "chars": chars, "note": "下一段对话开始时生效"}),
                )
            }
            TOOL_SKILL_VIEW if s("name").is_empty() => Ok(json!({"skills": ws
                .skills()
                .into_iter()
                .map(|k| json!({"name": k.name, "description": k.description}))
                .collect::<Vec<_>>()})),
            TOOL_SKILL_VIEW => {
                let path = arguments.get("path").and_then(Value::as_str);
                Ok(json!({"name": s("name"), "content": ws.skill_view(&s("name"), path)?}))
            }
            TOOL_SKILL_MANAGE => {
                let skill = s("name");
                match s("action").as_str() {
                    "create" => ws.skill_create(&skill, &s("content"))?,
                    "patch" => ws.skill_patch(&skill, &s("old"), &s("new"))?,
                    "delete" => ws.skill_delete(&skill)?,
                    "write_file" => ws.skill_write_file(&skill, &s("path"), &s("content"))?,
                    "remove_file" => ws.skill_remove_file(&skill, &s("path"))?,
                    _ => {
                        return Err(
                            "action 只能是 create / patch / delete / write_file / remove_file"
                                .into(),
                        )
                    }
                }
                Ok(json!({"saved": true, "skills": ws.skills()}))
            }
            TOOL_SOUL_WRITE => {
                ws.write_file(crate::workspace::SOUL, &s("content"))?;
                Ok(json!({"saved": true, "note": "下一段对话开始时生效；告诉玩家你改了什么"}))
            }
            _ => Err(format!("没有名为 {name} 的工作区工具")),
        }
    }

    async fn call_saved(&self, id: &str, arguments: Value) -> ToolOutcome {
        let mut args = match arguments {
            Value::Object(map) => map,
            _ => Default::default(),
        };
        let delay = args.remove("delaySeconds");
        let mut request = json!({"id": id, "args": Value::Object(args)});
        if let Some(delay) = delay.filter(|d| d.as_u64().is_some_and(|d| d > 0)) {
            request["delaySeconds"] = delay;
        }
        self.call_service("automation.run", request).await
    }

    async fn execute(&self, call_id: &str, entry: &ToolEntry, arguments: Value) -> ToolOutcome {
        let mut outcome = match &entry.source {
            Source::Meta => self.find(&arguments),
            Source::Builtin => self.call_builtin(call_id, &entry.name, &arguments).await,
            Source::Saved { id } => self.call_saved(id, arguments).await,
            Source::Service => self.call_service(&entry.name, arguments).await,
            Source::Plugin { .. } => match self.host.plugin(&entry.name, &arguments).await {
                Ok(result) => plugin_outcome(result),
                Err(message) => ToolOutcome::error(message),
            },
            // 由 `call` 直接交给桌面（要带上调用 id、取消和“每次询问”），不会走到这里。
            Source::Exec => ToolOutcome::error(format!("{} 在这里不可用", entry.name)),
        };
        outcome.untrusted = entry.untrusted;
        outcome
    }
}

/// 插件工具的结果（照 MCP Apps）：只有 `content` 进她的上下文；`structuredContent` 和页面地址给界面
/// （存进对话记录，聊天卡片用它）。
pub fn plugin_outcome(result: kotone_plugin_api::ToolResult) -> ToolOutcome {
    let mut outcome = ToolOutcome::ok(match result.content {
        Value::String(text) => text,
        other => other.to_string(),
    });
    if let Some(ui) = result.ui {
        outcome.display = Some(json!({
            "structuredContent": result.structured_content.unwrap_or(Value::Null),
            "ui": ui,
        }));
    }
    outcome
}

fn parse_delivery(arguments: &Value) -> Result<Vec<DeliveryKind>, String> {
    let mut out = Vec::new();
    if let Some(items) = arguments.get("delivery").and_then(Value::as_array) {
        for item in items {
            let kind = match item.as_str() {
                Some("voice") => DeliveryKind::Voice,
                Some("danmaku") => DeliveryKind::Danmaku,
                _ => return Err("delivery 只能是 voice / danmaku".into()),
            };
            if !out.contains(&kind) {
                out.push(kind);
            }
        }
    }
    Ok(out)
}

impl<H: BuiltinHost, A: Asker> ToolPort for KotonePort<H, A> {
    fn definitions(&self) -> Vec<ToolDef> {
        // 一段对话里冻结（照 DeepSeek Harness）：中途改权限、保存新的自动化都不改清单，
        // 禁止的工具调用时再拒绝；新保存的自动化下一段对话才成为工具（这段里用 automation.run）。
        self.session
            .definitions
            .get_or_init(|| {
                self.catalog
                    .entries()
                    .iter()
                    .filter(|e| self.offered(e))
                    .map(|e| ToolDef {
                        name: e.name.clone(),
                        description: e.description.clone(),
                        parameters: e.parameters.clone(),
                    })
                    .collect()
            })
            .clone()
    }

    async fn call(
        &self,
        call_id: &str,
        name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> ToolOutcome {
        let Some(entry) = self.catalog.get(name).filter(|e| self.offered(e)) else {
            return ToolOutcome::error(format!("没有名为 {name} 的可用工具"));
        };
        // 琴音不能改自己的权限：在询问用户之前就拒绝，不受“全部允许”影响。
        if entry.name == "settings.update" {
            if let Some(reason) = arguments
                .get("patch")
                .and_then(kotone_core::settings_guide::agent_protected_violation)
            {
                return ToolOutcome::error(format!("没有执行：{reason}"));
            }
        }
        let decision = self
            .policy
            .decide_with(entry, &arguments, &self.session.grants);
        // 执行命令、读写文件（ADR 030）：什么时候先问由桌面端按 kotone-exec 的规则定，不走下面的
        // 询问（免得问两遍）；玩家把它设成“每次询问”时让桌面端无论如何先问一句。
        if entry.source == Source::Exec {
            let confirm = match decision {
                Decision::Allow => false,
                Decision::Ask { .. } => true,
                Decision::Deny { reason } => {
                    return ToolOutcome::error(format!("没有执行：{reason}"));
                }
            };
            let mut outcome = self
                .host
                .exec(call_id, &entry.name, &arguments, confirm, cancel)
                .await;
            outcome.untrusted = entry.untrusted;
            return outcome;
        }
        match decision {
            Decision::Allow => {}
            Decision::Deny { reason } => {
                return ToolOutcome::error(format!("没有执行：{reason}"));
            }
            Decision::Ask { .. } if self.host.preapproved(&entry.name, &arguments) => {}
            Decision::Ask { reason } => {
                let request = PermissionRequest {
                    call_id: call_id.to_string(),
                    tool: entry.name.clone(),
                    title: permissions::title(entry),
                    reason,
                    arguments: arguments.clone(),
                };
                match self.asker.ask(request, cancel).await {
                    Answer::AllowOnce => {}
                    Answer::AllowForConversation => {
                        permissions::remember(&self.session.grants, entry, &arguments)
                    }
                    Answer::Deny => {
                        return ToolOutcome::error(
                            "用户拒绝了这次操作；不要换个方式重试，先问用户想怎么做",
                        );
                    }
                }
                if cancel.is_cancelled() {
                    return ToolOutcome::error("已停止，未执行");
                }
            }
        }
        self.execute(call_id, entry, arguments).await
    }
}
