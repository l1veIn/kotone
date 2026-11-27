//! 内置 Agent 的权限（ADR 014 §5，参照 Claude Code / Codex / dsh）。
//!
//! - 每个工具一条规则：允许 / 询问 / 禁止；用户没设规则时，由工具的副作用标记推出默认规则；
//! - 整体模式：默认、全部允许、只读；
//! - 默认需要询问：改设置与配置、删除、退出 / 重启、管理员提权、开机自启、下载模型、
//!   **AI 新写的程序第一次运行**（`automation.run` 带 `source`）；读状态、截图、识别、播报、
//!   提醒、打字、运行已保存的自动化、停止，直接执行；不认识的副作用标记一律询问；
//! - 用户在询问时选“本段对话都允许”，同一段对话里不再问（AI 写的程序按源码区分）；
//! - 玩家设下的热键（ADR 040 D9）和自动化程序里的 `tool()`（ADR 046 D3）用同一张表、同一个判定
//!   （[`binding_decision`]、[`program_decision`]）：要问的在设下的时候确认一次，跑的时候不问。
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
    // ADR 030：后台执行命令、写文件。默认“允许”，真正什么时候先问玩家由 kotone-exec 的 `check`
    // 决定（读过外面的东西、删除类命令、第一次改游戏目录……）；玩家设成询问 / 禁止照常生效。
    "runs-command",
    "writes-file",
    // ADR 031：后台在 Kotone 的浏览器里操作网页。默认“允许”：第一次打开某个网站由桌面端经主会话问玩家，
    // 登录、验证码、付款交给玩家（browser.handover），密码框不让输入。播放 / 暂停 Kotone 浏览器里的歌。
    "browses-web",
    "plays-media",
    // ADR 038：在 Kotone 窗口里打开一个页面（不把窗口拉到前面）。
    "shows-ui",
    // ADR 046：她运行的已保存自动化里，玩家在「自动化」页允许过的工具（见 CONFIRMED_TOOL_EFFECT）。
    CONFIRMED_TOOL_EFFECT,
];

/// 她运行已保存的自动化（`saved.<id>`）时，程序里玩家在「自动化」页允许过的工具，默认要问的那些副作用换成这一个
/// （[`crate::catalog::saved_entry`]，ADR 046 D3）：默认允许——确认过的不在聊天里再问一次；但不算只读——只读模式照样拒。
/// 玩家把这个自动化设成“先问我”、禁止照常生效。
pub const CONFIRMED_TOOL_EFFECT: &str = "runs-confirmed-tools";

/// 这个副作用默认要问（要问的、不认识的）：确认过的工具只换掉这些（[`CONFIRMED_TOOL_EFFECT`]）。
pub fn asks_by_default(effect: &str) -> bool {
    !ALLOWED_EFFECTS.contains(&effect)
}

/// 默认询问的副作用及原因。
const ASK_EFFECTS: &[(&str, &str)] = &[
    ("writes-settings", "会修改设置"),
    ("writes-credentials", "会保存 Key"),
    ("deletes-credentials", "会删除 Key"),
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
        "会安装虚拟声卡 VB-CABLE（Windows 会请求管理员权限）",
    ),
    ("writes-system-audio", "会修改 Windows 的默认麦克风"),
    ("speaks-to-teammates", "会用麦克风对游戏里的队友说话"),
    ("writes-skills", "会修改技能"),
    ("writes-soul", "会修改琴音的人格（SOUL.md）"),
    // ADR 038 第 3.5 步：照 LeagueAkari 操作英雄联盟客户端（有封号风险，LeagueAkari #523）
    (
        "controls-game-client",
        "会操作英雄联盟客户端（接受对局、选 / 禁英雄）",
    ),
    // MCP 方案第七节第 3 条（照 Codex 的 writes）：第三方插件（别人的 MCP 服务器）的工具，由
    // kotone-remote-plugin 标。没标只读的第一次问（玩家可以对这个工具“总是允许”，就是权限里一条“允许”）；
    // 标了只读的，信任这个服务器之前也问一次（注解是服务器自己说的，不当安全边界）。
    ("third-party-action", "第三方插件的操作"),
    (
        "third-party-read",
        "第三方插件的工具（它说只读，但你还没信任这个插件）",
    ),
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

/// 玩家设下的（热键、自动化）调这个工具（ADR 040 D9、ADR 046 D3）：设下的时候问，跑的时候不问。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingDecision {
    /// 按了就跑（只读的、默认允许的、确认过的）。
    Run,
    /// 要玩家确认一次（热键在「设置 → 热键」，自动化在「自动化」页）：这个工具会做什么（权限的原因）。
    Confirm { reason: String },
    /// 不能跑：规则设成了禁止、只读模式；程序里调不了的工具（[`program_callable`]）。
    Refused { reason: String },
}

