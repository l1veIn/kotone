//! 英雄联盟插件的“读客户端数据”部分（ADR 035，数据开关 `client`）：这局有谁（隐藏了身份的人、电脑也列出来）、
//! 他们最近的战绩、对面谁交了闪。只数数，不评价人（ADR 043 D2）。
//!
//! 两个数据来源都只在本机、都只读：
//! - **LCU**（客户端本地接口）：端口和令牌在 `LeagueClientUx.exe` 的命令行里（[`auth`]），
//!   给对局阶段、这局 10 个人、战绩、段位、中文游戏资料（[`gamedata`]）；
//! - **Live Client Data**（局内接口，`127.0.0.1:2999`，Riot 官方）：英雄、召唤师技能、KDA、装备、
//!   游戏时间（[`live`]）。国服实测没有任何冷却字段，所以召唤师技能是玩家说了才记（[`spells`]）。
//!
//! 照 LeagueAkari（MIT，`dev` 分支）：读命令行、接口路径、模式急速；战绩的统计是自己写的（[`scout`]）。
//! 除了 [`http`] 里的网络请求，其余都是纯函数。
//!
//! [`assets`]：页面要的图（素材层，ADR 039 D6）：磁盘缓存 → 客户端 → CommunityDragon，只放行三个前缀下的图片。
//! [`abilities`]：英雄技能的名字和图标路径（英雄页加点那块）：客户端 → 磁盘缓存 → CommunityDragon 的英雄资料。

pub mod abilities;
pub mod assets;
pub mod auth;
pub mod events;
pub mod gamedata;
pub mod history;
pub mod http;
pub mod live;
pub mod scout;
pub mod spells;
pub mod text;

pub use auth::{parse_command_line, parse_port_token, LcuAuth};
pub use gamedata::GameData;
pub use http::{Lcu, Live, LolError};

/// 客户端进程名（端口和令牌在它的命令行里）。
pub const CLIENT_PROCESS: &str = "LeagueClientUx.exe";
/// 对局进程名。
pub const GAME_PROCESS: &str = "League of Legends.exe";

/// 对局阶段（`/lol-gameflow/v1/gameflow-phase`）里算“在对局里”的几个。
pub fn in_game_phase(phase: &str) -> bool {
    matches!(phase, "GameStart" | "InProgress" | "Reconnect")
}

/// 几分几秒（游戏时间，秒 → `14:32`）。
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats_game_time() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(872.4), "14:32");
        assert_eq!(clock(3605.0), "60:05");
        assert_eq!(clock(-3.0), "0:00");
    }

    #[test]
    fn in_game_phases() {
        assert!(in_game_phase("InProgress"));
        assert!(!in_game_phase("ChampSelect"));
        assert!(!in_game_phase("EndOfGame"));
    }
}
