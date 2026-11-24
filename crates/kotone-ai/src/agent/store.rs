//! 对话存储（ADR 014 §3 “对话保存在本机”）。
//!
//! 目录结构（目录由调用方给，桌面端为 `~/.kotone/agent/`）：
//! - `index.json`：版本、当前对话（游戏内与聊天窗口共用同一段，见 ADR 014“已确认”）、各对话摘要；
//! - `conversations/<id>.json`：一段对话的全部消息。
//!
//! - `subagents/<id>.json`：这段对话派出去的后台任务（ADR 029 第二期的子代理）登记。和对话文件分开存：
//!   对话正在回答时主循环会反复保存对话，后台任务的状态由别的线程更新，分开才不会互相覆盖。
//!
//! 都是原子写入。索引丢失或损坏时从对话文件重建；对话文件损坏时报错，不删除。
//! 上限：玩家看得到的对话最多保留 [`MAX_CONVERSATIONS`] 段（超出删最久没更新的，连同它的后台任务记录），
//! 每段最多
//! [`MAX_STORED_MESSAGES`] 条：超出时只丢最后一个压缩检查点之前的（那部分已经整理成摘要、
//! 不再发给模型，丢了不影响前缀缓存）；没有检查点时超过 [`HARD_MAX_MESSAGES`] 才从最早的一轮丢。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{now_ms, Message};

pub const MAX_CONVERSATIONS: usize = 100;
/// 后台任务的过程记录（隐藏的对话）最多留几段：平时随它的主对话一起删除 / 淘汰，这里只是兜底。
pub const MAX_BACKGROUND_RECORDS: usize = 300;
pub const MAX_STORED_MESSAGES: usize = 400;
pub const HARD_MAX_MESSAGES: usize = 2_000;
const VERSION: u32 = 1;
const TITLE_CHARS: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<Message>,
    /// 子会话（ADR 022）：派出它的会话 id；主会话为 None。旧对话没有这一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// 子会话的名字（如“同传”），也是它在对话列表里的标题。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// 模型起的标题（DeepSeek `title`，ADR 026）；没有时用第一句话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_title: Option<String>,
    /// 这段对话派出去的后台任务（ADR 029 第二期）。存在 `subagents/<id>.json`，不写进对话文件；
    /// 读对话给界面时由调用方填上（[`ConversationStore::load_subagents`]）。旧对话没有。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<Subagent>,
}

/// 后台任务（子代理）的状态（ADR 029 第二期）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentStatus {
    Running,
    /// 在等玩家确认（经主会话问）。
    Waiting,
    Done,
    Failed,
    /// 被玩家或主会话停掉了。
    Stopped,
    /// Kotone 重启时还没做完（不自动重跑，照 Codex）。
    Interrupted,
}