/// 热键能不能调这个工具（ADR 040 D9，照浏览器扩展装的时候问权限、按快捷键时不问）。和她用的是**同一张权限表、
/// 同一个判定**（[`Decide::decide`]，含整体模式）：允许 → 跑；禁止 → 不能跑；要问 → 绑的时候问过一次（`confirmed` 是当时
/// 这个工具的副作用）就跑，工具后来多了没确认过的副作用要重新确认（照 Chrome：扩展更新多要权限时先停用到用户同意）。
/// 参数变了要重新确认由存法保证（改了参数，确认就被抹掉，`kotone_core::hotkey_custom`）。
pub fn binding_decision(
    policy: &Policy,
    entry: &ToolEntry,
    arguments: &Value,
    confirmed: Option<&[&str]>,
) -> BindingDecision {
    match policy.decide(entry, arguments) {
        Decision::Allow => BindingDecision::Run,
        Decision::Deny { reason } => BindingDecision::Refused { reason },
        Decision::Ask { reason } => match confirmed {
            Some(ok) if entry.side_effects.iter().all(|e| ok.contains(&e.as_str())) => {
                BindingDecision::Run
            }
            _ => BindingDecision::Confirm { reason },
        },
    }
}

/// 往游戏里打字、按键的副作用（ADR 046 D2）：程序自己有 `chat` / `key` / `click`，按键只有一套规矩（绑窗口、输入锁、名额）。
const GAME_INPUT_EFFECTS: &[&str] = &["input", "sends-game-input", "may-type-into-game"];

/// 程序里能不能调这个工具（ADR 046 D2）：热键能绑的（[`ToolEntry::bindable`]：工具服务的、插件的、本体 ToolSpec 的）
/// 再去掉三类——
/// - 自动化自己的工具（`automation.*`）：程序运行、改、停自动化会绕圈，也能绕过确认；
/// - 往游戏里打字、按键的（副作用 `input` / `sends-game-input`）：`game_chat.send`、`lol.shout`、`game.act`；
/// - 结果不可信又只读的：程序只拿得到 `{status}`（D4），调了只是白花时间和钱（在线文字识别、读聊天、`game.look`、
///   `lol.scout`）。第三方插件说只读的（`third-party-read`）也算只读。
///
/// 她对话里才有的（说话、问玩家、交给后台、联网搜索、浏览器、执行命令、记忆、`saved.*`）本来就不能绑。
pub fn program_callable(entry: &ToolEntry) -> Result<(), String> {
    let name = &entry.name;
    if !entry.bindable() {
        return Err(format!("{name} 只在和她的对话里用，程序里调不了"));
    }
    if kotone_core::automation::library::is_automation_tool(name) {
        return Err(format!(
            "程序里不能调自动化自己的工具（{name}）：会绕圈，也能绕过确认"
        ));
    }
    if entry
        .side_effects
        .iter()
        .any(|e| GAME_INPUT_EFFECTS.contains(&e.as_str()))
    {
        return Err(format!(
            "{name} 会往游戏里打字或按键，程序里不能调：用程序自己的 chat() / key() / click()"
        ));
    }
    let only_reads = entry
        .side_effects
        .iter()
        .all(|e| READ_ONLY_EFFECTS.contains(&e.as_str()) || e == "third-party-read");
    if entry.untrusted && only_reads {
        return Err(format!(
            "{name} 的结果是别人写的字，程序拿不到（只回 status），调了也白调"
        ));
    }
    Ok(())
}

/// 程序里这一次 `tool()` 能不能调（ADR 046 D3）：先看 [`program_callable`]，再和热键同一个判定（[`binding_decision`]，
/// 同一张权限表、含整体模式）——允许 → 调；禁止、只读模式下有副作用的 → 不能调；要问的 → 保存的自动化确认过才调
/// （`confirmed` 是确认时这个工具的副作用，工具后来多了副作用要重新确认），没确认是 `Confirm`。
/// 临时程序（`saved = false`：她和外部 Agent 的 `automation.run {source}`）没人确认过，要问的就是不能调，先存起来让
/// 玩家确认。宿主在安排时和每次 `tool()` 时都判一次（规则可能刚改过）；插件、数据开关关着由宿主先查。
pub fn program_decision(
    policy: &Policy,
    entry: &ToolEntry,
    confirmed: Option<&[&str]>,
    saved: bool,
) -> BindingDecision {
    if let Err(reason) = program_callable(entry) {
        return BindingDecision::Refused { reason };
    }
    let confirmed = if saved { confirmed } else { None };
    // 确认的是“这段程序可以调这个工具”，不看某一次的参数
    match binding_decision(policy, entry, &Value::Object(Default::default()), confirmed) {
        BindingDecision::Confirm { reason } if !saved => BindingDecision::Refused {
            reason: format!(
                "{reason}：临时程序只能调不用确认的工具，先存成自动化，让玩家在「自动化」页允许"
            ),
        },
        other => other,
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
