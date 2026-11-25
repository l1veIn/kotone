//! 她正忙时插进来、要进聊天的东西怎么排——只写在这里（共享资源评估第二节第 9 条、第四节“聊天流”：排队，不丢）。
//!
//! 谁用：自动卡片（[`post_auto_card`]）、对局复盘的那一条（[`post_notice`]，`recap.rs`）、自动化跑完的结果
//! （[`on_run_finished`]）、交回不是当前对话的后台结果（`relay.rs`）、“让她做”没做成时留的那一句（[`run_prompt`]）。
//!
//! 规矩（[`post_locked`]）：
//! 1. **她这一轮就该看的**（[`IfBusy::Steer`]：自动化的结果、交回的后台结果）：那段对话正有模型在跑，就作为插话
//!    交给这一轮（插话不算排队）。
//! 2. **她空着**（[`main_is_quiet`]：没在回答、没在说话、没有提问在等、玩家没在说话），**那段对话前面也没有排着的**：
//!    马上记进对话末尾。
//! 3. **其余排队**：每段对话照来的先后排着，存一份在盘上；她空了（每 500 毫秒看一次）照先后记进去。一段对话在跑不挡
//!    别的对话。一直不空的话 10 分钟后只等那段对话不在跑（这时记进去不会被那一轮盖掉）。Kotone 退出了，下次启动接着记
//!    （[`resume_held_cards`]）。
//! 4. 只有真记不进去才放弃（记日志）：那段对话被删了；读写对话连着出错 5 次（`kotone_agent::auto_card::MAX_FAILURES`）。
//!
//! 记进对话都经 [`append_locked`]（和认领主会话同一把锁，不会被正要开始的一轮盖掉）。排队用的是自动卡片原来那张
//! “桌子”（`kotone_agent::auto_card::Desk`，卡片的去重也在它上面）；盘上的文件名照旧是 `agent/auto-cards-held.json`
//! （名字是只有自动卡片时起的；里面就是对话消息，旧版本读得懂）。锁的先后：先“桌子”再 `Inner`，反过来的地方没有。
//!
//! 不在这里的：要叫她开口说一句的（后台的结果交回当前对话 `relay::schedule_report`、“让她做” `relay::run_prompt`）——
//! 那要花一次 AI 调用，另有等法；“让她做”等过 2 分钟就不做了（ADR 036，事件已经过去了），不做时照这里的规矩在聊天里
//! 留一句。

use std::collections::HashSet;
use std::sync::OnceLock;

use kotone_agent::auto_card::{self as cards, Desk, Held};

use super::*;

/// 排着的多久看一次她空了没有。
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// 一直不空（玩家一直在和她说话）最多等多久：之后只等那段对话不在跑（这时记进对话不会被覆盖），不再等她说完。
const QUIET_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// 往对话末尾记时，那段对话正在跑一轮怎么办。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum IfBusy {
    /// 排着，等这一轮结束、她空了再记（自动卡片、复盘：不插进正在跑的那一轮）。
    Wait,
    /// 有模型在跑就作为插话交给这一轮（自动化的结果、交回的后台结果：本来就要她看）；插不进话（转到后台的工具在跑）
    /// 就排着。
    Steer,
}

/// [`post`] 的结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Posted {
    /// 记进对话了。
    Saved,
    /// 作为插话交给正在跑的那一轮了。
    Steered,
    /// 排着，她空了再记。
    Held,
    /// 那段对话没了（被删了）。
    Gone,
}

/// 照上面的规矩：现在记、插话，还是排着（纯逻辑，`quiet` 是 [`main_is_quiet`]）。排着的放进 `desk`，调用方存盘。
pub(super) fn post_locked(
    inner: &Mutex<Inner>,
    store: &ConversationStore,
    desk: &mut Desk,
    id: &str,
    messages: &[Message],
    if_busy: IfBusy,
    quiet: bool,
) -> Posted {
    if if_busy == IfBusy::Steer && steer_locked(inner, id, messages) {
        return Posted::Steered;
    }
    let queued = desk.held().iter().any(|h| h.conversation == id);
    if quiet && !queued {
        match append_locked(inner, store, id, messages) {
            Ok(Appended::Saved) => return Posted::Saved,
            Ok(Appended::Gone) => return Posted::Gone,
            // 正好开始了一轮：排着
            Ok(Appended::Busy) => {}
            Err(e) => log::log(&format!("agent: 记进对话 {id} 失败，排着等下次：{e}")),
        }
    }
    desk.hold(id, messages.to_vec());
    Posted::Held
}