impl SubagentStatus {
    /// 正在占用一个名额（在跑或在等确认）。
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

/// 主会话里登记的一个后台任务。界面上是一张卡片（“写喝药脚本 · 进行中”）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    /// 这段对话里稳定的英文短名（`agent.followup` 靠它找回同一个）。
    pub name: String,
    /// 三五个字的卡片标题。
    pub label: String,
    /// 它的过程记录（一段隐藏的对话，`parent` = 主会话）。
    pub conversation_id: String,
    pub status: SubagentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    /// 一共调用了几次模型。
    #[serde(default)]
    pub steps: u32,
    /// 在等玩家确认时问的那句话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// 做完（done / failed）的结果已经交给主会话了（先存盘再交付，重启也不丢）。
    #[serde(default)]
    pub reported: bool,
    /// 派它出去（或最近一次追加任务）的那一轮：玩家那句话的时间，费用记到那一轮。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    /// ADR 030：它读过外面的东西（屏幕、聊天、网页、Kotone 文件区以外的文件……）——第一次的说明。
    /// 读过之后执行命令、写文件前要先经主会话问玩家一句（kotone-exec `Taint`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taint: Option<String>,
    /// 这一轮里玩家已经同意过（追加任务开新一轮时清掉）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taint_approved: bool,
    /// 不是派出去的子代理，而是主会话的一个工具超过 3 秒没回来、转到后台接着做的（工具名，ADR 029 §一）。
    /// 界面按工具名显示卡片标题（和工具卡片一样的叫法）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

impl Conversation {
    fn summary(&self) -> ConversationSummary {
        ConversationSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            message_count: self.messages.len(),
            parent: self.parent.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub message_count: usize,
    /// 子会话：派出它的会话 id（界面据此标出、只读）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    version: u32,
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    conversations: Vec<ConversationSummary>,
}

pub struct ConversationStore {
    dir: PathBuf,
    index: Mutex<Index>,
}

/// 对话 id 只允许小写字母、数字和连字符（防止拼路径时越出目录）。
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("c-{}-{}", now_ms(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn title_of(messages: &[Message]) -> String {
    messages
        .iter()
        .find_map(|m| match m {
            // 自动卡片给模型的那句（ADR 039 D3）不是玩家说的，不拿来当标题
            Message::User { text, source, .. } if source != "auto-card" => Some(text.trim()),
            _ => None,
        })
        .map(|text| {
            let line = text.lines().next().unwrap_or("");
            let mut title: String = line.chars().take(TITLE_CHARS).collect();
            if line.chars().count() > TITLE_CHARS {
                title.push('…');
            }
            title
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "新对话".into())
}

/// 超出上限时从索引里拿掉的对话 id（最近更新的在前，从末尾淘汰）。上限只按玩家看得到的对话
/// （[`MAX_CONVERSATIONS`]）算，后台任务的过程记录（有 `parent` 的）随它的主对话一起拿掉，
/// 不挤占看得到的对话；它们自己另有兜底上限 [`MAX_BACKGROUND_RECORDS`]。
fn evicted(conversations: &mut Vec<ConversationSummary>) -> Vec<String> {
    let mut out = Vec::new();
    while conversations.iter().filter(|c| c.parent.is_none()).count() > MAX_CONVERSATIONS {
        let Some(pos) = conversations.iter().rposition(|c| c.parent.is_none()) else {
            break;
        };
        let old = conversations.remove(pos);
        conversations.retain(|c| {
            let child = c.parent.as_deref() == Some(old.id.as_str());
            if child {
                out.push(c.id.clone());
            }
            !child
        });
        out.push(old.id);
    }
    while conversations.iter().filter(|c| c.parent.is_some()).count() > MAX_BACKGROUND_RECORDS {
        let Some(pos) = conversations.iter().rposition(|c| c.parent.is_some()) else {
            break;
        };
        out.push(conversations.remove(pos).id);
    }
    out
}

/// 超出条数上限时，从最早的一轮（以用户消息为界）开始丢；不丢最后一个检查点和它之后的。
fn trim_messages(messages: &mut Vec<Message>) {
    if messages.len() <= MAX_STORED_MESSAGES {
        return;
    }
    if let Some(checkpoint) = messages.iter().rposition(Message::is_checkpoint) {
        let excess = messages.len() - MAX_STORED_MESSAGES;
        // 从一条用户消息（或检查点本身）开始，界面上不留半截的工具调用。
        let cut = (excess.min(checkpoint)..=checkpoint)
            .find(|&i| i == checkpoint || matches!(messages[i], Message::User { .. }))
            .unwrap_or(checkpoint);
        messages.drain(..cut);
        return;
    }
    if messages.len() <= HARD_MAX_MESSAGES {
        return;
    }
    let excess = messages.len() - HARD_MAX_MESSAGES;
    let cut = messages
        .iter()
        .enumerate()
        .skip(excess)
        .find(|(_, m)| matches!(m, Message::User { .. }))
        .map(|(i, _)| i)
        .unwrap_or(excess);
    messages.drain(..cut);
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建对话目录失败：{e}"))?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| format!("写入对话失败：{e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("写入对话失败：{e}"))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| format!("保存对话失败：{e}"))
}

impl ConversationStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        std::fs::create_dir_all(dir.join("conversations"))
            .map_err(|e| format!("创建对话目录失败：{e}"))?;
        let index = match std::fs::read(dir.join("index.json")) {
            Ok(bytes) => match serde_json::from_slice::<Index>(&bytes) {
                Ok(index) if index.version <= VERSION => index,
                Ok(_) => return Err("对话索引来自更新的版本，无法读取".into()),
                Err(_) => Self::rebuild(&dir),
            },
            Err(_) => Self::rebuild(&dir),
        };
        Ok(Self {
            dir,
            index: Mutex::new(index),
        })
    }

    fn rebuild(dir: &Path) -> Index {
        let mut conversations = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir.join("conversations")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Some(c) = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Conversation>(&b).ok())
                    .filter(|c| valid_id(&c.id))
                {
                    conversations.push(c.summary());
                }
            }
        }
        conversations.sort_by_key(|c| std::cmp::Reverse(c.updated_at_ms));
        Index {
            version: VERSION,
            current: None,
            conversations,
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join("conversations").join(format!("{id}.json"))
    }

