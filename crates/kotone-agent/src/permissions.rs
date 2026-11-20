//! 内置 Agent 的权限（ADR 014 §5，参照 Claude Code / Codex / dsh）。
//!
//! - 每个工具一条规则：允许 / 询问 / 禁止；用户没设规则时，由工具的副作用标记推出默认规则；
//! - 整体模式：默认、全部允许、只读；
//! - 默认需要询问：改设置与配置、删除、退出 / 重启、管理员提权、开机自启、下载模型、
//!   **AI 新写的程序第一次运行**（`automation.run` 带 `source`）；读状态、截图、识别、播报、
//!   提醒、打字、运行已保存的自动化、停止，直接执行；不认识的副作用标记一律询问；
//! - 用户在询问时选“本段对话都允许”，同一段对话里不再问（AI 写的程序按源码区分）。
//!
//! 外部 Agent 不经过这里（全部能力开放，权限由它们自己的运行框架管）。

use std::collections::HashSet;
use std::future::Future;
use std::sync::Mutex;

use kotone_core::cancel::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::catalog::{Source, ToolEntry};

/// 权限设置的类型在 kotone-core（随设置存盘），这里负责判定。
pub use kotone_core::agent::{Mode, Policy, Rule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

/// 只读模式下仍可用的副作用（读屏幕、临时产物、播报）。
const READ_ONLY_EFFECTS: &[&str] = &[
    "creates-artifact",
    "registers-window-references",
    "releases-artifact",
    "creates-ocr-result",
    "mutates-chat-tracking",
    "captures-screen",
    "speaks-or-shows-text",
];

/// 默认直接执行的副作用。
const ALLOWED_EFFECTS: &[&str] = &[
    "creates-artifact",
    "registers-window-references",
    "releases-artifact",
    "creates-ocr-result",
    "mutates-chat-tracking",
    "captures-screen",
    "speaks-or-shows-text",
    "may-speak",
    "input",
    "records-run",
    "creates-reminder",
    "stops-automation",
    "cancels-download",
    "writes-automations",
    "focuses-window",
    // ADR 021 / 022：记忆过注入扫描；监听有“正在听”的提示。
    // ADR 029 第二期：交给后台只在玩家开口的那一轮能做（桌面端检查），后台要确认的事经主会话问玩家。
    "writes-memory",
    "spawns-session",
    "sends-session",
    "stops-session",
    "delegates-task",
    "stops-task",
    "listens-to-game-audio",
    // 请玩家粘贴 Key：输入框本身就是玩家的同意，Key 不经过模型。
    "requests-credentials",
    // ADR 026：联网搜索、读网页。只读、每次 0.005 元；网页由 DeepSeek 的服务器去抓，碰不到本机。
    "web-request",
];

/// 默认询问的副作用及原因。
const ASK_EFFECTS: &[(&str, &str)] = &[
    ("writes-settings", "会修改设置"),
    ("writes-credentials", "会保存密钥"),
    ("deletes-credentials", "会删除密钥"),
    ("writes-profiles", "会修改游戏配置"),
    ("deletes-files", "会删除文件"),
    ("deletes-history", "会删除历史记录"),
    ("writes-files", "会写入文件"),
    ("moves-files", "会移动文件"),
    ("downloads-model", "会下载模型（可能很大）"),
    ("writes-registry", "会修改开机自启"),
    ("quits-application", "会退出 Kotone"),
    ("restarts-application", "会重启 Kotone"),
    ("shows-uac-prompt", "会请求管理员权限"),
    ("starts-runtime", "会启动语音识别"),
    ("stops-runtime", "会停止语音识别"),
    ("registers-hotkeys", "会注册热键"),
    ("may-register-hotkeys", "可能改动热键"),
    ("llm-request", "会调用付费的 AI 接口"),
    ("launches-program", "会打开游戏"),
    ("closes-program", "会关闭游戏"),
    ("sends-game-input", "会在游戏里操作鼠标键盘"),
    (
        "installs-driver",
        "会安装虚拟声卡 VB-CABLE（Windows 会请求管理员授权）",
    ),
    ("writes-system-audio", "会修改 Windows 的默认麦克风"),
    ("speaks-to-teammates", "会用麦克风对游戏里的队友说话"),
    ("writes-skills", "会修改 Skill"),
    ("writes-soul", "会修改琴音的人格（SOUL.md）"),
];

/// 一段对话里“本段对话都允许”的记录。
#[derive(Default)]
pub struct Grants {
    keys: Mutex<HashSet<String>>,
}

impl Grants {
    fn has(&self, key: &str) -> bool {
        self.keys.lock().unwrap().contains(key)
    }

    fn add(&self, key: String) {
        self.keys.lock().unwrap().insert(key);
    }
}

/// AI 新写的程序（`automation.run` 带 `source`）的授权键：按源码区分。
fn grant_key(entry: &ToolEntry, arguments: &Value) -> String {
    match new_program(entry, arguments) {
        Some(source) => {
            let digest = Sha256::digest(source.as_bytes());
            let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
            format!("{}#{hex}", entry.name)
        }
        None => entry.name.clone(),
    }
}

fn new_program<'a>(entry: &ToolEntry, arguments: &'a Value) -> Option<&'a str> {
    if entry.name == "automation.run" && entry.source == Source::Service {
        arguments.get("source").and_then(Value::as_str)
    } else {
        None
    }
}