/// 把排着的照先后记进对话（她空了时调）。一段对话在跑：它排着的接着排（保持先后），别的对话照记。返回记进去的
/// （发界面事件用）。
pub(super) fn flush_locked(
    inner: &Mutex<Inner>,
    store: &ConversationStore,
    desk: &mut Desk,
) -> Vec<Held> {
    let mut saved = Vec::new();
    let mut back = Vec::new();
    // 这一次不再试的对话（在跑、刚出错）：它后面的也不能先记
    let mut waiting: HashSet<String> = HashSet::new();
    for h in desk.take() {
        if waiting.contains(&h.conversation) {
            back.push(h);
            continue;
        }
        match append_locked(inner, store, &h.conversation, &h.messages) {
            Ok(Appended::Saved) => saved.push(h),
            Ok(Appended::Gone) => log::log(&format!(
                "agent: 对话 {} 没了，排着的 {} 条不记了",
                h.conversation,
                h.messages.len()
            )),
            Ok(Appended::Busy) => {
                waiting.insert(h.conversation.clone());
                back.push(h);
            }
            Err(e) => {
                let conversation = h.conversation.clone();
                waiting.insert(conversation.clone());
                match Desk::failed(h) {
                    Some(h) => back.push(h),
                    None => log::log(&format!(
                        "agent: 记进对话 {conversation} 连着失败 {} 次，排着的这一组不记了：{e}",
                        cards::MAX_FAILURES
                    )),
                }
            }
        }
    }
    desk.put_back(back);
    saved
}

/// 往对话 `id` 末尾记几条不是玩家说的话（照上面的规矩）。记进去了通知界面；排着的存盘、等她空了再记。
pub(super) fn post(
    app: &AppHandle,
    id: &str,
    messages: &[Message],
    if_busy: IfBusy,
) -> Result<Posted, String> {
    // 先看她空不空（要 `Inner` 的锁），再拿“桌子”（锁的先后见模块说明）
    let quiet = main_is_quiet(app);
    let rt = runtime(app)?;
    let store = store(app)?;
    let posted = {
        let mut desk = desk().lock().unwrap();
        let posted = post_locked(&rt.inner, &store, &mut desk, id, messages, if_busy, quiet);
        if posted == Posted::Held {
            save_held(&desk);
        }
        posted
    };
    match posted {
        Posted::Saved => emit_appended(app, id, messages),
        Posted::Held => schedule_flush(app),
        Posted::Steered | Posted::Gone => {}
    }
    Ok(posted)
}

/// 往当前的主会话（没有就最近的主会话，一段都没有就开一段新的）末尾记一条说明（`Notice` 不发给模型）：她忙就排着。
/// 返回记到（或排到）了哪段对话。
pub fn post_notice(app: &AppHandle, notice: Message) -> Option<String> {
    let id = match store(app).and_then(|store| target_conversation(&store)) {
        Ok(id) => id,
        Err(e) => {
            log::log(&format!("agent: 说明没地方记：{e}"));
            return None;
        }
    };
    match post(app, &id, &[notice], IfBusy::Wait) {
        Ok(Posted::Gone) => None,
        Ok(_) => Some(id),
        Err(e) => {
            log::log(&format!("agent: 说明没记进对话：{e}"));
            None
        }
    }
}

/// 发到哪段对话：当前的主会话；没有就最近的主会话；一段都没有就开一段新的。
pub(super) fn target_conversation(store: &ConversationStore) -> Result<String, String> {
    if let Some(id) = store.current() {
        if store.load(&id)?.is_some_and(|c| c.parent.is_none()) {
            return Ok(id);
        }
    }
    if let Some(summary) = store.list().into_iter().find(|c| c.parent.is_none()) {
        return Ok(summary.id);
    }
    let mut conversation = store.create();
    store.save(&mut conversation)?;
    store.set_current(Some(&conversation.id))?;
    Ok(conversation.id)
}

fn emit_appended(app: &AppHandle, id: &str, messages: &[Message]) {
    for message in messages {
        emit(
            app,
            id,
            event_json(&AgentEvent::Appended {
                message: message.clone(),
            }),
        );
    }
}

