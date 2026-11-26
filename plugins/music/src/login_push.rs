//! 网易云登录过期的推送（ADR 043 D11，讨论记录第九节“其它插件”：网易云音乐主要是 App 加工具，内置推送只做必要的）。
//!
//! - 什么时候算过期：以前登录过（凭据库里存着登录凭据），网易云却说没登录——启动时查账号查不到（[`expired_account`]），
//!   或者用着用着某个接口回“要登录”（[`lost_login`]）。从没登录过的不算（她要放歌时聊天页照旧出“登录网易云音乐”那一条）。
//! - 推送 `login`（登录过期，卡片）：一张卡片，上面是“登录网易云音乐”按钮（打开网易云官方的登录页扫码，和音乐页、聊天页
//!   那个按钮是同一个 `music.login_start`）。登好了，这张卡片原地变成“已登录”（同一个 key 再推一次 = 更新，D5）。
//! - 同一次过期只有一张：key 记在插件目录的 `login-push.json` 里，Kotone 重启以后（凭据还是过期的）再推一次是同一张
//!   （内容一样就什么都不做）；登好了删掉。不给她留说明：她要放歌时工具会告诉她要登录。
//!
//! 卡片里的数据（页面 `ui://kotone/music/login`，`plugins/music/ui/MusicLoginPush.svelte` 照它画）：
//!
//! ```text
//! { stage: "expired" }                      // 过期了，等你登录
//! { stage: "done", nickname }               // 登好了
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushOutcome};
use serde_json::{json, Value};

use crate::{Account, MusicError};

/// 清单 `[[pushes]]` 的 id。
pub const PUSH_LOGIN: &str = "login";
/// 卡片的页面（清单 `[[pages]]`）。
pub const PAGE_LOGIN: &str = "ui://kotone/music/login";
/// 卡片的名字（和清单里这条推送的 title 一样，测试查）。
pub const TITLE_LOGIN: &str = "登录过期";
/// 插件目录里记着这次过期那张卡片的 key。
const FILE: &str = "login-push.json";

/// 启动时查账号的结果说明登录过期了没有：存着登录凭据，网易云却说没有账号。查不到（网络、风控）不算。
pub fn expired_account(stored: bool, account: &Result<Option<Account>, MusicError>) -> bool {
    stored && matches!(account, Ok(None))
}

