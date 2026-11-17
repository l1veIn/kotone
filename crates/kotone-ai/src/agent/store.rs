//! 对话存储（ADR 014 §3 “对话保存在本机”）。
//!
//! 目录结构（目录由调用方给，桌面端为 `~/.kotone/agent/`）：
//! - `index.json`：版本、当前对话（游戏内与聊天窗口共用同一段，见 ADR 014“已确认”）、各对话摘要；
//! - `conversations/<id>.json`：一段对话的全部消息。
//!
//! 都是原子写入。索引丢失或损坏时从对话文件重建；对话文件损坏时报错，不删除。
//! 上限：最多保留 [`MAX_CONVERSATIONS`] 段（超出删最久没更新的），每段最多
//! [`MAX_STORED_MESSAGES`] 条（超出从最早的一轮开始丢）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{now_ms, Message};

pub const MAX_CONVERSATIONS: usize = 100;
pub const MAX_STORED_MESSAGES: usize = 400;
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
            Message::User { text, .. } => Some(text.trim()),
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

/// 超出条数上限时，从最早的一轮（以用户消息为界）开始丢。
fn trim_messages(messages: &mut Vec<Message>) {
    if messages.len() <= MAX_STORED_MESSAGES {
        return;
    }
    let excess = messages.len() - MAX_STORED_MESSAGES;
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
        }
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
            Some(label) => format!("子会话：{label}"),
            None => title_of(&conversation.messages),
        };
        conversation.updated_at_ms = now_ms().max(conversation.created_at_ms);
        // 截图不存盘（很大且很快过时）；调用方手里的对话保持原样，本轮还要用。
        let mut stored = conversation.clone();
        stored.messages.iter_mut().for_each(super::strip_images);
        let bytes = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
        let mut index = self.index.lock().unwrap();
        write_atomic(&self.path(&conversation.id), &bytes)?;
        index.conversations.retain(|c| c.id != conversation.id);
        index.conversations.insert(0, conversation.summary());
        while index.conversations.len() > MAX_CONVERSATIONS {
            if let Some(old) = index.conversations.pop() {
                let _ = std::fs::remove_file(self.path(&old.id));
                if index.current.as_deref() == Some(old.id.as_str()) {
                    index.current = None;
                }
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