/// 排着的存在哪（只有排着时有这个文件）。名字是只有自动卡片时起的，旧版本也认得。
fn held_path() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir()
        .join("agent")
        .join("auto-cards-held.json")
}

/// 排着的（全进程一份）。第一次用时接上盘上存着的（上次退出时还没记进对话的）。
pub(super) fn desk() -> &'static Mutex<Desk> {
    static DESK: OnceLock<Mutex<Desk>> = OnceLock::new();
    DESK.get_or_init(|| {
        let mut desk = Desk::default();
        if let Ok(bytes) = std::fs::read(held_path()) {
            match serde_json::from_slice::<Vec<Held>>(&bytes) {
                Ok(held) => desk.restore(held),
                Err(e) => log::log(&format!("agent: 排着的消息读不了，不要了：{e}")),
            }
        }
        Mutex::new(desk)
    })
}

/// 把排着的写到盘上（空了就删掉文件）。调用方拿着 `desk()` 的锁。
fn save_held(desk: &Desk) {
    let path = held_path();
    let result = if desk.held().is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    } else {
        serde_json::to_vec(desk.held())
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes)
                    .and_then(|_| std::fs::rename(&tmp, &path))
                    .map_err(|e| e.to_string())
            })
    };
    if let Err(e) = result {
        log::log(&format!("agent: 排着的消息存不了：{e}"));
    }
}

/// 启动时：上次退出时还排着的（自动卡片、复盘、自动化的结果……），等她空下来接着记进对话。名字是只有自动卡片时起的
/// （`lib.rs` 在调）。
pub fn resume_held_cards(app: &AppHandle) {
    if desk().lock().unwrap().has_held() {
        log::log("agent: 上次还有排着的消息，她空下来接着记进对话");
        schedule_flush(app);
    }
}

/// 排着的：等她空下来再记进对话（全进程一个循环）。
fn schedule_flush(app: &AppHandle) {
    static FLUSHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FLUSHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(FLUSH_EVERY).await;
            let ready = main_is_quiet(&app) || started.elapsed() >= QUIET_WAIT;
            if ready && flush(&app) {
                FLUSHING.store(false, std::sync::atomic::Ordering::SeqCst);
                // 交还以后又排进来的（同一把锁外，极少）：再排一次
                if desk().lock().unwrap().has_held() {
                    schedule_flush(&app);
                }
                return;
            }
        }
    });
}

/// 把排着的都记进对话、存盘、通知界面。没有排着的了返回 true。
fn flush(app: &AppHandle) -> bool {
    let (saved, done) = {
        let mut desk = desk().lock().unwrap();
        let saved = match (runtime(app), store(app)) {
            (Ok(rt), Ok(store)) => flush_locked(&rt.inner, &store, &mut desk),
            (Err(e), _) | (_, Err(e)) => {
                // 对话存储打不开：每组都算失败一次，到了上限就放弃
                let back = desk
                    .take()
                    .into_iter()
                    .filter_map(|h| {
                        let conversation = h.conversation.clone();
                        let kept = Desk::failed(h);
                        if kept.is_none() {
                            log::log(&format!(
                                "agent: 记进对话 {conversation} 连着失败 {} 次，排着的这一组不记了：{e}",
                                cards::MAX_FAILURES
                            ));
                        }
                        kept
                    })
                    .collect();
                desk.put_back(back);
                Vec::new()
            }
        };
        save_held(&desk);
        (saved, !desk.has_held())
    };
    for h in &saved {
        log::log(&format!(
            "agent: 排着的 {} 条记进对话 {}（她空了）",
            h.messages.len(),
            h.conversation
        ));
        emit_appended(app, &h.conversation, &h.messages);
    }
    done
}