/// 一个接口出的错说明登录过期了没有：存着登录凭据，网易云却回“要登录”。
pub fn lost_login(stored: bool, error: &MusicError) -> bool {
    stored && matches!(error, MusicError::NeedLogin)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn card_push(key: &str, data: Value) -> Push {
    Push {
        trigger: PUSH_LOGIN.into(),
        key: key.into(),
        card: Some(PushCard {
            title: TITLE_LOGIN.into(),
            uri: PAGE_LOGIN.into(),
            params: Value::Null,
            data,
        }),
        ..Push::default()
    }
}

/// 登录过期的那张卡片。
pub(crate) struct LoginPush {
    host: Arc<dyn Host>,
    /// 这次过期那张卡片的 key（没过期 = None）；和 `login-push.json` 一致
    key: Mutex<Option<String>>,
}

impl LoginPush {
    /// 读回上次记着的那张（Kotone 重启前过期了、还没登好）。
    pub(crate) fn new(host: Arc<dyn Host>) -> Self {
        let key = std::fs::read_to_string(Self::file_of(host.as_ref()))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| v["key"].as_str().map(str::to_string))
            .filter(|k| !k.is_empty());
        Self {
            host,
            key: Mutex::new(key),
        }
    }

    fn file_of(host: &dyn Host) -> PathBuf {
        host.data_dir().join(FILE)
    }

    /// 有一张过期的卡片在等玩家登录。
    pub(crate) fn pending(&self) -> bool {
        self.key.lock().unwrap().is_some()
    }

    /// 登录过期了：推那张卡片（同一次过期是同一张；内容一样宿主什么都不做）。
    pub(crate) fn expired(&self) -> Option<PushOutcome> {
        let key = {
            let mut key = self.key.lock().unwrap();
            if key.is_none() {
                let new = format!("{PUSH_LOGIN}:{}", now_ms());
                let file = Self::file_of(self.host.as_ref());
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&file, json!({"key": new}).to_string()) {
                    log::log(&format!("music: 登录过期的卡片没记下来：{e}"));
                }
                *key = Some(new);
            }
            key.clone().expect("刚放进去")
        };
        self.send(card_push(&key, json!({"stage": "expired"})))
    }

    /// 登好了：那张卡片原地变成“已登录”，之后不再管它。
    pub(crate) fn logged_in(&self, account: &Account) -> Option<PushOutcome> {
        let key = self.key.lock().unwrap().take()?;
        let _ = std::fs::remove_file(Self::file_of(self.host.as_ref()));
        self.send(card_push(
            &key,
            json!({"stage": "done", "nickname": account.nickname}),
        ))
    }

    fn send(&self, push: Push) -> Option<PushOutcome> {
        let key = push.key.clone();
        match self.host.push(push) {
            Ok(outcome) => {
                log::log(&format!("music push {key}: {outcome:?}"));
                Some(outcome)
            }
            Err(e) => {
                log::log(&format!("music push {key} 没推出去：{e}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use kotone_plugin_api::{BoxFuture, Observation};
    use serde_json::Map;

    fn account() -> Account {
        Account {
            id: "1".into(),
            nickname: "小琴".into(),
            avatar_url: None,
            vip: false,
        }
    }

    /// 以前登录过、网易云说没有账号 / 回“要登录”才算过期；从没登录过的、查不到（网络、风控）的不算。
    #[test]
    fn expiry_needs_a_stored_login_that_netease_refuses() {
        assert!(expired_account(true, &Ok(None)));
        assert!(!expired_account(false, &Ok(None)), "从没登录过");
        assert!(!expired_account(true, &Ok(Some(account()))));
        assert!(!expired_account(
            true,
            &Err(MusicError::Network("超时".into()))
        ));
        assert!(lost_login(true, &MusicError::NeedLogin));
        assert!(!lost_login(false, &MusicError::NeedLogin));
        assert!(!lost_login(true, &MusicError::RateLimited("-460".into())));
    }

    struct DirHost {
        dir: PathBuf,
        pushes: Mutex<Vec<Push>>,
    }

    impl Host for DirHost {
        fn plugin(&self) -> &str {
            "music"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            json!({})
        }
        fn data_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn spawn(&self, _: BoxFuture<'static, ()>) {}
        fn process_ids(&self, _: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _: &str, _: Observation) {}
        fn game_claim(&self, _: &str, _: bool) {}
        fn game_event(&self, _: &str, _: &str, _: &str, _: Map<String, Value>, _: Option<f64>) {}
        fn push(&self, push: Push) -> Result<PushOutcome, String> {
            self.pushes.lock().unwrap().push(push);
            Ok(PushOutcome::Posted)
        }
    }

    /// 过期：一张卡片（登录过期、按钮的页面），同一次过期再推还是同一个 key，Kotone 重启以后也是；登好了原地变成
    /// “已登录”（带昵称），之后不再推；没过期时登好了什么都不推。
    #[test]
    fn one_card_per_expiry_that_turns_into_logged_in() {
        let temp = tempfile::tempdir().unwrap();
        let host = Arc::new(DirHost {
            dir: temp.path().join("plugins").join("music"),
            pushes: Mutex::new(Vec::new()),
        });
        let card = LoginPush::new(host.clone());
        assert!(!card.pending());
        assert!(card.logged_in(&account()).is_none(), "没过期：不推");
        card.expired();
        card.expired();
        assert!(card.pending());
        // Kotone 重启了，凭据还是过期的：读回同一个 key
        let again = LoginPush::new(host.clone());
        assert!(again.pending());
        again.expired();
        let pushes = host.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 3);
        assert!(pushes
            .iter()
            .all(|p| p.key == pushes[0].key && p.trigger == PUSH_LOGIN));
        assert!(pushes[0].key.starts_with("login:"));
        let first = pushes[0].card.as_ref().unwrap();
        assert_eq!(
            (first.title.as_str(), first.uri.as_str()),
            (TITLE_LOGIN, PAGE_LOGIN)
        );
        assert_eq!(first.data, json!({"stage": "expired"}));
        assert!(pushes[0].note.is_empty());

        again.logged_in(&account());
        let done = host.pushes.lock().unwrap().last().cloned().unwrap();
        assert_eq!(done.key, pushes[0].key, "原地变成已登录");
        assert_eq!(
            done.card.unwrap().data,
            json!({"stage": "done", "nickname": "小琴"})
        );
        assert!(!again.pending());
        assert!(
            !LoginPush::new(host.clone()).pending(),
            "登好了：记着的 key 删了"
        );
        assert!(again.logged_in(&account()).is_none());
    }
}
