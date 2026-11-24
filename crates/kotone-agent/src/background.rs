//! 主会话里慢的工具转到后台（ADR 029 §一“任何工具超过 3 秒没回来就转给子代理”；MCP 方案 4.3、第 1.5 期）。
//!
//! 照着做：LiveKit Agents（`voice/speech_handle.py`：工具执行超过 `INTERRUPTION_TIMEOUT` 就不再卡住这一轮）、
//! Pipecat（`cancel_on_interruption=False` 的异步工具：先给模型一个“在做”的结果，做完把结果放进上下文、
//! 再叫一次模型）。Kotone 的做法：
//!
//! - 主会话（和玩家说话的那段）里，权限过了、工具开始执行时计时；[`WINDOW`] 内做完照常返回；
//! - 没做完：这一轮不再等它。宿主把它登记成一件后台的事（和 `agent.start` 派出去的同一套卡片、交回），
//!   给模型的结果是“转到后台了，做完会交回”（[`moved_result`]）；语音回合这一步就结束、接着听
//!   （`AgentLimits::stop_after_background`），这一步没 `speak` 时 Kotone 念预先合成的
//!   “交给我，好了告诉你”（[`HandoffLine`]）；
//! - 工具在后台接着跑完，宿主照子代理做完的方式交回主会话（先存盘再交回、空闲时插一条带名字的“完成”消息
//!   并唤醒主会话说一句）；被停下（Esc、“停下”、卡片上的停止）就不交回。
//!
//! 不计时的（登记时标 `ToolEntry::own_wait`，本来就要等）：`speak`（念完才返回）、`reply.ask`（等玩家回答）、
//! `connections.request_key`（等玩家粘贴 Key）、`agent.start` / `agent.followup`（自己有等待窗口，
//! [`crate::subagents::sync_window`]）。等玩家确认权限的时间也不算（计时从执行开始）。
//! 后台的子代理不计时（它们本来就在后台）。

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kotone_ai::agent::store::SubagentStatus;
use kotone_ai::agent::{ToolCall, ToolOutcome};
use kotone_core::cancel::CancellationToken;
use serde_json::{json, Value};
use tokio::sync::Notify;

/// 主会话里工具最多等多久就转到后台（ADR 029 §一写的 3 秒；LiveKit 是 5 秒）。
pub const WINDOW: Duration = Duration::from_secs(3);

/// 交回主会话的结果最多多少字（工具的原始结果可能很长；完整的留在卡片展开的过程里）。
pub const MAX_RESULT_CHARS: usize = 4_000;

/// 转到后台的一次调用（交给宿主登记、做完时再交回宿主）。
#[derive(Clone)]
pub struct Moved {
    /// 模型给的调用 id（主会话里这一行工具变成后台任务卡片）。
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    /// 停下它用的（宿主登记进“在做的后台的事”，Esc、“停下”、卡片上的停止都经它）。
    pub cancel: CancellationToken,
}

/// 给模型的那份结果（照 `agent.start` 返回 `running` 的写法）。`background: true` 也让界面把这一行显示成
/// 后台任务卡片（[`is_moved_result`]）。
pub fn moved_result(name: &str, tool: &str) -> Value {
    json!({
        "name": name,
        "tool": tool,
        "status": "running",
        "background": true,
        "note": "这一步超过 3 秒还没做完，转到后台接着做了，做完会把结果交回给你。不要轮询，也不要再调用它；要说话的话只说一句“这个要一会儿，好了告诉你”。",
    })
}

/// 这条工具结果是不是“转到后台了”（[`moved_result`]）。
pub fn is_moved_result(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|v| v.get("background").and_then(Value::as_bool))
        == Some(true)
}

