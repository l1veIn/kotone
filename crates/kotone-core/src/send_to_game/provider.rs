//! 游戏插件提供的发法（ADR 041 D3，第 2 期）。
//!
//! 插件在 `plugin.toml` 的 `[[send_to_game]]` 里登记“前台是这些窗口时交给我”（[`SendToGameSpec`]），实现
//! `Plugin::send_to_game`；入口（[`super::GameChat`]）照前台挑：前台（语音输入：开始收音的那个窗口）是登记的窗口
//! 就交给插件——不按键、不拿输入锁、不等修饰键、不切窗口——否则照旧是本体的通用发法。英雄联盟登记的是客户端
//! （选英雄、房间里经客户端的聊天接口发），不登记游戏本身：对局里的规矩（聊天键、频道、时序）本来就在游戏配置里。
//!
//! 插件接手以后的结果就是最终结果：失败了也不退回按键（ADR 032、原则 4：客户端没回话时话可能已经发出去了，再按一遍
//! 队友就收到两遍）。只有插件或它要的开关关着时才不交给它。
//!
//! 类型在 core（入口在这里，core 不依赖插件契约，原则 13），`kotone-plugin-api` 重新导出；桌面端把内置插件包成
//! [`ChatProvider`]。第三方插件没有这个能力（D7）：宿主只从内置插件表里建发法。

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::inject::CancelToken;

/// 清单里的一项 `[[send_to_game]]`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct SendToGameSpec {
    /// 给哪个游戏（游戏配置 id），要在插件的 `games` 里。
    pub game: String,
    /// 前台是这些程序的窗口时交给插件（进程名，不分大小写）。第 2 期不能是游戏配置里的进程：
    /// 还没有“按键的发法”，对局里照旧是本体的通用发法。
    pub windows: Vec<String>,
    /// 要哪些数据开关开着；有一个关着，就当没有这个发法。
    #[serde(default)]
    pub needs: Vec<String>,
}

impl SendToGameSpec {
    /// 这个程序的窗口是不是登记过的（不分大小写）。
    pub fn covers(&self, process: &str) -> bool {
        self.windows.iter().any(|w| w.eq_ignore_ascii_case(process))
    }
}

/// 登记的窗口的写法：`程序名.exe`，不带路径，最多 64 字。
pub fn valid_window(name: &str) -> bool {
    let name = name.trim();
    name.len() > 4
        && name.chars().count() <= 64
        && name.to_ascii_lowercase().ends_with(".exe")
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
}

/// 宿主交给插件的一次发话。宿主已经整理好，插件不用再查。
#[derive(Debug, Clone)]
pub struct ChatSend {
    pub game: String,
    /// 1–10 行、每行 1–200 字、没有空行（[`super::split_lines`]）
    pub lines: Vec<String>,
    /// `team` / `all`：宿主已经按“缺省 = 游戏配置的默认频道”“语音输入记的频道属于别的游戏 → 默认频道”算好
    pub channel: String,
    /// 这之前一定返回
    pub deadline: Instant,
    pub cancel: CancelToken,
}

/// 发了（至少一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSent {
    /// 实际发到的频道 id（`team` / `all`）
    pub channel: String,
    /// 给人看的：“选英雄的聊天”“房间的聊天”
    pub channel_name: String,
    /// 发了几行（几行连成一条发的，发成了就是全部行数）
    pub sent_lines: usize,
    /// 只发了一部分时：为什么
    pub stopped: Option<ChatStop>,
}

/// 没发（或没发完）的原因。`code` 给记录和外部 Agent；`message` 给玩家和她看：怎么了、下一步怎么办（原则 21）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStop {
    pub code: StopCode,
    pub message: String,
}

impl ChatStop {
    pub fn new(code: StopCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ChatStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// 原因的种类：封闭的一组，外部 Agent 认这些值（`stopped.code`，单向门，ADR 041 第 2 期）。插件的具体情况写在
/// `message` 里。名字不带哪个游戏：`client` 指“游戏客户端”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCode {
    /// 现在这里没有能发的聊天（不在房间 / 选英雄里、聊天还没连上、在对局里、在结算）
    NoChatHere,
    /// 这里没有要的频道（和第 1 期同一个值）
    NoSuchChannel,
    /// 连不上游戏客户端
    ClientUnavailable,
    /// 客户端回了错误，没发
    ClientRejected,
    /// 客户端在期限内没回话：可能已经发出去了
    ClientNoReply,
    /// 来不及在截止时间前发（和第 1 期同一个值）
    TimeUp,
    /// 取消了（和第 1 期同一个值）
    Cancelled,
}

impl StopCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoChatHere => "noChatHere",
            Self::NoSuchChannel => "noSuchChannel",
            Self::ClientUnavailable => "clientUnavailable",
            Self::ClientRejected => "clientRejected",
            Self::ClientNoReply => "clientNoReply",
            Self::TimeUp => "timeUp",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 入口看到的发法（桌面端把内置插件包成它）。阻塞执行：入口本来就在阻塞线程里跑。
pub trait ChatProvider: Send + Sync {
    /// 哪个插件（结果里的 `plugin`）。
    fn plugin(&self) -> &str;
    fn spec(&self) -> &SendToGameSpec;
    /// None = 开着；Some(为什么关着)，例如“英雄联盟插件的「在客户端里发聊天」关着”。每次发的时候现看设置。
    fn off(&self) -> Option<String>;
    /// 截止时间前一定返回；`Ok` 至少发了一行。
    fn send(&self, request: ChatSend) -> Result<ChatSent, ChatStop>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_matched_by_process_name_and_written_as_exe_names() {
        let spec = SendToGameSpec {
            game: "lol".into(),
            windows: vec!["LeagueClientUx.exe".into()],
            needs: vec![],
        };
        assert!(spec.covers("leagueclientux.exe"));
        assert!(!spec.covers("League of Legends.exe"));
        assert!(valid_window("LeagueClientUx.exe"));
        assert!(valid_window("League of Legends.exe"));
        for bad in [
            "",
            ".exe",
            "LeagueClientUx",
            "C:\\Riot\\LeagueClientUx.exe",
            "a/b.exe",
            "*.exe",
        ] {
            assert!(!valid_window(bad), "{bad}");
        }
        assert!(!valid_window(&format!("{}.exe", "a".repeat(61))));
    }

    #[test]
    fn stop_codes_keep_their_names() {
        // 单向门：外部 Agent 认这些值
        let all = [
            (StopCode::NoChatHere, "noChatHere"),
            (StopCode::NoSuchChannel, "noSuchChannel"),
            (StopCode::ClientUnavailable, "clientUnavailable"),
            (StopCode::ClientRejected, "clientRejected"),
            (StopCode::ClientNoReply, "clientNoReply"),
            (StopCode::TimeUp, "timeUp"),
            (StopCode::Cancelled, "cancelled"),
        ];
        for (code, name) in all {
            assert_eq!(code.as_str(), name);
        }
        assert_eq!(
            ChatStop::new(StopCode::NoChatHere, "进房间以后再发").to_string(),
            "进房间以后再发"
        );
    }
}
