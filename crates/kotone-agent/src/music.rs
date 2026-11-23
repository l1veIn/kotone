//! 放歌（ADR 034）：主会话的 `music` 工具，一次调用就出声，不交给后台。
//! 播放器和网易云接口在 `kotone-music`；这里只有工具的名字、说明和参数，由桌面端执行。

use serde_json::json;

use crate::catalog::{Source, ToolEntry};

pub const TOOL_MUSIC: &str = "music";

pub fn entries() -> Vec<ToolEntry> {
    vec![ToolEntry {
        name: TOOL_MUSIC.into(),
        description: "放歌（网易云音乐，Kotone 自己放，马上出声）。play：带 query 搜歌放（“晴天”“周杰伦的歌”），\
或带 from 放 daily（每日推荐）/ liked（我喜欢的音乐）/ fm（私人 FM）/ playlist（再带 playlist 写歌单名）；\
pause / resume / next / previous / stop；volume（value 0–100）；now 现在放的是什么；like 红心现在这首。\
结果里有正在放哪首、跳过了哪些（要 VIP、没版权）和原因：照实用一句话告诉玩家，例如“原唱要 VIP，先放了 xx 的版本”。\
need_login：玩家还没登录网易云（只要登一次），请玩家在 Kotone 的音乐页或聊天页点“登录网易云音乐”，\
在弹出的网易云页面里用网易云音乐 App 扫码，登好了再放。"
            .into(),
        parameters: json!({"type": "object", "additionalProperties": false, "required": ["action"], "properties": {
            "action": {"enum": ["play", "pause", "resume", "next", "previous", "stop", "volume", "now", "like"]},
            "query": {"type": "string", "maxLength": 100, "description": "play：要搜的歌名 / 歌手"},
            "from": {"enum": ["daily", "liked", "fm", "playlist"], "description": "play：不搜歌，放这里的歌"},
            "playlist": {"type": "string", "maxLength": 100, "description": "from = playlist 时：歌单名（玩家自己的或收藏的）"},
            "value": {"type": "integer", "minimum": 0, "maximum": 100, "description": "volume：音量 0–100"}
        }}),
        side_effects: vec!["plays-media".into()],
        group: "game",
        source: Source::Builtin,
        // 歌名、歌手、歌单名是网易云上别人写的
        untrusted: true,
    }]
}