fn read_only(entry: &ToolEntry) -> bool {
    entry
        .side_effects
        .iter()
        .all(|e| READ_ONLY_EFFECTS.contains(&e.as_str()))
}

/// 由副作用与名字推出的默认规则。
pub fn default_decision(entry: &ToolEntry, arguments: &Value) -> Decision {
    if new_program(entry, arguments).is_some() {
        return Decision::Ask {
            reason: "AI 新写的程序第一次运行".into(),
        };
    }
    let name = entry.name.as_str();
    if name.ends_with(".delete") || name.ends_with(".clear") {
        return Decision::Ask {
            reason: "会删除数据".into(),
        };
    }
    for effect in &entry.side_effects {
        if let Some((_, reason)) = ASK_EFFECTS.iter().find(|(e, _)| e == effect) {
            return Decision::Ask {
                reason: (*reason).into(),
            };
        }
        if !ALLOWED_EFFECTS.contains(&effect.as_str()) {
            return Decision::Ask {
                reason: format!("有未归类的副作用（{effect}）"),
            };
        }
    }
    Decision::Allow
}

/// 判定（`Policy` 定义在 kotone-core，判定逻辑在这里）。
pub trait Decide {
    fn decide(&self, entry: &ToolEntry, arguments: &Value) -> Decision;
    fn decide_with(&self, entry: &ToolEntry, arguments: &Value, grants: &Grants) -> Decision;
}

impl Decide for Policy {
    /// 这次调用该怎么处理（不含“本段对话都允许”的记录，见 [`Self::decide_with`]）。
    fn decide(&self, entry: &ToolEntry, arguments: &Value) -> Decision {
        let rule = self.rules.get(&entry.name).copied();
        if rule == Some(Rule::Deny) {
            return Decision::Deny {
                reason: "用户设置为禁止".into(),
            };
        }
        if self.mode == Mode::ReadOnly && !read_only(entry) {
            return Decision::Deny {
                reason: "当前是只读模式".into(),
            };
        }
        match rule {
            Some(Rule::Allow) => Decision::Allow,
            Some(Rule::Ask) => Decision::Ask {
                reason: "用户设置为每次询问".into(),
            },
            _ if self.mode == Mode::AllowAll => Decision::Allow,
            _ => default_decision(entry, arguments),
        }
    }

    /// 考虑本段对话里已经给过的授权。
    fn decide_with(&self, entry: &ToolEntry, arguments: &Value, grants: &Grants) -> Decision {
        match self.decide(entry, arguments) {
            Decision::Ask { .. } if grants.has(&grant_key(entry, arguments)) => Decision::Allow,
            other => other,
        }
    }
}

/// 一次询问（界面给按钮；游戏内用语音 / 弹幕问或留到窗口里确认）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    /// 对应的工具调用 id（界面把按钮放在那张工具卡片上）。
    pub call_id: String,
    pub tool: String,
    /// 一句话说明要做什么。
    pub title: String,
    pub reason: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Answer {
    AllowOnce,
    /// 本段对话里同一个工具（AI 写的程序：同一段源码）不再询问。
    AllowForConversation,
    Deny,
}

/// 询问用户的端口。取消时应尽快返回（返回值会被忽略）。
pub trait Asker: Send + Sync {
    fn ask(
        &self,
        request: PermissionRequest,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Answer> + Send;
}

/// 记录“本段对话都允许”。
pub(crate) fn remember(grants: &Grants, entry: &ToolEntry, arguments: &Value) {
    grants.add(grant_key(entry, arguments));
}

/// 询问里给用户看的一句话。
pub(crate) fn title(entry: &ToolEntry) -> String {
    let first = entry
        .description
        .split(['。', '.', '\n'])
        .next()
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        entry.name.clone()
    } else {
        format!(
            "{}（{}）",
            first.chars().take(80).collect::<String>(),
            entry.name
        )
    }
}