    fn persist_index(&self, index: &Index) -> Result<(), String> {
        let bytes = serde_json::to_vec(index).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join("index.json"), &bytes)
    }

    /// 新建一段空对话（第一次 [`save`](Self::save) 时才写盘）。
    pub fn create(&self) -> Conversation {
        let now = now_ms();
        let mut id = new_id();
        while self.path(&id).exists() {
            id = new_id();
        }
        Conversation {
            version: VERSION,
            id,
            title: "新对话".into(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: Vec::new(),
            parent: None,
            label: None,
            auto_title: None,
            subagents: Vec::new(),
        }
    }

    fn subagents_path(&self, id: &str) -> PathBuf {
        self.dir.join("subagents").join(format!("{id}.json"))
    }

    /// 这段对话派出去的后台任务（没有或读不了时为空）。
    pub fn load_subagents(&self, id: &str) -> Vec<Subagent> {
        if !valid_id(id) {
            return Vec::new();
        }
        std::fs::read(self.subagents_path(id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// 整体写回后台任务的登记（空 = 删掉文件）。
    pub fn save_subagents(&self, id: &str, subagents: &[Subagent]) -> Result<(), String> {
        if !valid_id(id) {
            return Err("对话 id 无效".into());
        }
        let path = self.subagents_path(id);
        if subagents.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("删除后台记录失败：{e}")),
            };
        }
        let bytes = serde_json::to_vec(subagents).map_err(|e| e.to_string())?;
        write_atomic(&path, &bytes)
    }

    /// 最近更新的在前。
    pub fn list(&self) -> Vec<ConversationSummary> {
        self.index.lock().unwrap().conversations.clone()
    }

    pub fn load(&self, id: &str) -> Result<Option<Conversation>, String> {
        if !valid_id(id) {
            return Err("对话 id 无效".into());
        }
        match std::fs::read(self.path(id)) {
            Ok(bytes) => serde_json::from_slice::<Conversation>(&bytes)
                .map(Some)
                .map_err(|_| "对话文件已损坏，无法读取".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取对话失败：{e}")),
        }
    }

    /// 保存（新建或覆盖）。会更新标题、时间，按上限裁剪消息与对话数。
    pub fn save(&self, conversation: &mut Conversation) -> Result<(), String> {
        if !valid_id(&conversation.id) {
            return Err("对话 id 无效".into());
        }
        trim_messages(&mut conversation.messages);
        conversation.title = match &conversation.label {
            Some(label) => label.clone(),
            None => match &conversation.auto_title {
                Some(title) if !title.trim().is_empty() => {
                    title.trim().chars().take(TITLE_CHARS).collect()
                }
                _ => title_of(&conversation.messages),
            },
        };
        conversation.updated_at_ms = now_ms().max(conversation.created_at_ms);
        // 截图不存盘（很大且很快过时）；调用方手里的对话保持原样，本轮还要用。
        let mut stored = conversation.clone();
        stored.messages.iter_mut().for_each(super::strip_images);
        // 后台任务的登记另存（见模块说明），对话文件里不写。
        stored.subagents.clear();
        let bytes = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
        let mut index = self.index.lock().unwrap();
        write_atomic(&self.path(&conversation.id), &bytes)?;
        index.conversations.retain(|c| c.id != conversation.id);
        index.conversations.insert(0, conversation.summary());
        for old in evicted(&mut index.conversations) {
            let _ = std::fs::remove_file(self.path(&old));
            let _ = std::fs::remove_file(self.subagents_path(&old));
            if index.current.as_deref() == Some(old.as_str()) {
                index.current = None;
            }
        }
        self.persist_index(&index)
    }

    /// 删除；不存在时返回 false。
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        if !valid_id(id) {
            return Err("对话 id 无效".into());
        }
        let mut index = self.index.lock().unwrap();
        let existed = match std::fs::remove_file(self.path(id)) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("删除对话失败：{e}")),
        };
        let _ = std::fs::remove_file(self.subagents_path(id));
        let before = index.conversations.len();
        index.conversations.retain(|c| c.id != id);
        if index.current.as_deref() == Some(id) {
            index.current = None;
        }
        self.persist_index(&index)?;
        Ok(existed || before != index.conversations.len())
    }

    /// 当前对话（游戏内与聊天窗口共用）。
    pub fn current(&self) -> Option<String> {
        self.index.lock().unwrap().current.clone()
    }

    pub fn set_current(&self, id: Option<&str>) -> Result<(), String> {
        if let Some(id) = id {
            if !valid_id(id) {
                return Err("对话 id 无效".into());
            }
        }
        let mut index = self.index.lock().unwrap();
        index.current = id.map(str::to_string);
        self.persist_index(&index)
    }
}