/// 某段对话里排着、还没记进对话的自动卡片（界面重新读对话时照样显示）。
pub(super) fn held_cards(id: &str) -> Vec<Message> {
    desk().lock().unwrap().held_cards(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_util::*;

    /// 对话存储、运行时状态、排队的“桌子”（不碰盘上的 `auto-cards-held.json`）。
    struct Fixture {
        _dir: tempfile::TempDir,
        store: ConversationStore,
        inner: Mutex<Inner>,
        desk: Desk,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = ConversationStore::open(dir.path()).unwrap();
            Self {
                _dir: dir,
                store,
                inner: Mutex::new(Inner::default()),
                desk: Desk::default(),
            }
        }

        /// 一段已有一问一答的当前对话。
        fn current(&self) -> String {
            let mut c = self.store.create();
            c.messages = vec![user("chat"), assistant("在呢", &[])];
            self.store.save(&mut c).unwrap();
            self.store.set_current(Some(&c.id)).unwrap();
            c.id
        }

        /// 一个转到后台的工具（没有模型读收件箱）在跑的那段隐藏对话。
        fn moved_tool(&self) -> String {
            let mut child = self.store.create();
            child.parent = Some("c-main".into());
            self.store.save(&mut child).unwrap();
            self.inner.lock().unwrap().children.insert(
                child.id.clone(),
                Child {
                    spec: ChildSpec {
                        parent: "c-main".into(),
                        name: "x".into(),
                        label: "x".into(),
                        owner: kotone_core::usage::Owner {
                            conversation: "c-main".into(),
                            turn: None,
                        },
                    },
                    running: Some(running(&child.id, true)),
                    waiters: HashMap::new(),
                    ask: None,
                },
            );
            child.id
        }

        fn post(&mut self, id: &str, message: Message, if_busy: IfBusy, quiet: bool) -> Posted {
            post_locked(
                &self.inner,
                &self.store,
                &mut self.desk,
                id,
                &[message],
                if_busy,
                quiet,
            )
        }

        /// 她空了：记进去了几组。
        fn flush(&mut self) -> usize {
            flush_locked(&self.inner, &self.store, &mut self.desk).len()
        }

        fn saved(&self, id: &str) -> Vec<String> {
            self.store
                .load(id)
                .unwrap()
                .unwrap()
                .messages
                .iter()
                .map(|m| match m {
                    Message::User { text, .. }
                    | Message::Assistant { text, .. }
                    | Message::Notice { text, .. } => text.clone(),
                    _ => String::new(),
                })
                .collect()
        }
    }

    fn running(id: &str, moved_tool: bool) -> Running {
        Running {
            conversation_id: id.to_string(),
            cancel: CancellationToken::default(),
            inbox: Arc::default(),
            moved_tool,
        }
    }

    const RECAP: &str = "对局复盘：英雄联盟（32 分钟）";

    fn recap() -> Message {
        Message::Notice {
            text: RECAP.into(),
            at_ms: 1,
            compaction: None,
            usage: None,
            recap: Some("- 中期团战站位太靠前".into()),
            card: None,
        }
    }

    /// 共享资源评估第二节第 9 条（复现过：原来等 3 分钟没空就放弃，那条既没进聊天、也不排着）：复盘那条来的时候她在
    /// 跑一轮（假的“她正忙”），等多久都排着；她这一轮说完，那条接在她的回答后面进聊天。
    #[test]
    fn a_recap_posted_while_she_stays_busy_still_reaches_the_chat() {
        let mut f = Fixture::new();
        let id = f.current();
        f.inner.lock().unwrap().running = Some(running(&id, false));
        assert_eq!(f.post(&id, recap(), IfBusy::Wait, false), Posted::Held);
        // 她一直在忙：看多少次都还排着，对话一个字没动
        for _ in 0..100 {
            assert_eq!(f.flush(), 0);
        }
        assert!(f.desk.has_held());
        assert_eq!(f.saved(&id), ["嗨", "在呢"]);
        // 这一轮说完了（照 `turn` 存盘）
        let mut c = f.store.load(&id).unwrap().unwrap();
        c.messages.push(assistant("这把打得不错", &[]));
        f.store.save(&mut c).unwrap();
        f.inner.lock().unwrap().running = None;
        assert_eq!(f.flush(), 1);
        assert!(!f.desk.has_held());
        assert_eq!(f.saved(&id), ["嗨", "在呢", "这把打得不错", RECAP]);
    }

    /// 共享资源评估第二节第 9 条（函数这一层复现过：原来插不进话就只记一行日志）：自动化跑完时，发起它的那段对话是一个
    /// 转到后台的工具在跑（假的“她正忙”：没有模型读收件箱）。结果排着；那个工具做完，结果进了那段对话。
    #[test]
    fn an_automation_result_is_not_lost_when_its_conversation_cannot_take_it() {
        let mut f = Fixture::new();
        let child = f.moved_tool();
        let result = Message::user("自动化「连招」已完成");
        assert_eq!(f.post(&child, result, IfBusy::Steer, true), Posted::Held);
        assert_eq!(f.flush(), 0);
        // 转到后台的工具做完了
        f.inner
            .lock()
            .unwrap()
            .children
            .get_mut(&child)
            .unwrap()
            .running = None;
        assert_eq!(f.flush(), 1);
        assert_eq!(f.saved(&child), ["自动化「连招」已完成"]);
    }

    /// 有模型在跑：要她看的作为插话交给这一轮（文件不动）；要等的排着。
    #[test]
    fn results_for_her_are_steered_into_the_running_turn() {
        let mut f = Fixture::new();
        let id = f.current();
        let main = running(&id, false);
        let inbox = main.inbox.clone();
        f.inner.lock().unwrap().running = Some(main);
        let result = Message::user("自动化跑完了");
        assert_eq!(f.post(&id, result, IfBusy::Steer, false), Posted::Steered);
        assert!(
            matches!(inbox.next_follow_up(), Some(Message::User { text, .. }) if text == "自动化跑完了")
        );
        assert_eq!(f.post(&id, recap(), IfBusy::Wait, false), Posted::Held);
        assert_eq!(f.saved(&id), ["嗨", "在呢"]);
    }

    /// 她空着、前面没有排着的：马上记。她不空（在说话、玩家在说话）：排着。前面有排着的：排在它们后面（照来的先后）。
    #[test]
    fn queued_ones_go_first_in_the_order_they_came() {
        let mut f = Fixture::new();
        let id = f.current();
        let note = |text: &str| Message::user(text);
        assert_eq!(f.post(&id, note("一"), IfBusy::Wait, true), Posted::Saved);
        // 她在说话（这一轮已经结束）：排着
        assert_eq!(f.post(&id, note("二"), IfBusy::Wait, false), Posted::Held);
        // 空下来了，但前面还排着一条：这条排在它后面，不抢先
        assert_eq!(f.post(&id, note("三"), IfBusy::Steer, true), Posted::Held);
        assert_eq!(f.saved(&id), ["嗨", "在呢", "一"]);
        assert_eq!(f.flush(), 2);
        assert_eq!(f.saved(&id), ["嗨", "在呢", "一", "二", "三"]);
    }

    /// 一段对话在跑不挡别的对话；被删了的对话：现在记报 Gone，排着的不记了（记日志）。
    #[test]
    fn one_busy_conversation_does_not_hold_up_the_others() {
        let mut f = Fixture::new();
        let id = f.current();
        let child = f.moved_tool();
        let mut gone = f.store.create();
        f.store.save(&mut gone).unwrap();
        // 都排着（她在说话）
        for (target, text) in [
            (&child, "后台一"),
            (&id, "主一"),
            (&gone.id, "删掉的"),
            (&id, "主二"),
        ] {
            let posted = f.post(target, Message::user(text), IfBusy::Wait, false);
            assert_eq!(posted, Posted::Held);
        }
        f.store.delete(&gone.id).unwrap();
        // 后台那段还在跑：它的接着排；主会话的照先后记进去；删掉的那段不记了
        assert_eq!(f.flush(), 2);
        assert_eq!(f.saved(&id), ["嗨", "在呢", "主一", "主二"]);
        assert_eq!(f.desk.held().len(), 1);
        assert_eq!(f.desk.held()[0].conversation, child);
        assert_eq!(f.post("c-gone", recap(), IfBusy::Wait, true), Posted::Gone);
    }

    /// 复盘、“让她做”留的那句发到哪：当前的主会话；当前是后台那段隐藏的对话时发到最近的主会话；一段都没有就开一段。
    #[test]
    fn notices_go_to_the_current_main_conversation() {
        let f = Fixture::new();
        let fresh = target_conversation(&f.store).unwrap();
        assert_eq!(f.store.current().as_deref(), Some(fresh.as_str()));
        let id = f.current();
        assert_eq!(target_conversation(&f.store).unwrap(), id);
        let mut child = f.store.create();
        child.parent = Some(id.clone());
        f.store.save(&mut child).unwrap();
        f.store.set_current(Some(&child.id)).unwrap();
        assert_ne!(target_conversation(&f.store).unwrap(), child.id);
    }
}
