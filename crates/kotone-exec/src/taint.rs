//! “读过外面的东西就先问一句”（ADR 030 第 3 节，照 Meta “Agents Rule of Two”）。
//!
//! 一个子代理一份。读不可信的内容、碰私密数据或系统、改东西或往外发——一次任务里最多占两样；
//! 命令行一个就占了后两样，所以读过外面的东西之后，下一次执行命令或写文件前先问玩家一句。
//! 玩家同意后，这一轮剩下的步骤不再因为这条规则问；玩家追加任务（新的一轮）时重新算：
//! 读过的东西还在上下文里，所以“读过”不清掉，只清掉“同意过”。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Taint {
    /// 第一次读到外面的东西时的说明（例如“读了 Kotone 文件区以外的文件 D:\\x.txt”）。
    reason: Option<String>,
    /// 这一轮里玩家已经同意过。
    approved: bool,
}

impl Taint {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读到了外面的东西：不可信的工具结果（屏幕、聊天、网页……）、Kotone 文件区以外的文件、
    /// 读文件内容或下载网页的命令。只记第一次的说明。
    pub fn mark(&mut self, why: &str) {
        if self.reason.is_none() {
            self.reason = Some(why.to_string());
        }
    }

    pub fn is_tainted(&self) -> bool {
        self.reason.is_some()
    }

    /// 第一次读到外面的东西时的说明。
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// 读过、并且这一轮玩家还没同意过：执行命令、写文件前要先问。
    pub fn needs_approval(&self) -> bool {
        self.is_tainted() && !self.approved
    }

    /// 玩家同意了一次询问：这一轮剩下的步骤不再因为读过外面的东西而问。
    pub fn approve(&mut self) {
        self.approved = true;
    }

    /// 玩家追加了任务（子代理开始新的一轮）：重新算。
    pub fn new_round(&mut self) {
        self.approved = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_lasts_one_round_and_the_taint_stays() {
        let mut t = Taint::new();
        assert!(!t.needs_approval());
        t.mark("读了网页");
        t.mark("读了别的");
        assert_eq!(t.reason(), Some("读了网页"));
        assert!(t.needs_approval());
        t.approve();
        assert!(!t.needs_approval());
        t.new_round();
        assert!(t.needs_approval());
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Taint>(&json).unwrap(), t);
    }
}