/// 这件后台的事在登记里的名字：工具名改成小写字母、数字、连字符（`lol.scout` → `lol-scout`），
/// 和已有的重了就加 `-2`、`-3`……（[`crate::subagents::valid_name`]）。
pub fn name_for<'a>(tool: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let mut slug = String::new();
    for c in tool.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.trim_matches('-').chars().take(26).collect();
    slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        slug = "tool".into();
    }
    let taken: Vec<&str> = taken.into_iter().collect();
    if !taken.contains(&slug.as_str()) {
        return slug;
    }
    (2..)
        .map(|n| format!("{slug}-{n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .unwrap_or(slug)
}

/// 这件事的任务消息（它那段隐藏对话的第一条，卡片展开时看得到）：玩家原话在前（有的话），
/// 然后是“哪个工具、什么参数”。Kotone 关掉时还没做完、玩家点了“继续”，后台照这句重新做一遍。
pub fn task_text(player_words: Option<&str>, tool: &str, arguments: &Value) -> String {
    let mut out = String::new();
    if let Some(words) = player_words.map(str::trim).filter(|w| !w.is_empty()) {
        out.push_str(&format!("<player-request>{words}</player-request>\n\n"));
    }
    out.push_str(&format!(
        "（Kotone：{tool} 超过 3 秒还没做完，转到后台接着做。）要重新做就调用 {tool}，参数是 {arguments}；\
做完把结果写成简短的总结。"
    ));
    out
}

/// 做完的结果怎么记：出错算 failed；结果太长截断（完整的在卡片展开的过程里）。
pub fn finished(outcome: &ToolOutcome) -> (SubagentStatus, String) {
    let status = if outcome.is_error {
        SubagentStatus::Failed
    } else {
        SubagentStatus::Done
    };
    let text = if outcome.is_error {
        format!("工具执行失败：{}", outcome.content)
    } else {
        outcome.content.clone()
    };
    let summary = match text.char_indices().nth(MAX_RESULT_CHARS) {
        None => text,
        Some((cut, _)) => format!(
            "{}\n…（结果太长，只交回前 {MAX_RESULT_CHARS} 字）",
            &text[..cut]
        ),
    };
    (status, summary)
}

/// 语音回合里“交给我，好了告诉你”什么时候由 Kotone 替她说（一轮一份，按步更新）。
/// 一步里只要有 `speak` 就不替她说（她自己说了）；一步里最多说一次。
#[derive(Debug, Default)]
pub struct HandoffLine {
    spoke: bool,
    said: bool,
}

impl HandoffLine {
    /// 模型这一步要调用的工具（执行前）。交给了后台（`agent.start` / `agent.followup`）、却没有 `speak`：
    /// 返回 true，马上说（不等派活的结果）。
    pub fn on_step(&mut self, calls: &[ToolCall]) -> bool {
        self.spoke = calls.iter().any(|c| c.name == crate::catalog::TOOL_SPEAK);
        self.said = !self.spoke
            && calls
                .iter()
                .any(|c| crate::subagents::is_handoff_tool(&c.name));
        self.said
    }

    /// 这一步里有个工具超过 [`WINDOW`] 转到后台了：这一步没 `speak`、也还没说过，返回 true（说一次）。
    pub fn on_moved(&mut self) -> bool {
        if self.spoke || self.said {
            return false;
        }
        self.said = true;
        true
    }
}

// ---------- 计时执行 ----------

type Deliver = Box<dyn FnOnce(ToolOutcome) + Send>;

enum Stage {
    Running,
    Done(ToolOutcome),
    /// 转到后台了：做完交给它。
    Moved(Deliver),
    Closed,
}

struct Shared {
    stage: Mutex<Stage>,
    done: Notify,
}

/// 在独立的任务里跑一个工具，调用方可以只等一会儿（[`Timed::wait`]），没做完就转到后台（[`Timed::moved`]）。
/// 没转到后台之前 `Timed` 被丢掉（这一轮被停下、到了时限），工具跟着取消。
pub(crate) struct Timed {
    shared: Arc<Shared>,
    token: CancellationToken,
    armed: bool,
}

impl Timed {
    pub(crate) fn spawn<F>(work: F) -> Self
    where
        F: Future<Output = ToolOutcome> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            stage: Mutex::new(Stage::Running),
            done: Notify::new(),
        });
        let token = CancellationToken::default();
        let (task_shared, task_token) = (shared.clone(), token.clone());
        tokio::spawn(async move {
            let outcome = tokio::select! {
                biased;
                _ = task_token.cancelled() => return,
                outcome = work => outcome,
            };
            let deliver = {
                let mut stage = task_shared.stage.lock().unwrap();
                match std::mem::replace(&mut *stage, Stage::Closed) {
                    Stage::Running => {
                        *stage = Stage::Done(outcome);
                        drop(stage);
                        task_shared.done.notify_one();
                        return;
                    }
                    Stage::Moved(deliver) => deliver,
                    Stage::Done(_) | Stage::Closed => return,
                }
            };
            // 停下了的不交回（宿主那边也会再看一眼登记）。
            if !task_token.is_cancelled() {
                deliver(outcome);
            }
        });
        Self {
            shared,
            token,
            armed: true,
        }
    }

    /// 停下它用的令牌（转到后台时交给宿主登记）。
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    fn take(&self) -> Option<ToolOutcome> {
        let mut stage = self.shared.stage.lock().unwrap();
        if !matches!(*stage, Stage::Done(_)) {
            return None;
        }
        match std::mem::replace(&mut *stage, Stage::Closed) {
            Stage::Done(outcome) => Some(outcome),
            _ => None,
        }
    }

    /// 等它做完，最多 `window`（None = 一直等）。到点还没做完返回 None（它接着跑）。
    pub(crate) async fn wait(&self, window: Option<Duration>) -> Option<ToolOutcome> {
        let wait = async {
            loop {
                if let Some(outcome) = self.take() {
                    return outcome;
                }
                // `notify_one` 在没人等时会留一个许可：做完和这里开始等之间不会漏掉。
                self.shared.done.notified().await;
            }
        };
        match window {
            Some(window) => tokio::time::timeout(window, wait).await.ok(),
            None => Some(wait.await),
        }
    }

    /// 转到后台：之后做完的结果交给 `deliver`；恰好已经做完了就马上交给它。之后丢掉 `Timed` 也不取消。
    pub(crate) fn moved(mut self, deliver: impl FnOnce(ToolOutcome) + Send + 'static) {
        self.armed = false;
        let mut stage = self.shared.stage.lock().unwrap();
        match std::mem::replace(&mut *stage, Stage::Closed) {
            Stage::Running => *stage = Stage::Moved(Box::new(deliver)),
            Stage::Done(outcome) => {
                drop(stage);
                deliver(outcome);
            }
            Stage::Moved(_) | Stage::Closed => {}
        }
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: "{}".into(),
        }
    }

    async fn slow(ms: u64, text: &str) -> ToolOutcome {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        ToolOutcome::ok(text)
    }

    #[tokio::test(start_paused = true)]
    async fn a_quick_tool_comes_back_within_the_window() {
        let timed = Timed::spawn(slow(1_000, "好了"));
        let started = tokio::time::Instant::now();
        let out = timed.wait(Some(WINDOW)).await.expect("1 秒内做完");
        assert_eq!(out.content, "好了");
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_tool_is_handed_to_the_background_once() {
        let timed = Timed::spawn(slow(5_000, "战绩"));
        let started = tokio::time::Instant::now();
        assert!(timed.wait(Some(WINDOW)).await.is_none());
        assert_eq!(started.elapsed(), WINDOW, "3 秒就不再等");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let token = timed.token();
        timed.moved(move |outcome| {
            tx.send(outcome.content).unwrap();
        });
        assert_eq!(rx.await.unwrap(), "战绩");
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        assert!(!token.is_cancelled(), "转到后台以后不随调用方取消");
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_a_moved_tool_hands_nothing_back() {
        let timed = Timed::spawn(slow(5_000, "战绩"));
        assert!(timed.wait(Some(WINDOW)).await.is_none());
        let token = timed.token();
        let delivered = Arc::new(Mutex::new(0));
        let count = delivered.clone();
        timed.moved(move |_| *count.lock().unwrap() += 1);
        token.cancel();
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(*delivered.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_before_the_move_cancels_the_tool() {
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let timed = Timed::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            *flag.lock().unwrap() = true;
            ToolOutcome::ok("x")
        });
        let token = timed.token();
        drop(timed);
        assert!(token.is_cancelled());
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(!*ran.lock().unwrap(), "这一轮被停下：工具跟着停");
    }

    #[tokio::test(start_paused = true)]
    async fn finishing_right_at_the_move_is_still_handed_back() {
        let timed = Timed::spawn(slow(3_000, "刚好"));
        assert!(timed
            .wait(Some(Duration::from_millis(2_999)))
            .await
            .is_none());
        tokio::time::sleep(Duration::from_millis(10)).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        timed.moved(move |outcome| tx.send(outcome.content).unwrap());
        assert_eq!(rx.await.unwrap(), "刚好", "登记时已经做完：马上交回，不丢");
    }

    #[test]
    fn the_handoff_line_is_said_once_and_only_when_she_did_not_speak() {
        // 交给后台、没 speak：马上说；同一步里再转走一个工具不再说
        let mut line = HandoffLine::default();
        assert!(line.on_step(&[call("agent.start")]));
        assert!(!line.on_moved());
        // 和 speak 一起：不说
        assert!(!line.on_step(&[call("agent.start"), call("speak")]));
        assert!(!line.on_moved());
        assert!(!line.on_step(&[call("web.search")]));
        // 慢的工具转走了、这一步没 speak：说一次
        assert!(line.on_moved());
        assert!(!line.on_moved());
        // 下一步重新算
        assert!(!line.on_step(&[call("lol.scout"), call("speak")]));
        assert!(!line.on_moved());
        assert!(line.on_step(&[call("agent.followup")]));
    }

    #[test]
    fn moved_results_are_recognised_and_named_like_tasks() {
        let v = moved_result("lol-scout", "lol.scout");
        assert!(is_moved_result(&v.to_string()));
        assert_eq!(v["status"], "running");
        assert!(!is_moved_result(r#"{"status":"running"}"#));
        assert!(!is_moved_result("不是 JSON"));
        assert_eq!(name_for("lol.scout", []), "lol-scout");
        assert_eq!(name_for("game_chat.send", []), "game-chat-send");
        assert_eq!(
            name_for("lol.scout", ["lol-scout", "lol-scout-2"]),
            "lol-scout-3"
        );
        assert_eq!(name_for("...", []), "tool");
        let long = name_for(&"a".repeat(80), []);
        assert!(crate::subagents::valid_name(&long), "{long}");
        assert!(crate::subagents::valid_name(&name_for(
            &"a".repeat(80),
            [long.as_str()]
        )));
        let task = task_text(Some("查一下对面中单"), "lol.scout", &json!({"player": "x"}));
        assert!(task.starts_with("<player-request>查一下对面中单</player-request>"));
        assert!(task.contains("lol.scout") && task.contains("\"player\""));
        assert!(!task_text(None, "lol.scout", &json!({})).contains("player-request"));
    }

    #[test]
    fn finished_results_keep_errors_and_are_capped() {
        let (status, text) = finished(&ToolOutcome::ok("26.22"));
        assert_eq!((status, text.as_str()), (SubagentStatus::Done, "26.22"));
        let (status, text) = finished(&ToolOutcome::error("客户端没开"));
        assert_eq!(status, SubagentStatus::Failed);
        assert!(text.contains("客户端没开"));
        let (_, text) = finished(&ToolOutcome::ok("字".repeat(MAX_RESULT_CHARS + 10)));
        assert!(text.chars().count() < MAX_RESULT_CHARS + 40 && text.contains("太长"));
    }
}
